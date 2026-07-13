use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Matrix {
    schema_version: u16,
    required_per_category: usize,
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Scenario {
    id: String,
    category: String,
    test_target: String,
    test_filter: String,
    expectation: String,
}

fn target_source(target: &str) -> Option<&'static str> {
    match target {
        "core_store" => Some(include_str!("core_store.rs")),
        "integration_app_server" => Some(include_str!("integration_app_server.rs")),
        "integration_ingestion" => Some(include_str!("integration_ingestion.rs")),
        "integration_hook" => Some(include_str!("integration_hook.rs")),
        "core_git" => Some(include_str!("core_git.rs")),
        "integration_setup" => Some(include_str!("integration_setup.rs")),
        "core_redaction" => Some(include_str!("core_redaction.rs")),
        _ => None,
    }
}

#[test]
fn compatibility_fixture_declares_thirty_executable_scenarios() {
    let matrix: Matrix =
        serde_json::from_str(include_str!("../fixtures/compatibility/scenarios.json")).unwrap();
    assert_eq!(matrix.schema_version, 1);
    assert_eq!(matrix.scenarios.len(), 30);

    let mut ids = BTreeSet::new();
    let mut category_counts = BTreeMap::<String, usize>::new();
    for scenario in &matrix.scenarios {
        assert!(
            ids.insert(&scenario.id),
            "duplicate scenario {}",
            scenario.id
        );
        assert!(!scenario.expectation.trim().is_empty());
        *category_counts
            .entry(scenario.category.clone())
            .or_default() += 1;

        let source = target_source(&scenario.test_target)
            .unwrap_or_else(|| panic!("unknown test target {}", scenario.test_target));
        let sync_signature = format!("fn {}(", scenario.test_filter);
        let async_signature = format!("async fn {}(", scenario.test_filter);
        assert!(
            source.contains(&sync_signature) || source.contains(&async_signature),
            "scenario {} references missing test {}::{}",
            scenario.id,
            scenario.test_target,
            scenario.test_filter
        );
    }

    assert_eq!(category_counts.len(), 5);
    for category in [
        "lifecycle",
        "event_reconstruction",
        "concurrency_git",
        "app_server",
        "setup_privacy_recovery",
    ] {
        assert_eq!(
            category_counts.get(category),
            Some(&matrix.required_per_category),
            "category {category} must contain exactly six scenarios"
        );
    }
}
