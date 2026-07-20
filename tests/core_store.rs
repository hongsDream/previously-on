use chrono::{Duration, TimeZone, Utc};
use previously_on::domain::{
    CoverageV1, EventEnvelopeV1, EventKind, EvidenceV1, FactKind, FactLifecycle, FactV1, Freshness,
    RepositoryV1, SessionLifecycle, SessionV1, TaskLifecycle, TaskV1, SCHEMA_VERSION_V1,
};
use previously_on::hook::append_fallback;
use previously_on::store::{InsertOutcome, Store};
use serde_json::json;
use tempfile::TempDir;

#[cfg(unix)]
fn set_mode(path: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

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
fn read_only_store_reads_existing_data_and_rejects_writes() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("previously.sqlite3");
    let store = Store::open(&database).unwrap();
    store.upsert_task(&task()).unwrap();
    drop(store);

    let read_only = Store::open_read_only(&database).unwrap();
    assert_eq!(read_only.list_tasks(Some("repo-1")).unwrap(), vec![task()]);
    let error = read_only
        .insert_event(&event(
            EventKind::UserPrompt,
            2,
            json!({"prompt": "private"}),
        ))
        .unwrap_err();
    assert!(error.to_string().contains("readonly"));
    assert!(Store::open_read_only(temp.path().join("missing.sqlite3")).is_err());
}

