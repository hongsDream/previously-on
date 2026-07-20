use anyhow::Result;
use serde_json::{json, Value};

use crate::contracts::ContractReadinessV1;
use crate::domain::{
    deterministic_id, CoverageStatus, EventEnvelopeV1, EventKind, GitSnapshotV1, TaskLifecycle,
    TaskV1, SCHEMA_VERSION_V1,
};
use crate::store::Store;

use super::continuation_policy::{
    claim_continuation_suggestion, rollover_advice, ProposedContinuation,
};
use super::contract_policy::{
    append_contract_evaluation_event, append_regression_candidate_event,
    evaluate_contracts_for_source, invalid_contract_evaluation, pre_tool_contract_context,
    regression_candidate_for_passing_test, stop_block_reason,
};
use super::tool_evidence::{
    append_checkpoint_event, append_explicit_fact_candidates, event_snapshot,
    normalize_tool_result, prompt_text,
};
use super::{HookAckV1, HookDeliveryStatus, ResumeCandidateMetadata};

struct PreparedIngestion {
    event: EventEnvelopeV1,
    is_first_prompt: bool,
    historical_app_import: bool,
    snapshot: Option<GitSnapshotV1>,
    snapshot_path: String,
}

struct ResolvedIngestion {
    event: EventEnvelopeV1,
    snapshot: Option<GitSnapshotV1>,
    proposed_continuation: Option<ProposedContinuation>,
    ack: HookAckV1,
    deferred_prompt: Option<EventEnvelopeV1>,
}

struct PersistedIngestion {
    event: EventEnvelopeV1,
    durable_event: EventEnvelopeV1,
    snapshot: Option<GitSnapshotV1>,
    proposed_continuation: Option<ProposedContinuation>,
    ack: HookAckV1,
    deferred_prompt: Option<EventEnvelopeV1>,
}

pub(super) fn ingest_hook_event(store: &Store, event: EventEnvelopeV1) -> Result<HookAckV1> {
    let prepared = prepare(store, event)?;
    let resolved = resolve_task(store, prepared)?;
    let persisted = persist(store, resolved)?;
    let persisted = apply_policies(store, persisted)?;
    checkpoint(store, persisted)
}

fn prepare(store: &Store, mut event: EventEnvelopeV1) -> Result<PreparedIngestion> {
    let is_first_prompt = event.kind == EventKind::UserPrompt
        && store.session_event_count(&event.session_id, EventKind::UserPrompt)? == 0;
    let historical_app_import = is_historical_app_import(&event);
    let mut snapshot = None;
    let snapshot_path = event
        .payload
        .get("repository_path")
        .and_then(Value::as_str)
        .unwrap_or(&event.repository_id)
        .to_string();
    if !historical_app_import
        && matches!(
            event.kind,
            EventKind::SessionStarted
                | EventKind::UserPrompt
                | EventKind::ToolStarted
                | EventKind::ToolFinished
                | EventKind::Checkpoint
                | EventKind::ContextCompaction
                | EventKind::SessionStopped
        )
    {
        match crate::git::capture_snapshot(&snapshot_path) {
            Ok(current) => {
                event.repository_id = current.repository_id.clone();
                if let Some(object) = event.payload.as_object_mut() {
                    object.insert(
                        "repository_path".to_string(),
                        Value::String(current.root.clone()),
                    );
                    object.insert("branch".to_string(), json!(current.branch));
                    object.insert("head".to_string(), json!(current.head));
                    object.insert("git_snapshot".to_string(), serde_json::to_value(&current)?);
                }
                event.coverage.captured.push("git_snapshot".to_string());
                snapshot = Some(current);
            }
            Err(error) => {
                event.coverage.status = CoverageStatus::Degraded;
                event.coverage.missing.push("git_snapshot".to_string());
                event
                    .coverage
                    .warnings
                    .push(crate::redaction::redact_excerpt(&error.to_string()));
            }
        }
    } else if historical_app_import {
        event.coverage.status = event.coverage.status.worst(CoverageStatus::Degraded);
        event
            .coverage
            .missing
            .push("historical_git_snapshot".to_string());
        event.coverage.warnings.push(
            "Imported App Server history has no historical Git snapshot; current repository state is observed only during later revalidation."
                .to_string(),
        );
    }

    Ok(PreparedIngestion {
        event,
        is_first_prompt,
        historical_app_import,
        snapshot,
        snapshot_path,
    })
}

