use std::io::Cursor;
use std::process::Command;
use std::sync::{Arc, Barrier};

use chrono::Utc;
use previously_on::contracts::{
    CandidateEvidenceKindV1, ContractOriginV1, ContractReadinessV1, ContractStatusV1,
    ImpactPathSelectorV1, ImpactSelectorGroupV1, PathSelectorKindV1, RegressionContractV1,
    RequiredTestStateV1, RequiredTestV1,
};
use previously_on::domain::{
    ChangeAttribution, ContinuationReasonV1, ContinuationStateV1, EventEnvelopeV1, EventKind,
    TaskLifecycle, TaskV1, TestStatus, SCHEMA_VERSION_V1,
};
use previously_on::hook::{
    append_fallback, capture, ingest_hook_event, replay_fallback, HookAckV1, HookDeliveryStatus,
    HookEvent,
};
use previously_on::store::Store;
use serde_json::json;
use tempfile::TempDir;

fn git(repo: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

fn captured(event: HookEvent, value: serde_json::Value) -> previously_on::domain::EventEnvelopeV1 {
    let mut bytes = Cursor::new(serde_json::to_vec(&value).unwrap());
    capture(event, &mut bytes).unwrap()
}

#[test]
fn codex_hook_flow_creates_task_git_checkpoint_and_test_evidence() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    git(temp.path(), &["init", repo.to_str().unwrap()]);
    std::fs::write(repo.join("src/auth.ts"), "export const mode = 'legacy';\n").unwrap();
    git(&repo, &["add", "src/auth.ts"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Lineage Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "fixture",
        ],
    );

    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    let cwd = repo.to_string_lossy();
    let base = |name: &str| {
        json!({
            "session_id": "session-auth",
            "turn_id": "turn_001",
            "cwd": cwd,
            "hook_event_name": name
        })
    };

    ingest_hook_event(
        &store,
        captured(HookEvent::SessionStart, base("SessionStart")),
    )
    .unwrap();

    let mut prompt = base("UserPromptSubmit");
    prompt["prompt"] = json!(
        "Refactor the authentication boundary without changing behavior. password=never-store-this\nConstraint: Keep handlers stable"
    );
    let ack = ingest_hook_event(&store, captured(HookEvent::UserPromptSubmit, prompt)).unwrap();
    assert!(ack.candidate.is_none());
    assert_eq!(ack.status, HookDeliveryStatus::Persisted);

    let mut patch = base("PreToolUse");
    patch["tool_name"] = json!("apply_patch");
    patch["tool_use_id"] = json!("call-patch");
    patch["tool_input"] = json!({
        "command": "*** Begin Patch\n*** Update File: src/auth.ts\n*** End Patch"
    });
    ingest_hook_event(&store, captured(HookEvent::PreToolUse, patch.clone())).unwrap();
    std::fs::write(
        repo.join("src/auth.ts"),
        "export const mode = 'middleware';\n",
    )
    .unwrap();
    patch["hook_event_name"] = json!("PostToolUse");
    patch["tool_response"] = json!({"content":"Done. Authorization: Bearer hidden-token"});
    ingest_hook_event(&store, captured(HookEvent::PostToolUse, patch)).unwrap();

    let mut test = base("PostToolUse");
    test["tool_name"] = json!("Bash");
    test["tool_use_id"] = json!("call-test");
    test["tool_input"] = json!({"command":"npm test -- auth"});
    test["tool_response"] = json!({"exit_code":0,"output":"18 passed, 0 failed"});
    ingest_hook_event(&store, captured(HookEvent::PostToolUse, test)).unwrap();

    let mut stop = base("Stop");
    stop["last_assistant_message"] = json!("Authentication boundary moved; tests pass.");
    ingest_hook_event(&store, captured(HookEvent::Stop, stop)).unwrap();

    let tasks = store.search_tasks("", 10).unwrap();
    assert_eq!(tasks.len(), 1);
    let task_id = &tasks[0].task.id;
    let checkpoints = store.list_checkpoints(task_id).unwrap();
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(
        checkpoints[0].goal_hint.as_deref(),
        Some("Refactor the authentication boundary without changing behavior. password=[REDACTED]")
    );

    let changes = store.list_file_changes(task_id).unwrap();
    assert!(changes.iter().any(|change| {
        change.path == "src/auth.ts" && change.attribution == ChangeAttribution::ModifiedBy
    }));
    let tests = store.list_test_results(task_id).unwrap();
    assert!(tests.iter().any(|test| test.status == TestStatus::Passed));
    let facts = store.list_facts(task_id).unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].content, "Keep handlers stable");
    assert_eq!(store.list_evidence(task_id).unwrap().len(), 1);

    let export = store.export_json(None).unwrap().to_string();
    assert!(!export.contains("never-store-this"));
    assert!(!export.contains("hidden-token"));
}