#[test]
fn fact_origin_defaults_to_captured_and_replays_explicit_origin() {
    let legacy = json!({
        "schema_version": SCHEMA_VERSION_V1,
        "id": "legacy-fact",
        "repository_id": "repo-1",
        "task_id": "task-1",
        "kind": "decision",
        "lifecycle": "confirmed",
        "freshness": "fresh",
        "content": "legacy",
        "evidence_ids": [],
        "superseded_by": null,
        "created_at": at(1),
        "updated_at": at(1)
    });
    let parsed: FactV1 = serde_json::from_value(legacy).unwrap();
    assert_eq!(parsed.origin, previously_on::domain::FactOriginV1::Captured);

    let temp = TempDir::new().unwrap();
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    let mut manual = parsed;
    manual.id = "manual-fact".into();
    manual.origin = previously_on::domain::FactOriginV1::Manual;
    store
        .insert_event(&event(EventKind::FactConfirmed, 2, json!({"fact": manual})))
        .unwrap();
    store.rebuild_projections().unwrap();
    assert_eq!(
        store.get_fact("manual-fact").unwrap().unwrap().origin,
        previously_on::domain::FactOriginV1::Manual
    );
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
fn equal_instants_with_different_rfc3339_offsets_replay_by_sequence() {
    let offset = chrono::DateTime::parse_from_rfc3339("2023-11-14T23:13:20.123456+01:00")
        .unwrap()
        .with_timezone(&Utc);
    let zulu = chrono::DateTime::parse_from_rfc3339("2023-11-14T22:13:20.123456Z")
        .unwrap()
        .with_timezone(&Utc);
    assert_eq!(offset, zulu);

    let temp = TempDir::new().unwrap();
    let database_path = temp.path().join("previously.sqlite3");
    let store = Store::open(&database_path).unwrap();
    let mut first_task = task();
    first_task.title = "first by sequence".into();
    first_task.updated_at = offset;
    let mut second_task = first_task.clone();
    second_task.title = "second by sequence".into();

    let mut first = EventEnvelopeV1::new(
        "offset-first",
        "repo-1",
        "session-1",
        EventKind::TaskUpdated,
        offset,
        json!({ "task": first_task }),
    );
    first.task_id = Some("task-1".into());
    first.sequence = Some(1);
    first.received_at = offset;
    let mut second = EventEnvelopeV1::new(
        "zulu-second",
        "repo-1",
        "session-1",
        EventKind::TaskUpdated,
        zulu,
        json!({ "task": second_task }),
    );
    second.task_id = Some("task-1".into());
    second.sequence = Some(2);
    second.received_at = zulu;

    store.insert_event(&second).unwrap();
    store.insert_event(&first).unwrap();
    store.rebuild_projections().unwrap();
    assert_eq!(
        store.get_task("task-1").unwrap().unwrap().title,
        "second by sequence"
    );

    let connection = rusqlite::Connection::open(database_path).unwrap();
    let stored = connection
        .query_row(
            "SELECT COUNT(DISTINCT occurred_at), MIN(occurred_at) FROM canonical_events",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .unwrap();
    assert_eq!(stored.0, 1);
    assert_eq!(stored.1, "2023-11-14T22:13:20.123456Z");
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
        origin: previously_on::domain::FactOriginV1::Captured,
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
fn candidate_and_evaluation_projections_rebuild_export_retain_and_purge() {
    let temp = TempDir::new().unwrap();
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    store
        .insert_event(&event(
            EventKind::RegressionCandidateRecorded,
            2,
            json!({
                "regressionCandidate": {
                    "schemaVersion": 1,
                    "id": "00000000-0000-4000-8000-000000000001",
                    "repositoryId": "repo-1",
                    "taskId": "task-1",
                    "title": "Keep auth stable api_key=never-store-candidate",
                    "invariant": "Auth behavior remains stable",
                    "status": "pending",
                    "impactSelectors": [{"path":{"kind":"exact","value":"src/auth.rs"},"symbols":[]}],
                    "requiredTests": [{
                        "id":"auth-test",
                        "name":"auth test",
                        "program":"cargo",
                        "args":["test","auth"],
                        "workingDirectory":".",
                        "timeoutSeconds":900
                    }],
                    "origin":{
                        "fixedAtCommit":"0000000000000000000000000000000000000000",
                        "recordedAt":at(2),
                        "evidenceSha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    },
                    "createdAt":at(2),
                    "updatedAt":at(2),
                    "evidenceKind":"manual",
                    "evidenceSha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                }
            }),
        ))
        .unwrap();
    store
        .insert_event(&event(
            EventKind::ContractEvaluationRecorded,
            3,
            json!({
                "contractEvaluation": {
                    "schemaVersion": 1,
                    "id": "evaluation-1",
                    "repositoryId": "repo-1",
                    "taskId": "task-1",
                    "readiness": "contract_blocked",
                    "evaluatedAt":at(3),
                    "relevantContracts":[],
                    "requiredTests":[],
                    "warnings":["--token never-store-evaluation"],
                    "contentFingerprint":"fingerprint-1",
                    "continuationIssued":false
                }
            }),
        ))
        .unwrap();

    assert_eq!(store.list_regression_candidates(None).unwrap().len(), 1);
    assert_eq!(store.list_contract_evaluations(None).unwrap().len(), 1);
    let export = store.export_json(Some("repo-1")).unwrap();
    let serialized = serde_json::to_string(&export).unwrap();
    assert_eq!(export["regressionCandidates"].as_array().unwrap().len(), 1);
    assert_eq!(export["contractEvaluations"].as_array().unwrap().len(), 1);
    assert!(!serialized.contains("never-store-candidate"));
    assert!(!serialized.contains("never-store-evaluation"));

    store.rebuild_projections().unwrap();
    assert_eq!(store.list_regression_candidates(None).unwrap().len(), 1);
    assert_eq!(store.list_contract_evaluations(None).unwrap().len(), 1);

    let mut older_candidate = store
        .get_regression_candidate("00000000-0000-4000-8000-000000000001")
        .unwrap()
        .unwrap();
    older_candidate.title = "Older candidate snapshot".into();
    older_candidate.updated_at = at(1);
    store
        .insert_event(&event(
            EventKind::RegressionCandidateRecorded,
            1,
            json!({"regressionCandidate":older_candidate}),
        ))
        .unwrap();
    let mut older_evaluation = store
        .get_contract_evaluation("evaluation-1")
        .unwrap()
        .unwrap();
    older_evaluation.id = "evaluation-old".into();
    older_evaluation.evaluated_at = at(1);
    store
        .insert_event(&event(
            EventKind::ContractEvaluationRecorded,
            1,
            json!({"contractEvaluation":older_evaluation}),
        ))
        .unwrap();
    assert_eq!(
        store
            .get_regression_candidate("00000000-0000-4000-8000-000000000001")
            .unwrap()
            .unwrap()
            .title,
        "Keep auth stable api_key=[REDACTED]"
    );
    assert_eq!(store.list_contract_evaluations(None).unwrap().len(), 2);

    let report = store.apply_retention(at(120 * 24 * 60 * 60), 90).unwrap();
    assert_eq!(report.removed_events, 2);
    assert_eq!(store.list_regression_candidates(None).unwrap().len(), 1);
    assert_eq!(store.list_contract_evaluations(None).unwrap().len(), 1);
    let before_rebuild = serde_json::to_value((
        store.list_regression_candidates(None).unwrap(),
        store.list_contract_evaluations(None).unwrap(),
    ))
    .unwrap();
    store.rebuild_projections().unwrap();
    let after_rebuild = serde_json::to_value((
        store.list_regression_candidates(None).unwrap(),
        store.list_contract_evaluations(None).unwrap(),
    ))
    .unwrap();
    assert_eq!(before_rebuild, after_rebuild);

    store.purge_repository("repo-1").unwrap();
    assert!(store.list_regression_candidates(None).unwrap().is_empty());
    assert!(store.list_contract_evaluations(None).unwrap().is_empty());
}

#[test]
fn retention_preserves_once_per_fingerprint_continuation_guard() {
    let temp = TempDir::new().unwrap();
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    let evaluation = |id: &str, continuation_issued: bool, evaluated_at| {
        json!({
            "contractEvaluation": {
                "schemaVersion": 1,
                "id": id,
                "repositoryId": "repo-1",
                "taskId": "task-1",
                "readiness": "contract_blocked",
                "evaluatedAt": evaluated_at,
                "relevantContracts": [{
                    "id": "00000000-0000-4000-8000-000000000001",
                    "title": "Keep auth stable",
                    "invariant": "Auth remains stable",
                    "matchReasons": ["src/auth.rs matched"]
                }],
                "requiredTests": [],
                "warnings": [],
                "contentFingerprint": "same-related-fingerprint",
                "continuationIssued": continuation_issued
            }
        })
    };
    store
        .insert_event(&event(
            EventKind::ContractEvaluationRecorded,
            1,
            evaluation("evaluation-issued", true, at(1)),
        ))
        .unwrap();
    store
        .insert_event(&event(
            EventKind::ContractEvaluationRecorded,
            2,
            evaluation("evaluation-active-stop", false, at(2)),
        ))
        .unwrap();

    let report = store.apply_retention(at(120 * 24 * 60 * 60), 90).unwrap();
    assert_eq!(report.removed_events, 0);
    let evaluations = store.list_contract_evaluations(None).unwrap();
    assert_eq!(evaluations.len(), 2);
    assert!(evaluations
        .iter()
        .any(|evaluation| evaluation.continuation_issued));
    store.rebuild_projections().unwrap();
    assert!(store
        .list_contract_evaluations(None)
        .unwrap()
        .iter()
        .any(|evaluation| evaluation.continuation_issued));
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
    #[cfg(unix)]
    set_mode(&journal, 0o600);

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
    #[cfg(unix)]
    set_mode(&queue_dir, 0o700);
    let queue = queue_dir.join("events.jsonl");
    std::fs::write(&queue, b"malformed queue data\n").unwrap();
    #[cfg(unix)]
    set_mode(&queue, 0o600);
    let cache = temp.path().join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("stale-secret"), b"must disappear").unwrap();
    let journal = std::path::PathBuf::from(format!("{}.purge-recovery.json", database.display()));
    std::fs::write(
        &journal,
        br#"{"version":1,"repository_id":"repo-1","phase":"tombstoned"}"#,
    )
    .unwrap();
    #[cfg(unix)]
    set_mode(&journal, 0o600);

    let recovered = Store::open(&database).unwrap();
    assert!(!journal.exists());
    assert!(!queue.exists());
    assert!(!cache.exists());
    assert!(!std::path::PathBuf::from(format!("{}-wal", database.display())).exists());
    assert!(!std::path::PathBuf::from(format!("{}-shm", database.display())).exists());
    assert_eq!(recovered.health().unwrap().canonical_event_count, 0);
}

#[cfg(unix)]
#[test]
fn store_rejects_insecure_directory_and_symlinked_database_without_mutation() {
    use std::os::unix::fs::symlink;

    {
        let temp = TempDir::new().unwrap();
        let data = temp.path().join("data");
        std::fs::create_dir(&data).unwrap();
        set_mode(&data, 0o777);
        std::fs::write(data.join("marker"), b"safe").unwrap();

        let error = Store::open(data.join("previously.sqlite3")).unwrap_err();

        assert!(error.to_string().contains("group/world writable"));
        assert_eq!(std::fs::read(data.join("marker")).unwrap(), b"safe");
        assert!(!data.join("previously.sqlite3").exists());
    }

    {
        let temp = TempDir::new().unwrap();
        let data = temp.path().join("data");
        std::fs::create_dir(&data).unwrap();
        set_mode(&data, 0o700);
        let external = temp.path().join("outside.sqlite3");
        std::fs::write(&external, b"outside-safe").unwrap();
        set_mode(&external, 0o600);
        let database = data.join("previously.sqlite3");
        symlink(&external, &database).unwrap();

        let error = Store::open(&database).unwrap_err();

        assert!(error.to_string().contains("regular file"));
        assert_eq!(std::fs::read(&external).unwrap(), b"outside-safe");
        assert!(std::fs::symlink_metadata(&database)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(!data.join("previously.sqlite3.lock").exists());
    }
}

#[cfg(unix)]
#[test]
fn store_rejects_symlinked_sqlite_companions_before_opening_database() {
    use std::os::unix::fs::symlink;

    for suffix in ["-wal", "-shm", "-journal", ".lock", ".purge-recovery.json"] {
        let temp = TempDir::new().unwrap();
        let database = temp.path().join("previously.sqlite3");
        drop(Store::open(&database).unwrap());
        let database_before = std::fs::read(&database).unwrap();
        let companion = std::path::PathBuf::from(format!("{}{}", database.display(), suffix));
        if companion.exists() {
            std::fs::remove_file(&companion).unwrap();
        }
        let external = temp
            .path()
            .join(format!("outside-{}", suffix.replace('.', "dot")));
        std::fs::write(&external, b"outside-safe").unwrap();
        set_mode(&external, 0o600);
        symlink(&external, &companion).unwrap();

        let error = Store::open(&database).unwrap_err();

        assert!(
            error.to_string().contains("regular file"),
            "{suffix}: unexpected error: {error:#}"
        );
        assert_eq!(std::fs::read(&external).unwrap(), b"outside-safe");
        assert_eq!(std::fs::read(&database).unwrap(), database_before);
        assert!(std::fs::symlink_metadata(&companion)
            .unwrap()
            .file_type()
            .is_symlink());
    }
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
        origin: previously_on::domain::FactOriginV1::Captured,
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
