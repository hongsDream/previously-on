use std::path::Path;

use anyhow::Result;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::domain::{
    deterministic_id, ChangeAttribution, CheckpointV1, CoverageStatus, EventEnvelopeV1, EventKind,
    EvidenceIntegrity, EvidenceV1, FactKind, FactLifecycle, FactV1, FileChangeV1, Freshness,
    GitSnapshotV1, TestResultV1, TestStatus, SCHEMA_VERSION_V1,
};
use crate::store::Store;

pub(super) fn append_explicit_fact_candidates(
    store: &Store,
    source: &EventEnvelopeV1,
) -> Result<()> {
    let Some(task_id) = source.task_id.as_deref() else {
        return Ok(());
    };
    let text = match source.kind {
        EventKind::UserPrompt => prompt_text(&source.payload),
        EventKind::SessionStopped => source
            .payload
            .get("last_assistant_message")
            .and_then(Value::as_str),
        _ => None,
    };
    let Some(text) = text else {
        return Ok(());
    };
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        let lowered = trimmed.to_ascii_lowercase();
        let (kind, content) = if lowered.starts_with("decision:") {
            (FactKind::Decision, trimmed["decision:".len()..].trim())
        } else if lowered.starts_with("constraint:") {
            (FactKind::Constraint, trimmed["constraint:".len()..].trim())
        } else if lowered.starts_with("open item:") {
            (FactKind::OpenItem, trimmed["open item:".len()..].trim())
        } else if lowered.starts_with("unresolved:") {
            (FactKind::OpenItem, trimmed["unresolved:".len()..].trim())
        } else {
            continue;
        };
        if content.is_empty() {
            continue;
        }
        let fact_id = deterministic_id("fact", &[&source.repository_id, task_id, content]);
        let evidence_id = deterministic_id(
            "evidence",
            &[&source.source_id, &index.to_string(), content],
        );
        let evidence = EvidenceV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: evidence_id.clone(),
            repository_id: source.repository_id.clone(),
            task_id: task_id.to_string(),
            session_id: source.session_id.clone(),
            fact_id: Some(fact_id.clone()),
            source_id: source.source_id.clone(),
            turn_index: source.sequence.and_then(|value| u32::try_from(value).ok()),
            item_index: u32::try_from(index).ok(),
            excerpt: content.to_string(),
            excerpt_sha256: hex::encode(Sha256::digest(content.as_bytes())),
            integrity: EvidenceIntegrity::Verified,
            created_at: source.occurred_at,
        };
        let fact = FactV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: fact_id,
            repository_id: source.repository_id.clone(),
            task_id: task_id.to_string(),
            kind,
            lifecycle: FactLifecycle::Candidate,
            freshness: Freshness::Fresh,
            origin: crate::domain::FactOriginV1::Captured,
            content: content.to_string(),
            evidence_ids: vec![evidence_id],
            superseded_by: None,
            created_at: source.occurred_at,
            updated_at: source.occurred_at,
        };
        let mut fact_event = EventEnvelopeV1::new(
            format!("fact-candidate:{}:{index}", source.source_id),
            &source.repository_id,
            &source.session_id,
            EventKind::FactCandidate,
            source.occurred_at,
            json!({"fact":fact,"evidence":evidence}),
        );
        fact_event.task_id = Some(task_id.to_string());
        fact_event.coverage = source.coverage.clone();
        store.insert_event(&fact_event)?;
    }
    Ok(())
}

