use chrono::{TimeZone, Utc};
use previously_on::contracts::{
    ContractEvaluationV1, ContractOriginV1, ContractReadinessV1, ContractStatusV1,
    ImpactPathSelectorV1, ImpactSelectorGroupV1, PathSelectorKindV1, RegressionContractV1,
    RelevantContractV1, RequiredTestV1,
};
use previously_on::domain::{
    deterministic_id, ChangeAttribution, ChangeStatus, CheckpointV1, CoverageV1, EventEnvelopeV1,
    EventKind, FileChangeV1, GitSnapshotV1, GraphEdgeKindV1, GraphSourceKindV1, SessionLifecycle,
    SessionV1, TaskLifecycle, TaskV1, SCHEMA_VERSION_V1,
};
use previously_on::graph::{compact_summary, derive_relationship_graph};
use previously_on::store::Store;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use tempfile::TempDir;

fn at(second: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_720_000_000 + second, 0)
        .single()
        .unwrap()
}

fn insert(
    store: &Store,
    session_id: &str,
    task_id: Option<&str>,
    kind: EventKind,
    second: i64,
    payload: serde_json::Value,
) {
    let mut event = EventEnvelopeV1::new(
        format!("source-{session_id}-{second}"),
        "repo-graph",
        session_id,
        kind,
        at(second),
        payload,
    );
    event.task_id = task_id.map(str::to_string);
    event.received_at = at(second);
    store.insert_event(&event).unwrap();
}

