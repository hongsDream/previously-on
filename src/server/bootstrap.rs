use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    change_status, continuation_state_name, empty_contract_evaluation, fact_kind_name,
    fact_lifecycle, freshness_name, iso, server_setup_paths, short_sha, task_lifecycle,
    temporal_freshness, temporal_status_name, ApiError, ApiResult, AppState,
    UI_HISTORY_CLASSIFICATION, UI_HISTORY_INSTRUCTION_POLICY,
};
use crate::contracts::{ContractEvaluationV1, RegressionCandidateV1, RegressionContractV1};
use crate::domain::{
    ContinuationReasonV1, ContinuationStateV1, CoverageStatus, EventKind, FactKind, FactLifecycle,
    Freshness, TemporalStatusV1, TestStatus,
};
use crate::redaction::{redact_excerpt, redact_value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct BootstrapResponseV1 {
    trust: BootstrapTrustV1,
    repository: BootstrapRepositoryV1,
    tasks: Vec<BootstrapTaskV1>,
    checkpoints: Vec<BootstrapCheckpointV1>,
    facts: Vec<BootstrapFactV1>,
    evidence: Vec<BootstrapEvidenceV1>,
    sessions: Vec<BootstrapSessionV1>,
    contracts: Vec<RegressionContractV1>,
    contract_candidates: Vec<RegressionCandidateV1>,
    contract_evaluation: Option<ContractEvaluationV1>,
    contract_evaluations: Vec<ContractEvaluationV1>,
    task_grouping_operations: Vec<crate::domain::TaskGroupingOperationV1>,
    graph_summary: crate::graph::RelationshipGraphSummaryV1,
    ai_refresh_capability: crate::ai_refresh::AiRefreshCapabilityV1,
    fact_refresh_operations: Vec<crate::domain::AiFactRefreshOperationV1>,
    agents: Vec<crate::domain::AgentV1>,
    context_packs: BTreeMap<String, crate::domain::ContextPackV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootstrapTrustV1 {
    classification: String,
    instruction_policy: String,
    source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootstrapRepositoryV1 {
    name: String,
    path: String,
    branch: String,
    connected: bool,
    state: String,
    capture_health: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootstrapTaskV1 {
    id: String,
    repository_id: String,
    title: String,
    title_suggestion: Option<crate::grouping::TaskTitleSuggestionV1>,
    status: String,
    updated_at: String,
    checkpoint_ids: Vec<String>,
    goal: String,
    decisions: BootstrapDecisionCountsV1,
    open_items: BootstrapOpenItemCountsV1,
    files: Vec<BootstrapDirectoryCountV1>,
    tests: BootstrapTestCountsV1,
    rollover: Option<BootstrapRolloverV1>,
    codebase: BootstrapCodebaseV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootstrapDecisionCountsV1 {
    confirmed: usize,
    proposed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootstrapOpenItemCountsV1 {
    risks: usize,
    questions: usize,
    actions: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootstrapDirectoryCountV1 {
    path: String,
    count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootstrapTestCountsV1 {
    passing: usize,
    failing: usize,
    skipped: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootstrapRolloverV1 {
    operation_id: Option<Value>,
    status: Option<Value>,
    source_session_id: Option<Value>,
    new_thread_id: Option<Value>,
    new_turn_id: Option<Value>,
    started_at: Option<Value>,
    message: Option<Value>,
    warnings: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootstrapCodebaseV1 {
    repository_name: String,
    registered_root: String,
    worktree_root: String,
    branch: String,
    baseline_sha: Option<String>,
    current_sha: Option<String>,
    status: String,
    source_thread_ids: Vec<String>,
    session_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootstrapCheckpointV1 {
    id: String,
    sequence: usize,
    session_title: String,
    captured_at: String,
    branch: String,
    sha: String,
    files_changed: usize,
    additions: u64,
    deletions: u64,
    tests_passed: usize,
    tests_failed: usize,
    coverage: usize,
    coverage_delta: i64,
    freshness: String,
    state: String,
    source_thread_id: Option<String>,
    last_activity_at: Option<String>,
    turn_count: Option<u32>,
    compaction_count: Option<u32>,
    context_usage: Option<BootstrapContextUsageV1>,
    continuation_state: Option<String>,
    continuation_advice: Option<BootstrapContinuationAdviceV1>,
    temporal_revalidation: BootstrapTemporalRevalidationV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootstrapContextUsageV1 {
    total_tokens: u64,
    model_context_window: u64,
    observed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootstrapContinuationAdviceV1 {
    action: String,
    reasons: Vec<ContinuationReasonV1>,
    task_id: String,
    task_title: String,
    last_activity_at: Option<String>,
    compaction_count: u32,
    context_usage: Option<BootstrapContextUsageV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootstrapTemporalRevalidationV1 {
    status: String,
    baseline_sha: Option<String>,
    current_sha: Option<String>,
    changes: Vec<BootstrapRelatedChangeV1>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootstrapRelatedChangeV1 {
    path: String,
    previous_path: Option<String>,
    status: String,
    additions: Option<u64>,
    deletions: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootstrapFactV1 {
    id: String,
    task_id: String,
    kind: String,
    text: String,
    status: String,
    confirmed_at: Option<String>,
    updated_at: String,
    evidence_ids: Vec<String>,
    selection_reason: Option<String>,
    related_files: Vec<String>,
    deprecated_after_commit: Option<String>,
    mixed_provenance: bool,
    provenance_session_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootstrapEvidenceV1 {
    id: String,
    checkpoint_id: String,
    fact_id: String,
    session_id: String,
    session_label: String,
    turn_label: String,
    captured_at: String,
    source: String,
    excerpt: String,
    code: String,
    freshness: String,
    selection_reason: String,
    excluded_session: bool,
    related_files: Vec<BootstrapEvidenceFileV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootstrapEvidenceFileV1 {
    path: String,
    additions: u64,
    deletions: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootstrapSessionV1 {
    id: String,
    task_id: String,
    source_thread_id: Option<String>,
    started_at: String,
    last_activity_at: Option<String>,
    turn_count: u32,
    compaction_count: u32,
    context_usage: Option<BootstrapContextUsageV1>,
    continuation_state: String,
    excluded: bool,
}

pub(super) async fn build_bootstrap(
    state: &AppState,
    repository_id: Option<&str>,
) -> ApiResult<BootstrapResponseV1> {
    let repositories = state
        .store
        .list_repositories()
        .map_err(ApiError::internal)?;
    let projects = super::registered_projects(state)?;
    let configured_repository = match (repository_id, projects.len()) {
        (Some(repository_id), _) => Some(super::registered_repository_by_id(state, repository_id)?),
        (None, 0) => None,
        (None, 1) => Some(super::registered_repository_by_id(
            state,
            &projects[0].repository_id,
        )?),
        (None, _) => {
            return Err(ApiError::conflict(
                "repositoryId is required when multiple repositories are registered",
            ))
        }
    };
    let stored_repository = configured_repository.as_ref().and_then(|registered| {
        repositories
            .iter()
            .find(|repository| repository.id == registered.id)
    });
    let repository = configured_repository
        .as_ref()
        .map(|registered| stored_repository.unwrap_or(registered));
    let contracts = repository
        .filter(|repository| std::path::Path::new(&repository.path).exists())
        .map(|repository| crate::contracts::load_contracts(&repository.path))
        .transpose()
        .map_err(ApiError::internal)?
        .unwrap_or_default();
    let contract_candidates = state
        .store
        .list_regression_candidates(repository.map(|repository| repository.id.as_str()))
        .map_err(ApiError::internal)?;
    let contract_evaluations = state
        .store
        .list_contract_evaluations(repository.map(|repository| repository.id.as_str()))
        .map_err(ApiError::internal)?;
    let contract_evaluation = contract_evaluations
        .first()
        .cloned()
        .or_else(|| repository.map(|item| empty_contract_evaluation(Some(&item.id))));
    let task_grouping_operations = state
        .store
        .list_task_grouping_operations(repository.map(|item| item.id.as_str()))
        .map_err(ApiError::internal)?;
    let graph_summary = stored_repository
        .map(|repository| {
            crate::graph::derive_relationship_graph(&state.store, &repository.id, None, &contracts)
                .map(|graph| crate::graph::compact_summary(&graph))
        })
        .transpose()
        .map_err(ApiError::internal)?
        .unwrap_or_default();
    let fact_refresh_operations = state
        .store
        .list_ai_fact_refresh_operations(repository.map(|item| item.id.as_str()))
        .map_err(ApiError::internal)?;
    let agents = state
        .store
        .list_agents(repository.map(|item| item.id.as_str()))
        .map_err(ApiError::internal)?;
    let ai_refresh_capability = if let Some(repository) = repository {
        crate::ai_refresh::inspect_capability(
            &server_setup_paths(state)?,
            std::path::Path::new(&repository.path),
        )
        .await
    } else {
        crate::ai_refresh::AiRefreshCapabilityV1 {
            status: crate::ai_refresh::AiRefreshCapabilityStatusV1::Blocked,
            profile_name: crate::ai_refresh::AI_REFRESH_PROFILE.to_string(),
            reason: Some("no registered repository".to_string()),
            checked_at: Utc::now(),
        }
    };
    let task_hits = state
        .store
        .search_tasks("", 100)
        .map_err(ApiError::internal)?;
    let tasks = task_hits
        .into_iter()
        .filter(|hit| {
            repository
                .map(|repo| repo.id == hit.task.repository_id)
                .unwrap_or(true)
        })
        .map(|hit| hit.task)
        .collect::<Vec<_>>();

    let mut checkpoint_values = Vec::new();
    let mut fact_values = Vec::new();
    let mut evidence_values = Vec::new();
    let mut session_values = Vec::new();
    let mut task_values = Vec::new();
    let mut context_packs = BTreeMap::new();
    let mut capture_degraded =
        repository.is_some_and(|item| !std::path::Path::new(&item.path).is_dir());

    for task in &tasks {
        let task_events = state
            .store
            .list_task_events(&task.repository_id, &task.id)
            .map_err(ApiError::internal)?;
        let mut excluded_session_states = BTreeMap::<String, bool>::new();
        let mut deprecated_facts = BTreeMap::<String, String>::new();
        let mut rollover = None;
        for event in &task_events {
            match event.kind {
                EventKind::SessionExcluded => {
                    if let Some(session_id) =
                        event.payload.get("session_id").and_then(Value::as_str)
                    {
                        excluded_session_states.insert(
                            session_id.to_string(),
                            event
                                .payload
                                .get("excluded")
                                .and_then(Value::as_bool)
                                .unwrap_or(true),
                        );
                    }
                }
                EventKind::FactDeprecated => {
                    if let (Some(fact_id), Some(commit)) = (
                        event.payload.get("fact_id").and_then(Value::as_str),
                        event
                            .payload
                            .get("deprecated_after_commit")
                            .and_then(Value::as_str),
                    ) {
                        if commit.is_empty() {
                            deprecated_facts.remove(fact_id);
                        } else {
                            deprecated_facts.insert(fact_id.to_string(), commit.to_string());
                        }
                    }
                }
                EventKind::ContinuationStarted => {
                    rollover = Some(BootstrapRolloverV1 {
                        operation_id: event.payload.get("operation_id").cloned(),
                        status: event.payload.get("status").cloned(),
                        source_session_id: event.payload.get("source_session_id").cloned(),
                        new_thread_id: event.payload.get("new_thread_id").cloned(),
                        new_turn_id: event.payload.get("new_turn_id").cloned(),
                        started_at: event.payload.get("started_at").cloned(),
                        message: event.payload.get("message").cloned(),
                        warnings: event.payload.get("warnings").cloned(),
                    });
                }
                _ => {}
            }
        }
        let sessions = state
            .store
            .get_task_timeline(&task.id)
            .map_err(ApiError::internal)?
            .map(|timeline| timeline.sessions)
            .unwrap_or_default();
        let checkpoints = state
            .store
            .list_checkpoints(&task.id)
            .map_err(ApiError::internal)?;
        let mut facts = state
            .store
            .list_facts(&task.id)
            .map_err(ApiError::internal)?;
        let evidence = state
            .store
            .list_evidence(&task.id)
            .map_err(ApiError::internal)?;
        let changes = state
            .store
            .list_file_changes(&task.id)
            .map_err(ApiError::internal)?;
        let tests = state
            .store
            .list_test_results(&task.id)
            .map_err(ApiError::internal)?;

        let registered_repository_path = repository
            .filter(|item| item.id == task.repository_id)
            .map(|item| item.path.as_str())
            .filter(|path| !path.is_empty())
            .unwrap_or(&task.repository_id);
        let repository_path = checkpoints
            .last()
            .map(|checkpoint| checkpoint.git_after.root.as_str())
            .filter(|path| !path.is_empty())
            .unwrap_or(registered_repository_path);
        let task_temporal = crate::git::revalidate_task(
            repository_path,
            checkpoints.last().map(|checkpoint| &checkpoint.git_after),
            &changes,
        )
        .unwrap_or_else(|error| crate::domain::TemporalRevalidationV1 {
            schema_version: crate::domain::SCHEMA_VERSION_V1,
            status: TemporalStatusV1::Degraded,
            baseline_head: checkpoints
                .last()
                .and_then(|checkpoint| checkpoint.git_after.head.clone()),
            current_head: None,
            merge_base: None,
            related_changes: Vec::new(),
            checked_paths: Vec::new(),
            warnings: vec![redact_excerpt(&error.to_string())],
        });
        for fact in &mut facts {
            fact.freshness = crate::mcp::fact_freshness(
                registered_repository_path,
                fact,
                &evidence,
                &checkpoints,
                &changes,
            );
            if let (Some(commit), Some(current_head)) = (
                deprecated_facts.get(&fact.id),
                task_temporal.current_head.as_deref(),
            ) {
                if crate::git::is_ancestor(repository_path, commit, current_head).unwrap_or(false) {
                    fact.freshness = Freshness::Stale;
                }
            }
        }

        let mut fact_selection_reasons = BTreeMap::<String, String>::new();
        if let Ok(pack) =
            crate::mcp::StoreMcpBackend::from_store(state.store.clone(), task.repository_id.clone())
                .verified_context_pack(&task.id, Some(crate::context_pack::DEFAULT_TOKEN_BUDGET))
        {
            for fact in pack.facts.iter().chain(pack.unresolved_items.iter()) {
                fact_selection_reasons.insert(fact.id.clone(), fact.selection_reason.clone());
            }
            context_packs.insert(task.id.clone(), pack);
        }

        let checkpoint_ids = checkpoints
            .iter()
            .map(|checkpoint| checkpoint.id.clone())
            .collect::<Vec<_>>();
        let confirmed_decisions = facts
            .iter()
            .filter(|fact| {
                fact.kind == FactKind::Decision
                    && matches!(
                        fact.lifecycle,
                        FactLifecycle::Confirmed | FactLifecycle::Pinned
                    )
            })
            .count();
        let proposed_decisions = facts
            .iter()
            .filter(|fact| {
                fact.kind == FactKind::Decision && fact.lifecycle == FactLifecycle::Candidate
            })
            .count();
        let open_count = facts
            .iter()
            .filter(|fact| {
                fact.kind == FactKind::OpenItem
                    && !matches!(
                        fact.lifecycle,
                        FactLifecycle::Invalid | FactLifecycle::Superseded
                    )
            })
            .count();
        let mut directories = BTreeMap::<String, usize>::new();
        for change in &changes {
            let directory = change
                .path
                .rsplit_once('/')
                .map(|(parent, _)| format!("{parent}/"))
                .unwrap_or_else(|| change.path.clone());
            *directories.entry(directory).or_default() += 1;
        }
        let passing = tests
            .iter()
            .filter(|test| test.status == TestStatus::Passed)
            .count();
        let failing = tests
            .iter()
            .filter(|test| test.status == TestStatus::Failed)
            .count();
        let skipped = tests
            .iter()
            .filter(|test| test.status == TestStatus::Skipped)
            .count();
        let repository_name = repository
            .filter(|item| item.id == task.repository_id)
            .and_then(|item| std::path::Path::new(&item.path).file_name())
            .and_then(|value| value.to_str())
            .unwrap_or(&task.repository_id);
        let source_thread_ids = sessions
            .iter()
            .filter_map(|session| session.source_thread_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let task_branch = checkpoints
            .last()
            .and_then(|checkpoint| checkpoint.git_after.branch.clone())
            .or_else(|| task.branch.clone())
            .unwrap_or_else(|| "detached".to_string());
        let session_ids = sessions
            .iter()
            .map(|session| session.id.clone())
            .collect::<Vec<_>>();
        let title_suggestion =
            crate::grouping::task_title_suggestion(&state.store, task, &session_ids);

        task_values.push(BootstrapTaskV1 {
            id: task.id.clone(),
            repository_id: task.repository_id.clone(),
            title: task.title.clone(),
            title_suggestion,
            status: task_lifecycle(task.lifecycle).to_string(),
            updated_at: iso(task.updated_at),
            checkpoint_ids,
            goal: task.goal.clone().unwrap_or_default(),
            decisions: BootstrapDecisionCountsV1 {
                confirmed: confirmed_decisions,
                proposed: proposed_decisions,
            },
            open_items: BootstrapOpenItemCountsV1 {
                risks: 0,
                questions: open_count,
                actions: 0,
            },
            files: directories
                .into_iter()
                .map(|(path, count)| BootstrapDirectoryCountV1 { path, count })
                .collect(),
            tests: BootstrapTestCountsV1 {
                passing,
                failing,
                skipped,
            },
            rollover,
            codebase: BootstrapCodebaseV1 {
                repository_name: repository_name.to_string(),
                registered_root: registered_repository_path.to_string(),
                worktree_root: repository_path.to_string(),
                branch: task_branch,
                baseline_sha: task_temporal.baseline_head,
                current_sha: task_temporal.current_head,
                status: temporal_status_name(task_temporal.status).to_string(),
                source_thread_ids,
                session_count: sessions.len(),
            },
        });

        for session in &sessions {
            session_values.push(BootstrapSessionV1 {
                id: session.id.clone(),
                task_id: task.id.clone(),
                source_thread_id: session.source_thread_id.clone(),
                started_at: iso(session.started_at),
                last_activity_at: session.last_activity_at.map(iso),
                turn_count: session.turn_count,
                compaction_count: session.compaction_count,
                context_usage: session.context_usage.as_ref().map(bootstrap_context_usage),
                continuation_state: continuation_state_name(session.continuation_state).to_string(),
                excluded: excluded_session_states
                    .get(&session.id)
                    .copied()
                    .unwrap_or(false),
            });
        }

        for (index, checkpoint) in checkpoints.iter().enumerate() {
            capture_degraded |= checkpoint.coverage.status != CoverageStatus::Complete;
            let additions = checkpoint
                .changed_files
                .iter()
                .filter_map(|change| change.additions)
                .sum::<u64>();
            let deletions = checkpoint
                .changed_files
                .iter()
                .filter_map(|change| change.deletions)
                .sum::<u64>();
            let tests_passed = checkpoint
                .tests
                .iter()
                .filter(|test| test.status == TestStatus::Passed)
                .count();
            let tests_failed = checkpoint
                .tests
                .iter()
                .filter(|test| test.status == TestStatus::Failed)
                .count();
            let coverage_total =
                checkpoint.coverage.captured.len() + checkpoint.coverage.missing.len();
            let coverage_percent = if coverage_total == 0 {
                0
            } else {
                checkpoint.coverage.captured.len() * 100 / coverage_total
            };
            let temporal = crate::git::revalidate_task(
                if checkpoint.git_after.root.is_empty() {
                    registered_repository_path
                } else {
                    checkpoint.git_after.root.as_str()
                },
                Some(&checkpoint.git_after),
                &checkpoint.changed_files,
            )
            .unwrap_or_else(|error| crate::domain::TemporalRevalidationV1 {
                schema_version: crate::domain::SCHEMA_VERSION_V1,
                status: TemporalStatusV1::Degraded,
                baseline_head: checkpoint.git_after.head.clone(),
                current_head: None,
                merge_base: None,
                related_changes: Vec::new(),
                checked_paths: Vec::new(),
                warnings: vec![redact_excerpt(&error.to_string())],
            });
            let freshness = temporal_freshness(temporal.status);
            let session = sessions
                .iter()
                .find(|session| session.id == checkpoint.session_id);
            let context_usage = session
                .and_then(|session| session.context_usage.as_ref().map(bootstrap_context_usage));
            let continuation_advice = session.and_then(|session| {
                (session.continuation_state != ContinuationStateV1::Normal).then(|| {
                    let mut reasons = Vec::new();
                    if session.compaction_count >= crate::domain::PROVISIONAL_COMPACTION_THRESHOLD {
                        reasons.push(ContinuationReasonV1::CompactionLimit);
                    }
                    if session
                        .context_usage
                        .as_ref()
                        .and_then(crate::domain::ContextUsageV1::utilization)
                        .is_some_and(|ratio| {
                            ratio >= crate::domain::PROVISIONAL_CONTEXT_USAGE_THRESHOLD
                        })
                    {
                        reasons.push(ContinuationReasonV1::ContextUsageLimit);
                    }
                    BootstrapContinuationAdviceV1 {
                        action: "new_thread".to_string(),
                        reasons,
                        task_id: task.id.clone(),
                        task_title: task.title.clone(),
                        last_activity_at: session.last_activity_at.map(iso),
                        compaction_count: session.compaction_count,
                        context_usage: context_usage.clone(),
                    }
                })
            });
            let state_name = if facts.iter().any(|fact| {
                matches!(
                    fact.lifecycle,
                    FactLifecycle::Confirmed | FactLifecycle::Pinned
                ) && fact.evidence_ids.iter().any(|evidence_id| {
                    evidence.iter().any(|item| {
                        item.id == *evidence_id && item.session_id == checkpoint.session_id
                    })
                })
            }) {
                "confirmed"
            } else {
                "draft"
            };
            checkpoint_values.push(BootstrapCheckpointV1 {
                id: checkpoint.id.clone(),
                sequence: index + 1,
                session_title: format!("Session {}", iso(checkpoint.created_at)),
                captured_at: iso(checkpoint.created_at),
                branch: checkpoint
                    .git_after
                    .branch
                    .clone()
                    .unwrap_or_else(|| "detached".to_string()),
                sha: short_sha(checkpoint.git_after.head.as_deref()),
                files_changed: checkpoint.changed_files.len(),
                additions,
                deletions,
                tests_passed,
                tests_failed,
                coverage: coverage_percent,
                coverage_delta: 0,
                freshness: freshness_name(freshness).to_string(),
                state: state_name.to_string(),
                source_thread_id: session.and_then(|value| value.source_thread_id.clone()),
                last_activity_at: session.and_then(|value| value.last_activity_at.map(iso)),
                turn_count: session.map(|value| value.turn_count),
                compaction_count: session.map(|value| value.compaction_count),
                context_usage,
                continuation_state: session
                    .map(|value| continuation_state_name(value.continuation_state).to_string()),
                continuation_advice,
                temporal_revalidation: BootstrapTemporalRevalidationV1 {
                    status: temporal_status_name(temporal.status).to_string(),
                    baseline_sha: temporal.baseline_head,
                    current_sha: temporal.current_head,
                    changes: temporal
                        .related_changes
                        .into_iter()
                        .map(|change| BootstrapRelatedChangeV1 {
                            path: change.path,
                            previous_path: change.previous_path,
                            status: change_status(change.status).to_string(),
                            additions: change.additions,
                            deletions: change.deletions,
                        })
                        .collect(),
                    warnings: temporal.warnings,
                },
            });
        }

        for fact in &facts {
            let provenance = fact
                .evidence_ids
                .iter()
                .filter_map(|id| state.store.get_evidence(id).ok().flatten())
                .collect::<Vec<_>>();
            let provenance_session_ids = provenance
                .iter()
                .map(|item| item.session_id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let mixed_provenance = provenance.iter().any(|item| item.task_id != fact.task_id);
            let evidence_sessions = fact
                .evidence_ids
                .iter()
                .filter_map(|id| evidence.iter().find(|item| item.id == *id))
                .map(|item| item.session_id.as_str())
                .collect::<BTreeSet<_>>();
            let related_files = changes
                .iter()
                .filter(|change| evidence_sessions.contains(change.session_id.as_str()))
                .take(5)
                .map(|change| change.path.clone())
                .collect::<Vec<_>>();
            fact_values.push(BootstrapFactV1 {
                id: fact.id.clone(),
                task_id: fact.task_id.clone(),
                kind: fact_kind_name(fact.kind).to_string(),
                text: fact.content.clone(),
                status: fact_lifecycle(fact.lifecycle).to_string(),
                confirmed_at: matches!(
                    fact.lifecycle,
                    FactLifecycle::Confirmed | FactLifecycle::Pinned
                )
                .then(|| iso(fact.updated_at)),
                updated_at: iso(fact.updated_at),
                evidence_ids: fact.evidence_ids.clone(),
                selection_reason: fact_selection_reasons.get(&fact.id).cloned(),
                related_files,
                deprecated_after_commit: deprecated_facts.get(&fact.id).cloned(),
                mixed_provenance,
                provenance_session_ids,
            });
        }

        for item in &evidence {
            let fact = item
                .fact_id
                .as_deref()
                .and_then(|id| facts.iter().find(|fact| fact.id == id));
            let checkpoint = checkpoints
                .iter()
                .find(|checkpoint| checkpoint.session_id == item.session_id);
            let related = changes
                .iter()
                .filter(|change| change.session_id == item.session_id)
                .take(5)
                .map(|change| BootstrapEvidenceFileV1 {
                    path: change.path.clone(),
                    additions: change.additions.unwrap_or_default(),
                    deletions: change.deletions.unwrap_or_default(),
                })
                .collect::<Vec<_>>();
            evidence_values.push(BootstrapEvidenceV1 {
                id: item.id.clone(),
                checkpoint_id: checkpoint
                    .map(|value| value.id.clone())
                    .unwrap_or_default(),
                fact_id: item.fact_id.clone().unwrap_or_default(),
                session_id: item.session_id.clone(),
                session_label: item.session_id.clone(),
                turn_label: item
                    .turn_index
                    .map(|turn| format!("Turn {turn}"))
                    .unwrap_or_else(|| "Observed item".to_string()),
                captured_at: iso(item.created_at),
                source: item.source_id.clone(),
                excerpt: item.excerpt.clone(),
                code: item.excerpt.clone(),
                freshness: freshness_name(
                    fact.map(|value| value.freshness)
                        .unwrap_or(Freshness::Fresh),
                )
                .to_string(),
                selection_reason: item
                    .fact_id
                    .as_ref()
                    .and_then(|id| fact_selection_reasons.get(id))
                    .cloned()
                    .unwrap_or_else(|| "Verified evidence is linked to this task but is not selected in the current Context Pack.".to_string()),
                excluded_session: excluded_session_states
                    .get(&item.session_id)
                    .copied()
                    .unwrap_or(false),
                related_files: related,
            });
        }
    }

    let current_task = tasks.first();
    let repository_state = if repository.is_none() {
        "unregistered"
    } else if capture_degraded {
        "degraded"
    } else if checkpoint_values.is_empty() {
        "registered-empty"
    } else {
        "active"
    };
    let repository = BootstrapRepositoryV1 {
        name: repository
            .and_then(|repo| std::path::Path::new(&repo.path).file_name())
            .and_then(|value| value.to_str())
            .unwrap_or("No repository")
            .to_string(),
        path: repository.map(|repo| repo.path.clone()).unwrap_or_default(),
        branch: current_task
            .and_then(|task| task.branch.clone())
            .unwrap_or_else(|| "detached".to_string()),
        connected: repository.is_some(),
        state: repository_state.to_string(),
        capture_health: match repository_state {
            "unregistered" => "offline",
            "degraded" => "degraded",
            _ => "good",
        }
        .to_string(),
    };

    let response = BootstrapResponseV1 {
        trust: BootstrapTrustV1 {
            classification: UI_HISTORY_CLASSIFICATION.to_string(),
            instruction_policy: UI_HISTORY_INSTRUCTION_POLICY.to_string(),
            source: "previously_on_local_history".to_string(),
        },
        repository,
        tasks: task_values,
        checkpoints: checkpoint_values,
        facts: fact_values,
        evidence: evidence_values,
        sessions: session_values,
        contracts,
        contract_candidates,
        contract_evaluation,
        contract_evaluations,
        task_grouping_operations,
        graph_summary,
        ai_refresh_capability,
        fact_refresh_operations,
        agents,
        context_packs,
    };
    let response = serde_json::to_value(response).map_err(ApiError::internal)?;
    serde_json::from_value(redact_value(&response)).map_err(ApiError::internal)
}

fn bootstrap_context_usage(usage: &crate::domain::ContextUsageV1) -> BootstrapContextUsageV1 {
    BootstrapContextUsageV1 {
        total_tokens: usage.total_tokens,
        model_context_window: usage.model_context_window,
        observed_at: usage.observed_at.map(iso),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unregistered_fixture_round_trips_through_the_bootstrap_dto() {
        let value = fixture(include_str!("../../fixtures/bootstrap/unregistered.json"));
        let response: BootstrapResponseV1 = serde_json::from_value(value.clone()).unwrap();

        assert_eq!(response.repository.state, "unregistered");
        assert!(response.contract_evaluation.is_none());
        assert_eq!(serde_json::to_value(response).unwrap(), value);
    }

    #[test]
    fn active_fixture_round_trips_through_every_nested_bootstrap_dto() {
        let value = fixture(include_str!("../../fixtures/bootstrap/active.json"));
        let response: BootstrapResponseV1 = serde_json::from_value(value.clone()).unwrap();

        assert_eq!(response.repository.state, "active");
        assert_eq!(response.tasks.len(), 1);
        assert_eq!(response.checkpoints.len(), 1);
        assert!(response.contract_evaluation.is_some());
        assert_eq!(serde_json::to_value(response).unwrap(), value);
    }

    fn fixture(source: &str) -> Value {
        serde_json::from_str(source).unwrap()
    }
}
