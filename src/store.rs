use crate::domain::{
    deterministic_id, CheckpointV1, ContextUsageV1, ContinuationAdviceV1, ContinuationReasonV1,
    ContinuationStateV1, CoverageV1, EventEnvelopeV1, EventKind, EvidenceV1, FactLifecycle, FactV1,
    FileChangeV1, GitSnapshotV1, RepositoryV1, SessionLifecycle, SessionV1, TaskLifecycle,
    TaskSuggestionV1, TaskTimelineV1, TaskV1, TestResultV1, SCHEMA_VERSION_V1,
};
use crate::redaction::{redact_excerpt, redact_text, redact_value};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Digest;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const DATABASE_SCHEMA_VERSION: i64 = 1;
const PURGE_TOMBSTONE_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct Store {
    path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertOutcome {
    Inserted,
    Duplicate,
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
    let path = repository_tombstone_path(data_dir, repository_id);
    match fs::metadata(&path) {
        Ok(_) => {
            bail!(
                "repository {repository_id} was purged; run setup again before capturing new data"
            )
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("inspect repository purge tombstone {}", path.display()))
        }
    }

    let journal_path = data_dir.join("previously.sqlite3.purge-recovery.json");
    let journal_bytes = match fs::read(&journal_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("inspect purge journal {}", journal_path.display()))
        }
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
    fs::create_dir_all(data_dir)?;
    set_directory_permissions(data_dir)?;
    let database = data_dir.join("previously.sqlite3");
    let _lock = acquire_database_lock(&database)?;
    let journal_path = data_dir.join("previously.sqlite3.purge-recovery.json");
    match fs::metadata(&journal_path) {
        Ok(_) => {
            bail!("cannot reactivate repository while purge recovery is pending; rerun purge first")
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("inspect purge journal {}", journal_path.display()))
        }
    }
    remove_file_and_sync_parent(&repository_tombstone_path(data_dir, repository_id))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskSearchHit {
    pub task: TaskV1,
    pub rank: f64,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create database directory {}", parent.display()))?;
            set_directory_permissions(parent)?;
        }
        let store = Self { path };
        let mut connection = store.connect()?;
        store.migrate(&mut connection)?;
        set_file_permissions(&store.path)?;
        for suffix in ["wal", "shm"] {
            let sidecar = PathBuf::from(format!("{}-{suffix}", store.path.to_string_lossy()));
            set_file_permissions(&sidecar)?;
        }
        store.recover_purge_if_ready()?;
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
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let latest_key = transaction
            .query_row(
                "SELECT occurred_at, COALESCE(sequence_no, 9223372036854775807), event_id
                 FROM canonical_events
                 ORDER BY occurred_at DESC, COALESCE(sequence_no, 9223372036854775807) DESC, event_id DESC
                 LIMIT 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)),
            )
            .optional()?;
        let serialized = serde_json::to_string(&event).context("serialize canonical event")?;
        if fault.before_insert {
            bail!("injected failure before canonical insert");
        }
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
            transaction.commit()?;
            return Ok(InsertOutcome::Duplicate);
        }

        let event_key = (
            timestamp(event.occurred_at),
            event.sequence.unwrap_or(i64::MAX),
            event.event_id.clone(),
        );
        if latest_key.map(|latest| event_key < latest).unwrap_or(false) {
            rebuild_projections_tx(&transaction)?;
        } else {
            apply_event_projection(&transaction, &event)?;
        }
        if fault.before_commit {
            bail!("injected failure before canonical commit");
        }
        transaction.commit()?;
        if fault.after_commit {
            bail!("injected failure after canonical commit");
        }
        Ok(InsertOutcome::Inserted)
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
                    if session.compaction_count >= 6 {
                        reasons.push(ContinuationReasonV1::CompactionLimit);
                    }
                    if session
                        .context_usage
                        .as_ref()
                        .and_then(ContextUsageV1::utilization)
                        .is_some_and(|ratio| ratio >= 0.8)
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

        let retained = events
            .iter()
            .filter(|event| {
                event.occurred_at >= cutoff
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
        let connection = Connection::open(&self.path)
            .with_context(|| format!("open PreviouslyOn database {}", self.path.display()))?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA temp_store = MEMORY;",
        )?;
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
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&serde_json::to_vec(journal)?)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &path)?;
        set_file_permissions(&path)?;
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
        match fs::metadata(&path) {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("inspect repository purge tombstone {}", path.display())
                })
            }
        }
        let directory = path.parent().context("purge tombstone has no parent")?;
        fs::create_dir_all(directory)?;
        set_directory_permissions(directory)?;
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
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
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
        set_file_permissions(&temp_path)?;
        fs::File::open(&temp_path)?.sync_all()?;
        {
            let connection = self.connect()?;
            connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        }
        remove_sidecar_if_present(&self.path, "wal")?;
        remove_sidecar_if_present(&self.path, "shm")?;
        fs::rename(&temp_path, &self.path).context("atomically replace compacted database")?;
        fs::File::open(parent)?.sync_all()?;
        set_file_permissions(&self.path)?;
        Ok(())
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
CREATE VIRTUAL TABLE IF NOT EXISTS task_fts USING fts5(task_id UNINDEXED, title, goal, tokenize='unicode61');
CREATE VIRTUAL TABLE IF NOT EXISTS fact_fts USING fts5(fact_id UNINDEXED, content, tokenize='unicode61');
"#;

