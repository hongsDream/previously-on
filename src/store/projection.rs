use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::Value;
use sha2::Digest;

use crate::domain::{
    deterministic_id, AgentV1, AiFactRefreshOperationV1, CheckpointV1, ContextUsageV1,
    ContinuationStateV1, CoverageV1, EventEnvelopeV1, EventKind, EvidenceV1, FactLifecycle, FactV1,
    FileChangeV1, RepositoryV1, SessionLifecycle, SessionV1, TaskGroupingOperationV1,
    TaskLifecycle, TaskV1, TestResultV1, SCHEMA_VERSION_V1,
};
use crate::redaction::{redact_excerpt, redact_text, redact_value};

use super::{
    enum_text, payload_array, payload_as, payload_text, query_json_optional, query_json_rows,
    timestamp, InsertOutcome,
};

pub(super) fn prepare_event(event: &EventEnvelopeV1) -> Result<EventEnvelopeV1> {
    let mut event = event.clone();
    event.source_id = redact_text(&event.source_id);
    event.payload = redact_value(&event.payload);
    event.coverage.captured = event
        .coverage
        .captured
        .into_iter()
        .map(|item| redact_excerpt(&item))
        .collect();
    event.coverage.missing = event
        .coverage
        .missing
        .into_iter()
        .map(|item| redact_excerpt(&item))
        .collect();
    event.coverage.warnings = event
        .coverage
        .warnings
        .into_iter()
        .map(|item| redact_excerpt(&item))
        .collect();
    Ok(event)
}

