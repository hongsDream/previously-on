use super::private_fs::{
    acquire_database_lock, ensure_private_directory, open_private_file, read_private_file,
    remove_file_and_sync_parent, remove_sidecar_if_present, repository_tombstone_path,
    validate_private_directory, validate_private_regular_file, write_private_atomic_file,
};
use super::{query_json_rows, timestamp, RetentionReport, Store};
use crate::contracts::ContractEvaluationV1;
use crate::domain::{EventEnvelopeV1, EventKind, EvidenceV1, FactV1, TaskV1, SCHEMA_VERSION_V1};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const PURGE_TOMBSTONE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PurgeJournalPhase {
    Tombstoned,
    RelatedDataPurged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PurgeRecoveryJournalV1 {
    version: u32,
    pub(super) repository_id: String,
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

impl Store {
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

    pub(super) fn acquire_maintenance_lock(&self) -> Result<fs::File> {
        acquire_database_lock(&self.path)
    }

    fn purge_journal_path(&self) -> PathBuf {
        PathBuf::from(format!(
            "{}.purge-recovery.json",
            self.path.to_string_lossy()
        ))
    }

    pub(super) fn read_purge_journal(&self) -> Result<Option<PurgeRecoveryJournalV1>> {
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

    pub(super) fn recover_purge_if_ready(&self) -> Result<()> {
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
}
