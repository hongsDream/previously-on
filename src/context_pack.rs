use crate::domain::{
    ContextFactV1, ContextPackV1, CoverageV1, CurrentValidationV1, EvidenceIntegrity, EvidenceV1,
    FactKind, FactLifecycle, FactV1, FileChangeV1, TemporalRevalidationV1, TestResultV1,
    SCHEMA_VERSION_V1,
};
use crate::redaction::{redact_excerpt, redact_text};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::cmp::Reverse;
use std::collections::BTreeMap;

pub const DEFAULT_TOKEN_BUDGET: u32 = 1_200;
pub const MAX_TOKEN_BUDGET: u32 = 2_000;
pub const MAX_FACTS: usize = 5;
pub const MAX_UNRESOLVED_ITEMS: usize = 5;
pub const MAX_FILES: usize = 8;
pub const MAX_TESTS: usize = 3;

#[derive(Debug, Clone)]
pub struct ContextPackBuilder {
    repository_id: String,
    task_id: String,
    token_budget: u32,
    generated_at: Option<DateTime<Utc>>,
    temporal_revalidation: Option<TemporalRevalidationV1>,
    current_validation: Option<CurrentValidationV1>,
}

impl ContextPackBuilder {
    pub fn new(repository_id: impl Into<String>, task_id: impl Into<String>) -> Self {
        Self {
            repository_id: repository_id.into(),
            task_id: task_id.into(),
            token_budget: DEFAULT_TOKEN_BUDGET,
            generated_at: None,
            temporal_revalidation: None,
            current_validation: None,
        }
    }

    pub fn token_budget(mut self, token_budget: u32) -> Self {
        self.token_budget = token_budget.min(MAX_TOKEN_BUDGET);
        self
    }

    pub fn generated_at(mut self, generated_at: DateTime<Utc>) -> Self {
        self.generated_at = Some(generated_at);
        self
    }

    pub fn temporal_revalidation(mut self, value: TemporalRevalidationV1) -> Self {
        self.temporal_revalidation = Some(value);
        self
    }

    pub fn current_validation(mut self, value: CurrentValidationV1) -> Self {
        self.current_validation = Some(value);
        self
    }