pub(super) fn insert_event_tx(
    transaction: &Transaction<'_>,
    event: &EventEnvelopeV1,
) -> Result<InsertOutcome> {
    let latest_key = transaction
        .query_row(
            "SELECT occurred_at, COALESCE(sequence_no, 9223372036854775807), event_id
             FROM canonical_events
             ORDER BY occurred_at DESC, COALESCE(sequence_no, 9223372036854775807) DESC, event_id DESC
             LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let serialized = serde_json::to_string(event).context("serialize canonical event")?;
    let inserted = transaction.execute(
        "INSERT OR IGNORE INTO canonical_events
         (event_id, dedupe_key, repository_id, session_id, task_id, sequence_no, occurred_at,
          received_at, kind, event_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            event.event_id,
            event.dedupe_key,
            event.repository_id,
            event.session_id,
            event.task_id,
            event.sequence,
            timestamp(event.occurred_at),
            timestamp(event.received_at),
            enum_text(event.kind)?,
            serialized,
        ],
    )?;
    if inserted == 0 {
        return Ok(InsertOutcome::Duplicate);
    }
    let event_key = (
        timestamp(event.occurred_at),
        event.sequence.unwrap_or(i64::MAX),
        event.event_id.clone(),
    );
    if latest_key.map(|latest| event_key < latest).unwrap_or(false) {
        rebuild_projections_tx(transaction)?;
    } else {
        apply_event_projection(transaction, event)?;
    }
    Ok(InsertOutcome::Inserted)
}

pub(super) fn rebuild_projections_tx(transaction: &Transaction<'_>) -> Result<()> {
    let events = load_events(transaction, None)?;
    transaction.execute_batch(
        "DELETE FROM task_fts;
         DELETE FROM fact_fts;
         DELETE FROM agents;
         DELETE FROM ai_fact_refresh_operations;
         DELETE FROM session_grouping_assignments;
         DELETE FROM contract_evaluations;
         DELETE FROM regression_candidates;
         DELETE FROM test_results;
         DELETE FROM file_changes;
         DELETE FROM evidence;
         DELETE FROM facts;
         DELETE FROM checkpoints;
         DELETE FROM sessions;
         DELETE FROM tasks;",
    )?;
    for event in events {
        apply_event_projection(transaction, &event)?;
    }
    Ok(())
}

fn apply_event_projection(transaction: &Transaction<'_>, event: &EventEnvelopeV1) -> Result<()> {
    ensure_repository_tx(transaction, event)?;
    if event.kind == EventKind::TaskGroupingChanged {
        let operation = payload_as::<TaskGroupingOperationV1>(&event.payload, "operation")
            .context("task grouping event is missing operation")?;
        return apply_task_grouping_projection(transaction, &operation);
    }
    if event.kind == EventKind::TaskUpdated {
        return ensure_task_tx(transaction, event);
    }
    let mut event = event.clone();
    if let Some(task_id) = effective_grouping_task_id(transaction, &event)? {
        event.task_id = Some(task_id);
    }
    ensure_session_tx(transaction, &event)?;
    ensure_task_tx(transaction, &event)?;
    match event.kind {
        EventKind::SessionStarted => {
            if let Some(session) = payload_as::<SessionV1>(&event.payload, "session") {
                upsert_session_tx(transaction, &session)?;
            }
        }
        EventKind::SessionStopped => {
            if let Some(session) = payload_as::<SessionV1>(&event.payload, "session") {
                upsert_session_tx(transaction, &session)?;
            } else {
                let existing: Option<SessionV1> = query_json_optional(
                    transaction,
                    "SELECT session_json FROM sessions WHERE id = ?1",
                    [&event.session_id],
                )?;
                if let Some(mut session) = existing {
                    session.lifecycle = SessionLifecycle::Completed;
                    session.ended_at = Some(event.occurred_at);
                    upsert_session_tx(transaction, &session)?;
                }
            }
        }
        EventKind::Checkpoint => {
            if let Some(checkpoint) = payload_as::<CheckpointV1>(&event.payload, "checkpoint") {
                let mut checkpoint = checkpoint;
                if let Some(task_id) = event.task_id.as_ref() {
                    checkpoint.task_id = task_id.clone();
                    for change in &mut checkpoint.changed_files {
                        change.task_id = Some(task_id.clone());
                    }
                    for test in &mut checkpoint.tests {
                        test.task_id = Some(task_id.clone());
                    }
                }
                upsert_checkpoint_tx(transaction, &checkpoint)?;
            }
        }
        EventKind::FactCandidate | EventKind::FactConfirmed => {
            if let Some(mut fact) = payload_as::<FactV1>(&event.payload, "fact") {
                fact.content = redact_text(&fact.content);
                if event.kind == EventKind::FactConfirmed
                    && fact.lifecycle == FactLifecycle::Candidate
                {
                    fact.lifecycle = FactLifecycle::Confirmed;
                }
                if let Some(task_id) = event.task_id.as_ref() {
                    fact.task_id = task_id.clone();
                }
                upsert_fact_tx(transaction, &fact)?;
            }
            if let Some(mut evidence) = payload_as::<EvidenceV1>(&event.payload, "evidence") {
                evidence.excerpt = redact_excerpt(&evidence.excerpt);
                if let Some(task_id) = event.task_id.as_ref() {
                    evidence.task_id = task_id.clone();
                }
                upsert_evidence_tx(transaction, &evidence)?;
            }
        }
        EventKind::ToolFinished => {
            for mut test in payload_array::<TestResultV1>(&event.payload, "test_results") {
                test.command = redact_text(&test.command);
                test.summary = test.summary.map(|summary| redact_excerpt(&summary));
                if let Some(task_id) = event.task_id.as_ref() {
                    test.task_id = Some(task_id.clone());
                }
                upsert_test_result_tx(transaction, &test)?;
            }
            if let Some(mut test) = payload_as::<TestResultV1>(&event.payload, "test_result") {
                test.command = redact_text(&test.command);
                test.summary = test.summary.map(|summary| redact_excerpt(&summary));
                if let Some(task_id) = event.task_id.as_ref() {
                    test.task_id = Some(task_id.clone());
                }
                upsert_test_result_tx(transaction, &test)?;
            }
            for mut change in payload_array::<FileChangeV1>(&event.payload, "file_changes") {
                if let Some(task_id) = event.task_id.as_ref() {
                    change.task_id = Some(task_id.clone());
                }
                upsert_file_change_tx(transaction, &change)?;
            }
        }
        EventKind::RegressionCandidateRecorded => {
            let candidate = event
                .payload
                .get("regressionCandidate")
                .context("regression candidate event is missing regressionCandidate")?;
            upsert_regression_candidate_tx(transaction, &event, candidate)?;
        }
        EventKind::ContractEvaluationRecorded => {
            let evaluation = event
                .payload
                .get("contractEvaluation")
                .context("contract evaluation event is missing contractEvaluation")?;
            upsert_contract_evaluation_tx(transaction, &event, evaluation)?;
        }
        EventKind::AiFactRefreshOperationRecorded => {
            let operation = payload_as::<AiFactRefreshOperationV1>(&event.payload, "operation")
                .context("AI fact refresh event is missing operation")?;
            upsert_ai_fact_refresh_operation_tx(transaction, &operation)?;
        }
        EventKind::AgentObserved => {
            let agent = payload_as::<AgentV1>(&event.payload, "agent")
                .context("agent observation event is missing agent")?;
            upsert_agent_tx(transaction, &agent)?;
        }
        _ => {}
    }
    Ok(())
}

fn effective_grouping_task_id(
    transaction: &Transaction<'_>,
    event: &EventEnvelopeV1,
) -> Result<Option<String>> {
    if event.session_id == "local-ui" || event.session_id == "task-grouping" {
        return Ok(None);
    }
    transaction
        .query_row(
            "SELECT task_id FROM session_grouping_assignments
             WHERE session_id = ?1 AND repository_id = ?2",
            params![event.session_id, event.repository_id],
            |row| row.get(0),
        )
        .optional()
        .context("load effective task grouping assignment")
}

fn apply_task_grouping_projection(
    transaction: &Transaction<'_>,
    operation: &TaskGroupingOperationV1,
) -> Result<()> {
    if operation.schema_version != SCHEMA_VERSION_V1 {
        bail!("unsupported task grouping operation schema");
    }
    if let Some(created_task) = operation.created_task.as_ref() {
        let should_exist = operation
            .task_lifecycle
            .iter()
            .any(|snapshot| snapshot.task_id == created_task.id && snapshot.after.is_some());
        if should_exist {
            upsert_task_tx(transaction, created_task)?;
        }
    }
    for item in &operation.session_moves {
        let mut session: SessionV1 = query_json_optional(
            transaction,
            "SELECT session_json FROM sessions WHERE id = ?1",
            [&item.session_id],
        )?
        .with_context(|| format!("grouping session projection missing: {}", item.session_id))?;
        if session.repository_id != operation.repository_id {
            bail!("grouping operation crossed repository boundary");
        }
        session.task_id = Some(item.to_task_id.clone());
        upsert_session_tx(transaction, &session)?;
        transaction.execute(
            "INSERT INTO session_grouping_assignments
             (session_id, repository_id, task_id, operation_id, occurred_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(session_id) DO UPDATE SET repository_id=excluded.repository_id,
               task_id=excluded.task_id, operation_id=excluded.operation_id,
               occurred_at=excluded.occurred_at",
            params![
                item.session_id,
                operation.repository_id,
                item.to_task_id,
                operation.operation_id,
                timestamp(operation.occurred_at)
            ],
        )?;
        relocate_session_owned_projections(transaction, &item.session_id, &item.to_task_id)?;
    }
    for impact in &operation.fact_impacts {
        if impact.mixed_provenance {
            continue;
        }
        let Some(to_task_id) = impact.to_task_id.as_deref() else {
            continue;
        };
        let Some(mut fact) = query_json_optional::<FactV1, _>(
            transaction,
            "SELECT fact_json FROM facts WHERE id = ?1",
            [&impact.fact_id],
        )?
        else {
            continue;
        };
        fact.task_id = to_task_id.to_string();
        fact.updated_at = operation.occurred_at;
        upsert_fact_tx(transaction, &fact)?;
    }
    for snapshot in &operation.task_lifecycle {
        match snapshot.after {
            Some(lifecycle) => {
                let Some(mut task) = query_json_optional::<TaskV1, _>(
                    transaction,
                    "SELECT task_json FROM tasks WHERE id = ?1",
                    [&snapshot.task_id],
                )?
                else {
                    bail!("grouping lifecycle task missing: {}", snapshot.task_id);
                };
                task.lifecycle = lifecycle;
                task.updated_at = operation.occurred_at;
                upsert_task_tx(transaction, &task)?;
            }
            None => {
                transaction.execute(
                    "DELETE FROM task_fts WHERE task_id = ?1",
                    [&snapshot.task_id],
                )?;
                transaction.execute("DELETE FROM tasks WHERE id = ?1", [&snapshot.task_id])?;
            }
        }
    }
    Ok(())
}

fn relocate_session_owned_projections(
    transaction: &Transaction<'_>,
    session_id: &str,
    task_id: &str,
) -> Result<()> {
    for mut checkpoint in query_json_rows::<CheckpointV1, _>(
        transaction,
        "SELECT checkpoint_json FROM checkpoints WHERE session_id = ?1 ORDER BY created_at, id",
        [session_id],
    )? {
        checkpoint.task_id = task_id.to_string();
        for change in &mut checkpoint.changed_files {
            change.task_id = Some(task_id.to_string());
        }
        for test in &mut checkpoint.tests {
            test.task_id = Some(task_id.to_string());
        }
        upsert_checkpoint_tx(transaction, &checkpoint)?;
    }
    for mut evidence in query_json_rows::<EvidenceV1, _>(
        transaction,
        "SELECT evidence_json FROM evidence WHERE session_id = ?1 ORDER BY created_at, id",
        [session_id],
    )? {
        evidence.task_id = task_id.to_string();
        upsert_evidence_tx(transaction, &evidence)?;
    }
    let mut changes = query_json_rows::<FileChangeV1, _>(
        transaction,
        "SELECT change_json FROM file_changes WHERE session_id = ?1 ORDER BY path, id",
        [session_id],
    )?;
    transaction.execute(
        "DELETE FROM file_changes WHERE session_id = ?1",
        [session_id],
    )?;
    for mut change in changes.drain(..) {
        change.task_id = Some(task_id.to_string());
        upsert_file_change_tx(transaction, &change)?;
    }
    for mut test in query_json_rows::<TestResultV1, _>(
        transaction,
        "SELECT test_json FROM test_results WHERE session_id = ?1 ORDER BY occurred_at, id",
        [session_id],
    )? {
        test.task_id = Some(task_id.to_string());
        upsert_test_result_tx(transaction, &test)?;
    }
    Ok(())
}

fn ensure_repository_tx(transaction: &Transaction<'_>, event: &EventEnvelopeV1) -> Result<()> {
    let exists = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM repositories WHERE id = ?1)",
        [&event.repository_id],
        |row| row.get::<_, bool>(0),
    )?;
    if exists {
        return Ok(());
    }
    let repository = RepositoryV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: event.repository_id.clone(),
        path: event
            .payload
            .get("repository_path")
            .and_then(Value::as_str)
            .map(redact_text)
            .unwrap_or_default(),
        remote_url: None,
        created_at: event.occurred_at,
        updated_at: event.occurred_at,
    };
    upsert_repository_tx(transaction, &repository)
}

