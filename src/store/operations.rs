use super::{
    payload_as, query_json_optional, query_json_rows, CandidateReviewOutcome, ClaimOutcome,
    InsertOutcome, Store,
};
use crate::domain::{
    deterministic_id, AgentV1, AiFactCandidateStatusV1, AiFactCandidateV1,
    AiFactRefreshOperationV1, AiFactRefreshStatusV1, EventEnvelopeV1, EventKind, EvidenceV1,
    FactKind, FactLifecycle, FactOriginV1, FactV1, Freshness, SessionV1, TaskGroupingActionV1,
    TaskGroupingOperationV1, TaskLifecycle, TaskV1, SCHEMA_VERSION_V1,
};
use crate::redaction::redact_text;
use anyhow::{bail, Context, Result};
use chrono::Utc;
use rusqlite::Transaction;
use serde_json::json;
use sha2::Digest;

use super::projection::{insert_event_tx, load_events, prepare_event};

impl Store {
    pub fn list_task_grouping_operations(
        &self,
        repository_id: Option<&str>,
    ) -> Result<Vec<TaskGroupingOperationV1>> {
        let mut operations = self
            .list_events(repository_id)?
            .into_iter()
            .filter(|event| event.kind == EventKind::TaskGroupingChanged)
            .filter_map(|event| payload_as::<TaskGroupingOperationV1>(&event.payload, "operation"))
            .collect::<Vec<_>>();
        operations.sort_by(|left, right| {
            (left.occurred_at, &left.operation_id).cmp(&(right.occurred_at, &right.operation_id))
        });
        Ok(operations)
    }

    pub fn get_ai_fact_refresh_operation(
        &self,
        operation_id: &str,
    ) -> Result<Option<AiFactRefreshOperationV1>> {
        query_json_optional(
            &self.connect()?,
            "SELECT operation_json FROM ai_fact_refresh_operations WHERE operation_id = ?1",
            [operation_id],
        )
    }

    pub fn list_ai_fact_refresh_operations(
        &self,
        repository_id: Option<&str>,
    ) -> Result<Vec<AiFactRefreshOperationV1>> {
        let connection = self.connect()?;
        if let Some(repository_id) = repository_id {
            query_json_rows(
                &connection,
                "SELECT operation_json FROM ai_fact_refresh_operations
                 WHERE repository_id = ?1 ORDER BY updated_at DESC, operation_id",
                [repository_id],
            )
        } else {
            query_json_rows(
                &connection,
                "SELECT operation_json FROM ai_fact_refresh_operations
                 ORDER BY updated_at DESC, operation_id",
                [],
            )
        }
    }

    pub fn append_ai_fact_refresh_operation(
        &self,
        operation: &AiFactRefreshOperationV1,
    ) -> Result<InsertOutcome> {
        let event = ai_fact_refresh_event(operation);
        self.insert_event(&event)
    }

    pub fn claim_ai_fact_refresh_operation(
        &self,
        operation: &AiFactRefreshOperationV1,
    ) -> Result<ClaimOutcome<AiFactRefreshOperationV1>> {
        let event = ai_fact_refresh_event(operation);
        let _lock = self.acquire_maintenance_lock()?;
        self.ensure_event_write_allowed(&event)?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        if let Some(existing) = query_json_optional::<AiFactRefreshOperationV1, _>(
            &transaction,
            "SELECT operation_json FROM ai_fact_refresh_operations WHERE operation_id = ?1",
            [&operation.operation_id],
        )? {
            if existing.repository_id != operation.repository_id
                || existing.task_id != operation.task_id
                || existing.request_fingerprint != operation.request_fingerprint
            {
                bail!("fact refresh operation id belongs to a different request");
            }
            transaction.commit()?;
            return Ok(ClaimOutcome::Existing(existing));
        }
        let event = prepare_event(&event)?;
        if insert_event_tx(&transaction, &event)? != InsertOutcome::Inserted {
            bail!("fact refresh claim canonical event already exists without a projection");
        }
        transaction.commit()?;
        Ok(ClaimOutcome::Claimed(operation.clone()))
    }