#[test]
fn resume_candidate_is_evaluated_only_on_the_first_prompt() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(temp.path(), &["init", repo.to_str().unwrap()]);
    let repository_id = previously_on::git::repository_identity(&repo).unwrap().id;
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    let now = Utc::now();
    store
        .upsert_task(&TaskV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: "previous-auth-task".into(),
            repository_id: repository_id.clone(),
            title: "Refactor authentication boundary".into(),
            goal: Some("Refactor authentication boundary".into()),
            lifecycle: TaskLifecycle::Active,
            branch: Some("master".into()),
            created_at: now,
            updated_at: now,
        })
        .unwrap();
    let first = json!({
        "session_id":"new-session",
        "turn_id":"turn_001",
        "cwd":repo,
        "prompt":"Refactor authentication boundary"
    });
    let first_ack =
        ingest_hook_event(&store, captured(HookEvent::UserPromptSubmit, first.clone())).unwrap();
    assert_eq!(
        first_ack
            .candidate
            .as_ref()
            .map(|item| item.task_id.as_str()),
        Some("previous-auth-task")
    );
    assert_eq!(
        store.get_session("new-session").unwrap().unwrap().task_id,
        None
    );
    assert_eq!(store.search_tasks("", 10).unwrap().len(), 1);

    let mut second = first;
    second["turn_id"] = json!("turn_002");
    let second_ack =
        ingest_hook_event(&store, captured(HookEvent::UserPromptSubmit, second)).unwrap();
    assert!(second_ack.candidate.is_none());
    let new_task_id = store
        .get_session("new-session")
        .unwrap()
        .unwrap()
        .task_id
        .unwrap();
    assert_ne!(new_task_id, "previous-auth-task");
    assert_eq!(store.search_tasks("", 10).unwrap().len(), 2);
}

#[test]
fn observed_resume_task_call_links_the_new_session_to_the_existing_task() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(temp.path(), &["init", repo.to_str().unwrap()]);
    let repository_id = previously_on::git::repository_identity(&repo).unwrap().id;
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    let now = Utc::now();
    store
        .upsert_task(&TaskV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: "previous-auth-task".into(),
            repository_id,
            title: "Refactor authentication boundary".into(),
            goal: Some("Refactor authentication boundary".into()),
            lifecycle: TaskLifecycle::Active,
            branch: Some("master".into()),
            created_at: now,
            updated_at: now,
        })
        .unwrap();

    let prompt = json!({
        "session_id":"resume-session",
        "turn_id":"turn_001",
        "cwd":repo,
        "prompt":"Refactor authentication boundary"
    });
    let ack = ingest_hook_event(&store, captured(HookEvent::UserPromptSubmit, prompt)).unwrap();
    assert_eq!(ack.candidate.unwrap().task_id, "previous-auth-task");
    assert_eq!(
        store
            .get_session("resume-session")
            .unwrap()
            .unwrap()
            .task_id,
        None
    );

    let resume_call = json!({
        "session_id":"resume-session",
        "turn_id":"turn_002",
        "cwd":repo,
        "tool_name":"mcp__previously_on__resume_task",
        "tool_use_id":"call-resume",
        "tool_input":{"task_id":"previous-auth-task","token_budget":1200},
        "tool_response":{"content":"verified pack returned"}
    });
    ingest_hook_event(&store, captured(HookEvent::PostToolUse, resume_call)).unwrap();

    let session = store.get_session("resume-session").unwrap().unwrap();
    assert_eq!(session.task_id.as_deref(), Some("previous-auth-task"));
    let timeline = store
        .get_task_timeline("previous-auth-task")
        .unwrap()
        .unwrap();
    assert!(timeline
        .sessions
        .iter()
        .any(|session| session.id == "resume-session"));
    assert_eq!(store.search_tasks("", 10).unwrap().len(), 1);
}

#[test]
fn current_conversation_suggests_a_new_thread_once_after_six_compactions() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(temp.path(), &["init", repo.to_str().unwrap()]);
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    let first = json!({
        "session_id":"long-session",
        "turn_id":"turn-001",
        "cwd":repo,
        "prompt":"Continue the authentication refactor"
    });
    let first_ack =
        ingest_hook_event(&store, captured(HookEvent::UserPromptSubmit, first)).unwrap();
    assert!(first_ack.continuation_advice.is_none());

    for index in 0..6 {
        let compact = json!({
            "session_id":"long-session",
            "turn_id":format!("compact-{index}"),
            "cwd":repo,
            "hook_event_name":"PreCompact"
        });
        ingest_hook_event(&store, captured(HookEvent::PreCompact, compact)).unwrap();
    }
    let before = store.get_session("long-session").unwrap().unwrap();
    assert_eq!(before.compaction_count, 6);
    assert_eq!(before.continuation_state, ContinuationStateV1::Eligible);

    let next = json!({
        "session_id":"long-session",
        "turn_id":"turn-002",
        "cwd":repo,
        "prompt":"Now update the integration test"
    });
    let next_event = captured(HookEvent::UserPromptSubmit, next);
    let advice = ingest_hook_event(&store, next_event.clone())
        .unwrap()
        .continuation_advice
        .expect("the next prompt should receive one rollover recommendation");
    assert_eq!(advice.action, "new_thread");
    assert!(advice
        .reasons
        .contains(&ContinuationReasonV1::CompactionLimit));
    assert_eq!(
        store
            .get_session("long-session")
            .unwrap()
            .unwrap()
            .continuation_state,
        ContinuationStateV1::Suggested
    );

    let retried_ack = ingest_hook_event(&store, next_event).unwrap();
    assert!(
        retried_ack.continuation_advice.is_some(),
        "the same stable prompt source must replay advice after an ACK loss"
    );

    let later = json!({
        "session_id":"long-session",
        "turn_id":"turn-003",
        "cwd":repo,
        "prompt":"Keep going in this conversation"
    });
    let later_ack =
        ingest_hook_event(&store, captured(HookEvent::UserPromptSubmit, later)).unwrap();
    assert!(later_ack.continuation_advice.is_none());
}