fn ensure_session_tx(transaction: &Transaction<'_>, event: &EventEnvelopeV1) -> Result<()> {
    let existing: Option<SessionV1> = query_json_optional(
        transaction,
        "SELECT session_json FROM sessions WHERE id = ?1",
        [&event.session_id],
    )?;
    if let Some(mut session) = existing {
        let mut changed = false;
        if event.task_id.is_some() && session.task_id != event.task_id {
            session.task_id = event.task_id.clone();
            changed = true;
        }
        if let Some(branch) = event.payload.get("branch").and_then(Value::as_str) {
            if session.branch.as_deref() != Some(branch) {
                session.branch = Some(branch.to_string());
                changed = true;
            }
        }
        if let Some(head) = event.payload.get("head").and_then(Value::as_str) {
            if session.head.as_deref() != Some(head) {
                session.head = Some(head.to_string());
                changed = true;
            }
        }
        if let Some(thread_id) = event
            .payload
            .get("source_thread_id")
            .or_else(|| event.payload.get("thread_id"))
            .and_then(Value::as_str)
        {
            if session.source_thread_id.as_deref() != Some(thread_id) {
                session.source_thread_id = Some(thread_id.to_string());
                changed = true;
            }
        }
        let last_activity = event
            .payload
            .get("thread_updated_at")
            .and_then(Value::as_str)
            .and_then(|value| value.parse().ok())
            .unwrap_or(event.occurred_at);
        if session
            .last_activity_at
            .is_none_or(|current| last_activity > current)
        {
            session.last_activity_at = Some(last_activity);
            changed = true;
        }
        if let Some(turn_count) = event
            .payload
            .get("turn_count")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
        {
            if turn_count > session.turn_count {
                session.turn_count = turn_count;
                changed = true;
            }
        } else if event.kind == EventKind::UserPrompt && session.source_thread_id.is_none() {
            session.turn_count = session.turn_count.saturating_add(1);
            changed = true;
        }
        if event.kind == EventKind::ContextCompaction {
            session.compaction_count = session.compaction_count.saturating_add(1);
            if session.compaction_count >= crate::domain::PROVISIONAL_COMPACTION_THRESHOLD
                && session.continuation_state == ContinuationStateV1::Normal
            {
                session.continuation_state = ContinuationStateV1::Eligible;
            }
            changed = true;
        }
        if let Some(usage) = context_usage(&event.payload, event.occurred_at) {
            if usage
                .utilization()
                .is_some_and(|ratio| ratio >= crate::domain::PROVISIONAL_CONTEXT_USAGE_THRESHOLD)
                && session.continuation_state == ContinuationStateV1::Normal
            {
                session.continuation_state = ContinuationStateV1::Eligible;
            }
            session.context_usage = Some(usage);
            changed = true;
        }
        if event.kind == EventKind::ContinuationSuggested {
            let next_state = if event.payload.get("delivery_state").and_then(Value::as_str)
                == Some("pending_replay")
            {
                ContinuationStateV1::Eligible
            } else {
                ContinuationStateV1::Suggested
            };
            if session.continuation_state != next_state {
                session.continuation_state = next_state;
                changed = true;
            }
        }
        let coverage = CoverageV1::merge([&session.coverage, &event.coverage]);
        if coverage != session.coverage {
            session.coverage = coverage;
            changed = true;
        }
        if changed {
            upsert_session_tx(transaction, &session)?;
        }
        return Ok(());
    }
    let started_at = event
        .payload
        .get("thread_created_at")
        .and_then(Value::as_str)
        .and_then(|value| value.parse().ok())
        .unwrap_or(event.occurred_at);
    let context_usage = context_usage(&event.payload, event.occurred_at);
    let compaction_count = u32::from(event.kind == EventKind::ContextCompaction);
    let session = SessionV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: event.session_id.clone(),
        repository_id: event.repository_id.clone(),
        task_id: event.task_id.clone(),
        lifecycle: SessionLifecycle::Active,
        started_at,
        ended_at: None,
        branch: event
            .payload
            .get("branch")
            .and_then(Value::as_str)
            .map(str::to_string),
        head: event
            .payload
            .get("head")
            .and_then(Value::as_str)
            .map(str::to_string),
        source_thread_id: event
            .payload
            .get("source_thread_id")
            .or_else(|| event.payload.get("thread_id"))
            .and_then(Value::as_str)
            .map(str::to_string),
        last_activity_at: Some(
            event
                .payload
                .get("thread_updated_at")
                .and_then(Value::as_str)
                .and_then(|value| value.parse().ok())
                .unwrap_or(event.occurred_at),
        ),
        turn_count: event
            .payload
            .get("turn_count")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(u32::from(event.kind == EventKind::UserPrompt)),
        compaction_count,
        context_usage: context_usage.clone(),
        continuation_state: if event.kind == EventKind::ContinuationSuggested {
            if event.payload.get("delivery_state").and_then(Value::as_str) == Some("pending_replay")
            {
                ContinuationStateV1::Eligible
            } else {
                ContinuationStateV1::Suggested
            }
        } else if compaction_count >= crate::domain::PROVISIONAL_COMPACTION_THRESHOLD
            || context_usage
                .as_ref()
                .and_then(ContextUsageV1::utilization)
                .is_some_and(|ratio| ratio >= crate::domain::PROVISIONAL_CONTEXT_USAGE_THRESHOLD)
        {
            ContinuationStateV1::Eligible
        } else {
            ContinuationStateV1::Normal
        },
        coverage: event.coverage.clone(),
    };
    upsert_session_tx(transaction, &session)
}

