use std::io::{BufRead, BufReader, Cursor, Write};

use previously_on::hook::{
    append_fallback, capture, contract_hook_response, hook_response, replay_fallback, run_hook,
    HookEvent, HookIngressConfig, ResumeCandidateMetadata, MAX_DAEMON_FRAME_BYTES,
    MAX_HOOK_PAYLOAD_BYTES,
};
use previously_on::store::Store;
use tempfile::TempDir;

#[test]
fn capture_redacts_secrets_and_sensitive_paths_before_fallback() {
    let mut input = Cursor::new(
        br#"{
          "session_id":"session-1",
          "turn_id":"turn-42",
          "cwd":"/tmp/repo",
          "prompt":"use api_key=very-secret-value and /Users/me/project/.env",
          "authorization":"Bearer actual-token"
        }"#,
    );
    let event = capture(HookEvent::UserPromptSubmit, &mut input).unwrap();
    assert_eq!(event.session_id, "session-1");
    assert_eq!(event.repository_id, "/tmp/repo");
    assert_eq!(event.sequence, Some(42));
    assert_eq!(event.payload["lineage_payload_mode"], "redacted_excerpt");
    let serialized = serde_json::to_string(&event).unwrap();
    assert!(!serialized.contains("very-secret-value"));
    assert!(!serialized.contains("actual-token"));
    assert!(!serialized.contains("project/.env"));

    let temp = TempDir::new().unwrap();
    let queue = temp.path().join("queue/events.jsonl");
    append_fallback(&queue, &event).unwrap();
    let queued = std::fs::read_to_string(queue).unwrap();
    assert!(!queued.contains("very-secret-value"));
    assert!(!queued.contains("actual-token"));
}

