use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::{watch, Mutex};

use crate::app_server::{
    AppServerCapabilityReport, AppServerCapabilityStatus, AppServerClient, ThreadImportNoticeV1,
};
use crate::domain::{CoverageStatus, CoverageV1, EventEnvelopeV1, EventKind, SCHEMA_VERSION_V1};
use crate::setup::{self, SetupProjectV2};
use crate::store::{InsertOutcome, Store};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexImportReportV1 {
    pub schema_version: u16,
    pub repository_id: String,
    pub status: AppServerCapabilityStatus,
    pub imported_task_count: usize,
    pub semantic_event_count: usize,
    pub duplicate_count: usize,
    pub missing_or_unknown_items: Vec<String>,
    pub last_synced_at: DateTime<Utc>,
    pub capability: AppServerCapabilityReport,
    pub coverage: CoverageV1,
    pub semantic_coverage: CoverageV1,
    pub notices: Vec<ThreadImportNoticeV1>,
    pub observed_agent_count: usize,
    pub technical_details: Vec<String>,
}

type FlightResult = std::result::Result<CodexImportReportV1, String>;
type ImportFuture = Pin<Box<dyn Future<Output = Result<CodexImportReportV1>> + Send>>;
type ImportExecutor = dyn Fn(SetupProjectV2) -> ImportFuture + Send + Sync;

#[derive(Clone)]
pub struct CodexImportService {
    manifest_path: Arc<PathBuf>,
    flights: Arc<Mutex<HashMap<String, watch::Receiver<Option<FlightResult>>>>>,
    executor: Arc<ImportExecutor>,
}

impl CodexImportService {
    pub fn new(database_path: impl Into<PathBuf>, manifest_path: impl Into<PathBuf>) -> Self {
        Self::with_program(database_path, manifest_path, "codex")
    }

    pub fn with_program(
        database_path: impl Into<PathBuf>,
        manifest_path: impl Into<PathBuf>,
        program: impl Into<PathBuf>,
    ) -> Self {
        let database_path = Arc::new(database_path.into());
        let program = Arc::new(program.into());
        let executor = Arc::new(move |project: SetupProjectV2| {
            let database_path = Arc::clone(&database_path);
            let program = Arc::clone(&program);
            Box::pin(async move { execute_import(&database_path, &program, project).await })
                as ImportFuture
        });
        Self {
            manifest_path: Arc::new(manifest_path.into()),
            flights: Arc::new(Mutex::new(HashMap::new())),
            executor,
        }
    }

    pub async fn sync_repository(&self, repository_id: &str) -> Result<CodexImportReportV1> {
        let repository_id = repository_id.trim();
        if repository_id.is_empty() {
            bail!("repositoryId is required");
        }
        let manifest = setup::read_manifest(&self.manifest_path)?;
        let project = manifest
            .projects
            .into_iter()
            .find(|project| project.repository_id == repository_id)
            .context("repository is not registered")?;
        self.sync_project(project).await
    }

    async fn sync_project(&self, project: SetupProjectV2) -> Result<CodexImportReportV1> {
        let repository_id = project.repository_id.clone();
        let (leader, mut receiver, sender) = {
            let mut flights = self.flights.lock().await;
            if let Some(receiver) = flights.get(&repository_id) {
                (false, receiver.clone(), None)
            } else {
                let (sender, receiver) = watch::channel(None);
                flights.insert(repository_id.clone(), receiver.clone());
                (true, receiver, Some(sender))
            }
        };

        if !leader {
            loop {
                if let Some(result) = receiver.borrow().clone() {
                    return flight_result(result);
                }
                receiver
                    .changed()
                    .await
                    .context("Codex import flight ended without a result")?;
            }
        }

        let result = (self.executor)(project)
            .await
            .map_err(|error| crate::redaction::redact_excerpt(&format!("{error:#}")));
        if let Some(sender) = sender {
            sender.send(Some(result.clone())).ok();
        }
        self.flights.lock().await.remove(&repository_id);
        flight_result(result)
    }
}

fn flight_result(result: FlightResult) -> Result<CodexImportReportV1> {
    result.map_err(anyhow::Error::msg)
}

