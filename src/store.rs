use crate::contracts::{ContractEvaluationV1, RegressionCandidateV1};
use crate::domain::{
    AiFactRefreshOperationV1, CheckpointV1, ContextUsageV1, ContinuationAdviceV1,
    ContinuationReasonV1, ContinuationStateV1, EventEnvelopeV1, EventKind, EvidenceV1, FactV1,
    FileChangeV1, GitSnapshotV1, RepositoryV1, SessionV1, TaskGroupingOperationV1, TaskLifecycle,
    TaskSuggestionV1, TaskTimelineV1, TaskV1, TestResultV1, SCHEMA_VERSION_V1,
};
use crate::redaction::{redact_excerpt, redact_text};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Digest;
use std::fs;
use std::path::{Path, PathBuf};

mod maintenance;
mod operations;
mod private_fs;
mod projection;

pub use maintenance::{ensure_repository_not_purged, reactivate_repository};
use private_fs::{
    database_companion_paths, ensure_private_regular_file, secure_new_private_file,
    validate_database_companions,
};
pub(crate) use private_fs::{
    ensure_private_directory, open_private_file, read_private_file, validate_private_directory,
    validate_private_regular_file, validate_private_socket,
};

use projection::{
    insert_event_tx, load_events, prepare_event, rebuild_projections_tx, upsert_checkpoint_tx,
    upsert_evidence_tx, upsert_fact_tx, upsert_file_change_tx, upsert_repository_tx,
    upsert_session_tx, upsert_task_tx, upsert_test_result_tx,
};

const DATABASE_SCHEMA_VERSION: i64 = 1;

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

    fn migrate(&self, connection: &mut Connection) -> Result<()> {
        connection.execute_batch(MIGRATION_V1)?;
        connection.pragma_update(None, "user_version", DATABASE_SCHEMA_VERSION)?;
        Ok(())
    }
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
