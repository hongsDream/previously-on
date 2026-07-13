use std::io::Cursor;
use std::sync::{Arc, Barrier};

use chrono::Utc;
use previously_on::domain::{EventEnvelopeV1, EventKind};
use previously_on::hook::{append_fallback, capture, replay_fallback, HookEvent};
use previously_on::store::{InsertOutcome, Store};
use serde_json::json;
use tempfile::TempDir;

const SECRETS: [&str; 10] = [
    "OPENAI_API_KEY=opaque-openai",
    "AWS_SECRET_ACCESS_KEY=opaque-aws",
    "NPM_TOKEN=opaque-npm",
    "Authorization: Bearer opaque-authorization",
    "https://person:opaque-url@example.invalid/path",
    "-----BEGIN PRIVATE KEY-----\nopaque-pem\n-----END PRIVATE KEY-----",
    "/tmp/project/.env.production",
    "/Users/person/.ssh/id_ed25519",
    "token=opaque-bare-token",
    "--token opaque-cli-token",
];

#[test]
fn one_secret_corpus_is_absent_from_queue_database_export_rebuild_and_retention() {
    let temp = TempDir::new().unwrap();
    let data = temp.path().join("data");
    let database = data.join("previously.sqlite3");
    let queue = data.join("queue/events.jsonl");
    let payload = json!({
        "session_id": "privacy-session",
        "turn_id": "turn-1",
        "cwd": "/tmp/not-a-repository",
        "tool_name": "Bash",
        "tool_use_id": "privacy-tool",
        "tool_input": {"command": SECRETS.join(" ")},
        "tool_response": {"output": "credentials.json Authorization: Basic opaque-basic"}
    });
    let mut input = Cursor::new(serde_json::to_vec(&payload).unwrap());
    let event = capture(HookEvent::PostToolUse, &mut input).unwrap();
    append_fallback(&queue, &event).unwrap();
    assert_no_secrets(&data);

    let store = Store::open(&database).unwrap();
    replay_fallback(&store, &queue).unwrap();
    let exported = serde_json::to_vec(&store.export_json(None).unwrap()).unwrap();
    assert_bytes_have_no_secrets(&exported);
    store.rebuild_projections().unwrap();
    store.apply_retention(Utc::now(), 90).unwrap();
    drop(store);
    assert_no_secrets(&data);

    // Malformed queue data is untrusted and must be redacted before quarantine too.
    std::fs::create_dir_all(queue.parent().unwrap()).unwrap();
    std::fs::write(&queue, format!("not-json {}\n", SECRETS.join(" "))).unwrap();
    let store = Store::open(&database).unwrap();
    replay_fallback(&store, &queue).unwrap();
    assert_no_secrets(&data);
}

#[test]
fn twenty_sessions_replay_duplicate_and_reordered_events_exactly_once() {
    let adversarial = std::env::var_os("PREVIOUSLY_ON_ADVERSARIAL_TESTS").is_some();
    let sessions = if adversarial { 20 } else { 4 };
    let submissions_per_session = if adversarial { 100 } else { 10 };
    let unique_per_session = submissions_per_session / 2;
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("previously.sqlite3");
    let store = Store::open(&database).unwrap();
    let barrier = Arc::new(Barrier::new(sessions));
    let mut workers = Vec::new();

    for session in 0..sessions {
        let store = store.clone();
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            let unique = (0..unique_per_session)
                .map(|index| {
                    EventEnvelopeV1::new(
                        format!("source-{session}-{index}"),
                        "repo-concurrency",
                        format!("session-{session}"),
                        EventKind::Unknown,
                        Utc::now(),
                        json!({"index":index}),
                    )
                })
                .collect::<Vec<_>>();
            let mut events = unique.clone();
            events.extend(unique);
            events.reverse();
            barrier.wait();
            for event in events {
                let outcome = store.insert_event(&event).unwrap();
                assert!(matches!(
                    outcome,
                    InsertOutcome::Inserted | InsertOutcome::Duplicate
                ));
            }
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }

    assert_eq!(
        store.health().unwrap().canonical_event_count,
        (sessions * unique_per_session) as u64
    );
    let before = store.list_events(None).unwrap();
    store.rebuild_projections().unwrap();
    let after = store.list_events(None).unwrap();
    assert_eq!(before, after);
}

fn assert_no_secrets(root: &std::path::Path) {
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let bytes = std::fs::read(entry.path()).unwrap();
        assert_bytes_have_no_secrets(&bytes);
    }
}

fn assert_bytes_have_no_secrets(bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    for secret in SECRETS {
        let raw_value = secret
            .split_once('=')
            .map(|(_, value)| value)
            .or_else(|| secret.split_once("Bearer ").map(|(_, value)| value))
            .or_else(|| secret.split_once("--token ").map(|(_, value)| value))
            .unwrap_or(secret);
        assert!(
            !text.contains(raw_value),
            "secret leaked into persisted bytes: {raw_value}"
        );
    }
    for extra in ["opaque-basic", "credentials.json"] {
        assert!(!text.contains(extra), "secret leaked: {extra}");
    }
    assert!(
        !text.contains("opaque-"),
        "partial secret marker leaked into persisted bytes"
    );
}