fn resolve_task(store: &Store, prepared: PreparedIngestion) -> Result<ResolvedIngestion> {
    let PreparedIngestion {
        mut event,
        is_first_prompt,
        historical_app_import,
        snapshot,
        snapshot_path,
    } = prepared;

    if event.task_id.is_none() {
        event.task_id = store
            .get_session(&event.session_id)?
            .and_then(|session| session.task_id);
    }

    if event.kind == EventKind::ToolFinished {
        if let Some(resume_task_id) = resume_task_id(&event.payload) {
            match store.get_task(resume_task_id)? {
                Some(task)
                    if task.repository_id == event.repository_id
                        && task.lifecycle == TaskLifecycle::Active =>
                {
                    event.task_id = Some(task.id);
                    event.coverage.captured.push("resume_task_link".to_string());
                }
                Some(_) => {
                    event.coverage.status = CoverageStatus::Degraded;
                    event.coverage.warnings.push(
                        "resume_task target is not an active task in this repository".to_string(),
                    );
                }
                None => {
                    event.coverage.status = CoverageStatus::Degraded;
                    event
                        .coverage
                        .warnings
                        .push("resume_task target was not found".to_string());
                }
            }
        }
    }

    let proposed_continuation = if event.kind == EventKind::UserPrompt && !historical_app_import {
        rollover_advice(store, &event)?
    } else {
        None
    };
    let ack = if !historical_app_import && proposed_continuation.is_none() && is_first_prompt {
        suggestion_ack(store, &event)?
    } else {
        HookAckV1::default()
    };

    let waiting_for_resume = is_first_prompt && ack.candidate.is_some();
    let deferred_prompt = if event.task_id.is_none()
        && event.kind != EventKind::UserPrompt
        && event.kind != EventKind::SessionStarted
    {
        store
            .list_session_events(&event.repository_id, &event.session_id)?
            .into_iter()
            .find(|item| item.kind == EventKind::UserPrompt)
    } else {
        None
    };
    // Metadata may legitimately arrive before the first prompt (notably App Server token and
    // compaction notifications). It updates the session projection but must not manufacture an
    // empty task. A non-prompt event can only attach a task when a real earlier prompt exists.
    let task_bearing_event = event.kind == EventKind::UserPrompt
        || (deferred_prompt.is_some()
            && matches!(
                event.kind,
                EventKind::AssistantFinal
                    | EventKind::ToolStarted
                    | EventKind::ToolFinished
                    | EventKind::Checkpoint
                    | EventKind::ContextCompaction
                    | EventKind::SessionStopped
            ));
    let should_create_task = event.task_id.is_none()
        && event.kind != EventKind::SessionStarted
        && !waiting_for_resume
        && task_bearing_event;
    if should_create_task {
        attach_new_task(&mut event, snapshot.as_ref(), deferred_prompt.as_ref())?;
    }

    if event.kind == EventKind::ToolFinished {
        normalize_tool_result(store, &mut event, snapshot.as_ref(), &snapshot_path)?;
    }

    Ok(ResolvedIngestion {
        event,
        snapshot,
        proposed_continuation,
        ack,
        deferred_prompt,
    })
}

fn persist(store: &Store, resolved: ResolvedIngestion) -> Result<PersistedIngestion> {
    let ResolvedIngestion {
        event,
        snapshot,
        proposed_continuation,
        mut ack,
        deferred_prompt,
    } = resolved;
    let inserted = store.insert_event(&event)?;
    let duplicate = inserted == crate::store::InsertOutcome::Duplicate;
    let durable_event = if duplicate {
        store
            .list_session_events(&event.repository_id, &event.session_id)?
            .into_iter()
            .find(|stored| stored.dedupe_key == event.dedupe_key)
            .unwrap_or_else(|| event.clone())
    } else {
        event.clone()
    };
    ack.status = if duplicate {
        HookDeliveryStatus::Duplicate
    } else {
        HookDeliveryStatus::Persisted
    };

    Ok(PersistedIngestion {
        event,
        durable_event,
        snapshot,
        proposed_continuation,
        ack,
        deferred_prompt,
    })
}

