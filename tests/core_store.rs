use chrono::{Duration, TimeZone, Utc};
use previously_on::domain::{
    CoverageV1, EventEnvelopeV1, EventKind, EvidenceV1, FactKind, FactLifecycle, FactV1, Freshness,
    RepositoryV1, SessionLifecycle, SessionV1, TaskLifecycle, TaskV1, SCHEMA_VERSION_V1,
};
use previously_on::hook::append_fallback;
use previously_on::store::{InsertOutcome, Store};
use serde_json::json;
use tempfile::TempDir;

fn at(second: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + second, 0)
        .single()
        .unwrap()
}

fn task() -> TaskV1 {
    TaskV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: "task-1".into(),
        repository_id: "repo-1".into(),
        title: "Authentication cleanup".into(),
        goal: Some("Make refresh tokens safe".into()),
        lifecycle: TaskLifecycle::Active,
        branch: Some("main".into()),
        created_at: at(1),
        updated_at: at(1),
    }
}

fn session(lifecycle: SessionLifecycle) -> SessionV1 {
    SessionV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: "session-1".into(),
        repository_id: "repo-1".into(),
        task_id: Some("task-1".into()),
        lifecycle,
        started_at: at(1),
        ended_at: None,
        branch: Some("main".into()),
        head: Some("abc".into()),
        source_thread_id: None,
        last_activity_at: Some(at(1)),
        turn_count: 0,
        compaction_count: 0,
        context_usage: None,
        continuation_state: Default::default(),
        coverage: CoverageV1::default(),
    }
}

fn event(kind: EventKind, time: i64, payload: serde_json::Value) -> EventEnvelopeV1 {
    let mut event = EventEnvelopeV1::new(
        format!("source-{time}-{kind:?}"),
        "repo-1",
        "session-1",
        kind,
        at(time),
        payload,
    );
    event.task_id = Some("task-1".into());
    event.received_at = at(time) + Duration::seconds(1);
    event
}

#[test]
fn deduplicates_and_replays_reordered_events_deterministically() {
    let temp = TempDir::new().unwrap();
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    let stopped = event(EventKind::SessionStopped, 2, json!({}));
    let started = event(
        EventKind::SessionStarted,
        1,
        json!({"task":task(), "session":session(SessionLifecycle::Active)}),
    );

    assert_eq!(
        store.insert_event(&stopped).unwrap(),
        InsertOutcome::Inserted
    );
    assert_eq!(
        store.insert_event(&started).unwrap(),
        InsertOutcome::Inserted
    );
    assert_eq!(
        store.insert_event(&started).unwrap(),
        InsertOutcome::Duplicate
    );

    let timeline = store.get_task_timeline("task-1").unwrap().unwrap();
    assert_eq!(timeline.sessions.len(), 1);
    assert_eq!(timeline.sessions[0].lifecycle, SessionLifecycle::Completed);
    assert_eq!(store.health().unwrap().canonical_event_count, 2);

    let before = serde_json::to_value(&timeline).unwrap();
    store.rebuild_projections().unwrap();
    let after = serde_json::to_value(store.get_task_timeline("task-1").unwrap().unwrap()).unwrap();
    assert_eq!(before, after);
}

#[test]
fn redacts_before_canonical_persistence_and_rebuilds_fact_projection() {
    let temp = TempDir::new().unwrap();
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    let started = event(
        EventKind::SessionStarted,
        1,
        json!({"task":task(), "session":session(SessionLifecycle::Active), "api_key":"never-store-this"}),
    );
    store.insert_event(&started).unwrap();

    let fact = FactV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: "fact-1".into(),
        repository_id: "repo-1".into(),
        task_id: "task-1".into(),
        kind: FactKind::Decision,
        lifecycle: FactLifecycle::Confirmed,
        freshness: Freshness::Fresh,
        content: "Tokens stay server-side".into(),
        evidence_ids: vec!["evidence-1".into()],
        superseded_by: None,
        created_at: at(2),
        updated_at: at(2),
    };
    let fact_event = event(EventKind::FactConfirmed, 2, json!({"fact":fact}));
    store.insert_event(&fact_event).unwrap();
    assert!(store.get_fact("fact-1").unwrap().is_some());

    let export = serde_json::to_string(&store.export_json(None).unwrap()).unwrap();
    assert!(!export.contains("never-store-this"));
    store.rebuild_projections().unwrap();
    assert_eq!(store.list_facts("task-1").unwrap().len(), 1);
    assert_eq!(store.search_tasks("authentication", 5).unwrap().len(), 1);
}

