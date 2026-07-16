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
#[tokio::test]
async fn uses_documented_start_name_turn_and_resume_shapes() {
    use previously_on::app_server::AppServerClient;

    let (_temp, fake) = fake_codex(
        r#"#!/bin/sh
IFS= read -r initialize
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"userAgent":"codex-cli/0.144.3"}}'
IFS= read -r initialized
IFS= read -r start
case "$start" in *'"method":"thread/start"'*) ;; *) exit 10 ;; esac
case "$start" in *'"cwd":"/tmp/repo"'*) ;; *) exit 11 ;; esac
case "$start" in *'"ephemeral":false'*) ;; *) exit 12 ;; esac
case "$start" in *'"serviceName":"previously-on"'*) ;; *) exit 13 ;; esac
case "$start" in *'"model":"gpt-5.6-sol"'*) ;; *) exit 14 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"thread":{"id":"thread-fresh","sessionId":"session-fresh"}}}'
IFS= read -r name
case "$name" in *'"method":"thread/name/set"'*) ;; *) exit 15 ;; esac
case "$name" in *'"threadId":"thread-fresh"'*) ;; *) exit 16 ;; esac
case "$name" in *'"name":"Task name"'*) ;; *) exit 17 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{}}'
IFS= read -r turn
case "$turn" in *'"method":"turn/start"'*) ;; *) exit 18 ;; esac
case "$turn" in *'"threadId":"thread-fresh"'*) ;; *) exit 19 ;; esac
case "$turn" in *'"clientUserMessageId":"message-1"'*) ;; *) exit 20 ;; esac
case "$turn" in *'"type":"text"'*) ;; *) exit 21 ;; esac
case "$turn" in *'"text":"Continue safely"'*) ;; *) exit 24 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":4,"result":{"turn":{"id":"turn-fresh"}}}'
IFS= read -r resume
case "$resume" in *'"method":"thread/resume"'*) ;; *) exit 22 ;; esac
case "$resume" in *'"threadId":"thread-fresh"'*) ;; *) exit 23 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":5,"result":{"thread":{"id":"thread-fresh","sessionId":"session-fresh"}}}'
"#,
    );

    let mut client = AppServerClient::connect_with_program(&fake).await.unwrap();
    let thread = client
        .start_thread(std::path::Path::new("/tmp/repo"), Some("gpt-5.6-sol"))
        .await
        .unwrap();
    assert_eq!(thread.id, "thread-fresh");
    assert_eq!(thread.session_id, "session-fresh");
    client
        .set_thread_name(&thread.id, "Task name")
        .await
        .unwrap();
    let turn = client
        .start_turn(
            &thread.id,
            std::path::Path::new("/tmp/repo"),
            "Continue safely",
            Some("gpt-5.6-sol"),
            "message-1",
        )
        .await
        .unwrap();
    assert_eq!(turn.id, "turn-fresh");
    let resumed = client.resume_thread(&thread.id).await.unwrap();
    assert_eq!(resumed, thread);
    client.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_an_oversized_unterminated_app_server_frame() {
    use previously_on::app_server::{AppServerClient, MAX_APP_SERVER_RPC_BYTES};

    let script = format!(
        "#!/bin/sh\nIFS= read -r initialize\nhead -c {} /dev/zero | tr '\\000' x\n",
        MAX_APP_SERVER_RPC_BYTES + 1
    );
    let (_temp, fake) = fake_codex(&script);
    let error = match AppServerClient::connect_with_program(&fake).await {
        Ok(client) => {
            client.shutdown().await.ok();
            panic!("oversized App Server frame was accepted")
        }
        Err(error) => error,
    };
    assert!(format!("{error:#}").contains("JSON-RPC frame exceeds"));
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
fn git_repository() -> (tempfile::TempDir, std::path::PathBuf) {
    use std::process::Command;

    let temp = tempfile::TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    let status = Command::new("git")
        .args(["init", "--quiet"])
        .arg(&repo)
        .status()
        .unwrap();
    assert!(status.success());
    (temp, repo)
}

#[cfg(unix)]
#[tokio::test]
async fn import_verifies_returned_cwd_and_accepts_only_the_registered_logical_repository() {
    use std::process::Command;

    use previously_on::app_server::{AppServerClient, ThreadImportDisposition};
    use previously_on::domain::CoverageStatus;

    let (_repo_temp, repo) = git_repository();
    for args in [
        ["config", "user.email", "tests@previously.local"].as_slice(),
        ["config", "user.name", "PreviouslyOn Tests"].as_slice(),
    ] {
        assert!(Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .status()
            .unwrap()
            .success());
    }
    std::fs::write(repo.join("tracked.txt"), "baseline\n").unwrap();
    assert!(Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["add", "tracked.txt"])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["commit", "--quiet", "-m", "baseline"])
        .status()
        .unwrap()
        .success());

    let linked = _repo_temp.path().join("linked");
    assert!(Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["worktree", "add", "--quiet", "--detach"])
        .arg(&linked)
        .arg("HEAD")
        .status()
        .unwrap()
        .success());

    // A nested repository is physically contained by the registered checkout but belongs to a
    // different logical Git repository and must never be projected into the parent.
    let nested = repo.join("nested-other-repository");
    assert!(Command::new("git")
        .args(["init", "--quiet"])
        .arg(&nested)
        .status()
        .unwrap()
        .success());

    let script = r#"#!/bin/sh