    pub fn review_ai_fact_candidate(
        &self,
        operation_id: &str,
        candidate_id: &str,
        accept: bool,
        edited_content: Option<&str>,
        edited_kind: Option<FactKind>,
    ) -> Result<CandidateReviewOutcome> {
        if !accept && (edited_content.is_some() || edited_kind.is_some()) {
            bail!("reject review cannot edit candidate content or kind");
        }
        let _lock = self.acquire_maintenance_lock()?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let mut operation: AiFactRefreshOperationV1 = query_json_optional(
            &transaction,
            "SELECT operation_json FROM ai_fact_refresh_operations WHERE operation_id = ?1",
            [operation_id],
        )?
        .with_context(|| format!("fact refresh operation not found: {operation_id}"))?;
        if operation.status != AiFactRefreshStatusV1::Completed {
            bail!("fact refresh operation is not complete");
        }
        let original = original_ai_fact_candidate_tx(&transaction, operation_id, candidate_id)?
            .with_context(|| format!("AI fact candidate not found: {candidate_id}"))?;
        let desired_content = redact_text(edited_content.unwrap_or(&original.content).trim());
        if accept && (desired_content.is_empty() || desired_content.chars().count() > 500) {
            bail!("accepted candidate content must contain 1-500 characters");
        }
        let desired_kind = edited_kind.unwrap_or(original.kind);
        let current = operation
            .candidates
            .iter()
            .find(|candidate| candidate.id == candidate_id)
            .cloned()
            .with_context(|| format!("AI fact candidate not found: {candidate_id}"))?;
        let fact_id = deterministic_id("ai-assisted-fact", &[operation_id, candidate_id]);
        if current.status != AiFactCandidateStatusV1::Pending {
            let expected_status = if accept {
                AiFactCandidateStatusV1::Accepted
            } else {
                AiFactCandidateStatusV1::Rejected
            };
            if current.status != expected_status
                || (accept && (current.content != desired_content || current.kind != desired_kind))
            {
                bail!("AI fact candidate was already reviewed with different content");
            }
            let fact = if accept {
                let fact = query_json_optional::<FactV1, _>(
                    &transaction,
                    "SELECT fact_json FROM facts WHERE id = ?1",
                    [&fact_id],
                )?
                .context("accepted AI fact candidate projection is missing")?;
                if fact.content != desired_content || fact.kind != desired_kind {
                    bail!("accepted AI fact candidate projection conflicts with the review");
                }
                Some(fact)
            } else {
                None
            };
            transaction.commit()?;
            return Ok(CandidateReviewOutcome {
                operation,
                fact,
                insert_outcome: InsertOutcome::Duplicate,
            });
        }

        let now = Utc::now();
        let candidate = operation
            .candidates
            .iter_mut()
            .find(|candidate| candidate.id == candidate_id)
            .context("AI fact candidate projection changed during review")?;
        candidate.status = if accept {
            AiFactCandidateStatusV1::Accepted
        } else {
            AiFactCandidateStatusV1::Rejected
        };
        let mut fact = None;
        if accept {
            candidate.content = desired_content.clone();
            candidate.kind = desired_kind;
            let accepted_fact = FactV1 {
                schema_version: SCHEMA_VERSION_V1,
                id: fact_id,
                repository_id: operation.repository_id.clone(),
                task_id: operation.task_id.clone(),
                kind: desired_kind,
                lifecycle: FactLifecycle::Candidate,
                freshness: Freshness::Fresh,
                origin: FactOriginV1::AiAssisted,
                content: desired_content,
                evidence_ids: Vec::new(),
                superseded_by: None,
                created_at: now,
                updated_at: now,
            };
            let mut event = EventEnvelopeV1::new(
                format!("local-ui:ai-fact-candidate:{}", accepted_fact.id),
                &accepted_fact.repository_id,
                "ai-fact-refresh",
                EventKind::FactCandidate,
                now,
                json!({
                    "fact": accepted_fact,
                    "origin": FactOriginV1::AiAssisted,
                    "operationId": operation.operation_id,
                    "candidateId": candidate_id,
                    "candidateAction": candidate.action
                }),
            );
            event.task_id = Some(operation.task_id.clone());
            event.event_id = deterministic_id("event", &["ai-fact-candidate", &accepted_fact.id]);
            event.dedupe_key = event.event_id.clone();
            self.ensure_event_write_allowed(&event)?;
            let event = prepare_event(&event)?;
            if insert_event_tx(&transaction, &event)? != InsertOutcome::Inserted {
                bail!("AI fact candidate canonical event collided during atomic review");
            }
            fact = Some(accepted_fact);
        }
        operation.updated_at = now;
        let operation_event = ai_fact_refresh_event(&operation);
        self.ensure_event_write_allowed(&operation_event)?;
        let operation_event = prepare_event(&operation_event)?;
        if insert_event_tx(&transaction, &operation_event)? != InsertOutcome::Inserted {
            bail!("AI fact candidate review operation event collided");
        }
        transaction.commit()?;
        Ok(CandidateReviewOutcome {
            operation,
            fact,
            insert_outcome: InsertOutcome::Inserted,
        })
    }