#[test]
fn purge_removes_canonical_and_projected_repository_data() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("previously.sqlite3");
    let store = Store::open(&database).unwrap();
    store
        .insert_event(&event(
            EventKind::SessionStarted,
            1,
            json!({"task":task(), "session":session(SessionLifecycle::Active)}),
        ))
        .unwrap();
    store.purge_repository("repo-1").unwrap();
    assert_eq!(store.health().unwrap().canonical_event_count, 0);
    assert!(store.get_task("task-1").unwrap().is_none());
    assert!(
        store.export_json(Some("repo-1")).unwrap()["canonical_events"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let bytes = std::fs::read(database).unwrap();
    assert!(!bytes
        .windows(b"repo-1".len())
        .any(|window| window == b"repo-1"));
}

#[test]
fn failed_related_purge_keeps_db_and_tombstone_until_idempotent_resume() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("previously.sqlite3");
    let store = Store::open(&database).unwrap();
    let started = event(
        EventKind::SessionStarted,
        1,
        json!({"task":task(), "session":session(SessionLifecycle::Active)}),
    );
    store.insert_event(&started).unwrap();

    let error = store
        .purge_repository_with("repo-1", || anyhow::bail!("injected queue rewrite failure"))
        .unwrap_err();
    assert!(error.to_string().contains("injected queue rewrite failure"));
    assert_eq!(store.health().unwrap().canonical_event_count, 1);
    let ingest_error = store.insert_event(&started).unwrap_err();
    assert!(ingest_error.to_string().contains("purged"));

    store.purge_repository_with("repo-1", || Ok(())).unwrap();
    assert_eq!(store.health().unwrap().canonical_event_count, 0);
    assert!(
        !std::path::PathBuf::from(format!("{}.purge-recovery.json", database.display())).exists()
    );
}

#[test]
fn durable_purge_tombstone_blocks_waiting_database_and_queue_writers() {
    use std::sync::{Arc, Barrier};

    let temp = TempDir::new().unwrap();
    let database = temp.path().join("previously.sqlite3");
    let queue = temp.path().join("queue/events.jsonl");
    let store = Store::open(&database).unwrap();
    let started = event(
        EventKind::SessionStarted,
        1,
        json!({"task":task(), "session":session(SessionLifecycle::Active)}),
    );
    store.insert_event(&started).unwrap();

    let purge_entered = Arc::new(Barrier::new(2));
    let finish_related_cleanup = Arc::new(Barrier::new(2));
    let purge_store = store.clone();
    let purge_entered_worker = Arc::clone(&purge_entered);
    let finish_related_cleanup_worker = Arc::clone(&finish_related_cleanup);
    let purge_thread = std::thread::spawn(move || {
        purge_store.purge_repository_with("repo-1", || {
            purge_entered_worker.wait();
            finish_related_cleanup_worker.wait();
            Ok(())
        })
    });

    // The purge owns the shared maintenance/ingestion lock and has durably written the marker.
    purge_entered.wait();
    let insert_store = store.clone();
    let insert_event = started.clone();
    let insert_thread = std::thread::spawn(move || insert_store.insert_event(&insert_event));
    let queued_event = started.clone();
    let queue_for_thread = queue.clone();
    let append_thread =
        std::thread::spawn(move || append_fallback(&queue_for_thread, &queued_event));

    finish_related_cleanup.wait();
    purge_thread.join().unwrap().unwrap();

    let insert_error = insert_thread.join().unwrap().unwrap_err();
    assert!(insert_error.to_string().contains("was purged"));
    let append_error = append_thread.join().unwrap().unwrap_err();
    assert!(append_error.to_string().contains("was purged"));
    assert!(!queue.exists());
    assert_eq!(store.health().unwrap().canonical_event_count, 0);

    // Removing the transient recovery journal does not remove the deletion authorization gate.
    let reopened = Store::open(&database).unwrap();
    assert!(reopened
        .insert_event(&started)
        .unwrap_err()
        .to_string()
        .contains("was purged"));

    previously_on::store::reactivate_repository(temp.path(), "repo-1").unwrap();
    assert_eq!(
        reopened.insert_event(&started).unwrap(),
        InsertOutcome::Inserted
    );
}

#[test]
fn open_completes_a_purge_interrupted_after_related_data_cleanup() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("previously.sqlite3");
    {
        let store = Store::open(&database).unwrap();
        store
            .insert_event(&event(
                EventKind::SessionStarted,
                1,
                json!({"task":task(), "session":session(SessionLifecycle::Active)}),
            ))
            .unwrap();
    }
    let journal = std::path::PathBuf::from(format!("{}.purge-recovery.json", database.display()));
    std::fs::write(
        &journal,
        br#"{"version":1,"repository_id":"repo-1","phase":"related_data_purged"}"#,
    )
    .unwrap();

    let recovered = Store::open(&database).unwrap();
    assert_eq!(recovered.health().unwrap().canonical_event_count, 0);
    assert!(!journal.exists());
    assert!(recovered
        .insert_event(&event(EventKind::UserPrompt, 2, json!({})))
        .unwrap_err()
        .to_string()
        .contains("was purged"));
    let bytes = std::fs::read(database).unwrap();
    assert!(!bytes
        .windows(b"repo-1".len())
        .any(|window| window == b"repo-1"));
}

