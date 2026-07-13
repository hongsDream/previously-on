use chrono::{TimeZone, Utc};
use previously_on::context_pack::{count_pack_tokens, ContextPackBuilder, MAX_TOKEN_BUDGET};
use previously_on::domain::{
    ChangeAttribution, ChangeStatus, CoverageV1, CurrentValidationV1, EvidenceIntegrity,
    EvidenceV1, FactKind, FactLifecycle, FactV1, FileChangeV1, Freshness, TemporalRevalidationV1,
    TemporalStatusV1, MAX_CONTEXT_TEMPORAL_ITEMS, SCHEMA_VERSION_V1,
};

fn at(second: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + second, 0)
        .single()
        .unwrap()
}

fn evidence(id: &str, second: i64) -> EvidenceV1 {
    let mut evidence = EvidenceV1::new(
        id,
        "repo-1",
        "task-1",
        "session-1",
        format!("source-{id}"),
        format!("verified source for {id}"),
        at(second),
    );
    evidence.fact_id = Some(id.replace("evidence", "fact"));
    evidence.integrity = EvidenceIntegrity::Verified;
    evidence
}

fn fact(id: &str, lifecycle: FactLifecycle, freshness: Freshness, second: i64) -> FactV1 {
    FactV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: id.into(),
        repository_id: "repo-1".into(),
        task_id: "task-1".into(),
        kind: FactKind::Decision,
        lifecycle,
        freshness,
        content: format!("Decision {id}: {}", "details ".repeat(80)),
        evidence_ids: vec![id.replace("fact", "evidence")],
        superseded_by: None,
        created_at: at(second),
        updated_at: at(second),
    }
}

#[test]
fn build_is_deterministic_and_excludes_untrusted_or_stale_facts() {
    let facts = vec![
        fact("fact-1", FactLifecycle::Confirmed, Freshness::Fresh, 1),
        fact("fact-stale", FactLifecycle::Pinned, Freshness::Stale, 2),
        fact("fact-broken", FactLifecycle::Pinned, Freshness::Broken, 3),
        fact("fact-invalid", FactLifecycle::Invalid, Freshness::Fresh, 4),
        fact(
            "fact-candidate",
            FactLifecycle::Candidate,
            Freshness::Fresh,
            5,
        ),
    ];
    let evidence = vec![
        evidence("evidence-1", 1),
        evidence("evidence-stale", 2),
        evidence("evidence-broken", 3),
        evidence("evidence-invalid", 4),
        evidence("evidence-candidate", 5),
    ];
    let first = ContextPackBuilder::new("repo-1", "task-1")
        .generated_at(at(10))
        .build(
            Some("Continue auth work".into()),
            facts.clone(),
            evidence.clone(),
            vec![],
            vec![],
            CoverageV1::default(),
        )
        .unwrap();
    let second = ContextPackBuilder::new("repo-1", "task-1")
        .generated_at(at(10))
        .build(
            Some("Continue auth work".into()),
            facts.into_iter().rev().collect(),
            evidence.into_iter().rev().collect(),
            vec![],
            vec![],
            CoverageV1::default(),
        )
        .unwrap();
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    assert_eq!(
        first
            .facts
            .iter()
            .map(|fact| fact.id.as_str())
            .collect::<Vec<_>>(),
        ["fact-1"]
    );
    assert_eq!(first.token_count, count_pack_tokens(&first).unwrap());
}

#[test]
fn drops_whole_low_ranked_items_to_respect_hard_envelope_cap() {
    let facts = (0..12)
        .map(|index| {
            fact(
                &format!("fact-{index}"),
                FactLifecycle::Confirmed,
                Freshness::Fresh,
                index,
            )
        })
        .collect::<Vec<_>>();
    let evidence = (0..12)
        .map(|index| evidence(&format!("evidence-{index}"), index))
        .collect::<Vec<_>>();
    let pack = ContextPackBuilder::new("repo-1", "task-1")
        .token_budget(550)
        .generated_at(at(20))
        .build(None, facts, evidence, vec![], vec![], CoverageV1::default())
        .unwrap();
    assert!(pack.token_count <= 550);
    assert!(pack.token_count <= MAX_TOKEN_BUDGET);
    assert!(pack.facts.len() < 5);
    assert_eq!(pack.token_count, count_pack_tokens(&pack).unwrap());
}

#[test]
fn temporal_metadata_is_deterministically_bounded_with_explicit_omission_counts() {
    let changes = (0..100)
        .rev()
        .map(|index| FileChangeV1 {
            schema_version: SCHEMA_VERSION_V1,
            repository_id: "repo-1".into(),
            session_id: "session-1".into(),
            task_id: Some("task-1".into()),
            path: format!("src/file-{index:03}.rs"),
            previous_path: None,
            status: ChangeStatus::Modified,
            additions: Some(1),
            deletions: Some(1),
            attribution: ChangeAttribution::ObservedChangedIn,
            before_head: Some("before".into()),
            after_head: Some("after".into()),
        })
        .collect::<Vec<_>>();
    let paths = (0..100)
        .rev()
        .map(|index| format!("src/file-{index:03}.rs"))
        .collect::<Vec<_>>();
    let temporal = TemporalRevalidationV1 {
        schema_version: SCHEMA_VERSION_V1,
        status: TemporalStatusV1::Changed,
        baseline_head: Some("before".into()),
        current_head: Some("after".into()),
        merge_base: Some("before".into()),
        related_changes: changes,
        checked_paths: paths.clone(),
        warnings: Vec::new(),
    };
    let current = CurrentValidationV1 {
        schema_version: SCHEMA_VERSION_V1,
        status: TemporalStatusV1::Changed,
        current_head: Some("after".into()),
        verified_paths: paths,
        warnings: Vec::new(),
    };

    let pack = ContextPackBuilder::new("repo-1", "task-1")
        .generated_at(at(30))
        .temporal_revalidation(temporal)
        .current_validation(current)
        .build(None, vec![], vec![], vec![], vec![], CoverageV1::default())
        .unwrap();
    let temporal = pack.temporal_revalidation.as_ref().unwrap();
    assert_eq!(temporal.checked_paths.len(), MAX_CONTEXT_TEMPORAL_ITEMS);
    assert_eq!(temporal.related_changes.len(), MAX_CONTEXT_TEMPORAL_ITEMS);
    assert_eq!(temporal.checked_paths[0], "src/file-000.rs");
    assert_eq!(temporal.related_changes[0].path, "src/file-000.rs");
    assert!(temporal
        .warnings
        .contains(&"checked_paths_omitted_count=92; limit=8".to_string()));
    assert!(temporal
        .warnings
        .contains(&"related_changes_omitted_count=92; limit=8".to_string()));

    let current = pack.current_validation.as_ref().unwrap();
    assert_eq!(current.status, TemporalStatusV1::Changed);
    assert_eq!(current.current_head, None);
    assert!(current.verified_paths.is_empty());
    assert!(current.warnings.is_empty());
    assert_eq!(pack.token_count, count_pack_tokens(&pack).unwrap());
}