fn context_usage(payload: &Value, occurred_at: DateTime<Utc>) -> Option<ContextUsageV1> {
    let usage = payload.get("context_usage").unwrap_or(payload);
    let total_tokens = usage
        .get("total_tokens")
        .or_else(|| usage.get("totalTokens"))
        .and_then(Value::as_u64)?;
    let model_context_window = usage
        .get("model_context_window")
        .or_else(|| usage.get("modelContextWindow"))
        .and_then(Value::as_u64)?;
    Some(ContextUsageV1 {
        total_tokens,
        model_context_window,
        observed_at: Some(occurred_at),
    })
}

fn ensure_task_tx(transaction: &Transaction<'_>, event: &EventEnvelopeV1) -> Result<()> {
    let Some(task_id) = event.task_id.as_deref() else {
        return Ok(());
    };
    let existing: Option<TaskV1> = query_json_optional(
        transaction,
        "SELECT task_json FROM tasks WHERE id = ?1",
        [task_id],
    )?;
    if event.kind == EventKind::TaskUpdated {
        let mut task = payload_as::<TaskV1>(&event.payload, "task")
            .context("task update event is missing task")?;
        if task.id != task_id || task.repository_id != event.repository_id {
            bail!("task update identity does not match event envelope");
        }
        if event.occurred_at > task.updated_at {
            task.updated_at = event.occurred_at;
        }
        return upsert_task_tx(transaction, &task);
    }
    if let Some(mut task) = payload_as::<TaskV1>(&event.payload, "task") {
        if task.id == task_id && task.repository_id == event.repository_id {
            let replaces_placeholder = existing
                .as_ref()
                .is_some_and(|current| current.title == current.id && current.goal.is_none());
            let is_newer = existing
                .as_ref()
                .is_none_or(|current| task.updated_at > current.updated_at);
            if replaces_placeholder || is_newer {
                if event.occurred_at > task.updated_at {
                    task.updated_at = event.occurred_at;
                }
                return upsert_task_tx(transaction, &task);
            }
        }
    }
    if let Some(mut task) = existing {
        if event.occurred_at > task.updated_at {
            task.updated_at = event.occurred_at;
        }
        if task.branch.is_none() {
            task.branch = event
                .payload
                .get("branch")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        upsert_task_tx(transaction, &task)?;
        return Ok(());
    }
    let goal = payload_text(&event.payload);
    let title = event
        .payload
        .get("task_title")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            goal.as_ref().map(|goal| {
                goal.lines()
                    .next()
                    .unwrap_or(goal)
                    .chars()
                    .take(120)
                    .collect()
            })
        })
        .unwrap_or_else(|| task_id.to_string());
    upsert_task_tx(
        transaction,
        &TaskV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: task_id.to_string(),
            repository_id: event.repository_id.clone(),
            title: redact_excerpt(&title),
            goal: goal.map(|goal| redact_excerpt(&goal)),
            lifecycle: TaskLifecycle::Active,
            branch: event
                .payload
                .get("branch")
                .and_then(Value::as_str)
                .map(str::to_string),
            created_at: event.occurred_at,
            updated_at: event.occurred_at,
        },
    )
}