#[test]
fn contract_hook_responses_use_non_blocking_context_and_official_stop_block_shape() {
    let pre_tool = contract_hook_response(
        HookEvent::PreToolUse,
        Some("Relevant contract: keep auth stable; token=never-store-context"),
        None,
    );
    assert_eq!(
        pre_tool["hookSpecificOutput"]["hookEventName"],
        "PreToolUse"
    );
    assert!(pre_tool["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .contains("keep auth stable"));
    assert!(!pre_tool.to_string().contains("never-store-context"));
    assert!(pre_tool.get("decision").is_none());

    let stop = contract_hook_response(
        HookEvent::Stop,
        None,
        Some("Run required test: cargo test --locked --all"),
    );
    assert_eq!(stop["decision"], "block");
    assert_eq!(
        stop["reason"],
        "Run required test: cargo test --locked --all"
    );
    assert_eq!(
        contract_hook_response(HookEvent::Stop, None, None),
        serde_json::json!({})
    );
}

#[test]
fn missing_stable_hook_ids_use_unique_sources_and_degraded_coverage() {
    let payload =
        br#"{"session_id":"session-unstable","cwd":"/tmp/missing-repo","prompt":"same text"}"#;
    let first = capture(HookEvent::UserPromptSubmit, &mut Cursor::new(payload)).unwrap();
    let second = capture(HookEvent::UserPromptSubmit, &mut Cursor::new(payload)).unwrap();
    assert_ne!(first.source_id, second.source_id);
    assert_ne!(first.dedupe_key, second.dedupe_key);
    assert_eq!(
        first.coverage.status,
        previously_on::domain::CoverageStatus::Degraded
    );
    assert!(first
        .coverage
        .missing
        .iter()
        .any(|item| item == "stable_source_id"));
}

#[test]
fn stable_turn_ids_make_hook_retries_idempotent_without_hashing_payloads() {
    let payload = br#"{"session_id":"session-stable","turn_id":"turn-7","cwd":"/tmp/missing-repo","prompt":"same text"}"#;
    let first = capture(HookEvent::UserPromptSubmit, &mut Cursor::new(payload)).unwrap();
    let second = capture(HookEvent::UserPromptSubmit, &mut Cursor::new(payload)).unwrap();
    assert_eq!(first.source_id, second.source_id);
    assert_eq!(first.dedupe_key, second.dedupe_key);
    assert!(!first
        .coverage
        .missing
        .iter()
        .any(|item| item == "stable_source_id"));
}

#[test]
fn capture_rejects_oversized_payloads() {
    let mut input = Cursor::new(vec![b'x'; MAX_HOOK_PAYLOAD_BYTES + 1]);
    let error = capture(HookEvent::Stop, &mut input).unwrap_err();
    assert!(error.to_string().contains("exceeds"));
}

#[test]
fn capture_stores_only_bounded_redacted_excerpts() {
    let payload = serde_json::json!({
        "session_id":"session-bounded",
        "cwd":"/tmp/repo",
        "prompt":"🙂".repeat(700)
    });
    let mut input = Cursor::new(serde_json::to_vec(&payload).unwrap());
    let event = capture(HookEvent::UserPromptSubmit, &mut input).unwrap();
    assert_eq!(
        event.payload["prompt"].as_str().unwrap().chars().count(),
        500
    );
    assert_eq!(event.payload["lineage_payload_mode"], "redacted_excerpt");
}

#[test]
fn prompt_hook_only_emits_resume_candidate_metadata() {
    let candidate = ResumeCandidateMetadata {
        task_id: "task-1".to_string(),
        title: "Authentication cleanup".to_string(),
        score: 0.91,
        matched_by: vec!["same repository".to_string()],
        last_activity_at: None,
        continuation_advice: None,
    };
    let response = hook_response(HookEvent::UserPromptSubmit, Some(&candidate));
    assert_eq!(
        response["hookSpecificOutput"]["hookEventName"],
        "UserPromptSubmit"
    );
    let context = response["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.contains("task-1"));
    assert!(context.contains("Ask the user"));
    assert!(context.contains("untrusted historical data"));
    assert!(!context.contains("facts"));
    assert_eq!(
        hook_response(HookEvent::Stop, Some(&candidate)),
        serde_json::json!({})
    );
}

#[test]
fn hook_ignores_work_outside_the_registered_repository() {
    let temp = TempDir::new().unwrap();
    let registered = temp.path().join("registered");
    std::fs::create_dir_all(&registered).unwrap();
    let config = HookIngressConfig {
        socket_path: temp.path().join("previously.sock"),
        queue_path: temp.path().join("queue/events.jsonl"),
        registered_repositories: vec![registered],
    };
    let mut input = Cursor::new(
        br#"{"session_id":"session-outside","cwd":"/tmp/unregistered-repository","prompt":"do not capture"}"#,
    );
    let mut output = Vec::new();
    run_hook(
        HookEvent::UserPromptSubmit,
        &config,
        &mut input,
        &mut output,
    )
    .unwrap();
    assert_eq!(String::from_utf8(output).unwrap().trim(), "{}");
    assert!(!config.queue_path.exists());
}

#[cfg(unix)]
#[test]
fn stop_hook_waits_for_large_slow_contract_ack_instead_of_bypassing_the_block() {
    use std::os::unix::net::UnixListener;
    use std::process::Command;
    use std::time::Duration;

    let temp = TempDir::new().unwrap();
    let registered = temp.path().join("registered");
    std::fs::create_dir_all(&registered).unwrap();
    assert!(Command::new("git")
        .arg("-C")
        .arg(&registered)
        .args(["init", "-q"])
        .status()
        .unwrap()
        .success());
    let socket_path = temp.path().join("previously.sock");
    let listener = UnixListener::bind(&socket_path).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut request)
            .unwrap();
        assert!(request.contains("session-slow-stop"));
        std::thread::sleep(Duration::from_millis(900));
        let exact_commands = format!(
            "PreviouslyOn blocks completion.\n{}\n- [missing] cwd=. argv=cargo test auth",
            "contract-context ".repeat(4_500)
        );
        serde_json::to_writer(
            &mut stream,
            &serde_json::json!({
                "status": "persisted",
                "stopBlockReason": exact_commands
            }),
        )
        .unwrap();
        stream.write_all(b"\n").unwrap();
    });
    let config = HookIngressConfig {
        socket_path,
        queue_path: temp.path().join("queue/events.jsonl"),
        registered_repositories: vec![registered.clone()],
    };
    let mut input = Cursor::new(
        serde_json::to_vec(&serde_json::json!({
            "session_id": "session-slow-stop",
            "turn_id": "stop-1",
            "cwd": registered,
            "stop_hook_active": false
        }))
        .unwrap(),
    );
    let mut output = Vec::new();

    run_hook(HookEvent::Stop, &config, &mut input, &mut output).unwrap();
    server.join().unwrap();

    let response: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(response["decision"], "block");
    assert!(response["reason"]
        .as_str()
        .unwrap()
        .contains("cwd=. argv=cargo test auth"));
    assert!(output.len() > 64 * 1024);
    assert!(!config.queue_path.exists());
}