    pub fn get_agent(&self, agent_id: &str) -> Result<Option<AgentV1>> {
        query_json_optional(
            &self.connect()?,
            "SELECT agent_json FROM agents WHERE id = ?1",
            [agent_id],
        )
    }

    pub fn list_agents(&self, repository_id: Option<&str>) -> Result<Vec<AgentV1>> {
        let connection = self.connect()?;
        if let Some(repository_id) = repository_id {
            query_json_rows(
                &connection,
                "SELECT agent_json FROM agents WHERE repository_id = ?1
                 ORDER BY observed_at, thread_id",
                [repository_id],
            )
        } else {
            query_json_rows(
                &connection,
                "SELECT agent_json FROM agents ORDER BY observed_at, thread_id",
                [],
            )
        }
    }

    pub fn append_agent_observation(&self, agent: &AgentV1) -> Result<InsertOutcome> {
        let observed_at = agent.observed_at.timestamp_micros().to_string();
        let payload_fingerprint =
            hex::encode(sha2::Sha256::digest(serde_json::to_vec(agent)?.as_slice()));
        let mut event = EventEnvelopeV1::new(
            format!("codex-app-server:agent:{}", agent.thread_id),
            &agent.repository_id,
            agent.session_id.as_deref().unwrap_or(&agent.thread_id),
            EventKind::AgentObserved,
            agent.observed_at,
            json!({ "agent": agent }),
        );
        event.task_id = agent.task_id.clone();
        event.event_id = deterministic_id(
            "event",
            &[
                &agent.repository_id,
                "agent",
                &agent.thread_id,
                &observed_at,
                &payload_fingerprint,
            ],
        );
        event.dedupe_key = event.event_id.clone();
        self.insert_event(&event)
    }

    pub fn get_task_grouping_operation(
        &self,
        repository_id: Option<&str>,
        operation_id: &str,
    ) -> Result<Option<TaskGroupingOperationV1>> {
        Ok(self
            .list_task_grouping_operations(repository_id)?
            .into_iter()
            .find(|operation| operation.operation_id == operation_id))
    }

    pub fn append_task_grouping_operation(
        &self,
        operation: &TaskGroupingOperationV1,
    ) -> Result<InsertOutcome> {
        Ok(match self.claim_task_grouping_operation(operation)? {
            ClaimOutcome::Claimed(_) => InsertOutcome::Inserted,
            ClaimOutcome::Existing(_) => InsertOutcome::Duplicate,
        })
    }