#[test]
fn graph_is_deterministic_provenanced_redacted_and_never_infers_edges() {
    let temp = TempDir::new().unwrap();
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    let task = TaskV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: "task-graph".into(),
        repository_id: "repo-graph".into(),
        title: "Graph truth".into(),
        goal: None,
        lifecycle: TaskLifecycle::Active,
        branch: Some("main".into()),
        created_at: at(1),
        updated_at: at(1),
    };
    insert(
        &store,
        "local-ui",
        Some(&task.id),
        EventKind::TaskUpdated,
        1,
        json!({ "task": task }),
    );
    let session = SessionV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: "session-graph".into(),
        repository_id: "repo-graph".into(),
        task_id: Some("task-graph".into()),
        lifecycle: SessionLifecycle::Active,
        started_at: at(2),
        ended_at: None,
        branch: Some("main".into()),
        head: Some("0123456789abcdef".into()),
        source_thread_id: Some("thread-graph".into()),
        last_activity_at: Some(at(3)),
        turn_count: 1,
        compaction_count: 0,
        context_usage: None,
        continuation_state: Default::default(),
        coverage: CoverageV1::default(),
    };
    insert(
        &store,
        &session.id,
        Some("task-graph"),
        EventKind::SessionStarted,
        2,
        json!({ "task": task, "session": session }),
    );
    let changed = FileChangeV1 {
        schema_version: SCHEMA_VERSION_V1,
        repository_id: "repo-graph".into(),
        session_id: "session-graph".into(),
        task_id: Some("task-graph".into()),
        path: "src/unrelated_verify_auth.rs".into(),
        previous_path: None,
        status: ChangeStatus::Modified,
        additions: Some(1),
        deletions: Some(0),
        attribution: ChangeAttribution::ObservedChangedIn,
        before_head: None,
        after_head: Some("0123456789abcdef".into()),
    };
    let checkpoint = CheckpointV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: "checkpoint-graph".into(),
        repository_id: "repo-graph".into(),
        task_id: "task-graph".into(),
        session_id: "session-graph".into(),
        created_at: at(3),
        goal_hint: None,
        git_before: None,
        git_after: GitSnapshotV1 {
            schema_version: SCHEMA_VERSION_V1,
            repository_id: "repo-graph".into(),
            root: "/tmp/repo".into(),
            remote_url: None,
            branch: Some("main".into()),
            head: Some("0123456789abcdef".into()),
            captured_at: at(3),
            dirty_files: Vec::new(),
            working_tree_changes: vec![changed.clone()],
            content_fingerprints: BTreeMap::new(),
        },
        changed_files: vec![changed],
        tests: Vec::new(),
        failures: Vec::new(),
        unresolved_items: Vec::new(),
        coverage: CoverageV1::default(),
    };
    insert(
        &store,
        "session-graph",
        Some("task-graph"),
        EventKind::Checkpoint,
        3,
        json!({ "checkpoint": checkpoint.clone() }),
    );
    let mut repeated_checkpoint = checkpoint;
    repeated_checkpoint.id = "checkpoint-graph-repeat".into();
    repeated_checkpoint.created_at = at(6);
    repeated_checkpoint.git_after.captured_at = at(6);
    insert(
        &store,
        "session-graph",
        Some("task-graph"),
        EventKind::Checkpoint,
        6,
        json!({ "checkpoint": repeated_checkpoint }),
    );

    let secret = "contract-secret-12345";
    let contract = RegressionContractV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: "contract-auth".into(),
        title: format!("password={secret}"),
        invariant: "Keep authentication stable".into(),
        status: ContractStatusV1::Active,
        superseded_by: None,
        impact_selectors: vec![ImpactSelectorGroupV1 {
            path: ImpactPathSelectorV1 {
                kind: PathSelectorKindV1::Prefix,
                value: "src/auth/".into(),
            },
            symbols: vec!["verify_auth".into()],
        }],
        required_tests: vec![RequiredTestV1 {
            id: "auth-test".into(),
            name: "Authentication test".into(),
            program: "cargo".into(),
            args: vec!["test".into(), "auth".into()],
            working_directory: ".".into(),
            timeout_seconds: 60,
        }],
        origin: ContractOriginV1 {
            fixed_at_commit: "fedcba9876543210".into(),
            recorded_at: at(4),
            evidence_sha256: hex::encode(Sha256::digest(b"contract evidence")),
        },
    };
    let evaluation = ContractEvaluationV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: "evaluation-graph".into(),
        repository_id: "repo-graph".into(),
        task_id: Some("task-graph".into()),
        readiness: ContractReadinessV1::Ready,
        evaluated_at: at(5),
        relevant_contracts: vec![RelevantContractV1 {
            id: contract.id.clone(),
            title: "Authentication contract".into(),
            invariant: "Keep authentication stable".into(),
            match_reasons: vec!["explicit evaluation".into()],
        }],
        required_tests: Vec::new(),
        warnings: Vec::new(),
        content_fingerprint: hex::encode(Sha256::digest(b"content")),
        continuation_issued: false,
        base: None,
        head: None,
        merge_base: None,
    };
    insert(
        &store,
        "contract-evaluation",
        Some("task-graph"),
        EventKind::ContractEvaluationRecorded,
        5,
        json!({ "contractEvaluation": evaluation }),
    );

    let other_task = TaskV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: "task-other".into(),
        repository_id: "repo-graph".into(),
        title: "Other task".into(),
        goal: None,
        lifecycle: TaskLifecycle::Active,
        branch: Some("main".into()),
        created_at: at(7),
        updated_at: at(7),
    };
    insert(
        &store,
        "other-ui",
        Some(&other_task.id),
        EventKind::TaskUpdated,
        7,
        json!({ "task": other_task }),
    );

    let all =
        derive_relationship_graph(&store, "repo-graph", None, std::slice::from_ref(&contract))
            .unwrap();
    let first = derive_relationship_graph(
        &store,
        "repo-graph",
        Some("task-graph"),
        std::slice::from_ref(&contract),
    )
    .unwrap();
    let second = derive_relationship_graph(
        &store,
        "repo-graph",
        Some("task-graph"),
        std::slice::from_ref(&contract),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    assert!(first.edges.iter().all(|edge| {
        edge.verified && !edge.provenance_ids.is_empty() && edge.observed_at <= at(6)
    }));
    assert!(first.edges.iter().any(|edge| {
        edge.kind == GraphEdgeKindV1::TaskRelevantContract
            && edge.source_kind == GraphSourceKindV1::ContractEvaluation
    }));
    assert!(first.edges.iter().any(|edge| {
        edge.kind == GraphEdgeKindV1::ContractDeclaresSymbol
            && edge.source_kind == GraphSourceKindV1::RegressionContract
    }));
    let contract_node = deterministic_id("contract", &["contract-auth"]);
    let observed_file = deterministic_id("file", &["src/unrelated_verify_auth.rs"]);
    let session_node = deterministic_id("session", &["session-graph"]);
    let repeated_file_edges = first
        .edges
        .iter()
        .filter(|edge| {
            edge.kind == GraphEdgeKindV1::SessionChangedFile
                && edge.from == session_node
                && edge.to == observed_file
                && edge.source_kind == GraphSourceKindV1::Projection
        })
        .collect::<Vec<_>>();
    assert_eq!(repeated_file_edges.len(), 1);
    assert_eq!(
        repeated_file_edges[0].provenance_ids,
        vec!["checkpoint-graph", "checkpoint-graph-repeat"]
    );
    assert_eq!(repeated_file_edges[0].observed_at, at(6));
    assert_eq!(
        repeated_file_edges[0].id,
        deterministic_id(
            "edge",
            &[
                "session-changed-file",
                &session_node,
                &observed_file,
                "projection"
            ]
        )
    );
    assert!(all
        .edges
        .iter()
        .any(|edge| edge.id == repeated_file_edges[0].id));
    assert!(!first
        .nodes
        .iter()
        .any(|node| node.task_id.as_deref() == Some("task-other")));
    let node_ids = first
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(first.edges.iter().all(|edge| {
        node_ids.contains(edge.from.as_str()) && node_ids.contains(edge.to.as_str())
    }));
    assert!(!first
        .edges
        .iter()
        .any(|edge| { edge.from == contract_node && edge.to == observed_file }));
    let serialized = serde_json::to_string(&first).unwrap();
    assert!(!serialized.contains(secret));
    let summary = compact_summary(&first);
    assert_eq!(summary.node_count, first.nodes.len());
    assert_eq!(summary.edge_count, first.edges.len());
    assert_eq!(summary.verified_edge_count, first.edges.len());
}
