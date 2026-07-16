use crate::domain::{
    deterministic_id, EvidenceIntegrity, FactGroupingImpactV1, FactKind, FactLifecycle,
    SessionMoveV1, TaskGroupingActionV1, TaskGroupingOperationV1, TaskLifecycle,
    TaskLifecycleSnapshotV1, TaskV1, SCHEMA_VERSION_V1,
};
use crate::redaction::redact_excerpt;
use crate::store::Store;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskTitleSuggestionSourceV1 {
    Goal,
    Branch,
    TouchedArea,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskTitleSuggestionV1 {
    pub value: String,
    pub source: TaskTitleSuggestionSourceV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskGroupingRequestV1 {
    pub operation_id: String,
    pub action: TaskGroupingActionV1,
    pub session_ids: Vec<String>,
    pub from_task_id: String,
    #[serde(default)]
    pub target_task_id: Option<String>,
    #[serde(default)]
    pub new_task_title: Option<String>,
    #[serde(default)]
    pub new_task_goal: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskGroupingCountsV1 {
    pub sessions: usize,
    pub facts_moved: usize,
    pub facts_mixed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskGroupingPreviewV1 {
    pub operation: TaskGroupingOperationV1,
    pub affected_sessions: Vec<SessionMoveV1>,
    pub affected_facts: Vec<FactGroupingImpactV1>,
    pub counts: TaskGroupingCountsV1,
}

pub fn preview(store: &Store, request: &TaskGroupingRequestV1) -> Result<TaskGroupingPreviewV1> {
    validate_operation_id(&request.operation_id)?;
    if request.action == TaskGroupingActionV1::Undo {
        bail!("undo is only available through the operation-specific endpoint");
    }
    if request.session_ids.is_empty() {
        bail!("at least one session is required");
    }
    let session_ids = request
        .session_ids
        .iter()
        .map(|value| value.trim().to_string())
        .collect::<Vec<_>>();
    if session_ids.iter().any(String::is_empty) {
        bail!("session ids cannot be empty");
    }
    let unique = session_ids.iter().collect::<BTreeSet<_>>();
    if unique.len() != session_ids.len() {
        bail!("duplicate session ids are not allowed");
    }

    let source = store
        .get_task(&request.from_task_id)?
        .with_context(|| format!("source task not found: {}", request.from_task_id))?;
    let mut sessions = Vec::with_capacity(session_ids.len());
    for session_id in &session_ids {
        let session = store
            .get_session(session_id)?
            .with_context(|| format!("session not found: {session_id}"))?;
        if session.repository_id != source.repository_id {
            bail!("cross-repository grouping is not allowed");
        }
        if session.task_id.as_deref() != Some(source.id.as_str()) {
            bail!(
                "stale session association for {session_id}: expected {}, found {}",
                source.id,
                session.task_id.as_deref().unwrap_or("unlinked")
            );
        }
        sessions.push(session);
    }

    let (target_id, created_task) = match request.action {
        TaskGroupingActionV1::Move | TaskGroupingActionV1::Merge => {
            if request.new_task_title.is_some() || request.new_task_goal.is_some() {
                bail!("new task fields are only valid for split operations");
            }
            let target_id = request
                .target_task_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .context("move and merge require targetTaskId")?;
            if target_id == source.id {
                bail!("source and target task must differ");
            }
            let target = store
                .get_task(target_id)?
                .with_context(|| format!("target task not found: {target_id}"))?;
            if target.repository_id != source.repository_id {
                bail!("cross-repository grouping is not allowed");
            }
            if target.lifecycle == TaskLifecycle::Abandoned {
                bail!("an abandoned task cannot receive sessions");
            }
            (target.id, None)
        }
        TaskGroupingActionV1::Split => {
            if request.target_task_id.is_some() {
                bail!("split creates a new task and cannot target an existing task");
            }
            let target_id = deterministic_id(
                "task",
                &[&source.repository_id, "split", &request.operation_id],
            );
            if store.get_task(&target_id)?.is_some() {
                bail!("split target already exists for this operation id");
            }
            let goal =
                validate_optional_text(request.new_task_goal.as_deref(), 500, "new task goal")?;
            let title = match validate_optional_text(
                request.new_task_title.as_deref(),
                120,
                "new task title",
            )? {
                Some(title) => title,
                None => task_title_suggestion(store, &source, &session_ids)
                    .map(|suggestion| suggestion.value)
                    .unwrap_or_else(|| format!("Split from {}", source.title)),
            };
            let now = Utc::now();
            (
                target_id.clone(),
                Some(TaskV1 {
                    schema_version: SCHEMA_VERSION_V1,
                    id: target_id,
                    repository_id: source.repository_id.clone(),
                    title,
                    goal,
                    lifecycle: TaskLifecycle::Active,
                    branch: source.branch.clone(),
                    created_at: now,
                    updated_at: now,
                }),
            )
        }
        TaskGroupingActionV1::Undo => unreachable!(),
    };

    let mut session_moves = session_ids
        .iter()
        .map(|session_id| SessionMoveV1 {
            session_id: session_id.clone(),
            from_task_id: source.id.clone(),
            to_task_id: target_id.clone(),
        })
        .collect::<Vec<_>>();
    session_moves.sort();
    let moved = session_ids.iter().cloned().collect::<BTreeSet<_>>();
    let evidence = store.list_evidence(&source.id)?;
    let mut fact_impacts = Vec::new();
    for fact in store.list_facts(&source.id)? {
        let mut provenance_sessions = fact
            .evidence_ids
            .iter()
            .filter_map(|id| evidence.iter().find(|item| item.id == *id))
            .map(|item| item.session_id.clone())
            .collect::<BTreeSet<_>>();
        if provenance_sessions.is_empty() || provenance_sessions.is_disjoint(&moved) {
            continue;
        }
        let all_moved = provenance_sessions.iter().all(|id| moved.contains(id));
        fact_impacts.push(FactGroupingImpactV1 {
            fact_id: fact.id,
            from_task_id: source.id.clone(),
            to_task_id: all_moved.then(|| target_id.clone()),
            mixed_provenance: !all_moved,
            session_ids: std::mem::take(&mut provenance_sessions)
                .into_iter()
                .collect(),
        });
    }
    fact_impacts.sort_by(|left, right| left.fact_id.cmp(&right.fact_id));

    let mut task_lifecycle = Vec::new();
    if request.action == TaskGroupingActionV1::Merge {
        let source_session_count = store.list_sessions_for_task(&source.id)?.len();
        if source_session_count == session_moves.len() {
            task_lifecycle.push(TaskLifecycleSnapshotV1 {
                task_id: source.id.clone(),
                before: Some(source.lifecycle),
                after: Some(TaskLifecycle::Completed),
            });
        }
    }
    if created_task.is_some() {
        task_lifecycle.push(TaskLifecycleSnapshotV1 {
            task_id: target_id,
            before: None,
            after: Some(TaskLifecycle::Active),
        });
    }
    task_lifecycle.sort_by(|left, right| left.task_id.cmp(&right.task_id));
    let request_fingerprint = request_fingerprint(request, &session_ids);
    let occurred_at = created_task
        .as_ref()
        .map(|task| task.created_at)
        .unwrap_or_else(Utc::now);
    let operation = TaskGroupingOperationV1 {
        schema_version: SCHEMA_VERSION_V1,
        operation_id: request.operation_id.clone(),
        repository_id: source.repository_id,
        action: request.action,
        session_moves: session_moves.clone(),
        task_lifecycle,
        fact_impacts: fact_impacts.clone(),
        created_task,
        inverse_of: None,
        request_fingerprint,
        occurred_at,
    };
    let counts = TaskGroupingCountsV1 {
        sessions: session_moves.len(),
        facts_moved: fact_impacts
            .iter()
            .filter(|impact| !impact.mixed_provenance && impact.to_task_id.is_some())
            .count(),
        facts_mixed: fact_impacts
            .iter()
            .filter(|impact| impact.mixed_provenance)
            .count(),
    };
    Ok(TaskGroupingPreviewV1 {
        operation,
        affected_sessions: session_moves,
        affected_facts: fact_impacts,
        counts,
    })
}

pub fn inverse(operation: &TaskGroupingOperationV1) -> TaskGroupingOperationV1 {
    let mut session_moves = operation
        .session_moves
        .iter()
        .map(|item| SessionMoveV1 {
            session_id: item.session_id.clone(),
            from_task_id: item.to_task_id.clone(),
            to_task_id: item.from_task_id.clone(),
        })
        .collect::<Vec<_>>();
    session_moves.sort();
    let mut fact_impacts = operation
        .fact_impacts
        .iter()
        .map(|item| FactGroupingImpactV1 {
            fact_id: item.fact_id.clone(),
            from_task_id: item
                .to_task_id
                .clone()
                .unwrap_or_else(|| item.from_task_id.clone()),
            to_task_id: item.to_task_id.as_ref().map(|_| item.from_task_id.clone()),
            mixed_provenance: item.mixed_provenance,
            session_ids: item.session_ids.clone(),
        })
        .collect::<Vec<_>>();
    fact_impacts.sort_by(|left, right| left.fact_id.cmp(&right.fact_id));
    let mut task_lifecycle = operation
        .task_lifecycle
        .iter()
        .map(|item| TaskLifecycleSnapshotV1 {
            task_id: item.task_id.clone(),
            before: item.after,
            after: item.before,
        })
        .collect::<Vec<_>>();
    task_lifecycle.sort_by(|left, right| left.task_id.cmp(&right.task_id));
    let operation_id = deterministic_id("grouping-undo", &[&operation.operation_id]);
    let occurred_at = Utc::now();
    TaskGroupingOperationV1 {
        schema_version: SCHEMA_VERSION_V1,
        operation_id,
        repository_id: operation.repository_id.clone(),
        action: TaskGroupingActionV1::Undo,
        session_moves,
        task_lifecycle,
        fact_impacts,
        created_task: operation.created_task.clone(),
        inverse_of: Some(operation.operation_id.clone()),
        request_fingerprint: hex::encode(Sha256::digest(
            format!("undo\0{}", operation.operation_id).as_bytes(),
        )),
        occurred_at,
    }
}

pub fn suggest_task_title(store: &Store, task: &TaskV1, session_ids: &[String]) -> Option<String> {
    task_title_suggestion(store, task, session_ids).map(|suggestion| suggestion.value)
}

pub fn task_title_suggestion(
    store: &Store,
    task: &TaskV1,
    session_ids: &[String],
) -> Option<TaskTitleSuggestionV1> {
    let evidence = store.list_evidence(&task.id).unwrap_or_default();
    let verified_goal = store
        .list_facts(&task.id)
        .unwrap_or_default()
        .into_iter()
        .filter(|fact| {
            fact.kind == FactKind::Goal
                && matches!(
                    fact.lifecycle,
                    FactLifecycle::Confirmed | FactLifecycle::Pinned
                )
                && !fact.evidence_ids.is_empty()
                && fact.evidence_ids.iter().all(|evidence_id| {
                    evidence
                        .iter()
                        .find(|item| item.id == *evidence_id)
                        .is_some_and(|item| {
                            item.integrity == EvidenceIntegrity::Verified
                                && item.task_id == task.id
                                && item.excerpt_sha256
                                    == hex::encode(Sha256::digest(item.excerpt.as_bytes()))
                        })
                })
        })
        .max_by_key(|fact| (fact.updated_at, fact.id.clone()))
        .map(|fact| fact.content);
    if let Some(goal) = verified_goal
        .as_deref()
        .and_then(|goal| goal.lines().map(str::trim).find(|line| !line.is_empty()))
    {
        return Some(TaskTitleSuggestionV1 {
            value: redact_excerpt(goal).chars().take(120).collect(),
            source: TaskTitleSuggestionSourceV1::Goal,
        });
    }
    if let Some(branch) = task
        .branch
        .as_deref()
        .map(str::trim)
        .filter(|branch| !branch.is_empty() && *branch != "main" && *branch != "master")
    {
        return Some(TaskTitleSuggestionV1 {
            value: redact_excerpt(branch).chars().take(120).collect(),
            source: TaskTitleSuggestionSourceV1::Branch,
        });
    }
    let selected = session_ids.iter().collect::<BTreeSet<_>>();
    let mut areas = store
        .list_file_changes(&task.id)
        .ok()?
        .into_iter()
        .filter(|change| selected.is_empty() || selected.contains(&change.session_id))
        .map(|change| {
            change
                .path
                .split('/')
                .next()
                .unwrap_or(change.path.as_str())
                .to_string()
        })
        .filter(|area| !area.is_empty())
        .collect::<Vec<_>>();
    areas.sort();
    areas.into_iter().next().map(|area| TaskTitleSuggestionV1 {
        value: format!("Update {area}"),
        source: TaskTitleSuggestionSourceV1::TouchedArea,
    })
}

pub fn request_fingerprint(request: &TaskGroupingRequestV1, session_ids: &[String]) -> String {
    let mut session_ids = session_ids.to_vec();
    session_ids.sort();
    let normalized = serde_json::json!({
        "action": request.action,
        "sessionIds": session_ids,
        "fromTaskId": request.from_task_id.trim(),
        "targetTaskId": request.target_task_id.as_deref().map(str::trim),
        "newTaskTitle": request.new_task_title.as_deref().map(str::trim),
        "newTaskGoal": request.new_task_goal.as_deref().map(str::trim),
    });
    hex::encode(Sha256::digest(
        serde_json::to_vec(&normalized).unwrap_or_default(),
    ))
}

fn validate_operation_id(value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 120
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        bail!("operationId must be 1-120 URL-safe characters");
    }
    Ok(())
}

fn validate_optional_text(value: Option<&str>, max: usize, field: &str) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() || value.chars().count() > max {
        bail!("{field} must be between 1 and {max} characters");
    }
    Ok(Some(redact_excerpt(value)))
}