#[test]
fn historical_app_import_never_uses_import_time_git_as_a_checkpoint_baseline() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(temp.path(), &["init", repo.to_str().unwrap()]);
    std::fs::write(repo.join("state.txt"), "current import-time state\n").unwrap();
    let identity = previously_on::git::repository_identity(&repo).unwrap();
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    let occurred_at = Utc::now() - chrono::Duration::days(5);
    let historical = |source: &str, kind: EventKind, payload: serde_json::Value| {
        EventEnvelopeV1::new(
            source,
            &identity.id,
            "historical-session",
            kind,
            occurred_at,
            payload,
        )
    };
    ingest_hook_event(
        &store,
        historical(
            "codex-app-server:thread:old:item:prompt:user-message",
            EventKind::UserPrompt,
            json!({
                "app_server_source":"thread/read",
                "repository_path":repo,
                "prompt":"Historical task"
            }),
        ),
    )
    .unwrap();
    ingest_hook_event(
        &store,
        historical(
            "codex-app-server:thread:old:stop",
            EventKind::SessionStopped,
            json!({
                "app_server_source":"thread/read",
                "repository_path":repo,
                "last_assistant_message":"Historical final"
            }),
        ),
    )
    .unwrap();

    let task = store
        .search_tasks("Historical task", 1)
        .unwrap()
        .remove(0)
        .task;
    assert!(store.list_checkpoints(&task.id).unwrap().is_empty());
    let events = store
        .list_session_events(&identity.id, "historical-session")
        .unwrap();
    assert!(events
        .iter()
        .all(|event| event.payload.get("git_snapshot").is_none()));
    assert!(events.iter().all(|event| {
        event
            .coverage
            .missing
            .contains(&"historical_git_snapshot".to_string())
    }));
}

#[test]
fn historical_metadata_becomes_eligible_without_placeholder_tasks_or_import_advice() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(temp.path(), &["init", repo.to_str().unwrap()]);
    let identity = previously_on::git::repository_identity(&repo).unwrap();
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    let occurred_at = Utc::now() - chrono::Duration::days(2);
    let historical = |source: String, kind: EventKind, payload: serde_json::Value| {
        EventEnvelopeV1::new(
            source,
            &identity.id,
            "historical-eligible-session",
            kind,
            occurred_at,
            payload,
        )
    };

    for index in 0..6 {
        let ack = ingest_hook_event(
            &store,
            historical(
                format!("codex-app-server:thread:eligible:compact:{index}"),
                EventKind::ContextCompaction,
                json!({
                    "app_server_source":"thread/read",
                    "repository_path":repo,
                    "source_thread_id":"thread-eligible"
                }),
            ),
        )
        .unwrap();
        assert!(ack.candidate.is_none());
        assert!(ack.continuation_advice.is_none());
    }
    let usage_ack = ingest_hook_event(
        &store,
        historical(
            "codex-app-server:thread:eligible:usage".to_string(),
            EventKind::ContextUsageUpdated,
            json!({
                "app_server_source":"thread/read",
                "repository_path":repo,
                "source_thread_id":"thread-eligible",
                "context_usage":{"total_tokens":800,"model_context_window":1000}
            }),
        ),
    )
    .unwrap();
    assert!(usage_ack.candidate.is_none());
    assert!(usage_ack.continuation_advice.is_none());
    assert!(
        store.search_tasks("", 10).unwrap().is_empty(),
        "pre-prompt metadata must not create a placeholder task"
    );

    let imported_prompt_ack = ingest_hook_event(
        &store,
        historical(
            "codex-app-server:thread:eligible:prompt".to_string(),
            EventKind::UserPrompt,
            json!({
                "app_server_source":"thread/read",
                "repository_path":repo,
                "source_thread_id":"thread-eligible",
                "prompt":"Continue the imported authentication task"
            }),
        ),
    )
    .unwrap();
    assert!(imported_prompt_ack.candidate.is_none());
    assert!(imported_prompt_ack.continuation_advice.is_none());
    assert_eq!(store.search_tasks("", 10).unwrap().len(), 1);
    let imported_events = store
        .list_session_events(&identity.id, "historical-eligible-session")
        .unwrap();
    assert!(imported_events
        .iter()
        .all(|event| event.kind != EventKind::ContinuationSuggested));
    let session = store
        .get_session("historical-eligible-session")
        .unwrap()
        .unwrap();
    assert_eq!(session.compaction_count, 6);
    assert_eq!(session.continuation_state, ContinuationStateV1::Eligible);

    let next_live_prompt = captured(
        HookEvent::UserPromptSubmit,
        json!({
            "session_id":"historical-eligible-session",
            "turn_id":"live-turn-1",
            "cwd":repo,
            "prompt":"Apply the next live change"
        }),
    );
    let advice = ingest_hook_event(&store, next_live_prompt)
        .unwrap()
        .continuation_advice
        .expect("the next live prompt should receive imported rollover eligibility");
    assert!(advice
        .reasons
        .contains(&ContinuationReasonV1::CompactionLimit));
    assert!(advice
        .reasons
        .contains(&ContinuationReasonV1::ContextUsageLimit));
}

