use crate::contracts::{ContractEvaluationV1, RegressionCandidateV1};
use crate::domain::{
    deterministic_id, AgentV1, AiFactCandidateStatusV1, AiFactCandidateV1,
    AiFactRefreshOperationV1, AiFactRefreshStatusV1, CheckpointV1, ContextUsageV1,
    ContinuationAdviceV1, ContinuationReasonV1, ContinuationStateV1, EventEnvelopeV1, EventKind,
    EvidenceV1, FactKind, FactLifecycle, FactOriginV1, FactV1, FileChangeV1, Freshness,
    GitSnapshotV1, RepositoryV1, SessionV1, TaskGroupingActionV1, TaskGroupingOperationV1,
    TaskLifecycle, TaskSuggestionV1, TaskTimelineV1, TaskV1, TestResultV1, SCHEMA_VERSION_V1,
};
use crate::redaction::{redact_excerpt, redact_text};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, SecondsFormat, Utc};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Transaction};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Digest;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

mod projection;

use projection::{
    insert_event_tx, load_events, prepare_event, rebuild_projections_tx, upsert_checkpoint_tx,
    upsert_evidence_tx, upsert_fact_tx, upsert_file_change_tx, upsert_repository_tx,
    upsert_session_tx, upsert_task_tx, upsert_test_result_tx,
};