    pub fn claim_task_grouping_operation(
        &self,
        operation: &TaskGroupingOperationV1,
    ) -> Result<ClaimOutcome<TaskGroupingOperationV1>> {
        let _lock = self.acquire_maintenance_lock()?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        if let Some(existing) = task_grouping_operation_tx(&transaction, &operation.operation_id)? {
            if existing.request_fingerprint != operation.request_fingerprint
                || existing.repository_id != operation.repository_id
                || !same_grouping_operation_request(&existing, operation)
            {
                bail!("operation id already belongs to a different grouping request");
            }
            transaction.commit()?;
            return Ok(ClaimOutcome::Existing(existing));
        }
        let source_task_id = operation
            .session_moves
            .first()
            .map(|movement| movement.from_task_id.as_str())
            .context("grouping operation has no session movements")?;
        let source: TaskV1 = query_json_optional(
            &transaction,
            "SELECT task_json FROM tasks WHERE id = ?1",
            [source_task_id],
        )?
        .with_context(|| format!("grouping source task missing: {source_task_id}"))?;
        if source.repository_id != operation.repository_id {
            bail!("grouping source task crossed repository boundary");
        }
        if operation
            .session_moves
            .iter()
            .any(|movement| movement.from_task_id != source_task_id)
        {
            bail!("grouping operation has inconsistent source tasks");
        }
        validate_grouping_lifecycle_tx(&transaction, operation)?;
        if operation.action == TaskGroupingActionV1::Split {
            let created = operation
                .created_task
                .as_ref()
                .context("split grouping operation omitted its created task")?;
            let expected_target = operation
                .session_moves
                .first()
                .map(|movement| movement.to_task_id.as_str())
                .context("split grouping operation has no target")?;
            if created.id != expected_target
                || created.id
                    != deterministic_id(
                        "task",
                        &[&operation.repository_id, "split", &operation.operation_id],
                    )
                || created.repository_id != operation.repository_id
                || created.lifecycle != TaskLifecycle::Active
                || operation
                    .session_moves
                    .iter()
                    .any(|movement| movement.to_task_id != created.id)
            {
                bail!("split grouping target does not match the canonical created task");
            }
            if query_json_optional::<TaskV1, _>(
                &transaction,
                "SELECT task_json FROM tasks WHERE id = ?1",
                [&created.id],
            )?
            .is_some()
            {
                bail!("split target already exists for this operation id");
            }
        } else {
            let target_ids = operation
                .session_moves
                .iter()
                .map(|movement| movement.to_task_id.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            if target_ids.len() != 1 {
                bail!("grouping operation has inconsistent target tasks");
            }
            for target_id in target_ids {
                let target: TaskV1 = query_json_optional(
                    &transaction,
                    "SELECT task_json FROM tasks WHERE id = ?1",
                    [target_id],
                )?
                .with_context(|| format!("grouping target task missing: {target_id}"))?;
                if target.repository_id != operation.repository_id {
                    bail!("grouping target task crossed repository boundary");
                }
                if target.lifecycle == TaskLifecycle::Abandoned {
                    bail!("an abandoned task cannot receive sessions");
                }
            }
        }
        if operation.action != TaskGroupingActionV1::Undo {
            validate_grouping_fact_impacts_tx(&transaction, operation, source_task_id)?;
        }
        validate_grouping_task_deletions_tx(&transaction, operation)?;
        for movement in &operation.session_moves {
            let session: SessionV1 = query_json_optional(
                &transaction,
                "SELECT session_json FROM sessions WHERE id = ?1",
                [&movement.session_id],
            )?
            .with_context(|| format!("grouping session missing: {}", movement.session_id))?;
            if session.repository_id != operation.repository_id {
                bail!("grouping operation crossed repository boundary");
            }
            if session.task_id.as_deref() != Some(movement.from_task_id.as_str()) {
                bail!(
                    "stale session association for {}: expected {}, found {}",
                    movement.session_id,
                    movement.from_task_id,
                    session.task_id.as_deref().unwrap_or("unlinked")
                );
            }
        }
        let mut event = EventEnvelopeV1::new(
            format!("local-ui:task-grouping:{}", operation.operation_id),
            &operation.repository_id,
            "task-grouping",
            EventKind::TaskGroupingChanged,
            operation.occurred_at,
            json!({ "operation": operation }),
        );
        event.event_id = deterministic_id(
            "event",
            &[
                &operation.repository_id,
                "task-grouping",
                &operation.operation_id,
            ],
        );
        event.dedupe_key = deterministic_id(
            "dedupe",
            &[
                &operation.repository_id,
                "task-grouping",
                &operation.operation_id,
            ],
        );
        self.ensure_event_write_allowed(&event)?;
        let event = prepare_event(&event)?;
        if insert_event_tx(&transaction, &event)? != InsertOutcome::Inserted {
            bail!("grouping canonical event collided without a matching operation");
        }
        transaction.commit()?;
        Ok(ClaimOutcome::Claimed(operation.clone()))
    }
}

fn ai_fact_refresh_event(operation: &AiFactRefreshOperationV1) -> EventEnvelopeV1 {
    let mut event = EventEnvelopeV1::new(
        format!(
            "local-ui:ai-fact-refresh:{}:{}",
            operation.operation_id,
            operation.updated_at.timestamp_micros()
        ),
        &operation.repository_id,
        "ai-fact-refresh",
        EventKind::AiFactRefreshOperationRecorded,
        operation.updated_at,
        json!({ "operation": operation }),
    );
    event.task_id = Some(operation.task_id.clone());
    event.event_id = deterministic_id(
        "event",
        &[
            &operation.repository_id,
            "ai-fact-refresh",
            &operation.operation_id,
            &operation.updated_at.timestamp_micros().to_string(),
        ],
    );
    event.dedupe_key = event.event_id.clone();
    event
}

fn original_ai_fact_candidate_tx(
    transaction: &Transaction<'_>,
    operation_id: &str,
    candidate_id: &str,
) -> Result<Option<AiFactCandidateV1>> {
    for event in load_events(transaction, None)? {
        if event.kind != EventKind::AiFactRefreshOperationRecorded {
            continue;
        }
        let Some(operation) = payload_as::<AiFactRefreshOperationV1>(&event.payload, "operation")
        else {
            continue;
        };
        if operation.operation_id != operation_id {
            continue;
        }
        if let Some(candidate) = operation.candidates.into_iter().find(|candidate| {
            candidate.id == candidate_id && candidate.status == AiFactCandidateStatusV1::Pending
        }) {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn task_grouping_operation_tx(
    transaction: &Transaction<'_>,
    operation_id: &str,
) -> Result<Option<TaskGroupingOperationV1>> {
    Ok(load_events(transaction, None)?
        .into_iter()
        .filter(|event| event.kind == EventKind::TaskGroupingChanged)
        .filter_map(|event| payload_as::<TaskGroupingOperationV1>(&event.payload, "operation"))
        .find(|operation| operation.operation_id == operation_id))
}

fn same_grouping_operation_request(
    existing: &TaskGroupingOperationV1,
    requested: &TaskGroupingOperationV1,
) -> bool {
    let mut requested = requested.clone();
    requested.occurred_at = existing.occurred_at;
    match (
        existing.created_task.as_ref(),
        requested.created_task.as_mut(),
    ) {
        (Some(existing_task), Some(requested_task)) => {
            requested_task.created_at = existing_task.created_at;
            requested_task.updated_at = existing_task.updated_at;
        }
        (None, None) => {}
        _ => return false,
    }
    existing == &requested
}

fn validate_grouping_lifecycle_tx(
    transaction: &Transaction<'_>,
    operation: &TaskGroupingOperationV1,
) -> Result<()> {
    for snapshot in &operation.task_lifecycle {
        let current = query_json_optional::<TaskV1, _>(
            transaction,
            "SELECT task_json FROM tasks WHERE id = ?1",
            [&snapshot.task_id],
        )?;
        match (snapshot.before, current) {
            (Some(expected), Some(task)) if task.lifecycle == expected => {}
            (None, None) => {}
            (Some(expected), Some(task)) => bail!(
                "stale task lifecycle for {}: expected {:?}, found {:?}",
                snapshot.task_id,
                expected,
                task.lifecycle
            ),
            (Some(_), None) => bail!("grouping lifecycle task missing: {}", snapshot.task_id),
            (None, Some(_)) => bail!(
                "grouping lifecycle task already exists: {}",
                snapshot.task_id
            ),
        }
    }
    if operation.action == TaskGroupingActionV1::Merge
        && operation
            .task_lifecycle
            .iter()
            .any(|snapshot| snapshot.after == Some(TaskLifecycle::Completed))
    {
        let source_task_id = operation
            .session_moves
            .first()
            .map(|movement| movement.from_task_id.as_str())
            .context("merge grouping operation has no source task")?;
        let moved = operation
            .session_moves
            .iter()
            .map(|movement| movement.session_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let sessions = query_json_rows::<SessionV1, _>(
            transaction,
            "SELECT session_json FROM sessions WHERE task_id = ?1 ORDER BY started_at, id",
            [source_task_id],
        )?;
        if sessions
            .iter()
            .any(|session| !moved.contains(session.id.as_str()))
        {
            bail!("stale merge preview would complete a task with remaining sessions");
        }
    }
    Ok(())
}

fn validate_grouping_fact_impacts_tx(
    transaction: &Transaction<'_>,
    operation: &TaskGroupingOperationV1,
    source_task_id: &str,
) -> Result<()> {
    let moved = operation
        .session_moves
        .iter()
        .map(|movement| movement.session_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let target_task_id = operation
        .session_moves
        .first()
        .map(|movement| movement.to_task_id.as_str())
        .context("grouping operation has no target task")?;
    let evidence = query_json_rows::<EvidenceV1, _>(
        transaction,
        "SELECT evidence_json FROM evidence WHERE task_id = ?1 ORDER BY created_at, id",
        [source_task_id],
    )?
    .into_iter()
    .map(|evidence| (evidence.id.clone(), evidence))
    .collect::<std::collections::BTreeMap<_, _>>();
    let mut current = Vec::new();
    for fact in query_json_rows::<FactV1, _>(
        transaction,
        "SELECT fact_json FROM facts WHERE task_id = ?1 ORDER BY updated_at, id",
        [source_task_id],
    )? {
        let provenance_sessions = fact
            .evidence_ids
            .iter()
            .filter_map(|id| evidence.get(id))
            .map(|evidence| evidence.session_id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        if provenance_sessions.is_empty()
            || provenance_sessions
                .iter()
                .all(|session_id| !moved.contains(session_id.as_str()))
        {
            continue;
        }
        let all_moved = provenance_sessions
            .iter()
            .all(|session_id| moved.contains(session_id.as_str()));
        current.push(crate::domain::FactGroupingImpactV1 {
            fact_id: fact.id,
            from_task_id: source_task_id.to_string(),
            to_task_id: all_moved.then(|| target_task_id.to_string()),
            mixed_provenance: !all_moved,
            session_ids: provenance_sessions.into_iter().collect(),
        });
    }
    current.sort_by(|left, right| left.fact_id.cmp(&right.fact_id));
    if current != operation.fact_impacts {
        bail!("stale fact provenance requires a new grouping preview");
    }
    Ok(())
}

fn validate_grouping_task_deletions_tx(
    transaction: &Transaction<'_>,
    operation: &TaskGroupingOperationV1,
) -> Result<()> {
    let moved_sessions = operation
        .session_moves
        .iter()
        .map(|movement| movement.session_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let moved_facts = operation
        .fact_impacts
        .iter()
        .filter(|impact| impact.to_task_id.is_some())
        .map(|impact| impact.fact_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for snapshot in operation
        .task_lifecycle
        .iter()
        .filter(|snapshot| snapshot.after.is_none())
    {
        let task_id = snapshot.task_id.as_str();
        let sessions = query_json_rows::<SessionV1, _>(
            transaction,
            "SELECT session_json FROM sessions WHERE task_id = ?1 ORDER BY started_at, id",
            [task_id],
        )?;
        if sessions
            .iter()
            .any(|session| !moved_sessions.contains(session.id.as_str()))
        {
            bail!("cannot delete split task with additional sessions");
        }
        let facts = query_json_rows::<FactV1, _>(
            transaction,
            "SELECT fact_json FROM facts WHERE task_id = ?1 ORDER BY updated_at, id",
            [task_id],
        )?;
        if facts
            .iter()
            .any(|fact| !moved_facts.contains(fact.id.as_str()))
        {
            bail!("cannot delete split task with additional facts");
        }
        for (table, session_column) in [
            ("checkpoints", "session_id"),
            ("evidence", "session_id"),
            ("file_changes", "session_id"),
            ("test_results", "session_id"),
        ] {
            let sql = format!("SELECT {session_column} FROM {table} WHERE task_id = ?1");
            let mut statement = transaction.prepare(&sql)?;
            let rows = statement.query_map([task_id], |row| row.get::<_, String>(0))?;
            for session_id in rows {
                if !moved_sessions.contains(session_id?.as_str()) {
                    bail!("cannot delete split task with additional projections");
                }
            }
        }
        for table in [
            "regression_candidates",
            "contract_evaluations",
            "ai_fact_refresh_operations",
            "agents",
        ] {
            let sql = format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE task_id = ?1)");
            if transaction.query_row(&sql, [task_id], |row| row.get::<_, bool>(0))? {
                bail!("cannot delete split task with additional projections");
            }
        }
    }
    Ok(())
}
