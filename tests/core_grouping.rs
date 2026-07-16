use chrono::{TimeZone, Utc};
use previously_on::domain::{
    ChangeAttribution, ChangeStatus, CheckpointV1, CoverageV1, EventEnvelopeV1, EventKind,
    EvidenceV1, FactKind, FactLifecycle, FactV1, FileChangeV1, Freshness, GitSnapshotV1,
    SessionLifecycle, SessionV1, TaskGroupingActionV1, TaskLifecycle, TaskV1, TestResultV1,
    TestStatus, SCHEMA_VERSION_V1,
};
use previously_on::grouping::{inverse, preview, request_fingerprint, TaskGroupingRequestV1};
use previously_on::store::{ClaimOutcome, InsertOutcome, Store};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::{Arc, Barrier};
use tempfile::TempDir;

fn at(second: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_710_000_000 + second, 0)
        .single()
        .unwrap()
}

fn task(id: &str, repository_id: &str) -> TaskV1 {
    TaskV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: id.into(),
        repository_id: repository_id.into(),
        title: format!("Task {id}"),
        goal: None,
        lifecycle: TaskLifecycle::Active,
        branch: Some("feature/grouping".into()),
        created_at: at(1),
        updated_at: at(1),
    }
}

fn session(id: &str, repository_id: &str, task_id: &str) -> SessionV1 {
    SessionV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: id.into(),
        repository_id: repository_id.into(),
        task_id: Some(task_id.into()),
        lifecycle: SessionLifecycle::Active,
        started_at: at(2),
        ended_at: None,
        branch: Some("feature/grouping".into()),
        head: Some(format!("head-{id}")),
        source_thread_id: Some(format!("thread-{id}")),
        last_activity_at: Some(at(2)),
        turn_count: 1,
        compaction_count: 0,
        context_usage: None,
        continuation_state: Default::default(),
        coverage: CoverageV1::default(),
    }
}

fn event(
    repository_id: &str,
    session_id: &str,
    task_id: Option<&str>,
    kind: EventKind,
    second: i64,
    payload: serde_json::Value,
) -> EventEnvelopeV1 {
    let mut event = EventEnvelopeV1::new(
        format!("source-{session_id}-{second}-{kind:?}"),
        repository_id,
        session_id,
        kind,
        at(second),
        payload,
    );
    event.task_id = task_id.map(str::to_string);
    event.received_at = at(second);
    event
}

fn insert_task(store: &Store, task: &TaskV1, second: i64) {
    store
        .insert_event(&event(
            &task.repository_id,
            "local-ui",
            Some(&task.id),
            EventKind::TaskUpdated,
            second,
            json!({ "task": task }),
        ))
        .unwrap();
}

fn insert_session(store: &Store, task: &TaskV1, session: &SessionV1, second: i64) {
    store
        .insert_event(&event(
            &task.repository_id,
            &session.id,
            Some(&task.id),
            EventKind::SessionStarted,
            second,
            json!({ "task": task, "session": session }),
        ))
        .unwrap();
}

fn change(repository_id: &str, task_id: &str, session_id: &str, path: &str) -> FileChangeV1 {
    FileChangeV1 {
        schema_version: SCHEMA_VERSION_V1,
        repository_id: repository_id.into(),
        session_id: session_id.into(),
        task_id: Some(task_id.into()),
        path: path.into(),
        previous_path: None,
        status: ChangeStatus::Modified,
        additions: Some(1),
        deletions: Some(0),
        attribution: ChangeAttribution::ObservedChangedIn,
        before_head: Some("before".into()),
        after_head: Some("after".into()),
    }
}

fn test_result(repository_id: &str, task_id: &str, session_id: &str) -> TestResultV1 {
    TestResultV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: format!("test-{session_id}"),
        repository_id: repository_id.into(),
        session_id: session_id.into(),
        task_id: Some(task_id.into()),
        name: "targeted test".into(),
        command: "cargo test targeted".into(),
        status: TestStatus::Passed,
        summary: None,
        occurred_at: at(4),
    }
}