#[test]
fn replay_after_lost_ack_rearms_advice_for_the_next_live_prompt() {
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("data");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(temp.path(), &["init", repo.to_str().unwrap()]);
    let store = Store::open(data_dir.join("previously.sqlite3")).unwrap();
    let queue = data_dir.join("queue/events.jsonl");
    ingest_hook_event(
        &store,
        captured(
            HookEvent::UserPromptSubmit,
            json!({
                "session_id":"ack-loss-session",
                "turn_id":"turn-001",
                "cwd":repo,
                "prompt":"Start the long refactor"
            }),
        ),
    )
    .unwrap();
    for index in 0..6 {
        ingest_hook_event(
            &store,
            captured(
                HookEvent::PreCompact,
                json!({
                    "session_id":"ack-loss-session",
                    "turn_id":format!("compact-{index}"),
                    "cwd":repo
                }),
            ),
        )
        .unwrap();
    }

    let timed_out_prompt = captured(
        HookEvent::UserPromptSubmit,
        json!({
            "session_id":"ack-loss-session",
            "turn_id":"turn-002",
            "cwd":repo,
            "prompt":"This daemon commit loses its ACK"
        }),
    );
    assert!(ingest_hook_event(&store, timed_out_prompt.clone())
        .unwrap()
        .continuation_advice
        .is_some());
    // The hook queues the same redacted envelope after its socket ACK timeout.
    append_fallback(&queue, &timed_out_prompt).unwrap();
    replay_fallback(&store, &queue).unwrap();
    assert_eq!(
        store
            .get_session("ack-loss-session")
            .unwrap()
            .unwrap()
            .continuation_state,
        ContinuationStateV1::Eligible,
        "replay must re-arm advice because no hook process observed its ACK"
    );

    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for turn in ["turn-003-a", "turn-003-b"] {
        let barrier = Arc::clone(&barrier);
        let store = store.clone();
        let recovery_prompt = captured(
            HookEvent::UserPromptSubmit,
            json!({
                "session_id":"ack-loss-session",
                "turn_id":turn,
                "cwd":repo,
                "prompt":format!("Continue after recovery from {turn}")
            }),
        );
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            let received = ingest_hook_event(&store, recovery_prompt.clone())
                .unwrap()
                .continuation_advice
                .is_some();
            (recovery_prompt, received)
        }));
    }
    barrier.wait();
    let outcomes = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes.iter().filter(|(_, received)| *received).count(),
        1,
        "concurrent prompts must atomically consume one re-armed generation"
    );
    let winning_prompt = outcomes
        .iter()
        .find_map(|(prompt, received)| received.then_some(prompt.clone()))
        .unwrap();
    let losing_prompt = outcomes
        .into_iter()
        .find_map(|(prompt, received)| (!received).then_some(prompt));
    let losing_prompt = losing_prompt.unwrap_or_else(|| {
        panic!("the concurrent loser must remain available for a negative retry check")
    });
    assert!(
        ingest_hook_event(&store, winning_prompt)
            .unwrap()
            .continuation_advice
            .is_some(),
        "same-source retry must recover a second lost ACK"
    );
    assert!(
        ingest_hook_event(&store, losing_prompt)
            .unwrap()
            .continuation_advice
            .is_none(),
        "a losing concurrent source must not recover another source's consumed generation"
    );
    store.rebuild_projections().unwrap();
    assert_eq!(
        store
            .get_session("ack-loss-session")
            .unwrap()
            .unwrap()
            .continuation_state,
        ContinuationStateV1::Suggested,
        "projection rebuild must preserve consumption of the pending generation"
    );
    let later = captured(
        HookEvent::UserPromptSubmit,
        json!({
            "session_id":"ack-loss-session",
            "turn_id":"turn-004",
            "cwd":repo,
            "prompt":"A different later prompt must not receive it again"
        }),
    );
    assert!(ingest_hook_event(&store, later)
        .unwrap()
        .continuation_advice
        .is_none());
}

#[test]
fn concurrent_next_prompts_claim_only_one_continuation_suggestion() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(temp.path(), &["init", repo.to_str().unwrap()]);
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    ingest_hook_event(
        &store,
        captured(
            HookEvent::UserPromptSubmit,
            json!({
                "session_id":"concurrent-long-session",
                "turn_id":"turn-001",
                "cwd":repo,
                "prompt":"Continue the authentication refactor"
            }),
        ),
    )
    .unwrap();
    for index in 0..6 {
        ingest_hook_event(
            &store,
            captured(
                HookEvent::PreCompact,
                json!({
                    "session_id":"concurrent-long-session",
                    "turn_id":format!("compact-{index}"),
                    "cwd":repo,
                    "hook_event_name":"PreCompact"
                }),
            ),
        )
        .unwrap();
    }

    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for turn in ["turn-002-a", "turn-002-b"] {
        let barrier = Arc::clone(&barrier);
        let store = store.clone();
        let event = captured(
            HookEvent::UserPromptSubmit,
            json!({
                "session_id":"concurrent-long-session",
                "turn_id":turn,
                "cwd":repo,
                "prompt":format!("Continue from {turn}")
            }),
        );
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            ingest_hook_event(&store, event)
                .unwrap()
                .continuation_advice
                .is_some()
        }));
    }
    barrier.wait();
    let advice_count = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .filter(|received| *received)
        .count();
    assert_eq!(advice_count, 1);
    assert_eq!(
        store
            .list_session_events(
                &previously_on::git::repository_identity(&repo).unwrap().id,
                "concurrent-long-session",
            )
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == previously_on::domain::EventKind::ContinuationSuggested)
            .count(),
        1
    );
}