fn rebuild_projections_tx(transaction: &Transaction<'_>) -> Result<()> {
    let events = load_events(transaction, None)?;
    transaction.execute_batch(
        "DELETE FROM task_fts;
         DELETE FROM fact_fts;
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
    ensure_session_tx(transaction, event)?;
    ensure_task_tx(transaction, event)?;
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
                upsert_fact_tx(transaction, &fact)?;
            }
            if let Some(mut evidence) = payload_as::<EvidenceV1>(&event.payload, "evidence") {
                evidence.excerpt = redact_excerpt(&evidence.excerpt);
                upsert_evidence_tx(transaction, &evidence)?;
            }
        }
        EventKind::ToolFinished => {
            for mut test in payload_array::<TestResultV1>(&event.payload, "test_results") {
                test.command = redact_text(&test.command);
                test.summary = test.summary.map(|summary| redact_excerpt(&summary));
                upsert_test_result_tx(transaction, &test)?;
            }
            if let Some(mut test) = payload_as::<TestResultV1>(&event.payload, "test_result") {
                test.command = redact_text(&test.command);
                test.summary = test.summary.map(|summary| redact_excerpt(&summary));
                upsert_test_result_tx(transaction, &test)?;
            }
            for change in payload_array::<FileChangeV1>(&event.payload, "file_changes") {
                upsert_file_change_tx(transaction, &change)?;
            }
        }
        _ => {}
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
            if session.compaction_count >= 6
                && session.continuation_state == ContinuationStateV1::Normal
            {
                session.continuation_state = ContinuationStateV1::Eligible;
            }
            changed = true;
        }
        if let Some(usage) = context_usage(&event.payload, event.occurred_at) {
            if usage.utilization().is_some_and(|ratio| ratio >= 0.8)
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
        } else if compaction_count >= 6
            || context_usage
                .as_ref()
                .and_then(ContextUsageV1::utilization)
                .is_some_and(|ratio| ratio >= 0.8)
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
    if let Some(mut task) = payload_as::<TaskV1>(&event.payload, "task") {
        if task.id == task_id && task.repository_id == event.repository_id {
            if event.occurred_at > task.updated_at {
                task.updated_at = event.occurred_at;
            }
            return upsert_task_tx(transaction, &task);
        }
    }
    let existing: Option<TaskV1> = query_json_optional(
        transaction,
        "SELECT task_json FROM tasks WHERE id = ?1",
        [task_id],
    )?;
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