fn checkpoint(repository_id: &str, task_id: &str, session_id: &str) -> CheckpointV1 {
    let changed = change(
        repository_id,
        task_id,
        session_id,
        &format!("src/{session_id}.rs"),
    );
    let test = test_result(repository_id, task_id, session_id);
    CheckpointV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: format!("checkpoint-{session_id}"),
        repository_id: repository_id.into(),
        task_id: task_id.into(),
        session_id: session_id.into(),
        created_at: at(4),
        goal_hint: None,
        git_before: None,
        git_after: GitSnapshotV1 {
            schema_version: SCHEMA_VERSION_V1,
            repository_id: repository_id.into(),
            root: "/tmp/repository".into(),
            remote_url: None,
            branch: Some("feature/grouping".into()),
            head: Some(format!("commit-{session_id}")),
            captured_at: at(4),
            dirty_files: Vec::new(),
            working_tree_changes: vec![changed.clone()],
            content_fingerprints: BTreeMap::new(),
        },
        changed_files: vec![changed],
        tests: vec![test],
        failures: Vec::new(),
        unresolved_items: Vec::new(),
        coverage: CoverageV1::default(),
    }
}

fn insert_fact(
    store: &Store,
    repository_id: &str,
    task_id: &str,
    fact_id: &str,
    evidence: &[(&str, &str)],
    second: i64,
) {
    let fact = FactV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: fact_id.into(),
        repository_id: repository_id.into(),
        task_id: task_id.into(),
        kind: FactKind::Decision,
        lifecycle: FactLifecycle::Confirmed,
        freshness: Freshness::Fresh,
        origin: previously_on::domain::FactOriginV1::Captured,
        content: format!("Fact {fact_id}"),
        evidence_ids: evidence.iter().map(|(id, _)| (*id).to_string()).collect(),
        superseded_by: None,
        created_at: at(second),
        updated_at: at(second),
    };
    for (index, (evidence_id, session_id)) in evidence.iter().enumerate() {
        let mut item = EvidenceV1::new(
            *evidence_id,
            repository_id,
            task_id,
            *session_id,
            format!("source-{evidence_id}"),
            format!("Evidence {evidence_id}"),
            at(second + index as i64),
        );
        item.fact_id = Some(fact_id.into());
        store
            .insert_event(&event(
                repository_id,
                session_id,
                Some(task_id),
                EventKind::FactConfirmed,
                second + index as i64,
                json!({ "fact": fact, "evidence": item }),
            ))
            .unwrap();
    }
}

fn setup_store() -> (TempDir, Store, TaskV1, TaskV1) {
    let temp = TempDir::new().unwrap();
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    let source = task("source", "repo-1");
    let target = task("target", "repo-1");
    insert_task(&store, &source, 1);
    insert_task(&store, &target, 2);
    for (index, id) in ["session-1", "session-2"].iter().enumerate() {
        let session = session(id, "repo-1", "source");
        insert_session(&store, &source, &session, 10 + index as i64);
        let checkpoint = checkpoint("repo-1", "source", id);
        store
            .insert_event(&event(
                "repo-1",
                id,
                Some("source"),
                EventKind::Checkpoint,
                20 + index as i64,
                json!({ "checkpoint": checkpoint }),
            ))
            .unwrap();
    }
    insert_fact(
        &store,
        "repo-1",
        "source",
        "fact-moved",
        &[("evidence-moved", "session-1")],
        30,
    );
    insert_fact(
        &store,
        "repo-1",
        "source",
        "fact-mixed",
        &[
            ("evidence-mixed-1", "session-1"),
            ("evidence-mixed-2", "session-2"),
        ],
        40,
    );
    (temp, store, source, target)
}

