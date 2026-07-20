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
async fn resume_rejects_a_different_returned_thread_id() {
    use previously_on::app_server::AppServerClient;

    let (_temp, fake) = fake_codex(
        r#"#!/bin/sh
IFS= read -r initialize
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"userAgent":"codex-cli/0.144.3"}}'
IFS= read -r initialized
IFS= read -r resume
case "$resume" in *'"method":"thread/resume"'*'"threadId":"thread-expected"'*) ;; *) exit 10 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"thread":{"id":"thread-other","sessionId":"session-other"}}}'
"#,
    );

    let mut client = AppServerClient::connect_with_program(&fake).await.unwrap();
    let error = client.resume_thread("thread-expected").await.unwrap_err();
    assert!(error.to_string().contains("different thread.id"));
    client.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn experimental_permission_profile_and_structured_turn_use_exact_fail_closed_fields() {
    use previously_on::app_server::AppServerClient;
    use serde_json::json;

    let (_temp, fake) = fake_codex(
        r#"#!/bin/sh
IFS= read -r initialize
case "$initialize" in *'"experimentalApi":true'*) ;; *) exit 10 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"userAgent":"codex-cli/0.144.3"}}'
IFS= read -r initialized
IFS= read -r profiles
case "$profiles" in *'"method":"permissionProfile/list"'*'"cwd":"/tmp/empty"'*'"limit":100'*) ;; *) exit 11 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"data":[{"id":"previously-input-only","allowed":true}],"nextCursor":null}}'
IFS= read -r start
case "$start" in *'"method":"thread/start"'*) ;; *) exit 12 ;; esac
case "$start" in *'"permissions":"previously-input-only"'*) ;; *) exit 16 ;; esac
case "$start" in *'"approvalPolicy":"never"'*) ;; *) exit 17 ;; esac
case "$start" in *'"ephemeral":true'*) ;; *) exit 18 ;; esac
case "$start" in *'"sandbox"'*|*'"model"'*) exit 13 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"thread":{"id":"refresh-thread","sessionId":"refresh-thread"}}}'
IFS= read -r turn
case "$turn" in *'"method":"turn/start"'*) ;; *) exit 14 ;; esac
case "$turn" in *'"effort":"medium"'*) ;; *) exit 19 ;; esac
case "$turn" in *'"outputSchema"'*) ;; *) exit 20 ;; esac
case "$turn" in *'"clientUserMessageId":"request-1"'*) ;; *) exit 21 ;; esac
case "$turn" in *'"sandbox"'*|*'"permissions"'*|*'"model"'*) exit 15 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":4,"result":{"turn":{"id":"turn-1"}}}'
"#,
    );
    let mut client = AppServerClient::connect_with_program_experimental(&fake)
        .await
        .unwrap();
    let profiles = client
        .list_permission_profiles(std::path::Path::new("/tmp/empty"))
        .await
        .unwrap();
    assert_eq!(profiles.profiles[0].id, "previously-input-only");
    assert!(profiles.profiles[0].allowed);
    let thread = client
        .start_ephemeral_thread_with_permissions(
            std::path::Path::new("/tmp/empty"),
            "previously-input-only",
        )
        .await
        .unwrap();
    client
        .start_structured_fact_refresh_turn(
            &thread.id,
            std::path::Path::new("/tmp/empty"),
            "bounded input",
            "request-1",
            json!({"type":"object"}),
        )
        .await
        .unwrap();
    client.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn agent_lineage_paginates_skips_cross_repo_and_replays_new_observations() {
    use chrono::Utc;
    use previously_on::app_server::{collect_agent_lineage, AppServerClient};
    use previously_on::domain::{
        AgentAssociationStateV1, EventKind, GraphEdgeKindV1, RepositoryV1, SessionLifecycle,
        SessionV1, TaskLifecycle, TaskV1, SCHEMA_VERSION_V1,
    };
    use previously_on::store::{InsertOutcome, Store};
    use serde_json::json;
    use std::process::Command;

    fn git_init(path: &std::path::Path) {
        std::fs::create_dir_all(path).unwrap();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .current_dir(path)
            .status()
            .unwrap()
            .success());
    }

    fn summary(
        id: &str,
        session_id: &str,
        cwd: &std::path::Path,
        source: &str,
        parent: Option<&str>,
        updated_at: i64,
    ) -> serde_json::Value {
        json!({
            "id": id,
            "sessionId": session_id,
            "cwd": cwd,
            "cliVersion": "test",
            "createdAt": updated_at - 1,
            "updatedAt": updated_at,
            "ephemeral": false,
            "modelProvider": "openai",
            "preview": format!("preview {id}"),
            "name": format!("name {id}"),
            "source": source,
            "parentThreadId": parent,
            "forkedFromId": null,
            "status": {"type":"idle"},
            "turns": []
        })
    }

    let temp = tempfile::TempDir::new().unwrap();
    let repository = temp.path().join("repo");
    let foreign = temp.path().join("foreign");
    git_init(&repository);
    git_init(&foreign);
    let identity = previously_on::git::repository_identity(&repository).unwrap();
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    let now = Utc::now();
    store
        .upsert_repository(&RepositoryV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: identity.id.clone(),
            path: repository.to_string_lossy().into_owned(),
            remote_url: None,
            created_at: now,
            updated_at: now,
        })
        .unwrap();
    store
        .upsert_task(&TaskV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: "task-lineage".into(),
            repository_id: identity.id.clone(),
            title: "Observe agents".into(),
            goal: None,
            lifecycle: TaskLifecycle::Active,
            branch: Some("main".into()),
            created_at: now,
            updated_at: now,
        })
        .unwrap();
    store
        .upsert_session(&SessionV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: "session-parent".into(),
            repository_id: identity.id.clone(),
            task_id: Some("task-lineage".into()),
            lifecycle: SessionLifecycle::Active,
            started_at: now,
            ended_at: None,
            branch: Some("main".into()),
            head: None,
            source_thread_id: Some("thread-parent".into()),
            last_activity_at: Some(now),
            turn_count: 1,
            compaction_count: 0,
            context_usage: None,
            continuation_state: Default::default(),
            coverage: Default::default(),
        })
        .unwrap();

    let first_page = json!({
        "data": [
            summary("thread-parent", "session-parent", &repository, "appServer", None, 101),
            summary("thread-child", "session-parent", &repository, "subAgent", Some("thread-parent"), 102)
        ],
        "nextCursor": "page-2"
    });
    let second_page = json!({
        "data": [
            summary("thread-orphan", "session-orphan", &repository, "subAgentReview", Some("thread-missing"), 103),
            summary("thread-foreign", "session-foreign", &foreign, "subAgentOther", None, 104),
            summary("thread-mismatch", "session-mismatch", &repository, "subAgent", None, 105),
            summary("thread-cross-read", "session-cross-read", &repository, "subAgent", None, 106),
            summary("thread-unavailable", "session-unavailable", &repository, "subAgentOther", None, 107)
        ],
        "nextCursor": null
    });
    let read_response = |id: &str, cwd: &std::path::Path| {
        json!({"thread": {
            "id": id,
            "cwd": cwd,
            "status": {"type":"idle"},
            "turns": [{"items":[
                {"type":"agentMessage","text":format!("summary {id}")},
                {"type":"fileChange","changes":[
                    {"path":"src/lib.rs"},
                    {"path":"../escape.rs"},
                    {"path":"/tmp/absolute.rs"},
                    {"path":".env.production"},
                    {"path":"config/credentials.json"}
                ]}
            ]}]
        }})
    };
    let script = format!(
        r#"#!/bin/sh
IFS= read -r initialize
case "$initialize" in *'"experimentalApi":true'*) ;; *) exit 10 ;; esac
printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"userAgent":"codex-cli/test"}}}}'
IFS= read -r initialized
IFS= read -r first
case "$first" in *'"method":"thread/list"'*) ;; *) exit 11 ;; esac
case "$first" in *'"cli"'*'"vscode"'*'"exec"'*'"appServer"'*'"subAgent"'*'"subAgentReview"'*'"subAgentCompact"'*'"subAgentThreadSpawn"'*'"subAgentOther"'*) ;; *) exit 12 ;; esac
printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{first_page}}}'
IFS= read -r second
case "$second" in *'"cursor":"page-2"'*) ;; *) exit 13 ;; esac
printf '%s\n' '{{"jsonrpc":"2.0","id":3,"result":{second_page}}}'
IFS= read -r read_parent
case "$read_parent" in *'"threadId":"thread-parent"'*) ;; *) exit 14 ;; esac
printf '%s\n' '{{"jsonrpc":"2.0","id":4,"result":{read_parent}}}'
IFS= read -r read_child
case "$read_child" in *'"threadId":"thread-child"'*) ;; *) exit 15 ;; esac
printf '%s\n' '{{"jsonrpc":"2.0","id":5,"result":{read_child}}}'
IFS= read -r read_orphan
case "$read_orphan" in *'"threadId":"thread-orphan"'*) ;; *) exit 16 ;; esac
printf '%s\n' '{{"jsonrpc":"2.0","id":6,"result":{read_orphan}}}'
IFS= read -r read_mismatch
case "$read_mismatch" in *'"threadId":"thread-mismatch"'*) ;; *) exit 17 ;; esac
printf '%s\n' '{{"jsonrpc":"2.0","id":7,"result":{read_mismatch}}}'
IFS= read -r read_cross
case "$read_cross" in *'"threadId":"thread-cross-read"'*) ;; *) exit 18 ;; esac
printf '%s\n' '{{"jsonrpc":"2.0","id":8,"result":{read_cross}}}'
IFS= read -r read_unavailable
case "$read_unavailable" in *'"threadId":"thread-unavailable"'*) ;; *) exit 19 ;; esac
printf '%s\n' '{{"jsonrpc":"2.0","id":9,"error":{{"code":-32004,"message":"thread unavailable"}}}}'
"#,
        first_page = first_page,
        second_page = second_page,
        read_parent = read_response("thread-parent", &repository),
        read_child = read_response("thread-child", &repository),
        read_orphan = read_response("thread-orphan", &repository),
        read_mismatch = read_response("wrong-thread", &repository),
        read_cross = read_response("thread-cross-read", &foreign),
    );
    let (_fake_temp, fake) = fake_codex(&script);
    let mut client = AppServerClient::connect_with_program_experimental(&fake)
        .await
        .unwrap();
    let agents = collect_agent_lineage(&mut client, &store, &repository, &identity.id)
        .await
        .unwrap();
    client.shutdown().await.unwrap();

    assert_eq!(agents.len(), 4);
    assert!(!agents
        .iter()
        .any(|agent| agent.thread_id == "thread-foreign"));
    assert!(!agents
        .iter()
        .any(|agent| agent.thread_id == "thread-mismatch"));
    assert!(!agents
        .iter()
        .any(|agent| agent.thread_id == "thread-cross-read"));
    let unavailable = agents
        .iter()
        .find(|agent| agent.thread_id == "thread-unavailable")
        .unwrap();
    assert_eq!(
        unavailable.association_state,
        AgentAssociationStateV1::Degraded
    );
    assert_eq!(
        unavailable.degraded_reason.as_deref(),
        Some("thread/read unavailable; summary-only observation")
    );
    assert!(unavailable.output_summary.is_none());
    assert!(unavailable.files.is_empty());
    assert!(unavailable.tests.is_empty());
    let parent = agents
        .iter()
        .find(|agent| agent.thread_id == "thread-parent")
        .unwrap();
    let child = agents
        .iter()
        .find(|agent| agent.thread_id == "thread-child")
        .unwrap();
    let orphan = agents
        .iter()
        .find(|agent| agent.thread_id == "thread-orphan")
        .unwrap();
    assert_eq!(parent.task_id.as_deref(), Some("task-lineage"));
    assert_eq!(child.task_id.as_deref(), Some("task-lineage"));
    assert_eq!(child.parent_thread_id.as_deref(), Some("thread-parent"));
    assert_eq!(orphan.task_id, None);
    assert_eq!(orphan.association_state, AgentAssociationStateV1::Unlinked);
    assert!(agents
        .iter()
        .filter(|agent| agent.thread_id != "thread-unavailable")
        .all(|agent| agent.files == ["src/lib.rs".to_string()]));

    let mut updated_parent = parent.clone();
    updated_parent.status = "completed".into();
    updated_parent.observed_at += chrono::Duration::seconds(10);
    assert_eq!(
        store.append_agent_observation(&updated_parent).unwrap(),
        InsertOutcome::Inserted
    );
    assert_eq!(
        store.append_agent_observation(&updated_parent).unwrap(),
        InsertOutcome::Duplicate
    );
    store.rebuild_projections().unwrap();
    assert_eq!(
        store.get_agent(&updated_parent.id).unwrap().unwrap().status,
        "completed"
    );
    assert!(
        store
            .list_events(Some(&identity.id))
            .unwrap()
            .iter()
            .filter(|event| event.kind == EventKind::AgentObserved)
            .count()
            >= 4
    );
    let graph =
        previously_on::graph::derive_relationship_graph(&store, &identity.id, None, &[]).unwrap();
    assert_eq!(
        graph
            .edges
            .iter()
            .filter(|edge| edge.kind == GraphEdgeKindV1::AgentWorkedOnTask)
            .count(),
        2
    );
    assert_eq!(
        graph
            .edges
            .iter()
            .filter(|edge| edge.kind == GraphEdgeKindV1::AgentParent)
            .count(),
        1
    );
    assert!(graph.edges.iter().all(|edge| edge.verified));
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
#[tokio::test]
async fn capability_readiness_comes_from_schema_and_safe_runtime_not_version() {
    use previously_on::app_server::{AppServerCapabilityStatus, AppServerClient};

    let (_temp, fake) = fake_codex(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  printf '%s\n' 'codex-cli 9.9.9'
  exit 0
fi
if [ "$1" = "app-server" ] && [ "$2" = "generate-json-schema" ]; then
  out="$5"
  mkdir -p "$out/v2"
  for schema in ThreadList ThreadRead ThreadStart ThreadResume ThreadSetName TurnStart PermissionProfileList; do
    printf '%s\n' '{}' > "$out/v2/${schema}Params.json"
    printf '%s\n' '{}' > "$out/v2/${schema}Response.json"
  done
  printf '%s\n' '{"properties":{"permissions":{},"approvalPolicy":{}}}' > "$out/v2/ThreadStartParams.json"
  printf '%s\n' '{"properties":{"outputSchema":{}}}' > "$out/v2/TurnStartParams.json"
  exit 0
fi
IFS= read -r initialize
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"userAgent":"codex-cli/9.9.9"}}'
IFS= read -r initialized
IFS= read -r list
case "$list" in *'"method":"thread/list"'*) ;; *) exit 10 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"data":[],"nextCursor":null}}'
"#,
    );

    let mut client = AppServerClient::connect_with_program(&fake).await.unwrap();
    let report = client.capability_report().await;
    assert_eq!(report.status, AppServerCapabilityStatus::Complete);
    assert_eq!(
        report.capabilities.core_import,
        AppServerCapabilityStatus::Complete
    );
    assert_eq!(
        report.capabilities.continuation,
        AppServerCapabilityStatus::Complete
    );
    assert_eq!(
        report.capabilities.experimental_refresh,
        AppServerCapabilityStatus::Complete
    );
    assert_eq!(report.detected_codex_version.as_deref(), Some("9.9.9"));
    assert!(report
        .warnings
        .iter()
        .any(|warning| warning.contains("tested provenance")));
    assert_eq!(
        report.supported_methods,
        [
            "initialize",
            "initialized",
            "permissionProfile/list",
            "thread/list",
            "thread/name/set",
            "thread/read",
            "thread/resume",
            "thread/start",
            "turn/start",
        ]
    );
    client.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn capability_probe_degrades_incomplete_schema_and_malformed_runtime_list() {
    use previously_on::app_server::{AppServerCapabilityStatus, AppServerClient};

    let (_temp, fake) = fake_codex(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  printf '%s\n' 'codex-cli 0.144.3'
  exit 0
fi
if [ "$1" = "app-server" ] && [ "$2" = "generate-json-schema" ]; then
  out="$5"
  mkdir -p "$out/v2"
  printf '%s\n' '{}' > "$out/v2/ThreadListParams.json"
  printf '%s\n' '{}' > "$out/v2/ThreadListResponse.json"
  exit 0
fi
IFS= read -r initialize
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"userAgent":"codex-cli/0.144.3"}}'
IFS= read -r initialized
IFS= read -r list
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"data":"malformed"}}'
"#,
    );

    let mut client = AppServerClient::connect_with_program(&fake).await.unwrap();
    let report = client.capability_report().await;
    assert_eq!(report.status, AppServerCapabilityStatus::Degraded);
    assert_eq!(
        report.capabilities.core_import,
        AppServerCapabilityStatus::Degraded
    );
    assert_eq!(
        report.capabilities.continuation,
        AppServerCapabilityStatus::Unsupported
    );
    assert_eq!(
        report.capabilities.experimental_refresh,
        AppServerCapabilityStatus::Unsupported
    );
    assert_eq!(
        report.supported_methods,
        ["initialize", "initialized", "thread/list"]
    );
    assert!(report
        .warnings
        .iter()
        .any(|warning| warning.contains("omitted a data array")));
    client.shutdown().await.unwrap();
}