IFS= read -r initialize
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"userAgent":"codex-cli/0.144.3"}}'
IFS= read -r initialized
IFS= read -r list
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"data":[{"cliVersion":"0.144.3","createdAt":4,"cwd":"__NESTED__","id":"thread-nested","preview":"wrong repository","sessionId":"session-nested","updatedAt":5},{"cliVersion":"0.144.3","createdAt":3,"cwd":"relative/repository","id":"thread-relative","preview":"relative","sessionId":"session-relative","updatedAt":4},{"cliVersion":"0.144.3","createdAt":2,"id":"thread-missing","preview":"missing cwd","sessionId":"session-missing","updatedAt":3},{"cliVersion":"0.144.3","createdAt":1,"cwd":"__LINKED__","id":"thread-linked","preview":"same logical repository","sessionId":"session-linked","updatedAt":2}],"nextCursor":null}}'
IFS= read -r read_thread
case "$read_thread" in *'"method":"thread/read"'*'"threadId":"thread-linked"'*) ;; *) exit 20 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"thread":{"turns":[{"id":"turn-linked","items":[],"status":"completed"}]}}}'
"#
    .replace("__NESTED__", nested.to_str().unwrap())
    .replace("__LINKED__", linked.to_str().unwrap());
    let (_fake_temp, fake) = fake_codex(&script);

    let mut client = AppServerClient::connect_with_program(&fake).await.unwrap();
    let report = client.import_threads_report(&repo).await.unwrap();

    assert_eq!(report.coverage.status, CoverageStatus::Degraded);
    assert_eq!(report.threads.len(), 1);
    assert_eq!(report.threads[0].id, "thread-linked");
    assert_eq!(report.threads[0].cwd, linked);
    assert!(report
        .threads
        .iter()
        .all(|thread| thread.id != "thread-nested"));
    assert!(report
        .notices
        .iter()
        .filter(|notice| notice.disposition == ThreadImportDisposition::Skipped)
        .any(|notice| {
            notice.thread_id.as_deref() == Some("thread-nested")
                && notice.message.contains("different logical Git repository")
        }));
    assert!(report.notices.iter().any(|notice| {
        notice.thread_id.as_deref() == Some("thread-relative")
            && notice.message.contains("non-absolute cwd")
    }));
    assert!(report
        .coverage
        .warnings
        .iter()
        .any(|warning| warning.contains("omitted non-empty `cwd`")));
    client.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn isolates_deleted_and_degraded_threads_and_preserves_rpc_error_fields() {
    use previously_on::app_server::{
        AppServerClient, ThreadImportDisposition, TESTED_CODEX_VERSION,
    };
    use previously_on::domain::CoverageStatus;

    let (_repo_temp, repo) = git_repository();
    let script =
        r#"#!/bin/sh
IFS= read -r initialize
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"userAgent":"codex-cli/0.144.3"}}'
IFS= read -r initialized
IFS= read -r list
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"data":[{"cliVersion":"0.144.3","createdAt":3,"cwd":"__REPO__","id":"thread-deleted","preview":"gone","sessionId":"session-deleted","updatedAt":4},{"cliVersion":"0.141.0","createdAt":2,"cwd":"__REPO__","id":"thread-compact","preview":"compact","sessionId":"session-compact","updatedAt":3},{"cliVersion":"0.144.2","createdAt":1,"cwd":"__REPO__","id":"thread-good","preview":"good","updatedAt":2}],"nextCursor":null}}'
IFS= read -r deleted
printf '%s\n' '{"jsonrpc":"2.0","id":3,"error":{"code":-32004,"message":"thread not found; password=super-secret-rpc-password","data":{"kind":"thread_not_found","threadId":"thread-deleted","authorization":"Bearer super-secret-rpc-bearer","nested":{"password":"super-secret-rpc-data"}}}}'
IFS= read -r compact
printf '%s\n' '{"jsonrpc":"2.0","id":4,"result":{"thread":{"compacted":true,"status":{"type":"incomplete"},"turns":[{"items":[{"type":"futureItem","payload":"untrusted"}],"status":"interrupted"}]}}}'
IFS= read -r good
printf '%s\n' '{"jsonrpc":"2.0","id":5,"result":{"thread":{"turns":[{"id":"turn-good","items":[{"id":"item-good","type":"agentMessage"}],"status":"completed"}]}}}'
"#
        .replace("__REPO__", repo.to_str().unwrap());
    let (_temp, fake) = fake_codex(&script);

    let mut client = AppServerClient::connect_with_program(&fake).await.unwrap();
    let report = client.import_threads_report(&repo).await.unwrap();

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
    assert!(error.message.contains("password=[REDACTED]"));
    assert_eq!(error.data.as_ref().unwrap()["authorization"], "[REDACTED]");
    assert_eq!(
        error.data.as_ref().unwrap()["nested"]["password"],
        "[REDACTED]"
    );
    let serialized_report = serde_json::to_string(&report).unwrap();
    for secret in [
        "super-secret-rpc-password",
        "super-secret-rpc-bearer",
        "super-secret-rpc-data",
    ] {
        assert!(
            !serialized_report.contains(secret),
            "raw RPC secret remained in serialized import report"
        );
    }

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