#[test]
fn move_is_atomic_replayable_idempotent_and_preserves_mixed_fact_provenance() {
    let (_temp, store, _source, _target) = setup_store();
    let request = TaskGroupingRequestV1 {
        operation_id: "move-1".into(),
        action: TaskGroupingActionV1::Move,
        session_ids: vec!["session-1".into()],
        from_task_id: "source".into(),
        target_task_id: Some("target".into()),
        new_task_title: None,
        new_task_goal: None,
    };
    let preview = preview(&store, &request).unwrap();
    assert_eq!(preview.counts.sessions, 1);
    assert_eq!(preview.counts.facts_moved, 1);
    assert_eq!(preview.counts.facts_mixed, 1);
    assert_eq!(
        store
            .append_task_grouping_operation(&preview.operation)
            .unwrap(),
        InsertOutcome::Inserted
    );
    assert_eq!(
        store
            .append_task_grouping_operation(&preview.operation)
            .unwrap(),
        InsertOutcome::Duplicate
    );
    assert_eq!(
        store
            .get_session("session-1")
            .unwrap()
            .unwrap()
            .task_id
            .as_deref(),
        Some("target")
    );
    assert_eq!(store.list_checkpoints("target").unwrap().len(), 1);
    assert_eq!(store.list_evidence("target").unwrap().len(), 2);
    assert_eq!(store.list_file_changes("target").unwrap().len(), 1);
    assert_eq!(store.list_test_results("target").unwrap().len(), 1);
    assert_eq!(
        store.get_fact("fact-moved").unwrap().unwrap().task_id,
        "target"
    );
    assert_eq!(
        store.get_fact("fact-mixed").unwrap().unwrap().task_id,
        "source"
    );

    let stale_change = change("repo-1", "source", "session-1", "src/late.rs");
    store
        .insert_event(&event(
            "repo-1",
            "session-1",
            Some("source"),
            EventKind::ToolFinished,
            60,
            json!({ "file_changes": [stale_change] }),
        ))
        .unwrap();
    assert!(store
        .list_file_changes("target")
        .unwrap()
        .iter()
        .any(|change| change.path == "src/late.rs"));

    let before = serde_json::to_vec(&json!({
        "source": store.get_task_timeline("source").unwrap(),
        "target": store.get_task_timeline("target").unwrap(),
        "sourceEvidence": store.list_evidence("source").unwrap(),
        "targetEvidence": store.list_evidence("target").unwrap(),
        "sourceFiles": store.list_file_changes("source").unwrap(),
        "targetFiles": store.list_file_changes("target").unwrap(),
        "operations": store.list_task_grouping_operations(Some("repo-1")).unwrap(),
    }))
    .unwrap();
    store.rebuild_projections().unwrap();
    let after = serde_json::to_vec(&json!({
        "source": store.get_task_timeline("source").unwrap(),
        "target": store.get_task_timeline("target").unwrap(),
        "sourceEvidence": store.list_evidence("source").unwrap(),
        "targetEvidence": store.list_evidence("target").unwrap(),
        "sourceFiles": store.list_file_changes("source").unwrap(),
        "targetFiles": store.list_file_changes("target").unwrap(),
        "operations": store.list_task_grouping_operations(Some("repo-1")).unwrap(),
    }))
    .unwrap();
    assert_eq!(before, after);

    let undo = inverse(&preview.operation);
    store.append_task_grouping_operation(&undo).unwrap();
    assert_eq!(
        store
            .get_session("session-1")
            .unwrap()
            .unwrap()
            .task_id
            .as_deref(),
        Some("source")
    );
    assert_eq!(
        store.get_fact("fact-moved").unwrap().unwrap().task_id,
        "source"
    );
    assert_eq!(
        store
            .list_task_grouping_operations(Some("repo-1"))
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn concurrent_identical_grouping_claims_return_one_canonical_operation() {
    let (_temp, store, _source, _target) = setup_store();
    let request = TaskGroupingRequestV1 {
        operation_id: "concurrent-move-1".into(),
        action: TaskGroupingActionV1::Move,
        session_ids: vec!["session-1".into()],
        from_task_id: "source".into(),
        target_task_id: Some("target".into()),
        new_task_title: None,
        new_task_goal: None,
    };
    let first = preview(&store, &request).unwrap().operation;
    let mut second = first.clone();
    second.occurred_at += chrono::Duration::microseconds(1);
    assert_ne!(first.occurred_at, second.occurred_at);
    let event_count_before = store.health().unwrap().canonical_event_count;
    let barrier = Arc::new(Barrier::new(2));
    let handles = [first, second]
        .into_iter()
        .map(|operation| {
            let store = store.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                store.claim_task_grouping_operation(&operation).unwrap()
            })
        })
        .collect::<Vec<_>>();
    let outcomes = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, ClaimOutcome::Claimed(_)))
            .count(),
        1
    );
    let canonical = store
        .get_task_grouping_operation(None, "concurrent-move-1")
        .unwrap()
        .unwrap();
    assert!(outcomes.iter().all(|outcome| match outcome {
        ClaimOutcome::Claimed(operation) | ClaimOutcome::Existing(operation) => {
            operation == &canonical
        }
    }));
    assert_eq!(
        store.health().unwrap().canonical_event_count,
        event_count_before + 1
    );

    let mut conflicting = canonical.clone();
    conflicting.request_fingerprint = "different-request".into();
    assert!(store
        .claim_task_grouping_operation(&conflicting)
        .unwrap_err()
        .to_string()
        .contains("different grouping request"));
    let mut tampered = canonical;
    tampered.action = TaskGroupingActionV1::Merge;
    assert!(store
        .claim_task_grouping_operation(&tampered)
        .unwrap_err()
        .to_string()
        .contains("different grouping request"));
}