    pub fn build(
        mut self,
        goal: Option<String>,
        mut facts: Vec<FactV1>,
        evidence: Vec<EvidenceV1>,
        mut files: Vec<FileChangeV1>,
        mut tests: Vec<TestResultV1>,
        mut coverage: CoverageV1,
    ) -> Result<ContextPackV1> {
        let budget = self.token_budget.min(MAX_TOKEN_BUDGET);
        if budget == 0 {
            bail!("context pack token budget must be greater than zero");
        }

        self.temporal_revalidation = self.temporal_revalidation.map(|mut value| {
            value.checked_paths = value
                .checked_paths
                .into_iter()
                .map(|path| redact_excerpt(&path))
                .collect();
            for change in &mut value.related_changes {
                change.path = redact_excerpt(&change.path);
                change.previous_path = change
                    .previous_path
                    .take()
                    .map(|path| redact_excerpt(&path));
            }
            value.warnings = value
                .warnings
                .into_iter()
                .map(|warning| redact_excerpt(&warning))
                .collect();
            value
        });
        self.current_validation = self.current_validation.map(|mut value| {
            value.verified_paths = value
                .verified_paths
                .into_iter()
                .map(|path| redact_excerpt(&path))
                .collect();
            value.warnings = value
                .warnings
                .into_iter()
                .map(|warning| redact_excerpt(&warning))
                .collect();
            value.minimized_against(self.temporal_revalidation.as_ref())
        });
        self.temporal_revalidation = self
            .temporal_revalidation
            .map(TemporalRevalidationV1::bounded_for_context_pack);

        let evidence_by_id = evidence
            .into_iter()
            .map(|mut item| {
                item.excerpt = redact_excerpt(&item.excerpt);
                (item.id.clone(), item)
            })
            .collect::<BTreeMap<_, _>>();
        facts.retain(|fact| {
            fact.repository_id == self.repository_id
                && fact.task_id == self.task_id
                && fact.is_pack_eligible()
                && fact.evidence_ids.iter().all(|evidence_id| {
                    evidence_by_id
                        .get(evidence_id)
                        .map(|item| {
                            item.integrity == EvidenceIntegrity::Verified
                                && item.excerpt_sha256
                                    == hex::encode(Sha256::digest(item.excerpt.as_bytes()))
                                && item.repository_id == fact.repository_id
                                && item.task_id == fact.task_id
                                && item
                                    .fact_id
                                    .as_deref()
                                    .map(|fact_id| fact_id == fact.id)
                                    .unwrap_or(true)
                        })
                        .unwrap_or(false)
                })
        });
        facts.sort_by(fact_order);

        let mut selected_facts = facts
            .iter()
            .filter(|fact| fact.kind != FactKind::OpenItem)
            .take(MAX_FACTS)
            .map(|fact| context_fact(fact, &evidence_by_id))
            .collect::<Vec<_>>();
        let mut unresolved_items = facts
            .iter()
            .filter(|fact| fact.kind == FactKind::OpenItem)
            .take(MAX_UNRESOLVED_ITEMS)
            .map(|fact| context_fact(fact, &evidence_by_id))
            .collect::<Vec<_>>();

        files.retain(|file| {
            file.repository_id == self.repository_id
                && file
                    .task_id
                    .as_deref()
                    .map(|id| id == self.task_id)
                    .unwrap_or(true)
        });
        files.sort_by(|a, b| {
            (
                attribution_rank(a.attribution),
                &a.path,
                &a.previous_path,
                status_rank(a.status),
            )
                .cmp(&(
                    attribution_rank(b.attribution),
                    &b.path,
                    &b.previous_path,
                    status_rank(b.status),
                ))
        });
        files.dedup_by(|a, b| a.path == b.path && a.previous_path == b.previous_path);
        files.truncate(MAX_FILES);

        tests.retain(|test| {
            test.repository_id == self.repository_id
                && test
                    .task_id
                    .as_deref()
                    .map(|id| id == self.task_id)
                    .unwrap_or(true)
        });
        tests.sort_by(|a, b| {
            (
                test_status_rank(a.status),
                Reverse(a.occurred_at),
                &a.name,
                &a.id,
            )
                .cmp(&(
                    test_status_rank(b.status),
                    Reverse(b.occurred_at),
                    &b.name,
                    &b.id,
                ))
        });
        tests.dedup_by(|a, b| a.id == b.id);
        tests.truncate(MAX_TESTS);

        coverage.captured.sort();
        coverage.captured.dedup();
        coverage.missing.sort();
        coverage.missing.dedup();
        coverage.warnings = coverage
            .warnings
            .into_iter()
            .map(|warning| redact_excerpt(&warning))
            .collect();
        coverage.warnings.sort();
        coverage.warnings.dedup();

        let generated_at = self.generated_at.unwrap_or_else(|| {
            facts
                .iter()
                .map(|fact| fact.updated_at)
                .chain(tests.iter().map(|test| test.occurred_at))
                .chain(evidence_by_id.values().map(|evidence| evidence.created_at))
                .max()
                .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().expect("Unix epoch"))
        });
        let mut pack = ContextPackV1 {
            schema_version: SCHEMA_VERSION_V1,
            repository_id: self.repository_id,
            task_id: self.task_id,
            generated_at,
            token_budget: budget,
            token_count: 0,
            goal: goal
                .map(|goal| redact_excerpt(goal.trim()))
                .filter(|goal| !goal.is_empty()),
            facts: std::mem::take(&mut selected_facts),
            unresolved_items: std::mem::take(&mut unresolved_items),
            files,
            tests,
            temporal_revalidation: self.temporal_revalidation,
            current_validation: self.current_validation,
            coverage,
        };

        loop {
            stabilize_token_count(&mut pack)?;
            if pack.token_count <= budget {
                break;
            }
            if pack.tests.pop().is_some()
                || pack.files.pop().is_some()
                || pack.unresolved_items.pop().is_some()
                || pack.facts.pop().is_some()
            {
                continue;
            }
            bail!(
                "required context pack metadata needs {} tokens, exceeding budget {}",
                pack.token_count,
                budget
            );
        }
        Ok(pack)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_context_pack(
    repository_id: impl Into<String>,
    task_id: impl Into<String>,
    goal: Option<String>,
    facts: Vec<FactV1>,
    evidence: Vec<EvidenceV1>,
    files: Vec<FileChangeV1>,
    tests: Vec<TestResultV1>,
    coverage: CoverageV1,
    token_budget: Option<u32>,
) -> Result<ContextPackV1> {
    let mut builder = ContextPackBuilder::new(repository_id, task_id);
    if let Some(token_budget) = token_budget {
        builder = builder.token_budget(token_budget);
    }
    builder.build(goal, facts, evidence, files, tests, coverage)
}

pub fn serialize_pack(pack: &ContextPackV1) -> Result<String> {
    serde_json::to_string(pack).context("serialize context pack")
}

pub fn serialize_mcp_envelope(pack: &ContextPackV1) -> Result<String> {
    let pack_json = serialize_pack(pack)?;
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "result": {
            "content": [{"type": "text", "text": pack_json}],
            "isError": false
        }
    }))
    .context("serialize MCP context pack envelope")
}