async fn execute_import(
    database_path: &Path,
    program: &Path,
    project: SetupProjectV2,
) -> Result<CodexImportReportV1> {
    let repository_id = project.repository_id.clone();
    let store = Store::open(database_path)?;
    let mut client = match AppServerClient::connect_with_program(program).await {
        Ok(client) => client,
        Err(error) => {
            let capability = AppServerCapabilityReport::unsupported(
                crate::redaction::redact_excerpt(&format!("{error:#}")),
            );
            return Ok(empty_report(
                repository_id,
                AppServerCapabilityStatus::Unsupported,
                capability.clone(),
                capability.warnings,
            ));
        }
    };
    let capability = client.capability_report().await;
    if capability.capabilities.core_import == AppServerCapabilityStatus::Unsupported {
        client.shutdown().await.ok();
        return Ok(empty_report(
            repository_id,
            AppServerCapabilityStatus::Unsupported,
            capability.clone(),
            capability.warnings.clone(),
        ));
    }

    let import_report = match client.import_threads_report(&project.primary_root).await {
        Ok(report) => report,
        Err(error) => {
            client.shutdown().await.ok();
            let detail = crate::redaction::redact_excerpt(&format!(
                "Codex App Server import could not be interpreted: {error:#}"
            ));
            let mut report = empty_report(
                repository_id,
                AppServerCapabilityStatus::Degraded,
                capability,
                vec![detail.clone()],
            );
            report.coverage.status = CoverageStatus::Degraded;
            report.coverage.missing.push("thread import".to_string());
            report.coverage.warnings.push(detail);
            report.missing_or_unknown_items = vec!["thread import".to_string()];
            return Ok(report);
        }
    };

    let imported_task_count = import_report.threads.len();
    let mut semantic_event_count = 0usize;
    let mut duplicate_count = 0usize;
    let mut semantic_coverages = Vec::new();
    for thread in import_report.threads {
        let projection =
            crate::app_server::project_thread_events(&thread, &repository_id, &thread.cwd);
        semantic_coverages.push(projection.coverage.clone());
        for event in projection.events {
            let acknowledgement = crate::hook::ingest_hook_event(&store, event)?;
            semantic_event_count += 1;
            if acknowledgement.status == crate::hook::HookDeliveryStatus::Duplicate {
                duplicate_count += 1;
            }
        }
        let event = imported_thread_marker(&thread, &repository_id);
        if store.insert_event(&event)? == InsertOutcome::Duplicate {
            duplicate_count += 1;
        }
    }
    client.shutdown().await?;

    let agent_lineage = match AppServerClient::connect_with_program_experimental(program).await {
        Ok(mut lineage_client) => {
            let observed = crate::app_server::collect_agent_lineage(
                &mut lineage_client,
                &store,
                &project.primary_root,
                &repository_id,
            )
            .await;
            lineage_client.shutdown().await.ok();
            observed
        }
        Err(error) => Err(error),
    };
    let (observed_agent_count, lineage_detail) = match agent_lineage {
        Ok(agents) => (agents.len(), None),
        Err(error) => (
            0,
            Some(crate::redaction::redact_excerpt(&format!(
                "agent lineage unavailable: {error:#}"
            ))),
        ),
    };

    let semantic_coverage = CoverageV1::merge(semantic_coverages.iter());
    let mut missing_or_unknown_items = BTreeSet::new();
    missing_or_unknown_items.extend(import_report.coverage.missing.iter().cloned());
    missing_or_unknown_items.extend(semantic_coverage.missing.iter().cloned());
    for notice in &import_report.notices {
        missing_or_unknown_items.extend(notice.coverage.missing.iter().cloned());
    }
    let mut technical_details = BTreeSet::new();
    technical_details.extend(capability.warnings.iter().cloned());
    technical_details.extend(import_report.coverage.warnings.iter().cloned());
    technical_details.extend(semantic_coverage.warnings.iter().cloned());
    technical_details.extend(
        import_report
            .notices
            .iter()
            .map(|notice| notice.message.clone()),
    );
    if let Some(detail) = lineage_detail {
        technical_details.insert(detail);
    }
    let degraded = capability.capabilities.core_import != AppServerCapabilityStatus::Complete
        || import_report.coverage.status != CoverageStatus::Complete
        || semantic_coverage.status != CoverageStatus::Complete
        || !missing_or_unknown_items.is_empty();

    Ok(CodexImportReportV1 {
        schema_version: SCHEMA_VERSION_V1,
        repository_id,
        status: if degraded {
            AppServerCapabilityStatus::Degraded
        } else {
            AppServerCapabilityStatus::Complete
        },
        imported_task_count,
        semantic_event_count,
        duplicate_count,
        missing_or_unknown_items: missing_or_unknown_items.into_iter().collect(),
        last_synced_at: Utc::now(),
        capability,
        coverage: import_report.coverage,
        semantic_coverage,
        notices: import_report.notices,
        observed_agent_count,
        technical_details: technical_details.into_iter().collect(),
    })
}

fn imported_thread_marker(
    thread: &crate::app_server::ImportedThreadV1,
    repository_id: &str,
) -> EventEnvelopeV1 {
    let turns = thread
        .thread
        .get("turns")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let payload = json!({
        "id": thread.id,
        "sessionId": thread.session_id,
        "cwd": thread.cwd,
        "cliVersion": thread.cli_version,
        "createdAt": thread.created_at,
        "updatedAt": thread.updated_at,
        "turnCount": turns,
        "rawTranscriptStored": false
    });
    let occurred_at = DateTime::from_timestamp(thread.updated_at, 0).unwrap_or_else(Utc::now);
    let mut event = EventEnvelopeV1::new(
        format!("codex-app-server:thread:read:{}", thread.id),
        repository_id,
        thread.session_id.clone(),
        EventKind::Unknown,
        occurred_at,
        payload,
    );
    event.coverage = thread.coverage.clone();
    event.coverage.status = event.coverage.status.worst(CoverageStatus::Degraded);
    event.coverage.captured.extend([
        "thread/list".to_string(),
        "thread/read".to_string(),
        "allowlisted semantic item projection".to_string(),
    ]);
    event
}