#[test]
fn grouping_claim_rechecks_target_lifecycle_inside_the_transaction() {
    let (_temp, store, _source, mut target) = setup_store();
    let operation = preview(
        &store,
        &TaskGroupingRequestV1 {
            operation_id: "target-race".into(),
            action: TaskGroupingActionV1::Move,
            session_ids: vec!["session-1".into()],
            from_task_id: "source".into(),
            target_task_id: Some("target".into()),
            new_task_title: None,
            new_task_goal: None,
        },
    )
    .unwrap()
    .operation;
    target.lifecycle = TaskLifecycle::Abandoned;
    target.updated_at = Utc::now();
    store.upsert_task(&target).unwrap();
    assert!(store
        .claim_task_grouping_operation(&operation)
        .unwrap_err()
        .to_string()
        .contains("abandoned"));
    assert!(store
        .get_task_grouping_operation(None, "target-race")
        .unwrap()
        .is_none());
}

#[test]
fn grouping_claim_rechecks_fact_provenance_inside_the_transaction() {
    let (_temp, store, source, _target) = setup_store();
    let operation = preview(
        &store,
        &TaskGroupingRequestV1 {
            operation_id: "provenance-race".into(),
            action: TaskGroupingActionV1::Move,
            session_ids: vec!["session-1".into()],
            from_task_id: "source".into(),
            target_task_id: Some("target".into()),
            new_task_title: None,
            new_task_goal: None,
        },
    )
    .unwrap()
    .operation;
    let now = Utc::now();
    store
        .upsert_fact(&FactV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: "fact-added-after-preview".into(),
            repository_id: source.repository_id.clone(),
            task_id: source.id.clone(),
            kind: FactKind::Note,
            lifecycle: FactLifecycle::Confirmed,
            freshness: Freshness::Fresh,
            origin: Default::default(),
            content: "New provenance".into(),
            evidence_ids: vec!["evidence-added-after-preview".into()],
            superseded_by: None,
            created_at: now,
            updated_at: now,
        })
        .unwrap();
    let mut evidence = EvidenceV1::new(
        "evidence-added-after-preview",
        &source.repository_id,
        &source.id,
        "session-1",
        "fixture",
        "new evidence",
        now,
    );
    evidence.fact_id = Some("fact-added-after-preview".into());
    store.upsert_evidence(&evidence).unwrap();
    assert!(store
        .claim_task_grouping_operation(&operation)
        .unwrap_err()
        .to_string()
        .contains("new grouping preview"));
    assert_eq!(
        store
            .get_session("session-1")
            .unwrap()
            .unwrap()
            .task_id
            .as_deref(),
        Some("source")
    );
}