#[cfg(unix)]
#[tokio::test]
async fn malformed_token_usage_notification_degrades_import_without_stopping_rpc_collection() {
    use previously_on::app_server::{project_thread_events, AppServerClient};
    use previously_on::domain::{CoverageStatus, EventKind};

    let (_repo_temp, repo) = git_repository();
    let script =
        r#"#!/bin/sh
IFS= read -r initialize
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"userAgent":"codex-cli/0.144.3"}}'
IFS= read -r initialized
IFS= read -r list
printf '%s\n' '{"jsonrpc":"2.0","method":"thread/tokenUsage/updated","params":{"threadId":"thread-1"}}'
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"data":[{"cliVersion":"0.144.3","createdAt":1,"cwd":"__REPO__","id":"thread-1","preview":"one","sessionId":"session-1","updatedAt":2}],"nextCursor":null}}'
IFS= read -r read_thread
printf '%s\n' '{"jsonrpc":"2.0","method":"thread/tokenUsage/updated","params":{"threadId":"thread-1","turnId":"turn-1","tokenUsage":{"last":{"cachedInputTokens":10,"inputTokens":20,"outputTokens":30,"reasoningOutputTokens":40,"totalTokens":100},"total":{"cachedInputTokens":100,"inputTokens":200,"outputTokens":300,"reasoningOutputTokens":400,"totalTokens":1000},"modelContextWindow":128000}}}'
printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"thread":{"turns":[{"id":"turn-1","items":[],"status":"completed"}]}}}'
"#
        .replace("__REPO__", repo.to_str().unwrap());
    let (_temp, fake) = fake_codex(&script);

    let mut client = AppServerClient::connect_with_program(&fake).await.unwrap();
    let report = client.import_threads_report(&repo).await.unwrap();

    assert_eq!(report.threads.len(), 1);
    assert_eq!(report.coverage.status, CoverageStatus::Degraded);
    let warning = "ignored malformed thread/tokenUsage/updated notification; token-usage coverage is degraded";
    assert!(report
        .coverage
        .warnings
        .iter()
        .any(|candidate| candidate == warning));
    let imported = &report.threads[0];
    assert_eq!(
        imported.thread["_previously_token_usage"]["tokenUsage"]["last"]["totalTokens"],
        100
    );
    let projection = project_thread_events(imported, "repo-id", std::path::Path::new("/tmp/repo"));
    let usage = projection
        .events
        .iter()
        .find(|event| event.kind == EventKind::ContextUsageUpdated)
        .expect("valid notification collected after malformed notification");
    assert_eq!(usage.payload["context_usage"]["total_tokens"], 100);
    assert_eq!(
        usage.payload["context_usage"]["model_context_window"],
        128000
    );
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
                        "id": "item-compaction",
                        "type": "contextCompaction",
                        "createdAt": 1700000008
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
    assert_eq!(projection.events.len(), 7);
    assert!(projection
        .events
        .iter()
        .any(|event| event.kind == EventKind::UserPrompt));
    assert!(projection
        .events
        .iter()
        .any(|event| event.kind == EventKind::AssistantFinal));
    let compaction = projection
        .events
        .iter()
        .find(|event| event.kind == EventKind::ContextCompaction)
        .unwrap();
    assert_eq!(
        compaction.source_id,
        "codex-app-server:thread:thread-semantic:item:item-compaction:context-compaction"
    );
    assert_eq!(compaction.payload["thread_id"], "thread-semantic");
    assert_eq!(compaction.payload["turn_id"], "turn-1");
    assert_eq!(compaction.payload["item_id"], "item-compaction");
    assert_eq!(compaction.payload["raw_transcript_stored"], false);

    let started = projection
        .events
        .iter()
        .find(|event| event.kind == EventKind::SessionStarted)
        .unwrap();
    assert_eq!(started.payload["source_thread_id"], "thread-semantic");
    assert_eq!(started.payload["thread_created_at"], "2023-11-14T22:13:20Z");
    assert_eq!(started.payload["thread_updated_at"], "2023-11-14T22:13:30Z");
    assert_eq!(started.payload["turn_count"], 1);
    assert_eq!(started.payload["raw_transcript_stored"], false);
    let stopped = projection
        .events
        .iter()
        .find(|event| event.kind == EventKind::SessionStopped)
        .unwrap();
    assert_eq!(stopped.payload["source_thread_id"], "thread-semantic");
    assert_eq!(stopped.payload["thread_created_at"], "2023-11-14T22:13:20Z");
    assert_eq!(stopped.payload["thread_updated_at"], "2023-11-14T22:13:30Z");
    assert_eq!(stopped.payload["turn_count"], 1);
    assert_eq!(stopped.payload["raw_transcript_stored"], false);
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
    assert!(
        store.list_checkpoints(task_id).unwrap().is_empty(),
        "an import-time Git snapshot must never be presented as the historical baseline"
    );
    assert!(store
        .list_session_events(&identity.id, "session-semantic")
        .unwrap()
        .iter()
        .all(|event| event
            .coverage
            .missing
            .contains(&"historical_git_snapshot".to_string())));
    let session = store
        .get_session("session-semantic")
        .unwrap()
        .expect("projected session");
    assert_eq!(session.source_thread_id.as_deref(), Some("thread-semantic"));
    assert_eq!(session.turn_count, 1);
    assert_eq!(session.compaction_count, 1);
    assert_eq!(
        session.last_activity_at.unwrap().to_rfc3339(),
        "2023-11-14T22:13:30+00:00"
    );
    let exported = store.export_json(None).unwrap();
    let export = exported.to_string();
    assert!(export.contains("Auth work continued and tests pass."));
    assert!(
        !export.contains("do-not-store"),
        "secret remained at JSON paths: {:?}",
        json_paths_containing(&exported, "do-not-store", "$"),
    );
}