#[test]
fn open_completes_tombstoned_purge_and_discards_malformed_single_repo_queue() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("previously.sqlite3");
    {
        let store = Store::open(&database).unwrap();
        store
            .insert_event(&event(
                EventKind::SessionStarted,
                1,
                json!({"task":task(), "session":session(SessionLifecycle::Active)}),
            ))
            .unwrap();
    }
    let queue_dir = temp.path().join("queue");
    std::fs::create_dir_all(&queue_dir).unwrap();
    let queue = queue_dir.join("events.jsonl");
    std::fs::write(&queue, b"malformed queue data\n").unwrap();
    let cache = temp.path().join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("stale-secret"), b"must disappear").unwrap();
    let journal = std::path::PathBuf::from(format!("{}.purge-recovery.json", database.display()));
    std::fs::write(
        &journal,
        br#"{"version":1,"repository_id":"repo-1","phase":"tombstoned"}"#,
    )
    .unwrap();

    let recovered = Store::open(&database).unwrap();
    assert!(!journal.exists());
    assert!(!queue.exists());
    assert!(!cache.exists());
    assert!(!std::path::PathBuf::from(format!("{}-wal", database.display())).exists());
    assert!(!std::path::PathBuf::from(format!("{}-shm", database.display())).exists());
    assert_eq!(recovered.health().unwrap().canonical_event_count, 0);
}

#[test]
fn retention_keeps_only_recent_events_and_minimum_pinned_evidence() {
    let temp = TempDir::new().unwrap();
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    let mut pinned_evidence = EvidenceV1::new(
        "evidence-pinned",
        "repo-1",
        "task-1",
        "session-1",
        "source-pinned",
        "User confirmed the boundary decision.",
        at(2),
    );
    pinned_evidence.fact_id = Some("fact-pinned".into());
    let pinned_fact = FactV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: "fact-pinned".into(),
        repository_id: "repo-1".into(),
        task_id: "task-1".into(),
        kind: FactKind::Decision,
        lifecycle: FactLifecycle::Pinned,
        freshness: Freshness::Fresh,
        content: "Keep auth enforcement in middleware".into(),
        evidence_ids: vec![pinned_evidence.id.clone()],
        superseded_by: None,
        created_at: at(2),
        updated_at: at(2),
    };
    store
        .insert_event(&event(
            EventKind::FactConfirmed,
            2,
            json!({"fact":pinned_fact,"evidence":pinned_evidence}),
        ))
        .unwrap();
    store
        .insert_event(&event(
            EventKind::UserPrompt,
            3,
            json!({"prompt":"old unpinned context"}),
        ))
        .unwrap();
    let now = at(120 * 24 * 60 * 60);
    store
        .insert_event(&event(
            EventKind::SessionStopped,
            119 * 24 * 60 * 60,
            json!({}),
        ))
        .unwrap();

    let report = store.apply_retention(now, 90).unwrap();
    assert_eq!(report.removed_events, 1);
    assert!(store.get_fact("fact-pinned").unwrap().is_some());
    assert!(store.get_evidence("evidence-pinned").unwrap().is_some());
    assert_eq!(store.health().unwrap().integrity_check, "ok");
}

#[test]
fn resume_suggestion_uses_projected_worktree_path_not_logical_repository_id() {
    let temp = TempDir::new().unwrap();
    let worktree = temp.path().join("worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    let status = std::process::Command::new("git")
        .args(["init", "--quiet", "-b", "main"])
        .arg(&worktree)
        .status()
        .unwrap();
    assert!(status.success());
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    store
        .upsert_repository(&RepositoryV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: "logical-common-dir-id".into(),
            path: worktree.to_string_lossy().into_owned(),
            remote_url: None,
            created_at: at(1),
            updated_at: at(1),
        })
        .unwrap();
    let mut resume_task = task();
    resume_task.repository_id = "logical-common-dir-id".into();
    resume_task.branch = Some("main".into());
    store.upsert_task(&resume_task).unwrap();

    let suggestions = store
        .suggest_resume("logical-common-dir-id", "authentication", 1)
        .unwrap();
    assert_eq!(suggestions.len(), 1);
    assert!(suggestions[0]
        .matching_reasons
        .contains(&"same_branch".to_string()));
}
