use std::io::Cursor;
use std::process::Command;
use std::sync::{Arc, Barrier};

use chrono::Utc;
use previously_on::domain::{
    ChangeAttribution, ContinuationReasonV1, ContinuationStateV1, EventEnvelopeV1, EventKind,
    TaskLifecycle, TaskV1, TestStatus, SCHEMA_VERSION_V1,
};
use previously_on::hook::{
    append_fallback, capture, ingest_hook_event, replay_fallback, HookDeliveryStatus, HookEvent,
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