const DATABASE_SCHEMA_VERSION: i64 = 1;
const PURGE_TOMBSTONE_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct Store {
    path: PathBuf,
    read_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertOutcome {
    Inserted,
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome<T> {
    Claimed(T),
    Existing(T),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateReviewOutcome {
    pub operation: AiFactRefreshOperationV1,
    pub fact: Option<FactV1>,
    pub insert_outcome: InsertOutcome,
}

#[derive(Debug, Clone, Copy, Default)]
struct InsertFault {
    before_insert: bool,
    before_commit: bool,
    after_commit: bool,
}

pub fn is_sqlite_full(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<rusqlite::Error>()
            .is_some_and(|error| {
                matches!(
                    error,
                    rusqlite::Error::SqliteFailure(inner, _)
                        if inner.code == rusqlite::ErrorCode::DiskFull
                )
            })
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreHealth {
    pub schema_version: i64,
    pub journal_mode: String,
    pub integrity_check: String,
    pub canonical_event_count: u64,
    pub projection_task_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionReport {
    pub retained_events: usize,
    pub removed_events: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PurgeJournalPhase {
    Tombstoned,
    RelatedDataPurged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PurgeRecoveryJournalV1 {
    version: u32,
    repository_id: String,
    phase: PurgeJournalPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RepositoryPurgeTombstoneV1 {
    version: u32,
    generation: String,
    created_at: String,
}

/// Reject data for a repository that has been explicitly purged. The marker deliberately stores
/// only a hash of the repository identity so purge does not retain the deleted local path.
pub fn ensure_repository_not_purged(data_dir: &Path, repository_id: &str) -> Result<()> {
    validate_private_directory(data_dir, "PreviouslyOn data directory")?;
    let tombstone_dir = data_dir.join("purge-tombstones");
    match fs::symlink_metadata(&tombstone_dir) {
        Ok(_) => ensure_private_directory(&tombstone_dir, "purge tombstone directory")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let path = repository_tombstone_path(data_dir, repository_id);
    if validate_private_regular_file(&path, "repository purge tombstone")? {
        bail!("repository {repository_id} was purged; run setup again before capturing new data");
    }

    let journal_path = data_dir.join("previously.sqlite3.purge-recovery.json");
    let journal_bytes = match read_private_file(&journal_path, "purge recovery journal")? {
        Some(bytes) => bytes,
        None => return Ok(()),
    };
    let journal: PurgeRecoveryJournalV1 = serde_json::from_slice(&journal_bytes)
        .with_context(|| format!("parse purge recovery journal {}", journal_path.display()))?;
    if journal.version != 1 {
        bail!("unsupported purge recovery journal version");
    }
    if journal.repository_id == repository_id {
        bail!("repository {repository_id} is being purged; capture remains disabled");
    }
    Ok(())
}

/// A successful, explicit setup is the only operation that re-authorizes capture after purge.
pub fn reactivate_repository(data_dir: &Path, repository_id: &str) -> Result<()> {
    ensure_private_directory(data_dir, "PreviouslyOn data directory")?;
    let database = data_dir.join("previously.sqlite3");
    let _lock = acquire_database_lock(&database)?;
    let journal_path = data_dir.join("previously.sqlite3.purge-recovery.json");
    if validate_private_regular_file(&journal_path, "purge recovery journal")? {
        bail!("cannot reactivate repository while purge recovery is pending; rerun purge first");
    }
    let tombstone_dir = data_dir.join("purge-tombstones");
    match fs::symlink_metadata(&tombstone_dir) {
        Ok(_) => ensure_private_directory(&tombstone_dir, "purge tombstone directory")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let tombstone = repository_tombstone_path(data_dir, repository_id);
    validate_private_regular_file(&tombstone, "repository purge tombstone")?;
    remove_file_and_sync_parent(&tombstone)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskSearchHit {
    pub task: TaskV1,
    pub rank: f64,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let requested_path = path.as_ref();
        let parent = requested_path
            .parent()
            .context("database path has no parent")?;
        ensure_private_directory(parent, "PreviouslyOn data directory")?;
        let file_name = requested_path
            .file_name()
            .context("database path has no file name")?;
        let path = parent
            .canonicalize()
            .with_context(|| format!("canonicalize database directory {}", parent.display()))?
            .join(file_name);
        validate_private_directory(
            path.parent()
                .context("canonical database path has no parent")?,
            "PreviouslyOn data directory",
        )?;
        ensure_private_regular_file(&path, "PreviouslyOn database")?;
        validate_database_companions(&path)?;
        let store = Self {
            path,
            read_only: false,
        };
        let mut connection = store.connect()?;
        store.migrate(&mut connection)?;
        store.recover_purge_if_ready()?;
        Ok(store)
    }

    /// Open an existing database without creating files, migrating schemas, recovering journals,
    /// or enabling SQLite write paths. Intended for user-reviewed local diagnostics.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self> {
        let requested_path = path.as_ref();
        let parent = requested_path
            .parent()
            .context("database path has no parent")?;
        validate_private_directory(parent, "PreviouslyOn data directory")?;
        let file_name = requested_path
            .file_name()
            .context("database path has no file name")?;
        let path = parent
            .canonicalize()
            .with_context(|| format!("canonicalize database directory {}", parent.display()))?
            .join(file_name);
        if !validate_private_regular_file(&path, "PreviouslyOn database")? {
            bail!("PreviouslyOn database does not exist");
        }
        validate_database_companions(&path)?;
        let store = Self {
            path,
            read_only: true,
        };
        let connection = store.connect()?;
        let schema_version: i64 =
            connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if schema_version != DATABASE_SCHEMA_VERSION {
            bail!(
                "unsupported database schema version {schema_version}; expected {DATABASE_SCHEMA_VERSION}"
            );
        }
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn insert_event(&self, event: &EventEnvelopeV1) -> Result<InsertOutcome> {
        self.insert_event_inner(event, InsertFault::default())
    }

    fn insert_event_inner(
        &self,
        event: &EventEnvelopeV1,
        fault: InsertFault,
    ) -> Result<InsertOutcome> {
        let _lock = self.acquire_maintenance_lock()?;
        self.ensure_event_write_allowed(event)?;
        let event = prepare_event(event)?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        if fault.before_insert {
            bail!("injected failure before canonical insert");
        }
        let outcome = insert_event_tx(&transaction, &event)?;
        if fault.before_commit {
            bail!("injected failure before canonical commit");
        }
        transaction.commit()?;
        if fault.after_commit {
            bail!("injected failure after canonical commit");
        }
        Ok(outcome)
    }

    fn ensure_event_write_allowed(&self, event: &EventEnvelopeV1) -> Result<()> {
        ensure_repository_not_purged(
            self.path.parent().context("database path has no parent")?,
            &event.repository_id,
        )?;
        if self
            .read_purge_journal()?
            .is_some_and(|journal| journal.repository_id == event.repository_id)
        {
            bail!(
                "repository {} is tombstoned by an incomplete purge; resume purge before ingestion",
                event.repository_id
            );
        }
        if event.schema_version != SCHEMA_VERSION_V1 {
            bail!(
                "unsupported canonical event schema version {}; expected {}",
                event.schema_version,
                SCHEMA_VERSION_V1
            );
        }
        Ok(())
    }

    pub fn list_events(&self, repository_id: Option<&str>) -> Result<Vec<EventEnvelopeV1>> {
        let connection = self.connect()?;
        load_events(&connection, repository_id)
    }

    pub fn rebuild_projections(&self) -> Result<()> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        rebuild_projections_tx(&transaction)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn project_checkpoint(
        &self,
        repository_id: &str,
        session_id: &str,
        git_before: Option<GitSnapshotV1>,
        git_after: GitSnapshotV1,
        changes: Vec<FileChangeV1>,
        tests: Vec<TestResultV1>,
    ) -> Result<CheckpointV1> {
        let events = self
            .list_events(Some(repository_id))?
            .into_iter()
            .filter(|event| event.session_id == session_id)
            .collect::<Vec<_>>();
        let checkpoint = CheckpointV1::project(&events, git_before, git_after, changes, tests);
        self.upsert_checkpoint(&checkpoint)?;
        Ok(checkpoint)
    }

    pub fn upsert_repository(&self, repository: &RepositoryV1) -> Result<()> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        upsert_repository_tx(&transaction, repository)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn upsert_task(&self, task: &TaskV1) -> Result<()> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        upsert_task_tx(&transaction, task)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn upsert_session(&self, session: &SessionV1) -> Result<()> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        upsert_session_tx(&transaction, session)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn upsert_checkpoint(&self, checkpoint: &CheckpointV1) -> Result<()> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        upsert_checkpoint_tx(&transaction, checkpoint)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn upsert_fact(&self, fact: &FactV1) -> Result<()> {
        let mut fact = fact.clone();
        fact.content = redact_text(&fact.content);
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        upsert_fact_tx(&transaction, &fact)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn upsert_evidence(&self, evidence: &EvidenceV1) -> Result<()> {
        let mut evidence = evidence.clone();
        evidence.excerpt = redact_excerpt(&evidence.excerpt);
        evidence.excerpt_sha256 = hex::encode(sha2::Sha256::digest(evidence.excerpt.as_bytes()));
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        upsert_evidence_tx(&transaction, &evidence)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn upsert_file_change(&self, change: &FileChangeV1) -> Result<()> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        upsert_file_change_tx(&transaction, change)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn upsert_test_result(&self, test: &TestResultV1) -> Result<()> {
        let mut test = test.clone();
        test.command = redact_text(&test.command);
        test.summary = test.summary.map(|summary| redact_excerpt(&summary));
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        upsert_test_result_tx(&transaction, &test)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn list_repositories(&self) -> Result<Vec<RepositoryV1>> {
        query_json_rows(
            &self.connect()?,
            "SELECT repository_json FROM repositories ORDER BY path, id",
            [],
        )
    }

    pub fn get_task(&self, task_id: &str) -> Result<Option<TaskV1>> {
        query_json_optional(
            &self.connect()?,
            "SELECT task_json FROM tasks WHERE id = ?1",
            [task_id],
        )
    }

    pub fn get_session(&self, session_id: &str) -> Result<Option<SessionV1>> {
        query_json_optional(
            &self.connect()?,
            "SELECT session_json FROM sessions WHERE id = ?1",
            [session_id],
        )
    }

    pub fn list_tasks(&self, repository_id: Option<&str>) -> Result<Vec<TaskV1>> {
        let connection = self.connect()?;
        if let Some(repository_id) = repository_id {
            query_json_rows(
                &connection,
                "SELECT task_json FROM tasks WHERE repository_id = ?1 ORDER BY updated_at DESC, id",
                [repository_id],
            )
        } else {
            query_json_rows(
                &connection,
                "SELECT task_json FROM tasks ORDER BY updated_at DESC, id",
                [],
            )
        }
    }

    pub fn list_sessions(&self, repository_id: Option<&str>) -> Result<Vec<SessionV1>> {
        let connection = self.connect()?;
        if let Some(repository_id) = repository_id {
            query_json_rows(
                &connection,
                "SELECT session_json FROM sessions WHERE repository_id = ?1 ORDER BY started_at, id",
                [repository_id],
            )
        } else {
            query_json_rows(
                &connection,
                "SELECT session_json FROM sessions ORDER BY started_at, id",
                [],
            )
        }
    }

    pub fn list_sessions_for_task(&self, task_id: &str) -> Result<Vec<SessionV1>> {
        query_json_rows(
            &self.connect()?,
            "SELECT session_json FROM sessions WHERE task_id = ?1 ORDER BY started_at, id",
            [task_id],
        )
    }

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

    pub fn list_session_events(
        &self,
        repository_id: &str,
        session_id: &str,
    ) -> Result<Vec<EventEnvelopeV1>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT event_json FROM canonical_events
             WHERE repository_id = ?1 AND session_id = ?2
             ORDER BY occurred_at, COALESCE(sequence_no, 9223372036854775807), event_id",
        )?;
        let rows = statement.query_map(params![repository_id, session_id], |row| {
            row.get::<_, String>(0)
        })?;
        rows.map(|row| {
            let json = row?;
            serde_json::from_str(&json).context("deserialize session event")
        })
        .collect()
    }

    pub fn list_task_events(
        &self,
        repository_id: &str,
        task_id: &str,
    ) -> Result<Vec<EventEnvelopeV1>> {
        let events = self.list_events(Some(repository_id))?;
        let mut effective = Vec::new();
        for event in events {
            let belongs =
                if event.kind == EventKind::TaskGroupingChanged {
                    payload_as::<TaskGroupingOperationV1>(&event.payload, "operation").is_some_and(
                        |operation| {
                            operation.session_moves.iter().any(|item| {
                                item.from_task_id == task_id || item.to_task_id == task_id
                            }) || operation
                                .task_lifecycle
                                .iter()
                                .any(|item| item.task_id == task_id)
                        },
                    )
                } else if event.kind == EventKind::FactDeprecated {
                    event
                        .payload
                        .get("fact_id")
                        .and_then(Value::as_str)
                        .and_then(|fact_id| self.get_fact(fact_id).ok().flatten())
                        .is_some_and(|fact| fact.task_id == task_id)
                } else if !matches!(event.session_id.as_str(), "local-ui" | "task-grouping") {
                    self.get_session(&event.session_id)?
                        .and_then(|session| session.task_id)
                        .as_deref()
                        == Some(task_id)
                } else {
                    event.task_id.as_deref() == Some(task_id)
                };
            if belongs {
                effective.push(event);
            }
        }
        Ok(effective)
    }

    pub fn session_event_count(&self, session_id: &str, kind: EventKind) -> Result<u64> {
        let connection = self.connect()?;
        connection
            .query_row(
                "SELECT COUNT(*) FROM canonical_events WHERE session_id = ?1 AND kind = ?2",
                params![session_id, enum_text(kind)?],
                |row| row.get(0),
            )
            .context("count session events")
    }

    pub fn get_fact(&self, fact_id: &str) -> Result<Option<FactV1>> {
        query_json_optional(
            &self.connect()?,
            "SELECT fact_json FROM facts WHERE id = ?1",
            [fact_id],
        )
    }

    pub fn get_evidence(&self, evidence_id: &str) -> Result<Option<EvidenceV1>> {
        query_json_optional(
            &self.connect()?,
            "SELECT evidence_json FROM evidence WHERE id = ?1",
            [evidence_id],
        )
    }

    pub fn list_facts(&self, task_id: &str) -> Result<Vec<FactV1>> {
        query_json_rows(
            &self.connect()?,
            "SELECT fact_json FROM facts WHERE task_id = ?1 ORDER BY updated_at DESC, id",
            [task_id],
        )
    }

    pub fn list_evidence(&self, task_id: &str) -> Result<Vec<EvidenceV1>> {
        query_json_rows(
            &self.connect()?,
            "SELECT evidence_json FROM evidence WHERE task_id = ?1 ORDER BY created_at, id",
            [task_id],
        )
    }

    pub fn list_checkpoints(&self, task_id: &str) -> Result<Vec<CheckpointV1>> {
        query_json_rows(
            &self.connect()?,
            "SELECT checkpoint_json FROM checkpoints WHERE task_id = ?1 ORDER BY created_at, id",
            [task_id],
        )
    }

    pub fn list_file_changes(&self, task_id: &str) -> Result<Vec<FileChangeV1>> {
        query_json_rows(
            &self.connect()?,
            "SELECT change_json FROM file_changes WHERE task_id = ?1 ORDER BY path, id",
            [task_id],
        )
    }

    pub fn list_test_results(&self, task_id: &str) -> Result<Vec<TestResultV1>> {
        query_json_rows(
            &self.connect()?,
            "SELECT test_json FROM test_results WHERE task_id = ?1 ORDER BY occurred_at DESC, id",
            [task_id],
        )
    }

    pub fn get_regression_candidate(
        &self,
        candidate_id: &str,
    ) -> Result<Option<RegressionCandidateV1>> {
        query_json_optional(
            &self.connect()?,
            "SELECT candidate_json FROM regression_candidates WHERE id = ?1",
            [candidate_id],
        )
    }

    pub fn list_regression_candidates(
        &self,
        repository_id: Option<&str>,
    ) -> Result<Vec<RegressionCandidateV1>> {
        let connection = self.connect()?;
        if let Some(repository_id) = repository_id {
            query_json_rows(
                &connection,
                "SELECT candidate_json FROM regression_candidates
                 WHERE repository_id = ?1 ORDER BY updated_at DESC, id",
                [repository_id],
            )
        } else {
            query_json_rows(
                &connection,
                "SELECT candidate_json FROM regression_candidates ORDER BY updated_at DESC, id",
                [],
            )
        }
    }

    pub fn get_contract_evaluation(
        &self,
        evaluation_id: &str,
    ) -> Result<Option<ContractEvaluationV1>> {
        query_json_optional(
            &self.connect()?,
            "SELECT evaluation_json FROM contract_evaluations WHERE id = ?1",
            [evaluation_id],
        )
    }

    pub fn list_contract_evaluations(
        &self,
        repository_id: Option<&str>,
    ) -> Result<Vec<ContractEvaluationV1>> {
        let connection = self.connect()?;
        if let Some(repository_id) = repository_id {
            query_json_rows(
                &connection,
                "SELECT evaluation_json FROM contract_evaluations
                 WHERE repository_id = ?1 ORDER BY evaluated_at DESC, id",
                [repository_id],
            )
        } else {
            query_json_rows(
                &connection,
                "SELECT evaluation_json FROM contract_evaluations ORDER BY evaluated_at DESC, id",
                [],
            )
        }
    }

    pub fn get_task_timeline(&self, task_id: &str) -> Result<Option<TaskTimelineV1>> {
        let Some(task) = self.get_task(task_id)? else {
            return Ok(None);
        };
        let sessions = query_json_rows(
            &self.connect()?,
            "SELECT session_json FROM sessions WHERE task_id = ?1 ORDER BY started_at, id",
            [task_id],
        )?;
        Ok(Some(TaskTimelineV1 {
            task,
            sessions,
            checkpoints: self.list_checkpoints(task_id)?,
            facts: self.list_facts(task_id)?,
        }))
    }

    pub fn search_tasks(&self, query: &str, limit: usize) -> Result<Vec<TaskSearchHit>> {
        let connection = self.connect()?;
        let query = fts_query(query);
        if query.is_empty() {
            let tasks: Vec<TaskV1> = query_json_rows(
                &connection,
                "SELECT task_json FROM tasks ORDER BY updated_at DESC, id LIMIT ?1",
                [limit as i64],
            )?;
            return Ok(tasks
                .into_iter()
                .enumerate()
                .map(|(index, task)| TaskSearchHit {
                    task,
                    rank: index as f64,
                })
                .collect());
        }
        let mut statement = connection.prepare(
            "SELECT tasks.task_json, bm25(task_fts) AS rank
             FROM task_fts JOIN tasks ON tasks.id = task_fts.task_id
             WHERE task_fts MATCH ?1
             ORDER BY rank, tasks.updated_at DESC, tasks.id
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![query, limit as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })?;
        rows.map(|row| {
            let (json, rank) = row?;
            Ok(TaskSearchHit {
                task: serde_json::from_str(&json).context("deserialize task search result")?,
                rank,
            })
        })
        .collect()
    }

    pub fn search_facts(&self, query: &str, limit: usize) -> Result<Vec<FactV1>> {
        let query = fts_query(query);
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT facts.fact_json
             FROM fact_fts JOIN facts ON facts.id = fact_fts.fact_id
             WHERE fact_fts MATCH ?1
             ORDER BY bm25(fact_fts), facts.updated_at DESC, facts.id
             LIMIT ?2",
        )?;
        let rows =
            statement.query_map(params![query, limit as i64], |row| row.get::<_, String>(0))?;
        rows.map(|row| {
            let json = row?;
            serde_json::from_str(&json).context("deserialize fact search result")
        })
        .collect()
    }

    pub fn suggest_resume(
        &self,
        repository_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<TaskSuggestionV1>> {
        self.suggest_resume_for_branch(repository_id, query, limit, None)
    }

    pub fn suggest_resume_for_branch(
        &self,
        repository_id: &str,
        query: &str,
        limit: usize,
        current_branch: Option<&str>,
    ) -> Result<Vec<TaskSuggestionV1>> {
        let now = Utc::now();
        let repository_path = self
            .list_repositories()?
            .into_iter()
            .find(|repository| repository.id == repository_id)
            .map(|repository| repository.path);
        let current_snapshot = repository_path
            .as_deref()
            .and_then(|path| crate::git::capture_snapshot(path).ok());
        let effective_branch = current_branch.or_else(|| {
            current_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.branch.as_deref())
        });
        let current_head = current_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.head.as_deref());
        let mut suggestions = Vec::new();
        for hit in self.search_tasks(query, limit.saturating_mul(3).max(limit))? {
            if hit.task.repository_id != repository_id
                || hit.task.lifecycle != TaskLifecycle::Active
            {
                continue;
            }
            let age_days = now
                .signed_duration_since(hit.task.updated_at)
                .num_days()
                .max(0);
            let recency = (1.0 / (1.0 + age_days as f64 / 30.0)).min(1.0);
            let lexical = lexical_overlap(
                query,
                &format!(
                    "{} {}",
                    hit.task.title,
                    hit.task.goal.as_deref().unwrap_or_default()
                ),
            );
            let branch_match = effective_branch
                .zip(hit.task.branch.as_deref())
                .is_some_and(|(current, task)| current == task);
            let task_head = self
                .list_checkpoints(&hit.task.id)?
                .last()
                .and_then(|checkpoint| checkpoint.git_after.head.clone());
            let branch_ancestor = !branch_match
                && task_head
                    .as_deref()
                    .zip(current_head)
                    .is_some_and(|(ancestor, descendant)| {
                        repository_path.as_deref().is_some_and(|path| {
                            crate::git::is_ancestor(path, ancestor, descendant).unwrap_or(false)
                        })
                    });
            let mut matching_reasons = vec!["same_repository".to_string()];
            if lexical > 0.0 {
                matching_reasons.push("full_text_match".to_string());
            }
            if branch_match {
                matching_reasons.push("same_branch".to_string());
            } else if branch_ancestor {
                matching_reasons.push("branch_ancestor".to_string());
            }
            let latest_session = self.get_task_timeline(&hit.task.id)?.and_then(|timeline| {
                timeline
                    .sessions
                    .into_iter()
                    .max_by_key(|session| session.last_activity_at.unwrap_or(session.started_at))
            });
            let continuation_advice = latest_session.and_then(|session| {
                (session.continuation_state != ContinuationStateV1::Normal).then(|| {
                    let mut reasons = Vec::new();
                    if session.compaction_count >= crate::domain::PROVISIONAL_COMPACTION_THRESHOLD {
                        reasons.push(ContinuationReasonV1::CompactionLimit);
                    }
                    if session
                        .context_usage
                        .as_ref()
                        .and_then(ContextUsageV1::utilization)
                        .is_some_and(|ratio| {
                            ratio >= crate::domain::PROVISIONAL_CONTEXT_USAGE_THRESHOLD
                        })
                    {
                        reasons.push(ContinuationReasonV1::ContextUsageLimit);
                    }
                    ContinuationAdviceV1 {
                        action: "new_thread".to_string(),
                        reasons,
                        task_id: hit.task.id.clone(),
                        task_title: hit.task.title.clone(),
                        last_activity_at: session.last_activity_at.unwrap_or(session.started_at),
                        compaction_count: session.compaction_count,
                        context_usage: session.context_usage,
                    }
                })
            });
            suggestions.push(TaskSuggestionV1 {
                task_id: hit.task.id,
                title: hit.task.title,
                score: (0.65 * lexical
                    + 0.20 * recency
                    + if branch_match || branch_ancestor {
                        0.15
                    } else {
                        0.0
                    })
                .clamp(0.0, 1.0),
                last_activity_at: hit.task.updated_at,
                matching_reasons,
                continuation_advice,
            });
        }
        suggestions.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| b.last_activity_at.cmp(&a.last_activity_at))
                .then_with(|| a.task_id.cmp(&b.task_id))
        });
        suggestions.truncate(limit);
        Ok(suggestions)
    }

    pub fn export_json(&self, repository_id: Option<&str>) -> Result<Value> {
        let events = self.list_events(repository_id)?;
        let repositories = self
            .list_repositories()?
            .into_iter()
            .filter(|repository| repository_id.map(|id| id == repository.id).unwrap_or(true))
            .collect::<Vec<_>>();
        let connection = self.connect()?;
        let mut tasks: Vec<TaskV1> = query_json_rows(
            &connection,
            "SELECT task_json FROM tasks ORDER BY repository_id, updated_at, id",
            [],
        )?;
        tasks.retain(|task| {
            repository_id
                .map(|id| id == task.repository_id)
                .unwrap_or(true)
        });
        let task_ids = tasks.iter().map(|task| task.id.clone()).collect::<Vec<_>>();
        let mut sessions = Vec::new();
        let mut checkpoints = Vec::new();
        let mut facts = Vec::new();
        let mut evidence = Vec::new();
        let mut file_changes = Vec::new();
        let mut test_results = Vec::new();
        for task_id in task_ids {
            if let Some(timeline) = self.get_task_timeline(&task_id)? {
                sessions.extend(timeline.sessions);
                checkpoints.extend(timeline.checkpoints);
                facts.extend(timeline.facts);
            }
            evidence.extend(self.list_evidence(&task_id)?);
            file_changes.extend(self.list_file_changes(&task_id)?);
            test_results.extend(self.list_test_results(&task_id)?);
        }
        let regression_candidates = self.list_regression_candidates(repository_id)?;
        let contract_evaluations = self.list_contract_evaluations(repository_id)?;
        let fact_refresh_operations = self.list_ai_fact_refresh_operations(repository_id)?;
        let agents = self.list_agents(repository_id)?;
        Ok(json!({
            "schema_version": SCHEMA_VERSION_V1,
            "exported_at": timestamp(Utc::now()),
            "repositories": repositories,
            "tasks": tasks,
            "sessions": sessions,
            "canonical_events": events,
            "checkpoints": checkpoints,
            "facts": facts,
            "evidence": evidence,
            "file_changes": file_changes,
            "test_results": test_results,
            "regressionCandidates": regression_candidates,
            "contractEvaluations": contract_evaluations,
            "factRefreshOperations": fact_refresh_operations,
            "agents": agents,
        }))
    }

    pub fn purge_repository(&self, repository_id: &str) -> Result<()> {
        self.purge_repository_with(repository_id, || {
            self.purge_recovery_queue_files(repository_id)
        })
    }

    pub fn purge_repository_with<F>(&self, repository_id: &str, purge_related: F) -> Result<()>
    where
        F: FnOnce() -> Result<()>,
    {
        let _lock = self.acquire_maintenance_lock()?;
        let existing_journal = self.read_purge_journal()?;
        if let Some(existing) = existing_journal.as_ref() {
            if existing.repository_id != repository_id {
                bail!(
                    "cannot purge repository {repository_id}; purge for {} is already pending",
                    existing.repository_id
                );
            }
        }
        let mut journal = match existing_journal {
            Some(existing) => existing,
            None => {
                let journal = PurgeRecoveryJournalV1 {
                    version: 1,
                    repository_id: repository_id.to_string(),
                    phase: PurgeJournalPhase::Tombstoned,
                };
                self.write_purge_journal(&journal)?;
                journal
            }
        };
        // This marker outlives the recovery journal. It closes the race where a hook begins an
        // append during purge, waits for the maintenance lock, and would otherwise write after
        // the journal had been removed. The recovery journal is written first so every crash
        // boundary has an automatic completion path; queue append treats either file as a gate.
        self.write_repository_tombstone(repository_id)?;
        // Queue/cache cleanup must finish before the canonical DB is replaced. If this callback
        // fails, the durable tombstone remains and blocks ingestion/replay for this repository.
        if journal.phase == PurgeJournalPhase::Tombstoned {
            purge_related()?;
            journal.phase = PurgeJournalPhase::RelatedDataPurged;
            self.write_purge_journal(&journal)?;
        }
        let retained = self
            .list_events(None)?
            .into_iter()
            .filter(|event| event.repository_id != repository_id)
            .collect::<Vec<_>>();
        self.replace_with_events(&retained)?;
        self.verify_repository_absent(repository_id)?;
        self.clean_purge_artifacts()?;
        self.remove_purge_journal()?;
        Ok(())
    }

    pub fn apply_retention(
        &self,
        now: DateTime<Utc>,
        retention_days: i64,
    ) -> Result<RetentionReport> {
        let _lock = self.acquire_maintenance_lock()?;
        let events = self.list_events(None)?;
        let cutoff = now - ChronoDuration::days(retention_days.max(1));
        let connection = self.connect()?;
        let facts: Vec<FactV1> = query_json_rows(
            &connection,
            "SELECT fact_json FROM facts WHERE lifecycle = 'pinned'",
            [],
        )?;
        let pinned_fact_ids = facts
            .iter()
            .map(|fact| fact.id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let pinned_evidence_ids = facts
            .iter()
            .flat_map(|fact| fact.evidence_ids.iter().cloned())
            .collect::<std::collections::BTreeSet<_>>();
        let evidence: Vec<EvidenceV1> =
            query_json_rows(&connection, "SELECT evidence_json FROM evidence", [])?;
        let pinned_source_ids = evidence
            .iter()
            .filter(|item| pinned_evidence_ids.contains(&item.id))
            .map(|item| item.source_id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        drop(connection);

        // Candidates and readiness are durable local workflow state. Retain only the newest
        // canonical snapshot for each candidate and repository/task evaluation key so the
        // projections remain rebuildable without keeping an unbounded evaluation history.
        let mut latest_candidate_events = std::collections::BTreeMap::new();
        let mut latest_evaluation_events = std::collections::BTreeMap::new();
        let mut latest_passing_test_events = std::collections::BTreeMap::new();
        let mut latest_continuation_guard_events = std::collections::BTreeMap::new();
        let mut latest_fact_refresh_events = std::collections::BTreeMap::new();
        let mut latest_agent_events = std::collections::BTreeMap::new();
        for event in &events {
            match event.kind {
                EventKind::RegressionCandidateRecorded => {
                    if let Some(id) = event
                        .payload
                        .pointer("/regressionCandidate/id")
                        .and_then(Value::as_str)
                    {
                        latest_candidate_events.insert(id.to_string(), event.event_id.clone());
                    }
                }
                EventKind::ContractEvaluationRecorded => {
                    latest_evaluation_events.insert(
                        (event.repository_id.clone(), event.task_id.clone()),
                        event.event_id.clone(),
                    );
                    if let Ok(evaluation) = serde_json::from_value::<ContractEvaluationV1>(
                        event
                            .payload
                            .get("contractEvaluation")
                            .cloned()
                            .unwrap_or(Value::Null),
                    ) {
                        if evaluation.continuation_issued {
                            latest_continuation_guard_events.insert(
                                (
                                    event.repository_id.clone(),
                                    event.task_id.clone(),
                                    evaluation.content_fingerprint.clone(),
                                    evaluation
                                        .relevant_contracts
                                        .iter()
                                        .map(|contract| contract.id.clone())
                                        .collect::<std::collections::BTreeSet<_>>(),
                                ),
                                event.event_id.clone(),
                            );
                        }
                        for test in evaluation.required_tests.into_iter().filter(|test| {
                            test.state == crate::contracts::RequiredTestStateV1::Passed
                        }) {
                            latest_passing_test_events.insert(
                                (
                                    event.repository_id.clone(),
                                    event.task_id.clone(),
                                    test.program,
                                    test.args,
                                    test.working_directory,
                                ),
                                event.event_id.clone(),
                            );
                        }
                    }
                }
                EventKind::AiFactRefreshOperationRecorded => {
                    if let Some(id) = event
                        .payload
                        .pointer("/operation/operationId")
                        .and_then(Value::as_str)
                    {
                        latest_fact_refresh_events.insert(id.to_string(), event.event_id.clone());
                    }
                }
                EventKind::AgentObserved => {
                    if let Some(id) = event.payload.pointer("/agent/id").and_then(Value::as_str) {
                        latest_agent_events.insert(id.to_string(), event.event_id.clone());
                    }
                }
                _ => {}
            }
        }
        let retained_contract_event_ids = latest_candidate_events
            .into_values()
            .chain(latest_evaluation_events.into_values())
            .chain(latest_passing_test_events.into_values())
            .chain(latest_continuation_guard_events.into_values())
            .chain(latest_fact_refresh_events.into_values())
            .chain(latest_agent_events.into_values())
            .collect::<std::collections::BTreeSet<_>>();

        let retained = events
            .iter()
            .filter(|event| {
                event.occurred_at >= cutoff
                    || matches!(
                        event.kind,
                        EventKind::TaskUpdated | EventKind::TaskGroupingChanged
                    )
                    || pinned_source_ids.contains(&event.source_id)
                    || event
                        .payload
                        .pointer("/fact/id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| pinned_fact_ids.contains(id))
                    || event
                        .payload
                        .pointer("/evidence/id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| pinned_evidence_ids.contains(id))
                    || retained_contract_event_ids.contains(&event.event_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        let report = RetentionReport {
            retained_events: retained.len(),
            removed_events: events.len().saturating_sub(retained.len()),
        };
        if report.removed_events > 0 {
            self.replace_with_events(&retained)?;
        }
        Ok(report)
    }

    pub fn health(&self) -> Result<StoreHealth> {
        let connection = self.connect()?;
        let journal_mode = connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        let integrity_check =
            connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        let schema_version = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        let canonical_event_count =
            connection.query_row("SELECT COUNT(*) FROM canonical_events", [], |row| {
                row.get::<_, u64>(0)
            })?;
        let projection_task_count =
            connection.query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get::<_, u64>(0))?;
        Ok(StoreHealth {
            schema_version,
            journal_mode,
            integrity_check,
            canonical_event_count,
            projection_task_count,
        })
    }

    fn connect(&self) -> Result<Connection> {
        let parent = self.path.parent().context("database path has no parent")?;
        if self.read_only {
            validate_private_directory(parent, "PreviouslyOn data directory")?;
            if !validate_private_regular_file(&self.path, "PreviouslyOn database")? {
                bail!("PreviouslyOn database does not exist");
            }
        } else {
            ensure_private_directory(parent, "PreviouslyOn data directory")?;
            ensure_private_regular_file(&self.path, "PreviouslyOn database")?;
        }
        let companions = database_companion_paths(&self.path);
        let existed = companions
            .iter()
            .map(|(path, label)| validate_private_regular_file(path, label))
            .collect::<Result<Vec<_>>>()?;
        let flags = if self.read_only {
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW
        } else {
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW
        };
        let connection = Connection::open_with_flags(&self.path, flags)
            .with_context(|| format!("open PreviouslyOn database {}", self.path.display()))?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        if self.read_only {
            connection.execute_batch(
                "PRAGMA foreign_keys = ON;
                 PRAGMA query_only = ON;
                 PRAGMA temp_store = MEMORY;",
            )?;
        } else {
            connection.execute_batch(
                "PRAGMA foreign_keys = ON;
                 PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = FULL;
                 PRAGMA temp_store = MEMORY;",
            )?;
        }
        for ((path, label), existed) in companions.iter().zip(existed) {
            if !self.read_only && !existed && fs::symlink_metadata(path).is_ok() {
                secure_new_private_file(path, label)?;
            }
            validate_private_regular_file(path, label)?;
        }
        Ok(connection)
    }

    fn acquire_maintenance_lock(&self) -> Result<fs::File> {
        acquire_database_lock(&self.path)
    }

    fn purge_journal_path(&self) -> PathBuf {
        PathBuf::from(format!(
            "{}.purge-recovery.json",
            self.path.to_string_lossy()
        ))
    }

    fn read_purge_journal(&self) -> Result<Option<PurgeRecoveryJournalV1>> {
        let path = self.purge_journal_path();
        let bytes = match read_private_file(&path, "purge recovery journal")? {
            Some(bytes) => bytes,
            None => return Ok(None),
        };
        let journal: PurgeRecoveryJournalV1 = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse purge recovery journal {}", path.display()))?;
        if journal.version != 1 {
            bail!("unsupported purge recovery journal version");
        }
        Ok(Some(journal))
    }

    fn write_purge_journal(&self, journal: &PurgeRecoveryJournalV1) -> Result<()> {
        let path = self.purge_journal_path();
        let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::now_v7()));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        let mut file = open_private_file(&temporary, "temporary purge journal", &mut options)?;
        file.write_all(&serde_json::to_vec(journal)?)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &path)?;
        validate_private_regular_file(&path, "purge recovery journal")?;
        fs::File::open(path.parent().context("purge journal has no parent")?)?.sync_all()?;
        Ok(())
    }

    fn remove_purge_journal(&self) -> Result<()> {
        let path = self.purge_journal_path();
        match fs::remove_file(&path) {
            Ok(()) => {
                fs::File::open(path.parent().context("purge journal has no parent")?)?
                    .sync_all()?;
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn recover_purge_if_ready(&self) -> Result<()> {
        let Some(mut journal) = self.read_purge_journal()? else {
            return Ok(());
        };
        let _lock = self.acquire_maintenance_lock()?;
        self.write_repository_tombstone(&journal.repository_id)?;
        if journal.phase == PurgeJournalPhase::Tombstoned {
            // v0.1 supports a single registered repository. On recovery, malformed queue data
            // cannot be proven unrelated and is therefore discarded with the repository rather
            // than risking resurrection after the compacted DB is swapped in.
            self.purge_recovery_queue_files(&journal.repository_id)?;
            journal.phase = PurgeJournalPhase::RelatedDataPurged;
            self.write_purge_journal(&journal)?;
        }
        let retained = self
            .list_events(None)?
            .into_iter()
            .filter(|event| event.repository_id != journal.repository_id)
            .collect::<Vec<_>>();
        self.replace_with_events(&retained)?;
        self.verify_repository_absent(&journal.repository_id)?;
        self.clean_purge_artifacts()?;
        self.remove_purge_journal()
    }

    fn write_repository_tombstone(&self, repository_id: &str) -> Result<()> {
        let data_dir = self.path.parent().context("database path has no parent")?;
        let path = repository_tombstone_path(data_dir, repository_id);
        if validate_private_regular_file(&path, "repository purge tombstone")? {
            return Ok(());
        }
        let directory = path.parent().context("purge tombstone has no parent")?;
        ensure_private_directory(directory, "purge tombstone directory")?;
        fs::File::open(data_dir)?.sync_all()?;
        let tombstone = RepositoryPurgeTombstoneV1 {
            version: PURGE_TOMBSTONE_VERSION,
            generation: uuid::Uuid::now_v7().to_string(),
            created_at: timestamp(Utc::now()),
        };
        write_private_atomic_file(&path, &serde_json::to_vec(&tombstone)?)
    }

    fn purge_recovery_queue_files(&self, repository_id: &str) -> Result<()> {
        let data_dir = self.path.parent().context("database path has no parent")?;
        let queue = data_dir.join("queue/events.jsonl");
        for path in [
            queue.clone(),
            queue.with_extension("replay.jsonl"),
            queue.with_extension("corrupt.jsonl"),
        ] {
            self.rewrite_recovery_queue(&path, repository_id)?;
        }
        let cache = data_dir.join("cache");
        if cache.exists() {
            fs::remove_dir_all(&cache)
                .with_context(|| format!("remove purge cache {}", cache.display()))?;
            fs::File::open(data_dir)?.sync_all()?;
        }
        Ok(())
    }

    fn rewrite_recovery_queue(&self, path: &Path, repository_id: &str) -> Result<()> {
        let bytes = match read_private_file(path, "purge recovery queue")? {
            Some(bytes) => bytes,
            None => return Ok(()),
        };
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains("corrupt"))
        {
            return remove_file_and_sync_parent(path);
        }
        let Ok(contents) = std::str::from_utf8(&bytes) else {
            return remove_file_and_sync_parent(path);
        };
        let mut retained = Vec::new();
        for line in contents.lines().filter(|line| !line.trim().is_empty()) {
            let Ok(event) = serde_json::from_str::<EventEnvelopeV1>(line) else {
                return remove_file_and_sync_parent(path);
            };
            if event.repository_id != repository_id {
                retained.push(line);
            }
        }
        let mut replacement = retained.join("\n");
        if !replacement.is_empty() {
            replacement.push('\n');
        }
        write_private_atomic_file(path, replacement.as_bytes())
    }

    fn clean_purge_artifacts(&self) -> Result<()> {
        remove_sidecar_if_present(&self.path, "wal")?;
        remove_sidecar_if_present(&self.path, "shm")?;
        let data_dir = self.path.parent().context("database path has no parent")?;
        let cache = data_dir.join("cache");
        if cache.exists() {
            fs::remove_dir_all(&cache)
                .with_context(|| format!("remove purge cache {}", cache.display()))?;
        }
        fs::File::open(data_dir)?.sync_all()?;
        Ok(())
    }

    fn verify_repository_absent(&self, repository_id: &str) -> Result<()> {
        let connection = self.connect()?;
        let remaining: u64 = connection.query_row(
            "SELECT COUNT(*) FROM canonical_events WHERE repository_id = ?1",
            [repository_id],
            |row| row.get(0),
        )?;
        if remaining != 0 {
            bail!("purge verification failed for repository {repository_id}");
        }
        Ok(())
    }

    fn replace_with_events(&self, events: &[EventEnvelopeV1]) -> Result<()> {
        let parent = self.path.parent().context("database path has no parent")?;
        let temp_path = parent.join(format!(
            ".previously-compaction-{}.sqlite3",
            uuid::Uuid::now_v7()
        ));
        let temp_store = Store::open(&temp_path)?;
        for event in events {
            temp_store.insert_event(event)?;
        }
        {
            let connection = temp_store.connect()?;
            let integrity: String =
                connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
            if integrity != "ok" {
                bail!("compacted database failed integrity check: {integrity}");
            }
            connection.execute_batch(
                "PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE; VACUUM;",
            )?;
        }
        validate_private_regular_file(&temp_path, "compacted database")?;
        fs::File::open(&temp_path)?.sync_all()?;
        {
            let connection = self.connect()?;
            connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        }
        remove_sidecar_if_present(&self.path, "wal")?;
        remove_sidecar_if_present(&self.path, "shm")?;
        fs::rename(&temp_path, &self.path).context("atomically replace compacted database")?;
        fs::File::open(parent)?.sync_all()?;
        validate_private_regular_file(&self.path, "PreviouslyOn database")?;
        Ok(())
    }

    fn migrate(&self, connection: &mut Connection) -> Result<()> {
        connection.execute_batch(MIGRATION_V1)?;
        connection.pragma_update(None, "user_version", DATABASE_SCHEMA_VERSION)?;
        Ok(())
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

fn lexical_overlap(query: &str, candidate: &str) -> f64 {
    let query_tokens = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| token.chars().count() >= 2)
        .map(|token| token.to_lowercase())
        .collect::<std::collections::BTreeSet<_>>();
    if query_tokens.is_empty() {
        return 0.0;
    }
    let candidate_tokens = candidate
        .split(|character: char| !character.is_alphanumeric())
        .filter(|token| token.chars().count() >= 2)
        .map(|token| token.to_lowercase())
        .collect::<std::collections::BTreeSet<_>>();
    query_tokens.intersection(&candidate_tokens).count() as f64 / query_tokens.len() as f64
}

const MIGRATION_V1: &str = r#"
CREATE TABLE IF NOT EXISTS repositories (
  id TEXT PRIMARY KEY,
  repository_id TEXT NOT NULL UNIQUE,
  path TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  repository_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS canonical_events (
  event_id TEXT PRIMARY KEY,
  dedupe_key TEXT NOT NULL UNIQUE,
  repository_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  task_id TEXT,
  sequence_no INTEGER,
  occurred_at TEXT NOT NULL,
  received_at TEXT NOT NULL,
  kind TEXT NOT NULL,
  event_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS canonical_events_replay
  ON canonical_events(occurred_at, sequence_no, event_id);
CREATE INDEX IF NOT EXISTS canonical_events_repository
  ON canonical_events(repository_id, session_id, occurred_at);
CREATE TRIGGER IF NOT EXISTS canonical_events_no_update
BEFORE UPDATE ON canonical_events BEGIN SELECT RAISE(ABORT, 'canonical events are immutable'); END;

CREATE TABLE IF NOT EXISTS tasks (
  id TEXT PRIMARY KEY,
  repository_id TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  task_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS tasks_repository ON tasks(repository_id, updated_at);
CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY,
  repository_id TEXT NOT NULL,
  task_id TEXT,
  started_at TEXT NOT NULL,
  session_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_task ON sessions(task_id, started_at);
CREATE TABLE IF NOT EXISTS session_grouping_assignments (
  session_id TEXT PRIMARY KEY,
  repository_id TEXT NOT NULL,
  task_id TEXT NOT NULL,
  operation_id TEXT NOT NULL,
  occurred_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS session_grouping_assignments_task
  ON session_grouping_assignments(repository_id, task_id, occurred_at);
CREATE TABLE IF NOT EXISTS checkpoints (
  id TEXT PRIMARY KEY,
  repository_id TEXT NOT NULL,
  task_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  created_at TEXT NOT NULL,
  checkpoint_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS checkpoints_task ON checkpoints(task_id, created_at);
CREATE TABLE IF NOT EXISTS facts (
  id TEXT PRIMARY KEY,
  repository_id TEXT NOT NULL,
  task_id TEXT NOT NULL,
  lifecycle TEXT NOT NULL,
  freshness TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  fact_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS facts_task ON facts(task_id, updated_at);
CREATE TABLE IF NOT EXISTS evidence (
  id TEXT PRIMARY KEY,
  repository_id TEXT NOT NULL,
  task_id TEXT NOT NULL,
  fact_id TEXT,
  session_id TEXT NOT NULL,
  created_at TEXT NOT NULL,
  evidence_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS evidence_task ON evidence(task_id, fact_id, created_at);
CREATE TABLE IF NOT EXISTS file_changes (
  id TEXT PRIMARY KEY,
  repository_id TEXT NOT NULL,
  task_id TEXT,
  session_id TEXT NOT NULL,
  path TEXT NOT NULL,
  change_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS file_changes_task ON file_changes(task_id, path);
CREATE TABLE IF NOT EXISTS test_results (
  id TEXT PRIMARY KEY,
  repository_id TEXT NOT NULL,
  task_id TEXT,
  session_id TEXT NOT NULL,
  occurred_at TEXT NOT NULL,
  test_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS test_results_task ON test_results(task_id, occurred_at);
CREATE TABLE IF NOT EXISTS regression_candidates (
  id TEXT PRIMARY KEY,
  repository_id TEXT NOT NULL,
  task_id TEXT,
  updated_at TEXT NOT NULL,
  candidate_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS regression_candidates_repository
  ON regression_candidates(repository_id, updated_at);
CREATE TABLE IF NOT EXISTS contract_evaluations (
  id TEXT PRIMARY KEY,
  repository_id TEXT NOT NULL,
  task_id TEXT,
  evaluated_at TEXT NOT NULL,
  evaluation_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS contract_evaluations_repository
  ON contract_evaluations(repository_id, evaluated_at);
CREATE TABLE IF NOT EXISTS ai_fact_refresh_operations (
  operation_id TEXT PRIMARY KEY,
  repository_id TEXT NOT NULL,
  task_id TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  operation_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS ai_fact_refresh_operations_repository
  ON ai_fact_refresh_operations(repository_id, updated_at);
CREATE TABLE IF NOT EXISTS agents (
  id TEXT PRIMARY KEY,
  repository_id TEXT NOT NULL,
  task_id TEXT,
  thread_id TEXT NOT NULL,
  observed_at TEXT NOT NULL,
  agent_json TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS agents_repository_thread
  ON agents(repository_id, thread_id);
CREATE INDEX IF NOT EXISTS agents_task ON agents(task_id, observed_at);
CREATE VIRTUAL TABLE IF NOT EXISTS task_fts USING fts5(task_id UNINDEXED, title, goal, tokenize='unicode61');
CREATE VIRTUAL TABLE IF NOT EXISTS fact_fts USING fts5(fact_id UNINDEXED, content, tokenize='unicode61');
"#;

fn query_json_optional<T, P>(connection: &Connection, sql: &str, params: P) -> Result<Option<T>>
where
    T: DeserializeOwned,
    P: rusqlite::Params,
{
    let json = connection
        .query_row(sql, params, |row| row.get::<_, String>(0))
        .optional()?;
    json.map(|json| serde_json::from_str(&json).context("deserialize database projection"))
        .transpose()
}

fn query_json_rows<T, P>(connection: &Connection, sql: &str, params: P) -> Result<Vec<T>>
where
    T: DeserializeOwned,
    P: rusqlite::Params,
{
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map(params, |row| row.get::<_, String>(0))?;
    rows.map(|row| {
        let json = row?;
        serde_json::from_str(&json).context("deserialize database projection")
    })
    .collect()
}

fn payload_as<T: DeserializeOwned>(payload: &Value, key: &str) -> Option<T> {
    serde_json::from_value(payload.clone()).ok().or_else(|| {
        payload
            .get(key)
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
    })
}

fn payload_array<T: DeserializeOwned>(payload: &Value, key: &str) -> Vec<T> {
    payload
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| serde_json::from_value(item.clone()).ok())
        .collect()
}

fn payload_text(payload: &Value) -> Option<String> {
    payload
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            payload
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            payload
                .get("prompt")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            payload
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn fts_query(query: &str) -> String {
    query
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .filter(|token| !token.is_empty())
        .take(12)
        .map(|token| format!("\"{}\"*", token.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn enum_text<T: Serialize>(value: T) -> Result<String> {
    let value = serde_json::to_value(value)?;
    Ok(value.as_str().unwrap_or_default().to_string())
}

pub(crate) fn ensure_private_directory(path: &Path, label: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                bail!(
                    "{label} must be a real directory, not a symlink: {}",
                    path.display()
                );
            }
            validate_private_owner(&metadata, label, path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if metadata.mode() & 0o022 != 0 {
                    bail!("{label} is group/world writable: {}", path.display());
                }
                if metadata.mode() & 0o077 != 0 {
                    tighten_private_directory(path, label)?;
                }
            }
            validate_private_directory(path, label)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create parent directory {}", parent.display()))?;
            }
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            validate_private_directory(path, label)
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn tighten_private_directory(path: &Path, label: &str) -> Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
    let directory = options
        .open(path)
        .with_context(|| format!("open trusted {label} {}", path.display()))?;
    let metadata = directory.metadata()?;
    if !metadata.is_dir() {
        bail!("{label} must be a real directory: {}", path.display());
    }
    validate_private_owner(&metadata, label, path)?;
    directory.set_permissions(fs::Permissions::from_mode(0o700))?;
    Ok(())
}

pub(crate) fn validate_private_directory(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect {label} {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "{label} must be a real directory, not a symlink: {}",
            path.display()
        );
    }
    validate_private_metadata(&metadata, true, label, path)
}

pub(crate) fn validate_private_regular_file(path: &Path, label: &str) -> Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "{label} must be a regular file, not a symlink: {}",
            path.display()
        );
    }
    validate_private_metadata(&metadata, false, label, path)?;
    Ok(true)
}

#[cfg(unix)]
pub(crate) fn validate_private_socket(path: &Path, label: &str) -> Result<bool> {
    use std::os::unix::fs::FileTypeExt;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_socket() {
        bail!(
            "{label} must be a Unix socket, not a symlink: {}",
            path.display()
        );
    }
    validate_private_metadata(&metadata, false, label, path)?;
    Ok(true)
}

pub(crate) fn open_private_file(
    path: &Path,
    label: &str,
    options: &mut OpenOptions,
) -> Result<fs::File> {
    validate_private_regular_file(path, label)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .with_context(|| format!("open {label} {}", path.display()))?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        bail!("{label} must be a regular file: {}", path.display());
    }
    validate_private_metadata(&metadata, false, label, path)?;
    Ok(file)
}

pub(crate) fn read_private_file(path: &Path, label: &str) -> Result<Option<Vec<u8>>> {
    if !validate_private_regular_file(path, label)? {
        return Ok(None);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    let mut file = open_private_file(path, label, &mut options)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

fn ensure_private_regular_file(path: &Path, label: &str) -> Result<()> {
    if validate_private_regular_file(path, label)? {
        return Ok(());
    }
    let mut options = OpenOptions::new();
    options.create_new(true).read(true).write(true);
    open_private_file(path, label, &mut options)?;
    Ok(())
}

fn secure_new_private_file(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect new {label} {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "new {label} must be a regular file, not a symlink: {}",
            path.display()
        );
    }
    validate_private_owner(&metadata, label, path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .with_context(|| format!("open new {label} {}", path.display()))?;
    validate_private_owner(&file.metadata()?, label, path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    validate_private_metadata(&file.metadata()?, false, label, path)
}

fn database_companion_paths(database: &Path) -> Vec<(PathBuf, &'static str)> {
    let database = database.to_string_lossy();
    vec![
        (PathBuf::from(format!("{database}-wal")), "SQLite WAL"),
        (
            PathBuf::from(format!("{database}-shm")),
            "SQLite shared-memory file",
        ),
        (
            PathBuf::from(format!("{database}-journal")),
            "SQLite rollback journal",
        ),
        (
            PathBuf::from(format!("{database}.lock")),
            "database maintenance lock",
        ),
        (
            PathBuf::from(format!("{database}.purge-recovery.json")),
            "purge recovery journal",
        ),
    ]
}

fn validate_database_companions(database: &Path) -> Result<()> {
    for (path, label) in database_companion_paths(database) {
        validate_private_regular_file(&path, label)?;
    }
    Ok(())
}

#[cfg(unix)]
fn validate_private_owner(metadata: &fs::Metadata, label: &str, path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    if metadata.uid() != unsafe { libc::geteuid() } {
        bail!(
            "{label} is not owned by the current user: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_owner(_metadata: &fs::Metadata, _label: &str, _path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn validate_private_metadata(
    metadata: &fs::Metadata,
    directory: bool,
    label: &str,
    path: &Path,
) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    validate_private_owner(metadata, label, path)?;
    if metadata.mode() & 0o077 != 0 {
        let boundary = if directory { "0700" } else { "0600" };
        bail!(
            "{label} exceeds the private {boundary} boundary: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_metadata(
    _metadata: &fs::Metadata,
    _directory: bool,
    _label: &str,
    _path: &Path,
) -> Result<()> {
    Ok(())
}

fn repository_tombstone_path(data_dir: &Path, repository_id: &str) -> PathBuf {
    let identity_hash = hex::encode(sha2::Sha256::digest(repository_id.as_bytes()));
    data_dir
        .join("purge-tombstones")
        .join(format!("{identity_hash}.json"))
}

fn acquire_database_lock(database_path: &Path) -> Result<fs::File> {
    let lock_path = PathBuf::from(format!("{}.lock", database_path.to_string_lossy()));
    if let Some(parent) = lock_path.parent() {
        ensure_private_directory(parent, "PreviouslyOn data directory")?;
    }
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    let file = open_private_file(&lock_path, "database maintenance lock", &mut options)?;
    file.lock()
        .with_context(|| format!("lock database maintenance file {}", lock_path.display()))?;
    Ok(file)
}

fn write_private_atomic_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("atomic file path has no parent")?;
    ensure_private_directory(parent, "private data directory")?;
    validate_private_regular_file(path, "private data file")?;
    let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::now_v7()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    let mut file = open_private_file(&temporary, "temporary private data file", &mut options)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)?;
    validate_private_regular_file(path, "private data file")?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn remove_file_and_sync_parent(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => {
            if let Some(parent) = path.parent() {
                fs::File::open(parent)?.sync_all()?;
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn remove_sidecar_if_present(database: &Path, suffix: &str) -> Result<()> {
    let sidecar = PathBuf::from(format!("{}-{suffix}", database.to_string_lossy()));
    validate_private_regular_file(&sidecar, "SQLite sidecar")?;
    match fs::remove_file(&sidecar) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("remove database sidecar {}", sidecar.display()))
        }
    }
}

#[cfg(test)]
mod insert_fault_tests {
    use super::*;
    use crate::domain::{EventKind, SCHEMA_VERSION_V1};
    use serde_json::json;

    fn event() -> EventEnvelopeV1 {
        let mut event = EventEnvelopeV1::new(
            "fault-source",
            "repo-fault",
            "session-fault",
            EventKind::UserPrompt,
            Utc::now(),
            json!({"prompt":"safe retry"}),
        );
        event.schema_version = SCHEMA_VERSION_V1;
        event
    }

    #[test]
    fn failures_before_commit_roll_back_and_retry_to_one_canonical_event() {
        for fault in [
            InsertFault {
                before_insert: true,
                ..InsertFault::default()
            },
            InsertFault {
                before_commit: true,
                ..InsertFault::default()
            },
        ] {
            let temp = tempfile::TempDir::new().unwrap();
            let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
            let event = event();
            assert!(store.insert_event_inner(&event, fault).is_err());
            assert_eq!(store.health().unwrap().canonical_event_count, 0);
            assert_eq!(store.insert_event(&event).unwrap(), InsertOutcome::Inserted);
            assert_eq!(store.health().unwrap().canonical_event_count, 1);
        }
    }

    #[test]
    fn failure_after_commit_replays_as_duplicate_without_projection_duplication() {
        let temp = tempfile::TempDir::new().unwrap();
        let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
        let event = event();
        let error = store
            .insert_event_inner(
                &event,
                InsertFault {
                    after_commit: true,
                    ..InsertFault::default()
                },
            )
            .unwrap_err();
        assert!(error.to_string().contains("after canonical commit"));
        assert_eq!(store.health().unwrap().canonical_event_count, 1);
        assert_eq!(
            store.insert_event(&event).unwrap(),
            InsertOutcome::Duplicate
        );
        assert_eq!(store.health().unwrap().canonical_event_count, 1);
    }

    #[test]
    fn sqlite_full_errors_are_classified_for_reserve_queue_fallback() {
        let sqlite = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL),
            Some("disk full".to_string()),
        );
        assert!(is_sqlite_full(&anyhow::Error::new(sqlite)));
        assert!(!is_sqlite_full(&anyhow::anyhow!("other failure")));
    }
}