pub fn count_tokens(text: &str) -> Result<u32> {
    let tokenizer = tiktoken_rs::o200k_base().context("initialize o200k_base tokenizer")?;
    Ok(tokenizer.encode_with_special_tokens(text).len() as u32)
}

pub fn count_pack_tokens(pack: &ContextPackV1) -> Result<u32> {
    count_tokens(&serialize_mcp_envelope(pack)?)
}

fn stabilize_token_count(pack: &mut ContextPackV1) -> Result<()> {
    for _ in 0..4 {
        let count = count_pack_tokens(pack)?;
        if count == pack.token_count {
            return Ok(());
        }
        pack.token_count = count;
    }
    pack.token_count = count_pack_tokens(pack)?;
    Ok(())
}

fn context_fact(fact: &FactV1, evidence_by_id: &BTreeMap<String, EvidenceV1>) -> ContextFactV1 {
    let mut evidence = fact
        .evidence_ids
        .iter()
        .filter_map(|id| evidence_by_id.get(id).cloned())
        .collect::<Vec<_>>();
    evidence.sort_by(|a, b| {
        (&a.source_id, a.turn_index, a.item_index, &a.id).cmp(&(
            &b.source_id,
            b.turn_index,
            b.item_index,
            &b.id,
        ))
    });
    ContextFactV1 {
        id: fact.id.clone(),
        kind: fact.kind,
        lifecycle: fact.lifecycle,
        freshness: fact.freshness,
        content: redact_text(&fact.content),
        evidence,
        selection_reason: match fact.lifecycle {
            FactLifecycle::Pinned => "user_pinned_with_verified_evidence",
            _ => "user_confirmed_with_verified_evidence",
        }
        .to_string(),
    }
}

fn fact_order(a: &FactV1, b: &FactV1) -> std::cmp::Ordering {
    (
        lifecycle_rank(a.lifecycle),
        fact_kind_rank(a.kind),
        Reverse(a.updated_at),
        &a.id,
    )
        .cmp(&(
            lifecycle_rank(b.lifecycle),
            fact_kind_rank(b.kind),
            Reverse(b.updated_at),
            &b.id,
        ))
}

fn lifecycle_rank(lifecycle: FactLifecycle) -> u8 {
    match lifecycle {
        FactLifecycle::Pinned => 0,
        FactLifecycle::Confirmed => 1,
        FactLifecycle::Candidate => 2,
        FactLifecycle::Invalid => 3,
        FactLifecycle::Superseded => 4,
    }
}

fn fact_kind_rank(kind: FactKind) -> u8 {
    match kind {
        FactKind::Decision => 0,
        FactKind::Constraint => 1,
        FactKind::Goal => 2,
        FactKind::Progress => 3,
        FactKind::Note => 4,
        FactKind::OpenItem => 5,
    }
}

fn attribution_rank(attribution: crate::domain::ChangeAttribution) -> u8 {
    match attribution {
        crate::domain::ChangeAttribution::ModifiedBy => 0,
        crate::domain::ChangeAttribution::ObservedChangedIn => 1,
    }
}

fn status_rank(status: crate::domain::ChangeStatus) -> u8 {
    use crate::domain::ChangeStatus::*;
    match status {
        Added => 0,
        Modified => 1,
        Renamed => 2,
        Copied => 3,
        Deleted => 4,
        TypeChanged => 5,
        Unmerged => 6,
        Unknown => 7,
    }
}

fn test_status_rank(status: crate::domain::TestStatus) -> u8 {
    use crate::domain::TestStatus::*;
    match status {
        Failed => 0,
        Passed => 1,
        Skipped => 2,
        Unknown => 3,
    }
}

#[allow(dead_code)]
fn stable_json<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).context("serialize stable JSON")
}