#[test]
fn evidence_only_regression_candidates_require_fail_edit_pass_or_test_and_code_edits() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    git(temp.path(), &["init", repo.to_str().unwrap()]);
    std::fs::write(
        repo.join("src/auth.rs"),
        "pub fn auth() -> bool { false }\n",
    )
    .unwrap();
    git(&repo, &["add", "src/auth.rs"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Contract Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "fixture",
        ],
    );
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();

    begin_hook_task(&store, &repo, "candidate-failure", "Fix auth regression");
    record_test_hook(
        &store,
        &repo,
        "candidate-failure",
        "failure-test",
        "cargo test auth",
        1,
    );
    record_patch_hook(
        &store,
        &repo,
        "candidate-failure",
        "failure-fix",
        &[("src/auth.rs", "pub fn auth() -> bool { true }\n")],
    );
    record_test_hook(
        &store,
        &repo,
        "candidate-failure",
        "passing-test",
        "cargo test auth",
        0,
    );
    let repository_id = previously_on::git::repository_identity(&repo).unwrap().id;
    let candidates = store
        .list_regression_candidates(Some(&repository_id))
        .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0].evidence_kind,
        CandidateEvidenceKindV1::FailureEditPass
    );
    assert_eq!(candidates[0].required_tests[0].program, "cargo");
    assert_eq!(candidates[0].required_tests[0].args, ["test", "auth"]);

    begin_hook_task(
        &store,
        &repo,
        "candidate-pass-only",
        "Ordinary passing work",
    );
    record_patch_hook(
        &store,
        &repo,
        "candidate-pass-only",
        "ordinary-edit",
        &[("src/ordinary.rs", "pub fn ordinary() {}\n")],
    );
    record_test_hook(
        &store,
        &repo,
        "candidate-pass-only",
        "ordinary-pass",
        "cargo test ordinary",
        0,
    );
    assert_eq!(
        store
            .list_regression_candidates(Some(&repository_id))
            .unwrap()
            .len(),
        1,
        "a pass-only ordinary task must not create a candidate"
    );

    begin_hook_task(
        &store,
        &repo,
        "candidate-test-edit",
        "Add a regression test and fix",
    );
    record_patch_hook(
        &store,
        &repo,
        "candidate-test-edit",
        "test-and-code-edit",
        &[
            ("src/feature.rs", "pub fn feature() -> bool { true }\n"),
            (
                "tests/feature_test.rs",
                "#[test]\nfn feature_stays_true() {}\n",
            ),
        ],
    );
    record_test_hook(
        &store,
        &repo,
        "candidate-test-edit",
        "test-file-pass",
        "cargo test feature",
        0,
    );
    let candidates = store
        .list_regression_candidates(Some(&repository_id))
        .unwrap();
    assert_eq!(candidates.len(), 2);
    assert!(candidates
        .iter()
        .any(|candidate| candidate.evidence_kind == CandidateEvidenceKindV1::TestFileEditPass));
}

#[test]
fn relevant_contract_context_stop_freshness_and_loop_guard_are_persisted() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    git(temp.path(), &["init", repo.to_str().unwrap()]);
    std::fs::write(
        repo.join("src/auth.rs"),
        "pub fn auth() -> bool { false }\n",
    )
    .unwrap();
    git(&repo, &["add", "src/auth.rs"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Contract Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "fixture",
        ],
    );
    let snapshot = previously_on::git::capture_snapshot(&repo).unwrap();
    previously_on::contracts::write_contract(
        &repo,
        &RegressionContractV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: "00000000-0000-4000-8000-000000000101".into(),
            title: "Authentication must remain covered".into(),
            invariant: "Authentication behavior stays stable".into(),
            status: ContractStatusV1::Active,
            superseded_by: None,
            impact_selectors: vec![ImpactSelectorGroupV1 {
                path: ImpactPathSelectorV1 {
                    kind: PathSelectorKindV1::Exact,
                    value: "src/auth.rs".into(),
                },
                symbols: Vec::new(),
            }],
            required_tests: vec![RequiredTestV1 {
                id: "auth-test".into(),
                name: "auth test".into(),
                program: "cargo".into(),
                args: vec!["test".into(), "auth".into()],
                working_directory: ".".into(),
                timeout_seconds: 900,
            }],
            origin: ContractOriginV1 {
                fixed_at_commit: snapshot.head.unwrap(),
                recorded_at: Utc::now(),
                evidence_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .into(),
            },
        },
    )
    .unwrap();
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    begin_hook_task(&store, &repo, "contract-stop", "Change authentication");
    let pre_ack = record_patch_hook(
        &store,
        &repo,
        "contract-stop",
        "auth-edit",
        &[("src/auth.rs", "pub fn auth() -> bool { true }\n")],
    );
    let context = pre_ack
        .contract_context
        .expect("PreToolUse Contract context");
    assert!(context.contains("Authentication must remain covered"));
    assert!(context.contains("auth-test"));

    let first_stop = record_stop_hook(&store, &repo, "contract-stop", "stop-1", false);
    assert!(first_stop
        .stop_block_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("cargo test auth")));
    let second_stop = record_stop_hook(&store, &repo, "contract-stop", "stop-2", true);
    assert!(second_stop.stop_block_reason.is_none());
    let repeated_stop = record_stop_hook(&store, &repo, "contract-stop", "stop-2b", false);
    assert!(
        repeated_stop.stop_block_reason.is_none(),
        "the stored fingerprint must prevent a repeated continuation"
    );

    record_test_hook(
        &store,
        &repo,
        "contract-stop",
        "auth-pass",
        "cargo test auth",
        0,
    );
    let ready_stop = record_stop_hook(&store, &repo, "contract-stop", "stop-3", false);
    assert!(ready_stop.stop_block_reason.is_none());

    record_patch_hook(
        &store,
        &repo,
        "contract-stop",
        "auth-edit-again",
        &[("src/auth.rs", "pub fn auth() -> bool { false }\n")],
    );
    let stale_stop = record_stop_hook(&store, &repo, "contract-stop", "stop-4", false);
    assert!(stale_stop.stop_block_reason.is_some());
    let repository_id = previously_on::git::repository_identity(&repo).unwrap().id;
    let evaluations = store
        .list_contract_evaluations(Some(&repository_id))
        .unwrap();
    assert!(evaluations
        .iter()
        .any(|evaluation| evaluation.readiness == ContractReadinessV1::Ready));
    assert!(evaluations.iter().any(|evaluation| {
        evaluation.readiness == ContractReadinessV1::ContractBlocked
            && evaluation
                .required_tests
                .iter()
                .any(|test| test.state == RequiredTestStateV1::Stale)
    }));
    assert_eq!(
        evaluations
            .iter()
            .filter(|evaluation| evaluation.continuation_issued)
            .count(),
        2,
        "one continuation is allowed for each distinct related content fingerprint"
    );
}