#[test]
fn split_undo_refuses_to_delete_a_task_with_later_sessions() {
    let (_temp, store, _source, _target) = setup_store();
    let split = preview(
        &store,
        &TaskGroupingRequestV1 {
            operation_id: "split-with-later-session".into(),
            action: TaskGroupingActionV1::Split,
            session_ids: vec!["session-1".into()],
            from_task_id: "source".into(),
            target_task_id: None,
            new_task_title: Some("Split target".into()),
            new_task_goal: None,
        },
    )
    .unwrap();
    let split_task_id = split.operation.created_task.as_ref().unwrap().id.clone();
    store
        .append_task_grouping_operation(&split.operation)
        .unwrap();
    let later_move = preview(
        &store,
        &TaskGroupingRequestV1 {
            operation_id: "move-into-split".into(),
            action: TaskGroupingActionV1::Move,
            session_ids: vec!["session-2".into()],
            from_task_id: "source".into(),
            target_task_id: Some(split_task_id.clone()),
            new_task_title: None,
            new_task_goal: None,
        },
    )
    .unwrap();
    store
        .append_task_grouping_operation(&later_move.operation)
        .unwrap();
    assert!(store
        .claim_task_grouping_operation(&inverse(&split.operation))
        .unwrap_err()
        .to_string()
        .contains("additional sessions"));
    assert!(store.get_task(&split_task_id).unwrap().is_some());
    assert_eq!(
        store
            .get_session("session-2")
            .unwrap()
            .unwrap()
            .task_id
            .as_deref(),
        Some(split_task_id.as_str())
    );
}

#[test]
fn grouping_undo_refuses_to_overwrite_a_changed_task_lifecycle() {
    let (_temp, store, mut source, _target) = setup_store();
    let merge = preview(
        &store,
        &TaskGroupingRequestV1 {
            operation_id: "merge-before-status-change".into(),
            action: TaskGroupingActionV1::Merge,
            session_ids: vec!["session-1".into(), "session-2".into()],
            from_task_id: "source".into(),
            target_task_id: Some("target".into()),
            new_task_title: None,
            new_task_goal: None,
        },
    )
    .unwrap();
    store
        .append_task_grouping_operation(&merge.operation)
        .unwrap();
    source.lifecycle = TaskLifecycle::Abandoned;
    source.updated_at = Utc::now();
    store.upsert_task(&source).unwrap();
    assert!(store
        .claim_task_grouping_operation(&inverse(&merge.operation))
        .unwrap_err()
        .to_string()
        .contains("stale task lifecycle"));
    assert_eq!(
        store.get_task("source").unwrap().unwrap().lifecycle,
        TaskLifecycle::Abandoned
    );
}

#[test]
fn merge_claim_rechecks_that_the_source_will_be_empty() {
    let (_temp, store, _source, _target) = setup_store();
    let operation = preview(
        &store,
        &TaskGroupingRequestV1 {
            operation_id: "merge-before-new-session".into(),
            action: TaskGroupingActionV1::Merge,
            session_ids: vec!["session-1".into(), "session-2".into()],
            from_task_id: "source".into(),
            target_task_id: Some("target".into()),
            new_task_title: None,
            new_task_goal: None,
        },
    )
    .unwrap()
    .operation;
    store
        .upsert_session(&SessionV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: "session-added-after-preview".into(),
            repository_id: "repo-1".into(),
            task_id: Some("source".into()),
            lifecycle: SessionLifecycle::Active,
            started_at: Utc::now(),
            ended_at: None,
            branch: Some("main".into()),
            head: None,
            source_thread_id: None,
            last_activity_at: Some(Utc::now()),
            turn_count: 0,
            compaction_count: 0,
            context_usage: None,
            continuation_state: Default::default(),
            coverage: CoverageV1::default(),
        })
        .unwrap();
    assert!(store
        .claim_task_grouping_operation(&operation)
        .unwrap_err()
        .to_string()
        .contains("stale merge preview"));
    assert_eq!(
        store.get_task("source").unwrap().unwrap().lifecycle,
        TaskLifecycle::Active
    );
}

