use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

use crate::contracts::{ContractEvaluationV1, ContractReadinessV1, RequiredTestStateV1};
use crate::domain::{
    CheckpointV1, CoverageStatus, EventEnvelopeV1, EventKind, SessionV1, SCHEMA_VERSION_V1,
};

#[derive(Debug, Clone)]
pub struct DiagnosticsInputV1 {
    pub app_version: String,
    pub codex_version: Option<String>,
    pub os: String,
    pub arch: String,
    pub setup_at: DateTime<Utc>,
    pub sessions: Vec<SessionV1>,
    pub checkpoints: Vec<CheckpointV1>,
    pub events: Vec<EventEnvelopeV1>,
    pub contract_evaluations: Vec<ContractEvaluationV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PilotDiagnosticsV1 {
    pub schema_version: u16,
    pub app_version: String,
    pub codex_version: Option<String>,
    pub os: String,
    pub arch: String,
    pub setup_to_first_checkpoint_seconds: Option<u64>,
    pub counts: DiagnosticCountsV1,
    pub continuations: ContinuationCountsV1,
    pub contracts: ContractCountsV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticCountsV1 {
    pub sessions: u64,
    pub checkpoints: u64,
    pub coverage: CoverageCountsV1,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageCountsV1 {
    pub complete: u64,
    pub degraded: u64,
    pub unsupported: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContinuationCountsV1 {
    pub suggested: u64,
    pub started: u64,
    pub failed: u64,
    pub pending: u64,
    pub open: u64,
    pub unknown: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractCountsV1 {
    pub readiness: ContractReadinessCountsV1,
    pub tests: ContractTestCountsV1,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractReadinessCountsV1 {
    pub ready: u64,
    pub contract_blocked: u64,
    pub unknown: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractTestCountsV1 {
    pub passed: u64,
    pub failed: u64,
    pub missing: u64,
    pub stale: u64,
    pub unknown: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperationState {
    Started,
    Failed,
    Pending,
}

pub fn build_diagnostics(input: DiagnosticsInputV1) -> PilotDiagnosticsV1 {
    let mut coverage = CoverageCountsV1::default();
    for status in input
        .sessions
        .iter()
        .map(|session| session.coverage.status)
        .chain(
            input
                .checkpoints
                .iter()
                .map(|checkpoint| checkpoint.coverage.status),
        )
    {
        match status {
            CoverageStatus::Complete => coverage.complete += 1,
            CoverageStatus::Degraded => coverage.degraded += 1,
            CoverageStatus::Unsupported => coverage.unsupported += 1,
        }
    }

    let setup_to_first_checkpoint_seconds = input
        .checkpoints
        .iter()
        .filter(|checkpoint| checkpoint.created_at >= input.setup_at)
        .map(|checkpoint| (checkpoint.created_at - input.setup_at).num_seconds())
        .min()
        .and_then(|seconds| u64::try_from(seconds).ok());

    let continuations = continuation_counts(&input.events);
    let contracts = contract_counts(&input.contract_evaluations);

    PilotDiagnosticsV1 {
        schema_version: SCHEMA_VERSION_V1,
        app_version: normalized_version(&input.app_version).unwrap_or_else(|| "unknown".into()),
        codex_version: input.codex_version.as_deref().and_then(normalized_version),
        os: normalized_platform(&input.os),
        arch: normalized_platform(&input.arch),
        setup_to_first_checkpoint_seconds,
        counts: DiagnosticCountsV1 {
            sessions: input.sessions.len() as u64,
            checkpoints: input.checkpoints.len() as u64,
            coverage,
        },
        continuations,
        contracts,
    }
}

fn continuation_counts(events: &[EventEnvelopeV1]) -> ContinuationCountsV1 {
    let mut suggested_sessions = BTreeSet::new();
    let mut operation_sessions = BTreeSet::new();
    let mut operations = BTreeMap::<String, OperationState>::new();
    let mut unknown = 0_u64;
    let mut ordered = events.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        (left.occurred_at, &left.event_id).cmp(&(right.occurred_at, &right.event_id))
    });

    for event in ordered {
        if event.kind == EventKind::ContinuationSuggested
            && event.payload.get("delivery_state").and_then(Value::as_str) != Some("pending_replay")
        {
            suggested_sessions.insert(event.session_id.clone());
        }
        if event.kind != EventKind::ContinuationStarted {
            continue;
        }
        let Some(operation_id) = event.payload.get("operation_id").and_then(Value::as_str) else {
            unknown += 1;
            continue;
        };
        if operation_id.is_empty() {
            unknown += 1;
            continue;
        }
        let state = match event.payload.get("status").and_then(Value::as_str) {
            Some("started") => OperationState::Started,
            Some("failed") => OperationState::Failed,
            Some("pending" | "thread_created") => OperationState::Pending,
            _ => {
                unknown += 1;
                continue;
            }
        };
        operation_sessions.insert(event.session_id.clone());
        operations.insert(operation_id.to_string(), state);
    }

    let mut result = ContinuationCountsV1 {
        suggested: suggested_sessions.len() as u64,
        open: suggested_sessions.difference(&operation_sessions).count() as u64,
        unknown,
        ..ContinuationCountsV1::default()
    };
    for state in operations.into_values() {
        match state {
            OperationState::Started => result.started += 1,
            OperationState::Failed => result.failed += 1,
            OperationState::Pending => result.pending += 1,
        }
    }
    result
}

fn contract_counts(evaluations: &[ContractEvaluationV1]) -> ContractCountsV1 {
    let mut result = ContractCountsV1::default();
    for evaluation in evaluations {
        match evaluation.readiness {
            ContractReadinessV1::Ready => result.readiness.ready += 1,
            ContractReadinessV1::ContractBlocked => result.readiness.contract_blocked += 1,
        }
        for test in &evaluation.required_tests {
            match test.state {
                RequiredTestStateV1::Passed => result.tests.passed += 1,
                RequiredTestStateV1::Failed => result.tests.failed += 1,
                RequiredTestStateV1::Missing => result.tests.missing += 1,
                RequiredTestStateV1::Stale => result.tests.stale += 1,
            }
        }
    }
    result
}

fn normalized_version(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+')))
    .then(|| value.to_string())
}

fn normalized_platform(value: &str) -> String {
    let value = value.trim();
    if !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        value.to_string()
    } else {
        "unknown".to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use chrono::{Duration, TimeZone};
    use serde_json::{json, Value};

    use super::*;
    use crate::contracts::{RelevantContractV1, RequiredTestEvaluationV1};
    use crate::domain::{CheckpointV1, CoverageV1, GitSnapshotV1, SessionLifecycle};

    const SECRET: &str = "sk-secret-must-not-leak";
    const PATH: &str = "/Users/example/private/repository";
    const PRIVATE_ID: &str = "thread-private-id-123";

    #[test]
    fn zero_success_failure_and_unknown_fixtures_are_stable() {
        for (name, input) in [
            ("zero", fixture_input(FixtureState::Zero)),
            ("success", fixture_input(FixtureState::Success)),
            ("failure", fixture_input(FixtureState::Failure)),
            ("unknown", fixture_input(FixtureState::Unknown)),
        ] {
            let actual = serde_json::to_value(build_diagnostics(input)).unwrap();
            let expected: Value = serde_json::from_str(match name {
                "zero" => include_str!("../fixtures/diagnostics/zero.json"),
                "success" => include_str!("../fixtures/diagnostics/success.json"),
                "failure" => include_str!("../fixtures/diagnostics/failure.json"),
                _ => include_str!("../fixtures/diagnostics/unknown.json"),
            })
            .unwrap();
            assert_eq!(actual, expected, "fixture {name}");
        }
    }

    #[test]
    fn serialized_report_is_allowlisted_and_drops_private_inputs() {
        let value =
            serde_json::to_value(build_diagnostics(fixture_input(FixtureState::Failure))).unwrap();
        let serialized = serde_json::to_string(&value).unwrap();
        for private in [SECRET, PATH, PRIVATE_ID, "state.rs", "cargo test"] {
            assert!(!serialized.contains(private));
        }
        assert_allowlisted_keys(&value);
    }

    fn assert_allowlisted_keys(value: &Value) {
        let allowed = BTreeSet::from([
            "schemaVersion",
            "appVersion",
            "codexVersion",
            "os",
            "arch",
            "setupToFirstCheckpointSeconds",
            "counts",
            "sessions",
            "checkpoints",
            "coverage",
            "complete",
            "degraded",
            "unsupported",
            "continuations",
            "suggested",
            "started",
            "failed",
            "pending",
            "open",
            "unknown",
            "contracts",
            "readiness",
            "ready",
            "contractBlocked",
            "tests",
            "passed",
            "missing",
            "stale",
        ]);
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    assert!(allowed.contains(key.as_str()), "unexpected key {key}");
                    assert_allowlisted_keys(child);
                }
            }
            Value::Array(items) => {
                for item in items {
                    assert_allowlisted_keys(item);
                }
            }
            _ => {}
        }
    }

    #[derive(Clone, Copy)]
    enum FixtureState {
        Zero,
        Success,
        Failure,
        Unknown,
    }

    fn fixture_input(state: FixtureState) -> DiagnosticsInputV1 {
        let setup_at = Utc.with_ymd_and_hms(2026, 7, 20, 0, 0, 0).unwrap();
        let mut sessions = Vec::new();
        let mut checkpoints = Vec::new();
        let mut events = Vec::new();
        let mut contract_evaluations = Vec::new();
        let (codex_version, status) = match state {
            FixtureState::Zero => (Some("0.144.3".to_string()), None),
            FixtureState::Success => (Some("0.144.3".to_string()), Some(CoverageStatus::Complete)),
            FixtureState::Failure => (
                Some(format!("0.144.3-{SECRET}/{PATH}")),
                Some(CoverageStatus::Degraded),
            ),
            FixtureState::Unknown => (None, Some(CoverageStatus::Unsupported)),
        };
        if let Some(status) = status {
            sessions.push(session(status, setup_at));
            checkpoints.push(checkpoint(status, setup_at + Duration::seconds(12)));
        }
        match state {
            FixtureState::Zero => {}
            FixtureState::Success => {
                events.push(suggestion(setup_at));
                events.push(operation(setup_at + Duration::seconds(1), "started"));
                contract_evaluations.push(evaluation(
                    setup_at,
                    ContractReadinessV1::Ready,
                    RequiredTestStateV1::Passed,
                ));
            }
            FixtureState::Failure => {
                events.push(suggestion(setup_at));
                events.push(operation(setup_at + Duration::seconds(1), "failed"));
                contract_evaluations.push(evaluation(
                    setup_at,
                    ContractReadinessV1::ContractBlocked,
                    RequiredTestStateV1::Failed,
                ));
            }
            FixtureState::Unknown => {
                let mut event = operation(setup_at, "future-status");
                event.payload["operation_id"] = Value::Null;
                events.push(event);
            }
        }
        DiagnosticsInputV1 {
            app_version: "0.1.0-alpha.3".into(),
            codex_version,
            os: "macos".into(),
            arch: "aarch64".into(),
            setup_at,
            sessions,
            checkpoints,
            events,
            contract_evaluations,
        }
    }

    fn coverage(status: CoverageStatus) -> CoverageV1 {
        CoverageV1 {
            schema_version: SCHEMA_VERSION_V1,
            status,
            captured: vec![SECRET.into(), PATH.into()],
            missing: vec!["state.rs".into()],
            warnings: vec!["cargo test".into()],
        }
    }

    fn session(status: CoverageStatus, now: DateTime<Utc>) -> SessionV1 {
        SessionV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: PRIVATE_ID.into(),
            repository_id: SECRET.into(),
            task_id: Some("task-private".into()),
            lifecycle: SessionLifecycle::Active,
            started_at: now,
            ended_at: None,
            branch: Some(SECRET.into()),
            head: Some(PRIVATE_ID.into()),
            source_thread_id: Some(PRIVATE_ID.into()),
            last_activity_at: Some(now),
            turn_count: 1,
            compaction_count: 0,
            context_usage: None,
            continuation_state: Default::default(),
            coverage: coverage(status),
        }
    }

    fn checkpoint(status: CoverageStatus, now: DateTime<Utc>) -> CheckpointV1 {
        CheckpointV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: PRIVATE_ID.into(),
            repository_id: SECRET.into(),
            task_id: "task-private".into(),
            session_id: PRIVATE_ID.into(),
            created_at: now,
            goal_hint: Some(SECRET.into()),
            git_before: None,
            git_after: GitSnapshotV1 {
                schema_version: SCHEMA_VERSION_V1,
                repository_id: SECRET.into(),
                root: PATH.into(),
                remote_url: Some(SECRET.into()),
                branch: Some(SECRET.into()),
                head: Some(PRIVATE_ID.into()),
                captured_at: now,
                dirty_files: vec!["state.rs".into()],
                working_tree_changes: Vec::new(),
                content_fingerprints: BTreeMap::new(),
            },
            changed_files: Vec::new(),
            tests: Vec::new(),
            failures: vec![SECRET.into()],
            unresolved_items: vec![PATH.into()],
            coverage: coverage(status),
        }
    }

    fn suggestion(now: DateTime<Utc>) -> EventEnvelopeV1 {
        event(
            EventKind::ContinuationSuggested,
            now,
            json!({"delivery_state": "claimed", "prompt": SECRET}),
        )
    }

    fn operation(now: DateTime<Utc>, status: &str) -> EventEnvelopeV1 {
        event(
            EventKind::ContinuationStarted,
            now,
            json!({"operation_id": PRIVATE_ID, "status": status, "message": SECRET}),
        )
    }

    fn event(kind: EventKind, now: DateTime<Utc>, payload: Value) -> EventEnvelopeV1 {
        let mut event = EventEnvelopeV1::new(PRIVATE_ID, SECRET, PRIVATE_ID, kind, now, payload);
        event.event_id = format!("{PRIVATE_ID}-{}", now.timestamp());
        event.dedupe_key = event.event_id.clone();
        event
    }

    fn evaluation(
        now: DateTime<Utc>,
        readiness: ContractReadinessV1,
        state: RequiredTestStateV1,
    ) -> ContractEvaluationV1 {
        ContractEvaluationV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: PRIVATE_ID.into(),
            repository_id: SECRET.into(),
            task_id: Some("task-private".into()),
            readiness,
            evaluated_at: now,
            relevant_contracts: vec![RelevantContractV1 {
                id: PRIVATE_ID.into(),
                title: SECRET.into(),
                invariant: PATH.into(),
                match_reasons: vec!["state.rs".into()],
            }],
            required_tests: vec![RequiredTestEvaluationV1 {
                contract_id: PRIVATE_ID.into(),
                test_id: PRIVATE_ID.into(),
                name: SECRET.into(),
                program: "cargo".into(),
                args: vec!["test".into()],
                working_directory: PATH.into(),
                timeout_seconds: 60,
                state,
                detail: Some(SECRET.into()),
            }],
            warnings: vec![SECRET.into()],
            content_fingerprint: PRIVATE_ID.into(),
            continuation_issued: false,
            base: Some(PRIVATE_ID.into()),
            head: Some(PRIVATE_ID.into()),
            merge_base: Some(PRIVATE_ID.into()),
        }
    }
}