#[test]
fn invalid_contract_stop_error_does_not_reblock_an_active_stop_hook() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join(".previously-on/contracts")).unwrap();
    git(temp.path(), &["init", repo.to_str().unwrap()]);
    std::fs::write(
        repo.join(".previously-on/contracts/invalid.json"),
        r#"{"schemaVersion":1,"id":"invalid"}"#,
    )
    .unwrap();
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    begin_hook_task(
        &store,
        &repo,
        "invalid-contract",
        "Inspect invalid Contract",
    );
    let first = record_stop_hook(&store, &repo, "invalid-contract", "invalid-stop-1", false);
    assert!(first.stop_block_reason.is_some());
    let second = record_stop_hook(&store, &repo, "invalid-contract", "invalid-stop-2", true);
    assert!(second.stop_block_reason.is_none());
    let repeated = record_stop_hook(&store, &repo, "invalid-contract", "invalid-stop-3", false);
    assert!(repeated.stop_block_reason.is_none());
    let repository_id = previously_on::git::repository_identity(&repo).unwrap().id;
    let evaluations = store
        .list_contract_evaluations(Some(&repository_id))
        .unwrap();
    assert!(evaluations
        .iter()
        .all(|evaluation| { evaluation.readiness == ContractReadinessV1::ContractBlocked }));
}

#[test]
fn required_test_freshness_matches_repository_relative_subdirectory_cwd() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("ui/src")).unwrap();
    git(temp.path(), &["init", repo.to_str().unwrap()]);
    std::fs::write(repo.join("ui/src/app.ts"), "export const ready = false;\n").unwrap();
    git(&repo, &["add", "ui/src/app.ts"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Contract Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "fixture",
        ],
    );
    let snapshot = previously_on::git::capture_snapshot(&repo).unwrap();
    previously_on::contracts::write_contract(
        &repo,
        &RegressionContractV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: "00000000-0000-4000-8000-000000000102".into(),
            title: "UI behavior stays tested".into(),
            invariant: "UI readiness remains true".into(),
            status: ContractStatusV1::Active,
            superseded_by: None,
            impact_selectors: vec![ImpactSelectorGroupV1 {
                path: ImpactPathSelectorV1 {
                    kind: PathSelectorKindV1::Exact,
                    value: "ui/src/app.ts".into(),
                },
                symbols: Vec::new(),
            }],
            required_tests: vec![RequiredTestV1 {
                id: "ui-test".into(),
                name: "UI tests".into(),
                program: "npm".into(),
                args: vec!["test".into()],
                working_directory: "ui".into(),
                timeout_seconds: 900,
            }],
            origin: ContractOriginV1 {
                fixed_at_commit: snapshot.head.unwrap(),
                recorded_at: Utc::now(),
                evidence_sha256: "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                    .into(),
            },
        },
    )
    .unwrap();
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    begin_hook_task(&store, &repo, "subdir-cwd", "Update UI readiness");
    record_patch_hook(
        &store,
        &repo,
        "subdir-cwd",
        "ui-edit",
        &[("ui/src/app.ts", "export const ready = true;\n")],
    );
    record_test_hook(
        &store,
        &repo.join("ui"),
        "subdir-cwd",
        "ui-pass",
        "npm test",
        0,
    );
    let stop = record_stop_hook(&store, &repo, "subdir-cwd", "ui-stop", false);
    let repository_id = previously_on::git::repository_identity(&repo).unwrap().id;
    let evaluations = store
        .list_contract_evaluations(Some(&repository_id))
        .unwrap();
    assert!(
        stop.stop_block_reason.is_none(),
        "unexpected block: {:?}; evaluations={evaluations:#?}",
        stop.stop_block_reason
    );
    assert!(evaluations
        .iter()
        .any(|evaluation| evaluation.readiness == ContractReadinessV1::Ready));
}