fn apply_policies(store: &Store, mut persisted: PersistedIngestion) -> Result<PersistedIngestion> {
    append_explicit_fact_candidates(store, &persisted.durable_event)?;
    if let Some(proposed) = persisted.proposed_continuation.take() {
        persisted.ack.continuation_advice = claim_continuation_suggestion(
            store,
            &persisted.durable_event,
            &proposed.advice,
            &proposed.claim_generation,
        )?;
    }
    if let Some(mut prompt) = persisted.deferred_prompt.take() {
        prompt.task_id = persisted.event.task_id.clone();
        append_explicit_fact_candidates(store, &prompt)?;
    }

    match persisted.durable_event.kind {
        EventKind::ToolStarted => {
            match pre_tool_contract_context(&persisted.durable_event, persisted.snapshot.as_ref()) {
                Ok(context) => persisted.ack.contract_context = context,
                Err(error) => {
                    persisted.ack.contract_context = Some(format!(
                        "PreviouslyOn could not evaluate Regression Contracts before this edit: {}. Treat the Contract checkout as invalid until `previously contracts validate` succeeds.",
                        crate::redaction::redact_excerpt(&error.to_string())
                    ));
                }
            }
        }
        EventKind::ToolFinished => {
            if let Some(candidate) = regression_candidate_for_passing_test(
                store,
                &persisted.durable_event,
                persisted.snapshot.as_ref(),
            )? {
                append_regression_candidate_event(store, &persisted.durable_event, &candidate)?;
            }
            if let Some(evaluation) = evaluate_contracts_for_source(
                store,
                &persisted.durable_event,
                persisted.snapshot.as_ref(),
                false,
            )? {
                append_contract_evaluation_event(store, &persisted.durable_event, &evaluation)?;
            }
        }
        EventKind::SessionStopped => {
            match evaluate_contracts_for_source(
                store,
                &persisted.durable_event,
                persisted.snapshot.as_ref(),
                true,
            ) {
                Ok(Some(evaluation)) => {
                    let should_block = evaluation.readiness == ContractReadinessV1::ContractBlocked
                        && evaluation.continuation_issued;
                    append_contract_evaluation_event(store, &persisted.durable_event, &evaluation)?;
                    if should_block {
                        persisted.ack.stop_block_reason = Some(stop_block_reason(&evaluation));
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    let evaluation =
                        invalid_contract_evaluation(store, &persisted.durable_event, &error)?;
                    let should_block = evaluation.continuation_issued;
                    append_contract_evaluation_event(store, &persisted.durable_event, &evaluation)?;
                    if should_block {
                        persisted.ack.stop_block_reason = Some(format!(
                            "PreviouslyOn could not validate Regression Contracts: {}. Run `previously contracts validate` and resolve the error before completion.",
                            crate::redaction::redact_excerpt(&error.to_string())
                        ));
                    }
                }
            }
        }
        _ => {}
    }

    Ok(persisted)
}

fn checkpoint(store: &Store, persisted: PersistedIngestion) -> Result<HookAckV1> {
    if matches!(
        persisted.event.kind,
        EventKind::Checkpoint | EventKind::ContextCompaction | EventKind::SessionStopped
    ) {
        if let Some(after) = event_snapshot(&persisted.durable_event).or(persisted.snapshot) {
            append_checkpoint_event(store, &persisted.durable_event, after)?;
        }
    }
    Ok(persisted.ack)
}

fn is_historical_app_import(event: &EventEnvelopeV1) -> bool {
    event.source_id.starts_with("codex-app-server:")
        || event
            .payload
            .get("app_server_source")
            .and_then(Value::as_str)
            .is_some()
}

fn attach_new_task(
    event: &mut EventEnvelopeV1,
    snapshot: Option<&GitSnapshotV1>,
    deferred_prompt: Option<&EventEnvelopeV1>,
) -> Result<()> {
    let task_id = deterministic_id("task", &[&event.repository_id, &event.session_id]);
    let prompt_source = deferred_prompt.unwrap_or(event);
    let goal = prompt_text(&prompt_source.payload).map(str::to_string);
    let title = goal
        .as_deref()
        .and_then(|goal| goal.lines().find(|line| !line.trim().is_empty()))
        .map(|line| line.chars().take(120).collect::<String>())
        .unwrap_or_else(|| format!("Codex task {}", &event.session_id));
    let task = TaskV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: task_id.clone(),
        repository_id: event.repository_id.clone(),
        title,
        goal,
        lifecycle: TaskLifecycle::Active,
        branch: snapshot
            .and_then(|snapshot| snapshot.branch.clone())
            .or_else(|| {
                event
                    .payload
                    .get("branch")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            }),
        created_at: deferred_prompt
            .map(|prompt| prompt.occurred_at)
            .unwrap_or(event.occurred_at),
        updated_at: event.occurred_at,
    };
    event.task_id = Some(task_id);
    if let Some(object) = event.payload.as_object_mut() {
        object.insert("task".to_string(), serde_json::to_value(task)?);
    }
    Ok(())
}

fn resume_task_id(payload: &Value) -> Option<&str> {
    let tool_name = ["tool_name", "toolName", "name"]
        .into_iter()
        .find_map(|key| payload.get(key).and_then(Value::as_str))?;
    let normalized = tool_name.to_ascii_lowercase();
    if normalized != "resume_task" && !normalized.ends_with("__resume_task") {
        return None;
    }
    payload
        .pointer("/tool_input/task_id")
        .or_else(|| payload.pointer("/toolInput/task_id"))
        .or_else(|| payload.pointer("/arguments/task_id"))
        .and_then(Value::as_str)
}

fn suggestion_ack(store: &Store, event: &EventEnvelopeV1) -> Result<HookAckV1> {
    if event.kind != EventKind::UserPrompt {
        return Ok(HookAckV1::default());
    }
    let query = prompt_text(&event.payload).unwrap_or_default();
    if query.trim().is_empty() {
        return Ok(HookAckV1::default());
    }
    let suggestions = store.suggest_resume_for_branch(
        &event.repository_id,
        query,
        2,
        event.payload.get("branch").and_then(Value::as_str),
    )?;
    let Some(best) = suggestions.first() else {
        return Ok(HookAckV1::default());
    };
    let margin = suggestions
        .get(1)
        .map(|second| best.score - second.score)
        .unwrap_or(1.0);
    if best.score < 0.75 || margin < 0.15 {
        return Ok(HookAckV1::default());
    }
    Ok(HookAckV1 {
        candidate: Some(ResumeCandidateMetadata {
            task_id: best.task_id.clone(),
            title: best.title.clone(),
            score: best.score,
            matched_by: best.matching_reasons.clone(),
            last_activity_at: Some(best.last_activity_at),
            continuation_advice: best.continuation_advice.clone(),
        }),
        ..HookAckV1::default()
    })
}