pub(super) fn upsert_repository_tx(
    transaction: &Transaction<'_>,
    repository: &RepositoryV1,
) -> Result<()> {
    let mut repository = repository.clone();
    repository.path = redact_text(&repository.path);
    repository.remote_url = repository.remote_url.map(|url| redact_text(&url));
    let json = serde_json::to_string(&repository)?;
    transaction.execute(
        "INSERT INTO repositories(id, repository_id, path, updated_at, repository_json)
         VALUES (?1, ?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET path=excluded.path, updated_at=excluded.updated_at,
           repository_json=excluded.repository_json",
        params![
            repository.id,
            repository.path,
            timestamp(repository.updated_at),
            json
        ],
    )?;
    Ok(())
}

pub(super) fn upsert_task_tx(transaction: &Transaction<'_>, task: &TaskV1) -> Result<()> {
    let mut task = task.clone();
    task.title = redact_excerpt(&task.title);
    task.goal = task.goal.map(|goal| redact_excerpt(&goal));
    let json = serde_json::to_string(&task)?;
    transaction.execute(
        "INSERT INTO tasks(id, repository_id, updated_at, task_json) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET repository_id=excluded.repository_id,
           updated_at=excluded.updated_at, task_json=excluded.task_json",
        params![
            task.id,
            task.repository_id,
            timestamp(task.updated_at),
            json
        ],
    )?;
    transaction.execute("DELETE FROM task_fts WHERE task_id = ?1", [&task.id])?;
    transaction.execute(
        "INSERT INTO task_fts(task_id, title, goal) VALUES (?1, ?2, ?3)",
        params![task.id, task.title, task.goal.unwrap_or_default()],
    )?;
    Ok(())
}

pub(super) fn upsert_session_tx(transaction: &Transaction<'_>, session: &SessionV1) -> Result<()> {
    let json = serde_json::to_string(session)?;
    transaction.execute(
        "INSERT INTO sessions(id, repository_id, task_id, started_at, session_json)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET repository_id=excluded.repository_id,
           task_id=excluded.task_id, started_at=excluded.started_at, session_json=excluded.session_json",
        params![session.id, session.repository_id, session.task_id, timestamp(session.started_at), json],
    )?;
    Ok(())
}

pub(super) fn upsert_checkpoint_tx(
    transaction: &Transaction<'_>,
    checkpoint: &CheckpointV1,
) -> Result<()> {
    let checkpoint: CheckpointV1 = serde_json::from_value(redact_value(
        &serde_json::to_value(checkpoint).context("serialize checkpoint for redaction")?,
    ))
    .context("deserialize redacted checkpoint")?;
    let json = serde_json::to_string(&checkpoint)?;
    transaction.execute(
        "INSERT INTO checkpoints(id, repository_id, task_id, session_id, created_at, checkpoint_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET repository_id=excluded.repository_id,
           task_id=excluded.task_id, session_id=excluded.session_id,
           created_at=excluded.created_at, checkpoint_json=excluded.checkpoint_json",
        params![checkpoint.id, checkpoint.repository_id, checkpoint.task_id, checkpoint.session_id, timestamp(checkpoint.created_at), json],
    )?;
    for change in &checkpoint.changed_files {
        upsert_file_change_tx(transaction, change)?;
    }
    for test in &checkpoint.tests {
        upsert_test_result_tx(transaction, test)?;
    }
    Ok(())
}

pub(super) fn upsert_fact_tx(transaction: &Transaction<'_>, fact: &FactV1) -> Result<()> {
    let mut fact = fact.clone();
    fact.content = redact_text(&fact.content);
    let json = serde_json::to_string(&fact)?;
    transaction.execute(
        "INSERT INTO facts(id, repository_id, task_id, lifecycle, freshness, updated_at, fact_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET repository_id=excluded.repository_id,
           task_id=excluded.task_id, lifecycle=excluded.lifecycle, freshness=excluded.freshness,
           updated_at=excluded.updated_at, fact_json=excluded.fact_json",
        params![
            fact.id,
            fact.repository_id,
            fact.task_id,
            enum_text(fact.lifecycle)?,
            enum_text(fact.freshness)?,
            timestamp(fact.updated_at),
            json
        ],
    )?;
    transaction.execute("DELETE FROM fact_fts WHERE fact_id = ?1", [&fact.id])?;
    transaction.execute(
        "INSERT INTO fact_fts(fact_id, content) VALUES (?1, ?2)",
        params![fact.id, fact.content],
    )?;
    Ok(())
}

pub(super) fn upsert_evidence_tx(
    transaction: &Transaction<'_>,
    evidence: &EvidenceV1,
) -> Result<()> {
    let mut evidence = evidence.clone();
    evidence.excerpt = redact_excerpt(&evidence.excerpt);
    evidence.excerpt_sha256 = hex::encode(sha2::Sha256::digest(evidence.excerpt.as_bytes()));
    let json = serde_json::to_string(&evidence)?;
    transaction.execute(
        "INSERT INTO evidence(id, repository_id, task_id, fact_id, session_id, created_at, evidence_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET repository_id=excluded.repository_id,
           task_id=excluded.task_id, fact_id=excluded.fact_id, session_id=excluded.session_id,
           created_at=excluded.created_at, evidence_json=excluded.evidence_json",
        params![evidence.id, evidence.repository_id, evidence.task_id, evidence.fact_id, evidence.session_id, timestamp(evidence.created_at), json],
    )?;
    Ok(())
}

pub(super) fn upsert_file_change_tx(
    transaction: &Transaction<'_>,
    change: &FileChangeV1,
) -> Result<()> {
    let id = deterministic_id(
        "change",
        &[
            &change.repository_id,
            &change.session_id,
            change.task_id.as_deref().unwrap_or_default(),
            &change.path,
            change.previous_path.as_deref().unwrap_or_default(),
            change.before_head.as_deref().unwrap_or_default(),
            change.after_head.as_deref().unwrap_or_default(),
        ],
    );
    let mut change = change.clone();
    change.path = redact_text(&change.path);
    change.previous_path = change.previous_path.map(|path| redact_text(&path));
    let json = serde_json::to_string(&change)?;
    transaction.execute(
        "INSERT INTO file_changes(id, repository_id, task_id, session_id, path, change_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET repository_id=excluded.repository_id,
           task_id=excluded.task_id, session_id=excluded.session_id, path=excluded.path,
           change_json=excluded.change_json",
        params![
            id,
            change.repository_id,
            change.task_id,
            change.session_id,
            change.path,
            json
        ],
    )?;
    Ok(())
}

pub(super) fn upsert_test_result_tx(
    transaction: &Transaction<'_>,
    test: &TestResultV1,
) -> Result<()> {
    let mut test = test.clone();
    test.command = redact_text(&test.command);
    test.summary = test.summary.map(|summary| redact_excerpt(&summary));
    let json = serde_json::to_string(&test)?;
    transaction.execute(
        "INSERT INTO test_results(id, repository_id, task_id, session_id, occurred_at, test_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET repository_id=excluded.repository_id,
           task_id=excluded.task_id, session_id=excluded.session_id,
           occurred_at=excluded.occurred_at, test_json=excluded.test_json",
        params![
            test.id,
            test.repository_id,
            test.task_id,
            test.session_id,
            timestamp(test.occurred_at),
            json
        ],
    )?;
    Ok(())
}

fn upsert_ai_fact_refresh_operation_tx(
    transaction: &Transaction<'_>,
    operation: &AiFactRefreshOperationV1,
) -> Result<()> {
    let operation: AiFactRefreshOperationV1 = serde_json::from_value(redact_value(
        &serde_json::to_value(operation).context("serialize AI fact refresh operation")?,
    ))
    .context("deserialize redacted AI fact refresh operation")?;
    let json = serde_json::to_string(&operation)?;
    transaction.execute(
        "INSERT INTO ai_fact_refresh_operations
         (operation_id, repository_id, task_id, updated_at, operation_json)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(operation_id) DO UPDATE SET repository_id=excluded.repository_id,
           task_id=excluded.task_id, updated_at=excluded.updated_at,
           operation_json=excluded.operation_json",
        params![
            operation.operation_id,
            operation.repository_id,
            operation.task_id,
            timestamp(operation.updated_at),
            json
        ],
    )?;
    Ok(())
}

fn upsert_agent_tx(transaction: &Transaction<'_>, agent: &AgentV1) -> Result<()> {
    let agent: AgentV1 = serde_json::from_value(redact_value(
        &serde_json::to_value(agent).context("serialize agent observation")?,
    ))
    .context("deserialize redacted agent observation")?;
    let json = serde_json::to_string(&agent)?;
    transaction.execute(
        "INSERT INTO agents(id, repository_id, task_id, thread_id, observed_at, agent_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET repository_id=excluded.repository_id,
           task_id=excluded.task_id, thread_id=excluded.thread_id,
           observed_at=excluded.observed_at, agent_json=excluded.agent_json",
        params![
            agent.id,
            agent.repository_id,
            agent.task_id,
            agent.thread_id,
            timestamp(agent.observed_at),
            json
        ],
    )?;
    Ok(())
}

fn upsert_regression_candidate_tx(
    transaction: &Transaction<'_>,
    event: &EventEnvelopeV1,
    candidate: &Value,
) -> Result<()> {
    let candidate = redact_value(candidate);
    let id = candidate
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .context("regression candidate id is missing")?;
    let json = serde_json::to_string(&candidate)?;
    transaction.execute(
        "INSERT INTO regression_candidates(id, repository_id, task_id, updated_at, candidate_json)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET repository_id=excluded.repository_id,
           task_id=excluded.task_id, updated_at=excluded.updated_at,
           candidate_json=excluded.candidate_json",
        params![
            id,
            event.repository_id,
            event.task_id,
            timestamp(event.occurred_at),
            json
        ],
    )?;
    Ok(())
}

fn upsert_contract_evaluation_tx(
    transaction: &Transaction<'_>,
    event: &EventEnvelopeV1,
    evaluation: &Value,
) -> Result<()> {
    let evaluation = redact_value(evaluation);
    let id = evaluation
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .context("contract evaluation id is missing")?;
    let json = serde_json::to_string(&evaluation)?;
    transaction.execute(
        "INSERT INTO contract_evaluations(id, repository_id, task_id, evaluated_at, evaluation_json)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET repository_id=excluded.repository_id,
           task_id=excluded.task_id, evaluated_at=excluded.evaluated_at,
           evaluation_json=excluded.evaluation_json",
        params![
            id,
            event.repository_id,
            event.task_id,
            timestamp(event.occurred_at),
            json
        ],
    )?;
    Ok(())
}

pub(super) fn load_events(
    connection: &Connection,
    repository_id: Option<&str>,
) -> Result<Vec<EventEnvelopeV1>> {
    let (sql, parameter): (&str, Option<&str>) = if repository_id.is_some() {
        (
            "SELECT event_json FROM canonical_events WHERE repository_id = ?1
             ORDER BY occurred_at, COALESCE(sequence_no, 9223372036854775807), event_id",
            repository_id,
        )
    } else {
        (
            "SELECT event_json FROM canonical_events
             ORDER BY occurred_at, COALESCE(sequence_no, 9223372036854775807), event_id",
            None,
        )
    };
    let mut statement = connection.prepare(sql)?;
    let json_rows = if let Some(parameter) = parameter {
        statement
            .query_map([parameter], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    json_rows
        .into_iter()
        .map(|json| serde_json::from_str(&json).context("deserialize canonical event"))
        .collect()
}