#[test]
fn capability_report_defaults_new_feature_fields_when_reading_legacy_json() {
    use previously_on::app_server::{AppServerCapabilityReport, AppServerCapabilityStatus};
    use serde_json::json;

    let report: AppServerCapabilityReport = serde_json::from_value(json!({
        "schemaVersion": 1,
        "status": "degraded",
        "testedCodexVersion": "0.144.3",
        "detectedCodexVersion": "0.144.2",
        "appServerUserAgent": "codex-cli/0.144.2",
        "supportedMethods": ["thread/list"],
        "warnings": []
    }))
    .unwrap();
    assert_eq!(
        report.capabilities.core_import,
        AppServerCapabilityStatus::Unsupported
    );
    assert_eq!(
        report.capabilities.continuation,
        AppServerCapabilityStatus::Unsupported
    );
    assert_eq!(
        report.capabilities.experimental_refresh,
        AppServerCapabilityStatus::Unsupported
    );
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
async fn import_verifies_returned_cwd_and_accepts_only_the_registered_worktree() {
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
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"data":[{"cliVersion":"0.144.3","createdAt":5,"cwd":"__NESTED__","id":"thread-nested","preview":"wrong repository","sessionId":"session-nested","updatedAt":6},{"cliVersion":"0.144.3","createdAt":4,"cwd":"relative/repository","id":"thread-relative","preview":"relative","sessionId":"session-relative","updatedAt":5},{"cliVersion":"0.144.3","createdAt":3,"id":"thread-missing","preview":"missing cwd","sessionId":"session-missing","updatedAt":4},{"cliVersion":"0.144.3","createdAt":2,"cwd":"__LINKED__","id":"thread-linked","preview":"sibling worktree","sessionId":"session-linked","updatedAt":3},{"cliVersion":"0.144.3","createdAt":1,"cwd":"__REPO__","id":"thread-primary","preview":"registered worktree","sessionId":"session-primary","updatedAt":2}],"nextCursor":null}}'
IFS= read -r read_thread
case "$read_thread" in *'"method":"thread/read"'*'"threadId":"thread-primary"'*) ;; *) exit 20 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"thread":{"id":"thread-primary","cwd":"__REPO__","turns":[{"id":"turn-primary","items":[],"status":"completed"}]}}}'
"#
    .replace("__NESTED__", nested.to_str().unwrap())
    .replace("__LINKED__", linked.to_str().unwrap())
    .replace("__REPO__", repo.to_str().unwrap());
    let (_fake_temp, fake) = fake_codex(&script);

    let mut client = AppServerClient::connect_with_program(&fake).await.unwrap();
    let report = client.import_threads_report(&repo).await.unwrap();

    assert_eq!(report.coverage.status, CoverageStatus::Degraded);
    assert_eq!(report.threads.len(), 1);
    assert_eq!(report.threads[0].id, "thread-primary");
    assert_eq!(report.threads[0].cwd, repo);
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
    assert!(report.notices.iter().any(|notice| {
        notice.thread_id.as_deref() == Some("thread-linked")
            && notice.message.contains("different Git worktree")
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
async fn import_rejects_thread_read_identity_and_worktree_drift() {
    use std::process::Command;

    use previously_on::app_server::{AppServerClient, ThreadImportDisposition};

    let (_repo_temp, repo) = git_repository();
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
        .args([
            "-c",
            "user.email=tests@previously.local",
            "-c",
            "user.name=PreviouslyOn Tests",
            "commit",
            "--quiet",
            "-m",
            "baseline",
        ])
        .status()
        .unwrap()
        .success());
    let linked = _repo_temp.path().join("linked-read");
    assert!(Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["worktree", "add", "--quiet", "--detach"])
        .arg(&linked)
        .arg("HEAD")
        .status()
        .unwrap()
        .success());

    let script = r#"#!/bin/sh
IFS= read -r initialize
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"userAgent":"codex-cli/0.144.3"}}'
IFS= read -r initialized
IFS= read -r list
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"data":[{"cliVersion":"0.144.3","createdAt":3,"cwd":"__REPO__","id":"thread-id-drift","preview":"id drift","sessionId":"session-id-drift","updatedAt":4},{"cliVersion":"0.144.3","createdAt":2,"cwd":"__REPO__","id":"thread-worktree-drift","preview":"worktree drift","sessionId":"session-worktree-drift","updatedAt":3},{"cliVersion":"0.144.3","createdAt":1,"cwd":"__REPO__","id":"thread-missing-cwd","preview":"missing cwd","sessionId":"session-missing-cwd","updatedAt":2}],"nextCursor":null}}'
IFS= read -r id_drift
printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"thread":{"id":"thread-other","cwd":"__REPO__","turns":[]}}}'
IFS= read -r worktree_drift
printf '%s\n' '{"jsonrpc":"2.0","id":4,"result":{"thread":{"id":"thread-worktree-drift","cwd":"__LINKED__","turns":[]}}}'
IFS= read -r missing_cwd
printf '%s\n' '{"jsonrpc":"2.0","id":5,"result":{"thread":{"id":"thread-missing-cwd","turns":[]}}}'
"#
    .replace("__REPO__", repo.to_str().unwrap())
    .replace("__LINKED__", linked.to_str().unwrap());
    let (_temp, fake) = fake_codex(&script);

    let mut client = AppServerClient::connect_with_program(&fake).await.unwrap();
    let report = client.import_threads_report(&repo).await.unwrap();
    assert!(report.threads.is_empty());
    assert_eq!(
        report
            .notices
            .iter()
            .filter(|notice| notice.disposition == ThreadImportDisposition::Skipped)
            .count(),
        3
    );
    assert!(report
        .notices
        .iter()
        .any(|notice| notice.message.contains("different thread.id")));
    assert!(report
        .notices
        .iter()
        .any(|notice| notice.message.contains("different Git worktree")));
    assert!(report
        .notices
        .iter()
        .any(|notice| notice.message.contains("omitted cwd")));
    client.shutdown().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn isolates_deleted_and_degraded_threads_and_preserves_rpc_error_fields() {
    use previously_on::app_server::{AppServerClient, ThreadImportDisposition};
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
printf '%s\n' '{"jsonrpc":"2.0","id":4,"result":{"thread":{"id":"thread-compact","cwd":"__REPO__","compacted":true,"status":{"type":"incomplete"},"turns":[{"items":[{"type":"futureItem","payload":"untrusted"}],"status":"interrupted"}]}}}'
IFS= read -r good
printf '%s\n' '{"jsonrpc":"2.0","id":5,"result":{"thread":{"id":"thread-good","cwd":"__REPO__","turns":[{"id":"turn-good","items":[{"id":"item-good","type":"agentMessage"}],"status":"completed"}]}}}'
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
    assert!(!compact
        .coverage
        .warnings
        .iter()
        .any(|warning| warning.contains("supported versions")));
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
printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"thread":{"id":"thread-1","cwd":"__REPO__","turns":[{"id":"turn-1","items":[],"status":"completed"}]}}}'
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