#[test]
fn resumed_task_keeps_committed_related_change_blocked_until_the_required_test_runs() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    git(temp.path(), &["init", repo.to_str().unwrap()]);
    std::fs::write(repo.join("src/auth.rs"), "pub fn auth() -> bool { true }\n").unwrap();
    git(&repo, &["add", "src/auth.rs"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Contract Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "fixture",
        ],
    );
    let snapshot = previously_on::git::capture_snapshot(&repo).unwrap();
    previously_on::contracts::write_contract(
        &repo,
        &RegressionContractV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: "00000000-0000-4000-8000-000000000103".into(),
            title: "Authentication stays verified".into(),
            invariant: "Authentication remains enabled".into(),
            status: ContractStatusV1::Active,
            superseded_by: None,
            impact_selectors: vec![ImpactSelectorGroupV1 {
                path: ImpactPathSelectorV1 {
                    kind: PathSelectorKindV1::Exact,
                    value: "src/auth.rs".into(),
                },
                symbols: Vec::new(),
            }],
            required_tests: vec![RequiredTestV1 {
                id: "auth-test".into(),
                name: "Authentication regression".into(),
                program: "cargo".into(),
                args: vec!["test".into(), "auth".into()],
                working_directory: ".".into(),
                timeout_seconds: 900,
            }],
            origin: ContractOriginV1 {
                fixed_at_commit: snapshot.head.unwrap(),
                recorded_at: Utc::now(),
                evidence_sha256: "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                    .into(),
            },
        },
    )
    .unwrap();
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();

    begin_hook_task(&store, &repo, "contract-session-a", "Change authentication");
    let task_id = store
        .get_session("contract-session-a")
        .unwrap()
        .unwrap()
        .task_id
        .unwrap();
    record_patch_hook(
        &store,
        &repo,
        "contract-session-a",
        "auth-edit",
        &[("src/auth.rs", "pub fn auth() -> bool { false }\n")],
    );
    git(&repo, &["add", "src/auth.rs"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Contract Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "change auth",
        ],
    );
    assert!(
        record_stop_hook(&store, &repo, "contract-session-a", "stop-a", false)
            .stop_block_reason
            .is_some()
    );

    let prompt = json!({
        "session_id": "contract-session-b",
        "turn_id": "prompt-b",
        "cwd": repo,
        "prompt": "Change authentication"
    });
    let suggestion = ingest_hook_event(&store, captured(HookEvent::UserPromptSubmit, prompt))
        .unwrap()
        .candidate
        .unwrap();
    assert_eq!(suggestion.task_id, task_id);
    let resume = json!({
        "session_id": "contract-session-b",
        "turn_id": "resume-b",
        "cwd": repo,
        "tool_name": "mcp__previously_on__resume_task",
        "tool_use_id": "resume-task-b",
        "tool_input": {"task_id": task_id},
        "tool_response": {"content": "verified pack returned"}
    });
    ingest_hook_event(&store, captured(HookEvent::PostToolUse, resume)).unwrap();
    let resumed_stop = record_stop_hook(&store, &repo, "contract-session-b", "stop-b", false);
    assert!(
        resumed_stop.stop_block_reason.is_none(),
        "the persisted once-per-fingerprint continuation guard should prevent a loop"
    );

    let repository_id = previously_on::git::repository_identity(&repo).unwrap().id;
    let latest = store
        .list_contract_evaluations(Some(&repository_id))
        .unwrap()
        .into_iter()
        .find(|evaluation| evaluation.task_id.as_deref() == Some(task_id.as_str()))
        .unwrap();
    assert_eq!(latest.readiness, ContractReadinessV1::ContractBlocked);
    assert!(latest
        .required_tests
        .iter()
        .any(|test| test.state == RequiredTestStateV1::Missing));
}

#[test]
fn queued_passing_test_cannot_certify_related_content_changed_before_replay() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    git(temp.path(), &["init", repo.to_str().unwrap()]);
    std::fs::write(repo.join("src/auth.rs"), "pub fn auth() -> bool { true }\n").unwrap();
    git(&repo, &["add", "src/auth.rs"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Contract Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "fixture",
        ],
    );
    let snapshot = previously_on::git::capture_snapshot(&repo).unwrap();
    previously_on::contracts::write_contract(
        &repo,
        &RegressionContractV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: "00000000-0000-4000-8000-000000000104".into(),
            title: "Authentication stays verified".into(),
            invariant: "Authentication remains enabled".into(),
            status: ContractStatusV1::Active,
            superseded_by: None,
            impact_selectors: vec![ImpactSelectorGroupV1 {
                path: ImpactPathSelectorV1 {
                    kind: PathSelectorKindV1::Exact,
                    value: "src/auth.rs".into(),
                },
                symbols: Vec::new(),
            }],
            required_tests: vec![RequiredTestV1 {
                id: "auth-test".into(),
                name: "Authentication regression".into(),
                program: "cargo".into(),
                args: vec!["test".into(), "auth".into()],
                working_directory: ".".into(),
                timeout_seconds: 900,
            }],
            origin: ContractOriginV1 {
                fixed_at_commit: snapshot.head.unwrap(),
                recorded_at: Utc::now(),
                evidence_sha256: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
                    .into(),
            },
        },
    )
    .unwrap();
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    begin_hook_task(&store, &repo, "queued-pass", "Change authentication");
    record_patch_hook(
        &store,
        &repo,
        "queued-pass",
        "auth-edit-before-test",
        &[("src/auth.rs", "pub fn auth() -> bool { false }\n")],
    );

    let mut passing_payload = hook_base(&repo, "queued-pass", "auth-pass-queued");
    passing_payload["tool_name"] = json!("Bash");
    passing_payload["tool_use_id"] = json!("auth-pass-queued");
    passing_payload["tool_input"] = json!({"command":"cargo test auth"});
    passing_payload["tool_response"] = json!({"exit_code":0,"output":"passed"});
    let queued_pass = captured(HookEvent::PostToolUse, passing_payload);
    assert!(queued_pass
        .payload
        .get("source_test_git_snapshot")
        .is_some());
    let queue = temp.path().join("queue/events.jsonl");
    append_fallback(&queue, &queued_pass).unwrap();

    std::fs::write(
        repo.join("src/auth.rs"),
        "pub fn auth() -> bool { false }\npub fn changed_after_test() {}\n",
    )
    .unwrap();
    replay_fallback(&store, &queue).unwrap();
    let stop = record_stop_hook(&store, &repo, "queued-pass", "stop-after-replay", false);
    assert!(stop.stop_block_reason.is_some());

    let repository_id = previously_on::git::repository_identity(&repo).unwrap().id;
    let evaluations = store
        .list_contract_evaluations(Some(&repository_id))
        .unwrap();
    assert!(evaluations.iter().any(|evaluation| {
        evaluation.required_tests.iter().any(|test| {
            matches!(
                test.state,
                RequiredTestStateV1::Stale | RequiredTestStateV1::Missing
            )
        })
    }));
    assert!(!evaluations
        .iter()
        .any(|evaluation| evaluation.readiness == ContractReadinessV1::Ready));
}