fn upsert_repository_tx(transaction: &Transaction<'_>, repository: &RepositoryV1) -> Result<()> {
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

fn upsert_task_tx(transaction: &Transaction<'_>, task: &TaskV1) -> Result<()> {
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

fn upsert_session_tx(transaction: &Transaction<'_>, session: &SessionV1) -> Result<()> {
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

fn upsert_checkpoint_tx(transaction: &Transaction<'_>, checkpoint: &CheckpointV1) -> Result<()> {
    let checkpoint: CheckpointV1 = serde_json::from_value(redact_value(
        &serde_json::to_value(checkpoint).context("serialize checkpoint for redaction")?,
    ))
    .context("deserialize redacted checkpoint")?;
    let json = serde_json::to_string(&checkpoint)?;
    transaction.execute(
        "INSERT INTO checkpoints(id, repository_id, task_id, session_id, created_at, checkpoint_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET checkpoint_json=excluded.checkpoint_json",
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

fn upsert_fact_tx(transaction: &Transaction<'_>, fact: &FactV1) -> Result<()> {
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

fn upsert_evidence_tx(transaction: &Transaction<'_>, evidence: &EvidenceV1) -> Result<()> {
    let mut evidence = evidence.clone();
    evidence.excerpt = redact_excerpt(&evidence.excerpt);
    evidence.excerpt_sha256 = hex::encode(sha2::Sha256::digest(evidence.excerpt.as_bytes()));
    let json = serde_json::to_string(&evidence)?;
    transaction.execute(
        "INSERT INTO evidence(id, repository_id, task_id, fact_id, session_id, created_at, evidence_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET fact_id=excluded.fact_id, evidence_json=excluded.evidence_json",
        params![evidence.id, evidence.repository_id, evidence.task_id, evidence.fact_id, evidence.session_id, timestamp(evidence.created_at), json],
    )?;
    Ok(())
}

fn upsert_file_change_tx(transaction: &Transaction<'_>, change: &FileChangeV1) -> Result<()> {
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
         ON CONFLICT(id) DO UPDATE SET change_json=excluded.change_json",
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

fn upsert_test_result_tx(transaction: &Transaction<'_>, test: &TestResultV1) -> Result<()> {
    let mut test = test.clone();
    test.command = redact_text(&test.command);
    test.summary = test.summary.map(|summary| redact_excerpt(&summary));
    let json = serde_json::to_string(&test)?;
    transaction.execute(
        "INSERT INTO test_results(id, repository_id, task_id, session_id, occurred_at, test_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET test_json=excluded.test_json",
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

fn load_events(
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

fn repository_tombstone_path(data_dir: &Path, repository_id: &str) -> PathBuf {
    let identity_hash = hex::encode(sha2::Sha256::digest(repository_id.as_bytes()));
    data_dir
        .join("purge-tombstones")
        .join(format!("{identity_hash}.json"))
}

fn acquire_database_lock(database_path: &Path) -> Result<fs::File> {
    let lock_path = PathBuf::from(format!("{}.lock", database_path.to_string_lossy()));
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)?;
        set_directory_permissions(parent)?;
    }
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&lock_path)
        .with_context(|| format!("open database maintenance lock {}", lock_path.display()))?;
    file.lock()
        .with_context(|| format!("lock database maintenance file {}", lock_path.display()))?;
    set_file_permissions(&lock_path)?;
    Ok(file)
}

fn write_private_atomic_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("atomic file path has no parent")?;
    fs::create_dir_all(parent)?;
    set_directory_permissions(parent)?;
    let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::now_v7()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)?;
    set_file_permissions(path)?;
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
    match fs::remove_file(&sidecar) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("remove database sidecar {}", sidecar.display()))
        }
    }
}

#[cfg(unix)]
fn set_directory_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_directory_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_file_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if path.exists() {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_file_permissions(_path: &Path) -> Result<()> {
    Ok(())
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