#[test]
fn malformed_semantic_items_degrade_coverage_without_unstable_projection_ids() {
    use previously_on::app_server::{project_thread_events, ImportedThreadV1};
    use previously_on::domain::{CoverageStatus, CoverageV1, EventKind};
    use serde_json::json;

    let imported = ImportedThreadV1 {
        schema_version: 1,
        id: "thread-malformed".to_string(),
        session_id: "session-malformed".to_string(),
        cwd: std::path::PathBuf::from("/tmp/repo"),
        cli_version: "0.144.3".to_string(),
        created_at: 1_700_000_000,
        updated_at: 1_700_000_010,
        coverage: CoverageV1::default(),
        thread: json!({
            "turns": [
                {"items": [{"id": "would-be-unstable", "type": "contextCompaction"}]},
                {
                    "id": "turn-stable",
                    "items": [
                        {"type": "contextCompaction"},
                        {"id": "item-missing-type"},
                        {"id": "item-unknown", "type": "futureItem"},
                        "opaque"
                    ]
                }
            ]
        }),
    };

    let first = project_thread_events(&imported, "repo-id", std::path::Path::new("/tmp/repo"));
    let second = project_thread_events(&imported, "repo-id", std::path::Path::new("/tmp/repo"));

    assert_eq!(first.coverage.status, CoverageStatus::Degraded);
    assert!(first
        .coverage
        .missing
        .iter()
        .any(|missing| missing == "stable turn source ID"));
    assert!(first
        .coverage
        .missing
        .iter()
        .any(|missing| missing == "stable item source ID"));
    assert!(first
        .coverage
        .missing
        .iter()
        .any(|missing| missing == "known thread item schema"));
    assert!(!first
        .events
        .iter()
        .any(|event| event.kind == EventKind::ContextCompaction));
    assert_eq!(
        first
            .events
            .iter()
            .map(|event| event.source_id.as_str())
            .collect::<Vec<_>>(),
        second
            .events
            .iter()
            .map(|event| event.source_id.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn imported_file_changes_reject_unsafe_and_sensitive_repository_paths() {
    use previously_on::app_server::{project_thread_events, ImportedThreadV1};
    use previously_on::domain::{CoverageStatus, CoverageV1, EventKind};
    use serde_json::json;
    use tempfile::TempDir;

    let repo = TempDir::new().unwrap();
    std::fs::create_dir_all(repo.path().join("src")).unwrap();
    let imported = ImportedThreadV1 {
        schema_version: 1,
        id: "thread-path-safety".to_string(),
        session_id: "session-path-safety".to_string(),
        cwd: repo.path().to_path_buf(),
        cli_version: "0.144.3".to_string(),
        created_at: 1_700_000_000,
        updated_at: 1_700_000_010,
        coverage: CoverageV1::default(),
        thread: json!({
            "turns": [{
                "id": "turn-1",
                "items": [{
                    "id": "item-file",
                    "type": "fileChange",
                    "changes": [
                        {"path": "src/good.rs", "kind": "update"},
                        {"path": "/tmp/absolute.rs", "kind": "update"},
                        {"path": "../outside.rs", "kind": "update"},
                        {"path": "src/./non-normal.rs", "kind": "update"},
                        {"path": "src//non-normal.rs", "kind": "update"},
                        {"path": "src\\non-normal.rs", "kind": "update"},
                        {"path": ".env.production", "kind": "update"},
                        {"path": "credentials.json", "kind": "update"},
                        {
                            "path": "src/renamed.rs",
                            "previousPath": "../../outside.rs",
                            "kind": "rename"
                        }
                    ]
                }]
            }]
        }),
    };

    let projection = project_thread_events(&imported, "repo-id", repo.path());
    assert_eq!(projection.coverage.status, CoverageStatus::Degraded);
    assert!(projection
        .coverage
        .missing
        .iter()
        .any(|missing| missing == "safe repository-relative file change path"));
    let file_event = projection
        .events
        .iter()
        .find(|event| event.kind == EventKind::ToolFinished)
        .unwrap();
    let changes = file_event.payload["file_changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["path"], "src/good.rs");
    let serialized = serde_json::to_string(&projection).unwrap();
    for rejected in [
        "/tmp/absolute.rs",
        "../outside.rs",
        "src/./non-normal.rs",
        "src//non-normal.rs",
        "src\\non-normal.rs",
        ".env.production",
        "credentials.json",
    ] {
        assert!(!serialized.contains(rejected), "import leaked {rejected}");
    }
}

#[test]
fn parses_documented_token_usage_notification() {
    use previously_on::app_server::parse_token_usage_notification;
    use serde_json::json;

    let parsed = parse_token_usage_notification(&json!({
        "jsonrpc": "2.0",
        "method": "thread/tokenUsage/updated",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "tokenUsage": {
                "last": {
                    "cachedInputTokens": 10,
                    "inputTokens": 20,
                    "outputTokens": 30,
                    "reasoningOutputTokens": 40,
                    "totalTokens": 100
                },
                "total": {
                    "cachedInputTokens": 100,
                    "inputTokens": 200,
                    "outputTokens": 300,
                    "reasoningOutputTokens": 400,
                    "totalTokens": 1000
                },
                "modelContextWindow": 128000
            }
        }
    }))
    .unwrap()
    .unwrap();

    assert_eq!(parsed.thread_id, "thread-1");
    assert_eq!(parsed.turn_id, "turn-1");
    assert_eq!(parsed.token_usage.last.total_tokens, 100);
    assert_eq!(parsed.token_usage.total.total_tokens, 1000);
    assert_eq!(parsed.token_usage.model_context_window, Some(128000));
    assert!(
        parse_token_usage_notification(&json!({"method": "turn/completed"}))
            .unwrap()
            .is_none()
    );
    assert!(parse_token_usage_notification(&json!({
        "method": "thread/tokenUsage/updated",
        "params": {"threadId": "thread-1"}
    }))
    .is_err());
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
