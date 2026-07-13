#[cfg(unix)]
#[tokio::test]
async fn uses_only_documented_app_server_initialize_list_and_read_shapes() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use previously_on::app_server::AppServerClient;
    use tempfile::TempDir;

    let temp = TempDir::new().unwrap();
    let fake = temp.path().join("fake-codex");
    fs::write(
        &fake,
        r#"#!/bin/sh
IFS= read -r initialize
case "$initialize" in *'"method":"initialize"'*'"clientInfo"'*) ;; *) exit 10 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"codexHome":"/tmp/codex","platformFamily":"unix","platformOs":"macos","userAgent":"codex-cli/0.144.3"}}'
IFS= read -r initialized
case "$initialized" in *'"method":"initialized"'*) ;; *) exit 11 ;; esac
IFS= read -r list
case "$list" in *'"method":"thread/list"'*'"cwd":"/tmp/repo"'*) ;; *) exit 12 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"data":[{"cliVersion":"0.144.3","createdAt":100,"cwd":"/tmp/repo","ephemeral":false,"id":"thread-1","modelProvider":"openai","preview":"hello","sessionId":"session-1","source":"cli","status":{"type":"idle"},"turns":[],"updatedAt":101}],"nextCursor":"page-2"}}'
IFS= read -r list_page_2
case "$list_page_2" in *'"method":"thread/list"'*'"cursor":"page-2"'*) ;; *) exit 16 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"data":[{"cliVersion":"0.144.2","createdAt":90,"cwd":"/tmp/repo","ephemeral":false,"id":"thread-2","modelProvider":"openai","preview":"older","sessionId":"session-2","source":"cli","status":{"type":"idle"},"turns":[],"updatedAt":91}],"nextCursor":null}}'
IFS= read -r read_thread
case "$read_thread" in *'"method":"thread/read"'*) ;; *) exit 13 ;; esac
case "$read_thread" in *'"threadId":"thread-1"'*) ;; *) exit 14 ;; esac
case "$read_thread" in *'"includeTurns":true'*) ;; *) exit 15 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":4,"result":{"thread":{"cliVersion":"0.144.3","createdAt":100,"cwd":"/tmp/repo","ephemeral":false,"id":"thread-1","modelProvider":"openai","preview":"hello","sessionId":"session-1","source":"cli","status":{"type":"idle"},"turns":[{"id":"turn-1","items":[],"status":"completed"}],"updatedAt":101}}}'
"#,
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).unwrap();

    let mut client = AppServerClient::connect_with_program(&fake).await.unwrap();
    assert_eq!(client.initialize_result()["platformOs"], "macos");
    let listed = client
        .list_threads(Some(std::path::Path::new("/tmp/repo")))
        .await
        .unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, "thread-1");
    let read = client.read_thread("thread-1").await.unwrap();
    assert_eq!(read["turns"].as_array().unwrap().len(), 1);
    client.shutdown().await.unwrap();
}

#[cfg(unix)]
fn fake_codex(script: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::TempDir::new().unwrap();
    let fake = temp.path().join("fake-codex");
    fs::write(&fake, script).unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).unwrap();
    (temp, fake)
}