pub(super) fn normalize_tool_result(
    store: &Store,
    event: &mut EventEnvelopeV1,
    after: Option<&GitSnapshotV1>,
    snapshot_path: &str,
) -> Result<()> {
    let prior_events = store.list_session_events(&event.repository_id, &event.session_id)?;
    let current_tool_id = tool_use_id(&event.payload);
    let paired_pre = prior_events.iter().rev().find(|prior| {
        prior.kind == EventKind::ToolStarted
            && current_tool_id.is_some()
            && tool_use_id(&prior.payload) == current_tool_id
    });
    let before = paired_pre.and_then(event_snapshot);
    let paths = tool_evidence_paths(&event.payload);
    let test = tool_test_result(event);
    let mut changes = None;
    if let (Some(before), Some(after)) = (before.as_ref(), after) {
        let observed = crate::git::correlate_changes(
            snapshot_path,
            before,
            after,
            &event.session_id,
            event.task_id.as_deref(),
            &[],
        )?;
        let observed_paths = observed
            .iter()
            .flat_map(|change| {
                std::iter::once(change.path.clone()).chain(change.previous_path.clone())
            })
            .collect::<std::collections::BTreeSet<_>>();
        let evidence_paths = paths
            .iter()
            .map(|path| path.trim_start_matches("./").to_string())
            .collect::<std::collections::BTreeSet<_>>();
        let exact_structured_match = is_structured_file_tool(&event.payload)
            && !evidence_paths.is_empty()
            && observed_paths == evidence_paths;
        changes = Some(if exact_structured_match {
            crate::git::correlate_changes(
                snapshot_path,
                before,
                after,
                &event.session_id,
                event.task_id.as_deref(),
                &paths,
            )?
        } else {
            if !observed.is_empty() {
                event.coverage.status = CoverageStatus::Degraded;
                event.coverage.warnings.push(
                    "file changes were observed, but exact structured PreToolUse/PostToolUse evidence did not match; attribution was downgraded"
                        .to_string(),
                );
            }
            observed
        });
    } else if !paths.is_empty() {
        event.coverage.status = CoverageStatus::Degraded;
        event
            .coverage
            .missing
            .push("paired_pre_tool_snapshot".to_string());
    }
    let Some(object) = event.payload.as_object_mut() else {
        return Ok(());
    };
    if changes.is_none() {
        let mut projected = object
            .get("file_changes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|value| serde_json::from_value::<FileChangeV1>(value.clone()).ok())
            .collect::<Vec<_>>();
        if !projected.is_empty() {
            for change in &mut projected {
                change.repository_id = event.repository_id.clone();
                change.session_id = event.session_id.clone();
                change.task_id = event.task_id.clone();
                // Imported App Server file changes lack a paired before-snapshot and therefore
                // can never be promoted to MODIFIED_BY by merely naming a structured tool.
                if before.is_none() {
                    change.attribution = ChangeAttribution::ObservedChangedIn;
                }
            }
            changes = Some(projected);
        }
    }
    if let Some(changes) = changes {
        object.insert("file_changes".to_string(), serde_json::to_value(changes)?);
        event.coverage.captured.push("file_changes".to_string());
    }
    if let Some(test) = test {
        object.insert("test_result".to_string(), serde_json::to_value(test)?);
        event.coverage.captured.push("test_result".to_string());
    }
    Ok(())
}

fn tool_use_id(payload: &Value) -> Option<&str> {
    ["tool_use_id", "toolUseId", "call_id", "callId"]
        .into_iter()
        .find_map(|key| payload.get(key).and_then(Value::as_str))
        .or_else(|| payload.pointer("/tool/id").and_then(Value::as_str))
}

fn is_structured_file_tool(payload: &Value) -> bool {
    let name = ["tool_name", "toolName", "name"]
        .into_iter()
        .find_map(|key| payload.get(key).and_then(Value::as_str))
        .unwrap_or_default()
        .to_ascii_lowercase();
    ["apply_patch", "write_file", "edit_file", "create_file"]
        .iter()
        .any(|candidate| name == *candidate || name.ends_with(&format!("__{candidate}")))
}

pub(super) fn append_checkpoint_event(
    store: &Store,
    source: &EventEnvelopeV1,
    git_after: GitSnapshotV1,
) -> Result<()> {
    let Some(task_id) = source.task_id.as_deref() else {
        return Ok(());
    };
    let events = store.list_session_events(&source.repository_id, &source.session_id)?;
    let git_before = events.iter().find_map(event_snapshot);
    let mut changes = events
        .iter()
        .flat_map(event_file_changes)
        .collect::<Vec<_>>();
    if changes.is_empty() {
        changes = crate::git::correlate_changes(
            &git_after.root,
            git_before.as_ref().unwrap_or(&git_after),
            &git_after,
            &source.session_id,
            Some(task_id),
            &[],
        )?;
    }
    let tests = events
        .iter()
        .filter_map(event_test_result)
        .collect::<Vec<_>>();
    let mut checkpoint = CheckpointV1::project(&events, git_before, git_after, changes, tests);
    let required = [
        (EventKind::SessionStarted, "SessionStart"),
        (EventKind::UserPrompt, "UserPromptSubmit"),
        (EventKind::SessionStopped, "assistant_final"),
    ];
    for (kind, label) in required {
        let observed = if label == "assistant_final" {
            events.iter().any(|event| {
                event
                    .payload
                    .get("last_assistant_message")
                    .and_then(Value::as_str)
                    .is_some()
            })
        } else {
            events.iter().any(|event| event.kind == kind)
        };
        if !observed {
            checkpoint.coverage.missing.push(label.to_string());
        }
    }
    if events.iter().any(|event| {
        event.kind == EventKind::ToolFinished
            && !tool_evidence_paths(&event.payload).is_empty()
            && event_file_changes(event).is_empty()
    }) {
        checkpoint
            .coverage
            .missing
            .push("file_change_attribution".to_string());
    }
    if events.iter().any(|event| {
        event.kind == EventKind::ToolFinished
            && tool_test_result(event).is_some()
            && event_test_result(event).is_none()
    }) {
        checkpoint.coverage.missing.push("test_result".to_string());
    }
    checkpoint.coverage.missing.sort();
    checkpoint.coverage.missing.dedup();
    if !checkpoint.coverage.missing.is_empty() {
        checkpoint.coverage.status = CoverageStatus::Degraded;
    }
    let mut checkpoint_event = EventEnvelopeV1::new(
        format!("checkpoint:{}", source.source_id),
        &source.repository_id,
        &source.session_id,
        EventKind::Checkpoint,
        source.occurred_at,
        json!({ "checkpoint": checkpoint }),
    );
    checkpoint_event.task_id = Some(task_id.to_string());
    checkpoint_event.coverage = source.coverage.clone();
    store.insert_event(&checkpoint_event)?;
    Ok(())
}

pub(super) fn event_snapshot(event: &EventEnvelopeV1) -> Option<GitSnapshotV1> {
    serde_json::from_value(event.payload.get("git_snapshot")?.clone()).ok()
}

pub(super) fn source_test_snapshot(event: &EventEnvelopeV1) -> Option<GitSnapshotV1> {
    serde_json::from_value(event.payload.get("source_test_git_snapshot")?.clone()).ok()
}

pub(super) fn event_file_changes(event: &EventEnvelopeV1) -> Vec<FileChangeV1> {
    event
        .payload
        .get("file_changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| serde_json::from_value(value.clone()).ok())
        .collect()
}

pub(super) fn event_test_result(event: &EventEnvelopeV1) -> Option<TestResultV1> {
    serde_json::from_value(event.payload.get("test_result")?.clone()).ok()
}

pub(super) fn prompt_text(payload: &Value) -> Option<&str> {
    ["prompt", "text", "content"]
        .into_iter()
        .find_map(|key| payload.get(key).and_then(Value::as_str))
}

pub(super) fn tool_evidence_paths(payload: &Value) -> Vec<String> {
    let mut paths = Vec::new();
    if let Some(input) = payload.get("tool_input") {
        for key in ["path", "file_path", "filePath"] {
            if let Some(path) = input.get(key).and_then(Value::as_str) {
                paths.push(path.to_string());
            }
        }
        if let Some(command) = input.get("command").and_then(Value::as_str) {
            for line in command.lines() {
                for prefix in [
                    "*** Update File: ",
                    "*** Add File: ",
                    "*** Delete File: ",
                    "*** Move to: ",
                ] {
                    if let Some(path) = line.trim().strip_prefix(prefix) {
                        paths.push(path.trim().to_string());
                    }
                }
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn tool_test_result(event: &EventEnvelopeV1) -> Option<TestResultV1> {
    let command = event
        .payload
        .pointer("/tool_input/command")
        .and_then(Value::as_str)?;
    let normalized = normalize_test_command_for_event(event, command)?;
    let exit_code = event
        .payload
        .pointer("/tool_response/exit_code")
        .and_then(Value::as_i64);
    let status = match exit_code {
        Some(0) => TestStatus::Passed,
        Some(_) => TestStatus::Failed,
        None => TestStatus::Unknown,
    };
    let summary = event
        .payload
        .pointer("/tool_response/output")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(TestResultV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: deterministic_id("test", &[&event.source_id, command]),
        repository_id: event.repository_id.clone(),
        session_id: event.session_id.clone(),
        task_id: event.task_id.clone(),
        name: normalized.display().chars().take(120).collect(),
        command: command.to_string(),
        status,
        summary,
        occurred_at: event.occurred_at,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct NormalizedTestCommand {
    pub(super) program: String,
    pub(super) args: Vec<String>,
    pub(super) working_directory: String,
}

impl NormalizedTestCommand {
    pub(super) fn display(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .map(shell_display_word)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

pub(super) fn normalize_test_command(command: &str) -> Option<NormalizedTestCommand> {
    let argv = parse_simple_argv(command)?;
    let (program, args) = argv.split_first()?;
    if program.contains('=') || !is_validation_argv(program, args) {
        return None;
    }
    Some(NormalizedTestCommand {
        program: program.clone(),
        args: args.to_vec(),
        working_directory: ".".to_string(),
    })
}

pub(super) fn normalize_test_command_for_event(
    event: &EventEnvelopeV1,
    command: &str,
) -> Option<NormalizedTestCommand> {
    let mut normalized = normalize_test_command(command)?;
    normalized.working_directory = event_working_directory(event)?;
    Some(normalized)
}

fn event_working_directory(event: &EventEnvelopeV1) -> Option<String> {
    let cwd = ["cwd", "working_directory", "workingDirectory"]
        .into_iter()
        .find_map(|key| event.payload.get(key).and_then(Value::as_str));
    let root = event.payload.get("repository_path").and_then(Value::as_str);
    match (cwd, root) {
        (Some(cwd), Some(root)) => {
            let canonical_cwd = Path::new(cwd).canonicalize().ok();
            let canonical_root = Path::new(root).canonicalize().ok();
            let (cwd, root) = canonical_cwd
                .as_deref()
                .zip(canonical_root.as_deref())
                .unwrap_or((Path::new(cwd), Path::new(root)));
            let relative = cwd.strip_prefix(root).ok()?;
            if relative.as_os_str().is_empty() {
                Some(".".to_string())
            } else {
                crate::git::validated_repository_relative_path(&relative.to_string_lossy())
            }
        }
        (Some(cwd), None) if !Path::new(cwd).is_absolute() => {
            crate::git::validated_repository_relative_path(cwd)
        }
        (None, _) => Some(".".to_string()),
        _ => None,
    }
}

fn parse_simple_argv(command: &str) -> Option<Vec<String>> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Quote {
        None,
        Single,
        Double,
    }

    let mut argv = Vec::new();
    let mut current = String::new();
    let mut quote = Quote::None;
    let mut escaped = false;
    let mut started = false;
    for character in command.chars() {
        if escaped {
            current.push(character);
            started = true;
            escaped = false;
            continue;
        }
        match quote {
            Quote::Single => {
                if character == '\'' {
                    quote = Quote::None;
                } else {
                    current.push(character);
                    started = true;
                }
            }
            Quote::Double => match character {
                '"' => quote = Quote::None,
                '\\' => escaped = true,
                '$' | '`' | '\n' | '\r' => return None,
                _ => {
                    current.push(character);
                    started = true;
                }
            },
            Quote::None => match character {
                '\'' => {
                    quote = Quote::Single;
                    started = true;
                }
                '"' => {
                    quote = Quote::Double;
                    started = true;
                }
                '\\' => escaped = true,
                character if character.is_whitespace() => {
                    if started {
                        argv.push(std::mem::take(&mut current));
                        started = false;
                    }
                }
                ';' | '&' | '|' | '<' | '>' | '$' | '`' | '(' | ')' | '#' | '*' | '?' | '['
                | ']' | '{' | '}' | '~' => return None,
                _ => {
                    current.push(character);
                    started = true;
                }
            },
        }
    }
    if escaped || quote != Quote::None {
        return None;
    }
    if started {
        argv.push(current);
    }
    (!argv.is_empty()).then_some(argv)
}

fn is_validation_argv(program: &str, args: &[String]) -> bool {
    let basename = Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program)
        .to_ascii_lowercase();
    let arg = |index: usize| args.get(index).map(|value| value.to_ascii_lowercase());
    match basename.as_str() {
        "cargo" => {
            matches!(arg(0).as_deref(), Some("test" | "check" | "clippy"))
                || matches!(
                    (arg(0).as_deref(), arg(1).as_deref()),
                    (Some("nextest"), Some("run"))
                )
        }
        "npm" | "pnpm" | "yarn" | "bun" => {
            matches!(
                arg(0).as_deref(),
                Some("test" | "lint" | "typecheck" | "build")
            ) || matches!(
                (arg(0).as_deref(), arg(1).as_deref()),
                (Some("run"), Some("test" | "lint" | "typecheck" | "build"))
            )
        }
        "go" | "swift" => arg(0).as_deref() == Some("test"),
        "pytest" | "py.test" | "jest" | "vitest" => true,
        "bash" | "sh" | "zsh" | "python" | "python3" | "node" => args
            .iter()
            .find(|arg| !arg.starts_with('-'))
            .is_some_and(|script| validation_script_name(script)),
        _ => validation_script_name(&basename),
    }
}

fn validation_script_name(path: &str) -> bool {
    let stem = Path::new(path)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
        .to_ascii_lowercase();
    stem == "test"
        || stem.starts_with("test-")
        || stem.starts_with("test_")
        || stem.ends_with("-test")
        || stem.ends_with("_test")
        || matches!(stem.as_str(), "verify" | "lint" | "typecheck")
}

pub(super) fn shell_display_word(word: &str) -> String {
    if !word.is_empty()
        && word
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_./:@%+=,-".contains(character))
    {
        return word.to_string();
    }
    format!("'{}'", word.replace('\'', "'\\''"))
}