#[test]
fn replay_quarantines_a_truncated_tail_without_losing_valid_events() {
    let temp = TempDir::new().unwrap();
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    let queue = temp.path().join("queue/events.jsonl");
    let mut input = Cursor::new(
        br#"{"session_id":"session-queued","cwd":"/tmp/missing-repo","prompt":"queued task"}"#,
    );
    let event = capture(HookEvent::UserPromptSubmit, &mut input).unwrap();
    append_fallback(&queue, &event).unwrap();
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&queue)
        .unwrap();
    file.write_all(b"{\"truncated\":").unwrap();
    file.sync_all().unwrap();

    replay_fallback(&store, &queue).unwrap();

    assert_eq!(store.health().unwrap().canonical_event_count, 1);
    assert!(!queue.exists());
    let corrupt = queue.with_extension("corrupt.jsonl");
    let quarantined = std::fs::read_to_string(corrupt).unwrap();
    assert!(quarantined.contains("DISCARDED MALFORMED QUEUE RECORD"));
    assert!(!quarantined.contains("truncated"));
}

#[test]
fn replay_streams_past_an_oversized_record_and_deduplicates_later_events() {
    let temp = TempDir::new().unwrap();
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    let queue = temp.path().join("queue/events.jsonl");
    std::fs::create_dir_all(queue.parent().unwrap()).unwrap();
    let event = capture(
        HookEvent::UserPromptSubmit,
        &mut Cursor::new(
            br#"{"session_id":"session-streaming","turn_id":"turn-1","cwd":"/tmp/missing-repo","prompt":"stream safely"}"#,
        ),
    )
    .unwrap();
    let record = serde_json::to_vec(&event).unwrap();
    use std::io::Write as _;
    let mut file = std::fs::File::create(&queue).unwrap();
    for _ in 0..=MAX_DAEMON_FRAME_BYTES {
        file.write_all(b"x").unwrap();
    }
    file.write_all(b"\n").unwrap();
    file.write_all(&record).unwrap();
    file.write_all(b"\n").unwrap();
    file.write_all(&record).unwrap();
    file.write_all(b"\n").unwrap();
    file.sync_all().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .unwrap();
    }

    replay_fallback(&store, &queue).unwrap();

    assert_eq!(store.health().unwrap().canonical_event_count, 1);
    let quarantined = std::fs::read_to_string(queue.with_extension("corrupt.jsonl")).unwrap();
    assert!(quarantined.contains("line=1"));
    assert!(!quarantined.contains("xxx"));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_rejects_an_oversized_unterminated_frame() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    let bounded = std::time::Duration::from_secs(15);
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("data");
    let socket_path = data_dir.join("previously.sock");
    let daemon_data = data_dir.clone();
    let mut daemon =
        tokio::spawn(async move { previously_on::hook::run_daemon(daemon_data).await });
    let connect = async {
        loop {
            match tokio::net::UnixStream::connect(&socket_path).await {
                Ok(stream) => break stream,
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
            }
        }
    };
    let mut stream = tokio::select! {
        result = &mut daemon => panic!("daemon exited before binding its socket: {result:?}"),
        result = tokio::time::timeout(bounded, connect) => {
            result.expect("daemon socket did not become ready")
        }
    };
    tokio::time::timeout(
        bounded,
        stream.write_all(&vec![b'x'; MAX_DAEMON_FRAME_BYTES + 1]),
    )
    .await
    .expect("oversized daemon write timed out")
    .unwrap();
    let mut reader = tokio::io::BufReader::new(stream);
    let mut response = String::new();
    tokio::time::timeout(bounded, reader.read_line(&mut response))
        .await
        .expect("fatal daemon acknowledgement timed out")
        .unwrap();
    let ack: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert_eq!(ack["status"], "fatal");
    assert!(ack["diagnostic"]
        .as_str()
        .unwrap()
        .contains("exceeds daemon limit"));

    let mut shutdown = tokio::net::UnixStream::connect(&socket_path).await.unwrap();
    tokio::time::timeout(
        bounded,
        shutdown.write_all(b"{\"control\":\"shutdown\",\"managedId\":\"previously-on-v1\"}\n"),
    )
    .await
    .expect("daemon shutdown write timed out")
    .unwrap();
    let mut shutdown_reader = tokio::io::BufReader::new(shutdown);
    let mut shutdown_response = String::new();
    tokio::time::timeout(bounded, shutdown_reader.read_line(&mut shutdown_response))
        .await
        .expect("daemon shutdown acknowledgement timed out")
        .unwrap();
    assert!(shutdown_response.contains("\"ok\":true"));
    tokio::time::timeout(bounded, daemon)
        .await
        .expect("daemon task did not stop")
        .unwrap()
        .unwrap();
}