#[cfg(unix)]
#[tokio::test]
async fn isolates_deleted_and_degraded_threads_and_preserves_rpc_error_fields() {
    use previously_on::app_server::{
        AppServerClient, ThreadImportDisposition, TESTED_CODEX_VERSION,
    };
    use previously_on::domain::CoverageStatus;

    let (_temp, fake) = fake_codex(
        r#"#!/bin/sh
IFS= read -r initialize
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"userAgent":"codex-cli/0.144.3"}}'
IFS= read -r initialized
IFS= read -r list
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"data":[{"cliVersion":"0.144.3","createdAt":3,"cwd":"/tmp/repo","id":"thread-deleted","preview":"gone","sessionId":"session-deleted","updatedAt":4},{"cliVersion":"0.141.0","createdAt":2,"cwd":"/tmp/repo","id":"thread-compact","preview":"compact","sessionId":"session-compact","updatedAt":3},{"cliVersion":"0.144.2","createdAt":1,"cwd":"/tmp/repo","id":"thread-good","preview":"good","updatedAt":2}],"nextCursor":null}}'
IFS= read -r deleted
printf '%s\n' '{"jsonrpc":"2.0","id":3,"error":{"code":-32004,"message":"thread not found","data":{"kind":"thread_not_found","threadId":"thread-deleted"}}}'
IFS= read -r compact
printf '%s\n' '{"jsonrpc":"2.0","id":4,"result":{"thread":{"compacted":true,"status":{"type":"incomplete"},"turns":[{"items":[{"type":"futureItem","payload":"untrusted"}],"status":"interrupted"}]}}}'
IFS= read -r good
printf '%s\n' '{"jsonrpc":"2.0","id":5,"result":{"thread":{"turns":[{"id":"turn-good","items":[{"id":"item-good","type":"agentMessage"}],"status":"completed"}]}}}'
"#,
    );

    let mut client = AppServerClient::connect_with_program(&fake).await.unwrap();
    let report = client
        .import_threads_report(std::path::Path::new("/tmp/repo"))
        .await
        .unwrap();

    assert_eq!(report.threads.len(), 2);
    assert_eq!(report.coverage.status, CoverageStatus::Degraded);
    let deleted = report
        .notices
        .iter()
        .find(|notice| notice.thread_id.as_deref() == Some("thread-deleted"))
        .unwrap();
    assert_eq!(deleted.disposition, ThreadImportDisposition::Skipped);
    let error = deleted.rpc_error.as_ref().unwrap();
    assert_eq!(error.code, Some(-32004));
    assert_eq!(error.data.as_ref().unwrap()["kind"], "thread_not_found");

    let compact = report
        .threads
        .iter()
        .find(|thread| thread.id == "thread-compact")
        .unwrap();
    assert_eq!(compact.coverage.status, CoverageStatus::Degraded);
    assert!(compact
        .coverage
        .warnings
        .iter()
        .any(|warning| warning.contains(TESTED_CODEX_VERSION)));
    assert!(compact.thread["turns"][0]["id"]
        .as_str()
        .unwrap()
        .starts_with("app-import-"));
    assert!(compact.thread["turns"][0]["items"][0]["id"]
        .as_str()
        .unwrap()
        .starts_with("app-import-"));

    let good = report
        .threads
        .iter()
        .find(|thread| thread.id == "thread-good")
        .unwrap();
    assert!(good.session_id.starts_with("app-import-"));
    assert_eq!(good.coverage.status, CoverageStatus::Degraded);
    assert!(good
        .coverage
        .warnings
        .iter()
        .any(|warning| warning.contains("without payload-hash deduplication")));
    client.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn repeated_cursor_keeps_validated_pages_and_stops_degraded() {
    use previously_on::app_server::AppServerClient;
    use previously_on::domain::CoverageStatus;

    let (_temp, fake) = fake_codex(
        r#"#!/bin/sh
IFS= read -r initialize
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"userAgent":"codex-cli/0.144.3"}}'
IFS= read -r initialized
IFS= read -r list
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"data":[{"cliVersion":"0.144.3","createdAt":1,"cwd":"/tmp/repo","id":"thread-1","preview":"one","sessionId":"session-1","updatedAt":2}],"nextCursor":"repeat"}}'
IFS= read -r list_again
printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"data":[{"cliVersion":"0.144.2","createdAt":2,"cwd":"/tmp/repo","id":"thread-2","preview":"two","sessionId":"session-2","updatedAt":3}],"nextCursor":"repeat"}}'
"#,
    );
    let mut client = AppServerClient::connect_with_program(&fake).await.unwrap();
    let report = client
        .list_threads_report(Some(std::path::Path::new("/tmp/repo")))
        .await
        .unwrap();
    assert_eq!(report.threads.len(), 2);
    assert_eq!(report.coverage.status, CoverageStatus::Degraded);
    assert!(report
        .coverage
        .warnings
        .iter()
        .any(|warning| warning.contains("repeated cursor")));
    client.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn malformed_page_is_contained_without_accepting_partial_items() {
    use previously_on::app_server::AppServerClient;
    use previously_on::domain::CoverageStatus;

    let (_temp, fake) = fake_codex(
        r#"#!/bin/sh
IFS= read -r initialize
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"userAgent":"codex-cli/0.144.3"}}'
IFS= read -r initialized
IFS= read -r list
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"data":"not-an-array","nextCursor":null}}'
"#,
    );
    let mut client = AppServerClient::connect_with_program(&fake).await.unwrap();
    let report = client.list_threads_report(None).await.unwrap();
    assert!(report.threads.is_empty());
    assert_eq!(report.coverage.status, CoverageStatus::Degraded);
    assert!(report
        .coverage
        .warnings
        .iter()
        .any(|warning| warning.contains("invalid Codex thread/list response")));
    client.shutdown().await.unwrap();
}

