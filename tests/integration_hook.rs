use std::io::Cursor;

use previously_on::hook::{
    append_fallback, capture, hook_response, replay_fallback, run_hook, HookEvent,
    HookIngressConfig, ResumeCandidateMetadata, MAX_HOOK_PAYLOAD_BYTES,
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
        registered_repository: Some(registered),
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