#[test]
fn merge_completes_empty_source_and_split_undo_removes_created_task() {
    let (_temp, store, _source, _target) = setup_store();
    let merge = preview(
        &store,
        &TaskGroupingRequestV1 {
            operation_id: "merge-1".into(),
            action: TaskGroupingActionV1::Merge,
            session_ids: vec!["session-2".into(), "session-1".into()],
            from_task_id: "source".into(),
            target_task_id: Some("target".into()),
            new_task_title: None,
            new_task_goal: None,
        },
    )
    .unwrap();
    store
        .append_task_grouping_operation(&merge.operation)
        .unwrap();
    assert_eq!(
        store.get_task("source").unwrap().unwrap().lifecycle,
        TaskLifecycle::Completed
    );
    store
        .append_task_grouping_operation(&inverse(&merge.operation))
        .unwrap();
    assert_eq!(
        store.get_task("source").unwrap().unwrap().lifecycle,
        TaskLifecycle::Active
    );

    let split = preview(
        &store,
        &TaskGroupingRequestV1 {
            operation_id: "split-1".into(),
            action: TaskGroupingActionV1::Split,
            session_ids: vec!["session-1".into()],
            from_task_id: "source".into(),
            target_task_id: None,
            new_task_title: Some("New focused task".into()),
            new_task_goal: Some("Keep the focused work isolated".into()),
        },
    )
    .unwrap();
    let created_id = split.operation.created_task.as_ref().unwrap().id.clone();
    store
        .append_task_grouping_operation(&split.operation)
        .unwrap();
    assert_eq!(
        store.get_task(&created_id).unwrap().unwrap().lifecycle,
        TaskLifecycle::Active
    );
    store
        .append_task_grouping_operation(&inverse(&split.operation))
        .unwrap();
    assert!(store.get_task(&created_id).unwrap().is_none());
}

#[test]
fn grouping_preview_rejects_duplicate_missing_stale_invalid_and_cross_repository_inputs() {
    let (_temp, store, source, _target) = setup_store();
    let duplicate = TaskGroupingRequestV1 {
        operation_id: "bad-duplicate".into(),
        action: TaskGroupingActionV1::Move,
        session_ids: vec!["session-1".into(), "session-1".into()],
        from_task_id: source.id.clone(),
        target_task_id: Some("target".into()),
        new_task_title: None,
        new_task_goal: None,
    };
    assert!(preview(&store, &duplicate)
        .unwrap_err()
        .to_string()
        .contains("duplicate"));

    let mut missing = duplicate.clone();
    missing.operation_id = "bad-missing".into();
    missing.session_ids = vec!["missing".into()];
    assert!(preview(&store, &missing)
        .unwrap_err()
        .to_string()
        .contains("not found"));

    let mut stale = duplicate.clone();
    stale.operation_id = "bad-stale".into();
    stale.session_ids = vec!["session-1".into()];
    stale.from_task_id = "target".into();
    stale.target_task_id = Some("source".into());
    assert!(preview(&store, &stale)
        .unwrap_err()
        .to_string()
        .contains("stale"));

    let mut invalid_target = duplicate.clone();
    invalid_target.operation_id = "bad-target".into();
    invalid_target.session_ids = vec!["session-1".into()];
    invalid_target.target_task_id = Some("missing-target".into());
    assert!(preview(&store, &invalid_target)
        .unwrap_err()
        .to_string()
        .contains("not found"));

    let other = task("other-task", "repo-2");
    insert_task(&store, &other, 70);
    let mut cross = duplicate;
    cross.operation_id = "bad-cross".into();
    cross.session_ids = vec!["session-1".into()];
    cross.target_task_id = Some(other.id);
    assert!(preview(&store, &cross)
        .unwrap_err()
        .to_string()
        .contains("cross-repository"));
}

#[test]
fn grouping_request_identity_normalizes_session_ids_and_rejects_spaced_operation_ids() {
    let (_temp, store, _source, _target) = setup_store();
    let request = TaskGroupingRequestV1 {
        operation_id: "normalized-session-id".into(),
        action: TaskGroupingActionV1::Move,
        session_ids: vec!["  session-1  ".into()],
        from_task_id: "source".into(),
        target_task_id: Some("target".into()),
        new_task_title: None,
        new_task_goal: None,
    };
    let result = preview(&store, &request).unwrap();
    assert_eq!(
        result.operation.request_fingerprint,
        request_fingerprint(&request, &request.session_ids)
    );
    let mut spaced_operation_id = request;
    spaced_operation_id.operation_id = " normalized-session-id ".into();
    assert!(preview(&store, &spaced_operation_id)
        .unwrap_err()
        .to_string()
        .contains("operationId"));
}