#[test]
fn documented_turn_items_project_to_bounded_idempotent_lineage_events() {
    use std::process::Command;

    use previously_on::app_server::{project_thread_events, ImportedThreadV1};
    use previously_on::domain::{ChangeAttribution, CoverageV1, EventKind, TestStatus};
    use previously_on::hook::{ingest_hook_event, HookDeliveryStatus};
    use previously_on::store::Store;
    use serde_json::json;
    use tempfile::TempDir;

    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    assert!(Command::new("git")
        .args(["init", repo.to_str().unwrap()])
        .status()
        .unwrap()
        .success());
    std::fs::write(repo.join("src/auth.rs"), "pub fn auth() {}\n").unwrap();
    assert!(Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["add", "src/auth.rs"])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args([
            "-c",
            "user.name=PreviouslyOn Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "fixture",
        ])
        .status()
        .unwrap()
        .success());

    let identity = previously_on::git::repository_identity(&repo).unwrap();
    let imported = ImportedThreadV1 {
        schema_version: 1,
        id: "thread-semantic".to_string(),
        session_id: "session-semantic".to_string(),
        cwd: repo.clone(),
        cli_version: "0.144.3".to_string(),
        created_at: 1_700_000_000,
        updated_at: 1_700_000_010,
        coverage: CoverageV1::default(),
        thread: json!({
            "turns": [{
                "id": "turn-1",
                "status": "completed",
                "items": [
                    {
                        "id": "item-user",
                        "type": "userMessage",
                        "content": [{"type": "text", "text": "Continue auth work\nConstraint: token=do-not-store"}]
                    },
                    {
                        "id": "item-command",
                        "type": "commandExecution",
                        "command": "cargo test -- auth",
                        "status": "completed",
                        "aggregatedOutput": "1 passed; Authorization: Bearer do-not-store",
                        "exitCode": 0
                    },
                    {
                        "id": "item-file",
                        "type": "fileChange",
                        "status": "completed",
                        "changes": [{
                            "path": "src/auth.rs",
                            "kind": "update",
                            "diff": "+ password=do-not-store"
                        }]
                    },
                    {
                        "id": "item-final",
                        "type": "agentMessage",
                        "phase": "final_answer",
                        "text": "Auth work continued and tests pass."
                    }
                ]
            }]
        }),
    };

    let projection = project_thread_events(&imported, &identity.id, &identity.root);
    assert_eq!(projection.events.len(), 6);
    assert!(projection
        .events
        .iter()
        .any(|event| event.kind == EventKind::UserPrompt));
    assert!(projection
        .events
        .iter()
        .any(|event| event.kind == EventKind::AssistantFinal));
    assert_eq!(
        projection
            .events
            .iter()
            .map(|event| event.dedupe_key.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        projection.events.len()
    );
    let projected_json = serde_json::to_string(&projection).unwrap();
    assert!(!projected_json.contains("password=do-not-store"));
    assert!(!projected_json.contains("Bearer do-not-store"));

    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    for event in projection.events.clone() {
        assert_eq!(
            ingest_hook_event(&store, event).unwrap().status,
            HookDeliveryStatus::Persisted
        );
    }
    for event in projection.events {
        assert_eq!(
            ingest_hook_event(&store, event).unwrap().status,
            HookDeliveryStatus::Duplicate
        );
    }

    let tasks = store.search_tasks("Continue auth", 10).unwrap();
    assert_eq!(tasks.len(), 1);
    let task_id = &tasks[0].task.id;
    let changes = store.list_file_changes(task_id).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].path, "src/auth.rs");
    assert_eq!(changes[0].attribution, ChangeAttribution::ObservedChangedIn);
    let tests = store.list_test_results(task_id).unwrap();
    assert_eq!(tests.len(), 1);
    assert_eq!(tests[0].status, TestStatus::Passed);
    assert_eq!(store.list_checkpoints(task_id).unwrap().len(), 1);
    let exported = store.export_json(None).unwrap();
    let export = exported.to_string();
    assert!(export.contains("Auth work continued and tests pass."));
    assert!(
        !export.contains("do-not-store"),
        "secret remained at JSON paths: {:?}",
        json_paths_containing(&exported, "do-not-store", "$"),
    );
}

fn json_paths_containing(value: &serde_json::Value, needle: &str, prefix: &str) -> Vec<String> {
    match value {
        serde_json::Value::String(text) if text.contains(needle) => vec![prefix.to_string()],
        serde_json::Value::Array(values) => values
            .iter()
            .enumerate()
            .flat_map(|(index, value)| {
                json_paths_containing(value, needle, &format!("{prefix}[{index}]"))
            })
            .collect(),
        serde_json::Value::Object(values) => values
            .iter()
            .flat_map(|(key, value)| {
                json_paths_containing(value, needle, &format!("{prefix}.{key}"))
            })
            .collect(),
        _ => Vec::new(),
    }
}