#[test]
fn symbol_scoped_contract_ignores_inspectable_unrelated_text_changes() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    git(temp.path(), &["init", repo.to_str().unwrap()]);
    std::fs::write(
        repo.join("src/auth.rs"),
        "pub fn tenant_guard() -> bool { true }\npub fn unrelated() {}\n",
    )
    .unwrap();
    git(&repo, &["add", "src/auth.rs"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Contract Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "fixture",
        ],
    );
    let snapshot = previously_on::git::capture_snapshot(&repo).unwrap();
    previously_on::contracts::write_contract(
        &repo,
        &RegressionContractV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: "00000000-0000-4000-8000-000000000105".into(),
            title: "Tenant guard remains protected".into(),
            invariant: "Tenant guard remains enabled".into(),
            status: ContractStatusV1::Active,
            superseded_by: None,
            impact_selectors: vec![ImpactSelectorGroupV1 {
                path: ImpactPathSelectorV1 {
                    kind: PathSelectorKindV1::Exact,
                    value: "src/auth.rs".into(),
                },
                symbols: vec!["tenant_guard".into()],
            }],
            required_tests: vec![RequiredTestV1 {
                id: "tenant-test".into(),
                name: "Tenant regression".into(),
                program: "cargo".into(),
                args: vec!["test".into(), "tenant".into()],
                working_directory: ".".into(),
                timeout_seconds: 900,
            }],
            origin: ContractOriginV1 {
                fixed_at_commit: snapshot.head.unwrap(),
                recorded_at: Utc::now(),
                evidence_sha256: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                    .into(),
            },
        },
    )
    .unwrap();
    let store = Store::open(temp.path().join("previously.sqlite3")).unwrap();
    begin_hook_task(&store, &repo, "symbol-unrelated", "Update unrelated helper");
    let pre = record_patch_hook(
        &store,
        &repo,
        "symbol-unrelated",
        "unrelated-edit",
        &[(
            "src/auth.rs",
            "pub fn tenant_guard() -> bool { true }\npub fn unrelated() { println!(\"changed\"); }\n",
        )],
    );
    assert!(
        pre.contract_context.is_some(),
        "PreToolUse remains conservative"
    );
    let stop = record_stop_hook(&store, &repo, "symbol-unrelated", "symbol-stop", false);
    assert!(stop.stop_block_reason.is_none());
    let repository_id = previously_on::git::repository_identity(&repo).unwrap().id;
    let latest = store
        .list_contract_evaluations(Some(&repository_id))
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(latest.readiness, ContractReadinessV1::Ready);
    assert!(latest.relevant_contracts.is_empty());
}

fn hook_base(repo: &std::path::Path, session_id: &str, position: &str) -> serde_json::Value {
    json!({
        "session_id": session_id,
        "turn_id": position,
        "cwd": repo
    })
}

fn begin_hook_task(store: &Store, repo: &std::path::Path, session_id: &str, prompt: &str) {
    ingest_hook_event(
        store,
        captured(
            HookEvent::SessionStart,
            hook_base(repo, session_id, "session-start"),
        ),
    )
    .unwrap();
    let mut payload = hook_base(repo, session_id, "prompt-1");
    payload["prompt"] = json!(prompt);
    ingest_hook_event(store, captured(HookEvent::UserPromptSubmit, payload)).unwrap();
}

fn record_patch_hook(
    store: &Store,
    repo: &std::path::Path,
    session_id: &str,
    tool_id: &str,
    files: &[(&str, &str)],
) -> HookAckV1 {
    let mut payload = hook_base(repo, session_id, tool_id);
    payload["tool_name"] = json!("apply_patch");
    payload["tool_use_id"] = json!(tool_id);
    payload["tool_input"] = json!({
        "command": files
            .iter()
            .map(|(path, _)| format!("*** Update File: {path}"))
            .collect::<Vec<_>>()
            .join("\n")
    });
    let pre_ack =
        ingest_hook_event(store, captured(HookEvent::PreToolUse, payload.clone())).unwrap();
    for (path, contents) in files {
        let path = repo.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }
    payload["tool_response"] = json!({"content":"Done"});
    ingest_hook_event(store, captured(HookEvent::PostToolUse, payload)).unwrap();
    pre_ack
}

fn record_test_hook(
    store: &Store,
    repo: &std::path::Path,
    session_id: &str,
    tool_id: &str,
    command: &str,
    exit_code: i64,
) {
    let mut payload = hook_base(repo, session_id, tool_id);
    payload["tool_name"] = json!("Bash");
    payload["tool_use_id"] = json!(tool_id);
    payload["tool_input"] = json!({"command":command});
    payload["tool_response"] = json!({
        "exit_code":exit_code,
        "output":if exit_code == 0 { "passed" } else { "failed" }
    });
    ingest_hook_event(store, captured(HookEvent::PostToolUse, payload)).unwrap();
}

fn record_stop_hook(
    store: &Store,
    repo: &std::path::Path,
    session_id: &str,
    turn_id: &str,
    stop_hook_active: bool,
) -> HookAckV1 {
    let mut payload = hook_base(repo, session_id, turn_id);
    payload["stop_hook_active"] = json!(stop_hook_active);
    payload["last_assistant_message"] = json!("done");
    ingest_hook_event(store, captured(HookEvent::Stop, payload)).unwrap()
}
