use std::io::Cursor;
use std::process::Command;

use chrono::Utc;
use previously_on::domain::{
    ChangeAttribution, TaskLifecycle, TaskV1, TestStatus, SCHEMA_VERSION_V1,
};
use previously_on::hook::{capture, ingest_hook_event, HookDeliveryStatus, HookEvent};
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