fn empty_report(
    repository_id: String,
    status: AppServerCapabilityStatus,
    capability: AppServerCapabilityReport,
    technical_details: Vec<String>,
) -> CodexImportReportV1 {
    CodexImportReportV1 {
        schema_version: SCHEMA_VERSION_V1,
        repository_id,
        status,
        imported_task_count: 0,
        semantic_event_count: 0,
        duplicate_count: 0,
        missing_or_unknown_items: vec!["Codex App Server import".to_string()],
        last_synced_at: Utc::now(),
        capability,
        coverage: CoverageV1::default(),
        semantic_coverage: CoverageV1::default(),
        notices: Vec::new(),
        observed_agent_count: 0,
        technical_details,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn project(repository_id: &str) -> SetupProjectV2 {
        SetupProjectV2 {
            repository_id: repository_id.to_string(),
            primary_root: PathBuf::from(format!("/tmp/{repository_id}")),
            known_worktree_roots: Vec::new(),
            registered_at: Utc::now().to_rfc3339(),
        }
    }

    fn report(repository_id: &str) -> CodexImportReportV1 {
        let capability = AppServerCapabilityReport::unsupported("test");
        empty_report(
            repository_id.to_string(),
            AppServerCapabilityStatus::Unsupported,
            capability,
            vec!["test".to_string()],
        )
    }

    fn service(executor: Arc<ImportExecutor>) -> CodexImportService {
        CodexImportService {
            manifest_path: Arc::new(PathBuf::from("unused")),
            flights: Arc::new(Mutex::new(HashMap::new())),
            executor,
        }
    }

    #[test]
    fn repeated_thread_markers_keep_the_same_dedupe_key() {
        let thread = crate::app_server::ImportedThreadV1 {
            schema_version: 1,
            id: "thread-repeat".to_string(),
            session_id: "session-repeat".to_string(),
            cwd: PathBuf::from("/tmp/repo-a-worktree"),
            cli_version: "test".to_string(),
            created_at: 100,
            updated_at: 101,
            coverage: CoverageV1::default(),
            thread: json!({ "turns": [] }),
        };

        let first = imported_thread_marker(&thread, "repo-a");
        let repeated = imported_thread_marker(&thread, "repo-a");
        assert_eq!(first.dedupe_key, repeated.dedupe_key);

        let temp = tempfile::TempDir::new().unwrap();
        let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
        assert_eq!(store.insert_event(&first).unwrap(), InsertOutcome::Inserted);
        assert_eq!(
            store.insert_event(&repeated).unwrap(),
            InsertOutcome::Duplicate
        );
    }

    #[tokio::test]
    async fn concurrent_syncs_for_one_repository_share_one_execution() {
        let calls = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let executor = {
            let calls = Arc::clone(&calls);
            let started = Arc::clone(&started);
            let release = Arc::clone(&release);
            Arc::new(move |project: SetupProjectV2| {
                let calls = Arc::clone(&calls);
                let started = Arc::clone(&started);
                let release = Arc::clone(&release);
                Box::pin(async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    started.notify_one();
                    release.notified().await;
                    Ok(report(&project.repository_id))
                }) as ImportFuture
            }) as Arc<ImportExecutor>
        };
        let service = service(executor);
        let first_service = service.clone();
        let first =
            tokio::spawn(async move { first_service.sync_project(project("repo-a")).await });
        started.notified().await;
        let second_service = service.clone();
        let second =
            tokio::spawn(async move { second_service.sync_project(project("repo-a")).await });
        tokio::task::yield_now().await;
        release.notify_one();

        let first = first.await.unwrap().unwrap();
        let second = second.await.unwrap().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(first.repository_id, "repo-a");
        assert_eq!(second.repository_id, "repo-a");
        assert_eq!(first.last_synced_at, second.last_synced_at);
    }

    #[tokio::test]
    async fn different_repositories_execute_independently() {
        let calls = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let executor = {
            let calls = Arc::clone(&calls);
            let barrier = Arc::clone(&barrier);
            Arc::new(move |project: SetupProjectV2| {
                let calls = Arc::clone(&calls);
                let barrier = Arc::clone(&barrier);
                Box::pin(async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    barrier.wait().await;
                    Ok(report(&project.repository_id))
                }) as ImportFuture
            }) as Arc<ImportExecutor>
        };
        let service = service(executor);
        let first_service = service.clone();
        let first =
            tokio::spawn(async move { first_service.sync_project(project("repo-a")).await });
        let second_service = service.clone();
        let second =
            tokio::spawn(async move { second_service.sync_project(project("repo-b")).await });
        barrier.wait().await;

        let first = first.await.unwrap().unwrap();
        let second = second.await.unwrap().unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(first.repository_id, "repo-a");
        assert_eq!(second.repository_id, "repo-b");
    }
}
