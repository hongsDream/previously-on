use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use uuid::Uuid;

use crate::domain::{
    deterministic_id, AgentAssociationStateV1, AgentSourceKindV1, AgentV1, ChangeAttribution,
    ChangeStatus, CoverageStatus, CoverageV1, EventEnvelopeV1, EventKind, FileChangeV1,
    SCHEMA_VERSION_V1,
};
use crate::git::{repository_identity, RepositoryIdentity};
use crate::store::Store;
use crate::APP_VERSION;

pub const TESTED_CODEX_VERSION: &str = "0.144.3";
pub const SUPPORTED_CODEX_VERSIONS: [&str; 2] = ["0.144.3", "0.144.2"];
const MAX_PAGES: usize = 1_000;
pub const MAX_APP_SERVER_RPC_BYTES: usize = 8 * 1024 * 1024;
const MALFORMED_TOKEN_USAGE_WARNING: &str =
    "ignored malformed thread/tokenUsage/updated notification; token-usage coverage is degraded";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppServerCapabilityStatus {
    Complete,
    Degraded,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppServerCapabilityReport {
    pub schema_version: u16,
    pub status: AppServerCapabilityStatus,
    pub tested_codex_version: String,
    pub detected_codex_version: Option<String>,
    pub app_server_user_agent: Option<String>,
    pub supported_methods: Vec<String>,
    pub warnings: Vec<String>,
}

impl AppServerCapabilityReport {
    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self {
            schema_version: 1,
            status: AppServerCapabilityStatus::Unsupported,
            tested_codex_version: TESTED_CODEX_VERSION.to_string(),
            detected_codex_version: None,
            app_server_user_agent: None,
            supported_methods: Vec::new(),
            warnings: vec![reason.into()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppServerThreadSummary {
    pub id: String,
    pub session_id: String,
    pub cwd: PathBuf,
    pub cli_version: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub preview: String,
    pub name: Option<String>,
    pub source_kind: Option<String>,
    pub parent_thread_id: Option<String>,
    pub forked_from_id: Option<String>,
    pub status: Option<String>,
    pub coverage: CoverageV1,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionProfileV1 {
    pub id: String,
    pub allowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionProfileListV1 {
    pub schema_version: u16,
    pub profiles: Vec<PermissionProfileV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppServerStartedThreadV1 {
    pub id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppServerStartedTurnV1 {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedThreadV1 {
    pub schema_version: u16,
    pub id: String,
    pub session_id: String,
    pub cwd: PathBuf,
    pub cli_version: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub coverage: CoverageV1,
    /// The documented `thread/read` response. Only documented, allowlisted item variants are
    /// projected, and the raw transcript is never written to the canonical store.
    pub thread: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticThreadProjectionV1 {
    pub schema_version: u16,
    pub events: Vec<EventEnvelopeV1>,
    pub coverage: CoverageV1,
}

/// A validated `thread/tokenUsage/updated` notification payload.
///
/// The App Server client collects the latest valid notification observed while it waits for an RPC
/// response. Imports attach that bounded value to the matching thread without inferring token usage
/// from transcript size or turn count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppServerTokenUsageNotificationV1 {
    pub thread_id: String,
    pub turn_id: String,
    pub token_usage: AppServerThreadTokenUsageV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppServerThreadTokenUsageV1 {
    pub last: AppServerTokenUsageBreakdownV1,
    pub total: AppServerTokenUsageBreakdownV1,
    pub model_context_window: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppServerTokenUsageBreakdownV1 {
    pub cached_input_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
}

/// Parse the documented server notification without collecting or persisting it directly.
///
/// `Ok(None)` means the JSON-RPC message is not a token-usage notification. A matching method with
/// malformed parameters is an error so callers can degrade coverage instead of silently guessing.
pub fn parse_token_usage_notification(
    message: &Value,
) -> Result<Option<AppServerTokenUsageNotificationV1>> {
    if message.get("method").and_then(Value::as_str) != Some("thread/tokenUsage/updated") {
        return Ok(None);
    }
    let params = message
        .get("params")
        .cloned()
        .context("thread/tokenUsage/updated notification omitted params")?;
    let parsed = serde_json::from_value(params)
        .context("invalid thread/tokenUsage/updated notification params")?;
    Ok(Some(parsed))
}

/// Project the documented, stable `thread/read` item variants into bounded canonical events.
///
/// This deliberately ignores reasoning, plan, image, and tool-detail payloads. Text, command
/// output, and file metadata are redacted and capped before they cross the storage boundary; the
/// raw App Server response must never be passed to the canonical store.
pub fn project_thread_events(
    thread: &ImportedThreadV1,
    repository_id: &str,
    repository_root: &Path,
) -> SemanticThreadProjectionV1 {
    let mut coverage = thread.coverage.clone();
    let mut events = Vec::new();
    let created_at = timestamp_seconds(thread.created_at);
    let updated_at = timestamp_seconds(thread.updated_at);
    let repository_path = repository_root.to_string_lossy().to_string();
    let turn_count = thread
        .thread
        .get("turns")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();

    events.push(semantic_event(
        format!("codex-app-server:thread:{}:start", thread.id),
        repository_id,
        &thread.session_id,
        EventKind::SessionStarted,
        created_at,
        0,
        json!({
            "thread_id": thread.id,
            "source_thread_id": thread.id,
            "session_id": thread.session_id,
            "repository_path": repository_path,
            "thread_created_at": created_at,
            "thread_updated_at": updated_at,
            "turn_count": turn_count,
            "app_server_source": "thread/read",
            "untrusted_data": true,
            "raw_transcript_stored": false
        }),
        &coverage,
    ));

    if let Some(notification) = thread
        .thread
        .get("_previously_token_usage")
        .and_then(|value| {
            serde_json::from_value::<AppServerTokenUsageNotificationV1>(value.clone()).ok()
        })
    {
        events.push(semantic_event(
            format!(
                "codex-app-server:thread:{}:turn:{}:token-usage",
                thread.id, notification.turn_id
            ),
            repository_id,
            &thread.session_id,
            EventKind::ContextUsageUpdated,
            updated_at,
            1,
            json!({
                "thread_id": thread.id,
                "source_thread_id": thread.id,
                "turn_id": notification.turn_id,
                "session_id": thread.session_id,
                "repository_path": repository_path,
                "context_usage": {
                    "total_tokens": notification.token_usage.last.total_tokens,
                    "model_context_window": notification.token_usage.model_context_window
                },
                "app_server_source": "thread/tokenUsage/updated",
                "untrusted_data": true,
                "raw_transcript_stored": false
            }),
            &coverage,
        ));
    }

    let mut saw_prompt = false;
    let mut saw_final = false;
    let mut latest_final = None;
    let Some(turns) = thread.thread.get("turns").and_then(Value::as_array) else {
        degrade(
            &mut coverage,
            "thread turns",
            "thread/read omitted a turns array; only thread lifecycle metadata was projected",
        );
        return finish_semantic_projection(
            thread,
            repository_id,
            repository_root,
            updated_at,
            events,
            coverage,
            None,
        );
    };

    for (turn_index, turn) in turns.iter().enumerate() {
        let Some(turn_object) = turn.as_object() else {
            degrade(
                &mut coverage,
                "well-formed turn",
                format!("turn {turn_index} was not an object and was not projected"),
            );
            continue;
        };
        let Some(turn_id) = turn_object
            .get("id")
            .and_then(Value::as_str)
            .filter(|turn_id| !turn_id.is_empty())
        else {
            degrade(
                &mut coverage,
                "stable turn source ID",
                format!("turn {turn_index} omitted a stable ID and was not projected"),
            );
            continue;
        };
        let Some(items) = turn_object.get("items").and_then(Value::as_array) else {
            degrade(
                &mut coverage,
                "turn items",
                format!("turn {turn_index} omitted an items array"),
            );
            continue;
        };
        let mut turn_final: Option<(String, String, i64, DateTime<Utc>, bool)> = None;
        for (item_index, item) in items.iter().enumerate() {
            let Some(item_object) = item.as_object() else {
                degrade(
                    &mut coverage,
                    "well-formed thread item",
                    format!("turn {turn_index} item {item_index} was not an object"),
                );
                continue;
            };
            let Some(item_id) = item_object
                .get("id")
                .and_then(Value::as_str)
                .filter(|item_id| !item_id.is_empty())
            else {
                degrade(
                    &mut coverage,
                    "stable item source ID",
                    format!(
                        "turn {turn_index} item {item_index} omitted a stable ID and was not projected"
                    ),
                );
                continue;
            };
            let Some(item_type) = item_object.get("type").and_then(Value::as_str) else {
                degrade(
                    &mut coverage,
                    "known thread item schema",
                    format!("turn {turn_index} item {item_index} omitted its type"),
                );
                continue;
            };
            let sequence = 1 + (turn_index as i64 * 10_000) + item_index as i64;
            let occurred_at = item_timestamp(item)
                .unwrap_or_else(|| created_at + Duration::microseconds(sequence));
            match item_type {
                "userMessage" => {
                    if let Some(prompt) = bounded_message_content(item_object.get("content")) {
                        saw_prompt = true;
                        events.push(semantic_event(
                            format!(
                                "codex-app-server:thread:{}:item:{}:user-message",
                                thread.id, item_id
                            ),
                            repository_id,
                            &thread.session_id,
                            EventKind::UserPrompt,
                            occurred_at,
                            sequence,
                            json!({
                                "thread_id": thread.id,
                                "turn_id": turn_id,
                                "item_id": item_id,
                                "session_id": thread.session_id,
                                "repository_path": repository_path,
                                "prompt": prompt,
                                "app_server_item_type": item_type,
                                "untrusted_data": true,
                                "raw_transcript_stored": false
                            }),
                            &coverage,
                        ));
                    } else {
                        degrade(
                            &mut coverage,
                            "text user prompt",
                            format!("userMessage item `{item_id}` contained no text to project"),
                        );
                    }
                }
                "agentMessage" => {
                    if let Some(text) = item_object
                        .get("text")
                        .and_then(Value::as_str)
                        .map(crate::redaction::redact_excerpt)
                        .filter(|text| !text.trim().is_empty())
                    {
                        let explicit_final = item_object.get("phase").and_then(Value::as_str)
                            == Some("final_answer");
                        if explicit_final
                            || turn_final.as_ref().is_none_or(|candidate| !candidate.4)
                        {
                            turn_final = Some((
                                item_id.to_string(),
                                text,
                                sequence,
                                occurred_at,
                                explicit_final,
                            ));
                        }
                    }
                }
                "commandExecution" => {
                    let Some(command) = item_object
                        .get("command")
                        .and_then(Value::as_str)
                        .map(crate::redaction::redact_excerpt)
                        .filter(|command| !command.trim().is_empty())
                    else {
                        degrade(
                            &mut coverage,
                            "command execution command",
                            format!("commandExecution item `{item_id}` omitted command text"),
                        );
                        continue;
                    };
                    let output = item_object
                        .get("aggregatedOutput")
                        .and_then(Value::as_str)
                        .map(crate::redaction::redact_excerpt);
                    events.push(semantic_event(
                        format!(
                            "codex-app-server:thread:{}:item:{}:command-execution",
                            thread.id, item_id
                        ),
                        repository_id,
                        &thread.session_id,
                        EventKind::ToolFinished,
                        occurred_at,
                        sequence,
                        json!({
                            "thread_id": thread.id,
                            "turn_id": turn_id,
                            "item_id": item_id,
                            "session_id": thread.session_id,
                            "repository_path": repository_path,
                            "tool_name": "Bash",
                            "tool_use_id": item_id,
                            "tool_input": {"command": command},
                            "tool_response": {
                                "status": item_object.get("status").cloned(),
                                "exit_code": item_object.get("exitCode").cloned(),
                                "output": output
                            },
                            "app_server_item_type": item_type,
                            "untrusted_data": true,
                            "raw_transcript_stored": false
                        }),
                        &coverage,
                    ));
                }
                "fileChange" => {
                    let changes = project_file_changes(
                        item_object.get("changes"),
                        repository_id,
                        &thread.session_id,
                        repository_root,
                        &mut coverage,
                    );
                    if changes.is_empty() {
                        degrade(
                            &mut coverage,
                            "file change metadata",
                            format!("fileChange item `{item_id}` contained no usable paths"),
                        );
                        continue;
                    }
                    events.push(semantic_event(
                        format!(
                            "codex-app-server:thread:{}:item:{}:file-change",
                            thread.id, item_id
                        ),
                        repository_id,
                        &thread.session_id,
                        EventKind::ToolFinished,
                        occurred_at,
                        sequence,
                        json!({
                            "thread_id": thread.id,
                            "turn_id": turn_id,
                            "item_id": item_id,
                            "session_id": thread.session_id,
                            "repository_path": repository_path,
                            "tool_name": "CodexAppServerFileChange",
                            "tool_use_id": item_id,
                            "tool_input": {
                                "path": changes.first().map(|change| change.path.clone()),
                                "change_count": changes.len()
                            },
                            "tool_response": {"status": item_object.get("status").cloned()},
                            "file_changes": changes,
                            "attribution_policy": "observed_changed_in",
                            "app_server_item_type": item_type,
                            "untrusted_data": true,
                            "raw_transcript_stored": false
                        }),
                        &coverage,
                    ));
                }
                "contextCompaction" => {
                    events.push(semantic_event(
                        format!(
                            "codex-app-server:thread:{}:item:{}:context-compaction",
                            thread.id, item_id
                        ),
                        repository_id,
                        &thread.session_id,
                        EventKind::ContextCompaction,
                        occurred_at,
                        sequence,
                        json!({
                            "thread_id": thread.id,
                            "turn_id": turn_id,
                            "item_id": item_id,
                            "session_id": thread.session_id,
                            "repository_path": repository_path,
                            "app_server_item_type": item_type,
                            "untrusted_data": true,
                            "raw_transcript_stored": false
                        }),
                        &coverage,
                    ));
                }
                item_type if !is_known_thread_item_type(item_type) => degrade(
                    &mut coverage,
                    "known thread item schema",
                    format!(
                        "turn {turn_index} item {item_index} has unknown type `{item_type}` and was not projected"
                    ),
                ),
                _ => {}
            }
        }
        if let Some((item_id, text, sequence, occurred_at, explicit_final)) = turn_final {
            if !explicit_final {
                degrade(
                    &mut coverage,
                    "assistant final phase",
                    format!(
                        "agentMessage item `{item_id}` omitted `phase: final_answer`; the last agent message in its turn was used"
                    ),
                );
            }
            saw_final = true;
            latest_final = Some(text.clone());
            events.push(semantic_event(
                format!(
                    "codex-app-server:thread:{}:item:{}:assistant-final",
                    thread.id, item_id
                ),
                repository_id,
                &thread.session_id,
                EventKind::AssistantFinal,
                occurred_at,
                sequence,
                json!({
                    "thread_id": thread.id,
                    "turn_id": turn_id,
                    "item_id": item_id,
                    "session_id": thread.session_id,
                    "repository_path": repository_path,
                    "last_assistant_message": text,
                    "app_server_item_type": "agentMessage",
                    "untrusted_data": true,
                    "raw_transcript_stored": false
                }),
                &coverage,
            ));
        }
    }

    if !saw_prompt {
        degrade(
            &mut coverage,
            "user prompt",
            "no text userMessage item was available for semantic reconstruction",
        );
    }
    if !saw_final {
        degrade(
            &mut coverage,
            "assistant final",
            "no agentMessage item was available for semantic reconstruction",
        );
    }
    finish_semantic_projection(
        thread,
        repository_id,
        repository_root,
        updated_at,
        events,
        coverage,
        latest_final,
    )
}

fn finish_semantic_projection(
    thread: &ImportedThreadV1,
    repository_id: &str,
    repository_root: &Path,
    updated_at: DateTime<Utc>,
    mut events: Vec<EventEnvelopeV1>,
    coverage: CoverageV1,
    latest_final: Option<String>,
) -> SemanticThreadProjectionV1 {
    let sequence = i64::MAX - 1;
    let turn_count = thread
        .thread
        .get("turns")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    let created_at = timestamp_seconds(thread.created_at);
    events.push(semantic_event(
        format!("codex-app-server:thread:{}:stop", thread.id),
        repository_id,
        &thread.session_id,
        EventKind::SessionStopped,
        updated_at,
        sequence,
        json!({
            "thread_id": thread.id,
            "source_thread_id": thread.id,
            "session_id": thread.session_id,
            "repository_path": repository_root.to_string_lossy(),
            "thread_created_at": created_at,
            "thread_updated_at": updated_at,
            "turn_count": turn_count,
            "last_assistant_message": latest_final,
            "app_server_source": "thread/read",
            "untrusted_data": true,
            "raw_transcript_stored": false
        }),
        &coverage,
    ));
    for event in &mut events {
        event.coverage = CoverageV1::merge([&event.coverage, &coverage]);
        event.coverage.captured.extend([
            "thread/read semantic projection".to_string(),
            "stable source id".to_string(),
        ]);
        event.coverage.captured.sort();
        event.coverage.captured.dedup();
    }
    SemanticThreadProjectionV1 {
        schema_version: SCHEMA_VERSION_V1,
        events,
        coverage,
    }
}

#[allow(clippy::too_many_arguments)]
fn semantic_event(
    source_id: String,
    repository_id: &str,
    session_id: &str,
    kind: EventKind,
    occurred_at: DateTime<Utc>,
    sequence: i64,
    payload: Value,
    coverage: &CoverageV1,
) -> EventEnvelopeV1 {
    let mut event = EventEnvelopeV1::new(
        &source_id,
        repository_id,
        session_id,
        kind,
        occurred_at,
        payload,
    );
    event.dedupe_key = source_id.clone();
    event.event_id = format!(
        "evt-{}",
        &hex::encode(Sha256::digest(source_id.as_bytes()))[..24]
    );
    event.sequence = Some(sequence);
    event.coverage = coverage.clone();
    event
}

fn timestamp_seconds(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(seconds, 0)
        .single()
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
}

fn item_timestamp(item: &Value) -> Option<DateTime<Utc>> {
    for key in ["createdAt", "updatedAt", "timestamp"] {
        if let Some(seconds) = item.get(key).and_then(Value::as_i64) {
            return Utc.timestamp_opt(seconds, 0).single();
        }
        if let Some(timestamp) = item.get(key).and_then(Value::as_str) {
            if let Ok(timestamp) = DateTime::parse_from_rfc3339(timestamp) {
                return Some(timestamp.with_timezone(&Utc));
            }
        }
    }
    None
}

fn bounded_message_content(content: Option<&Value>) -> Option<String> {
    let content = content?;
    let text = match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                let object = part.as_object()?;
                (object.get("type").and_then(Value::as_str) == Some("text"))
                    .then(|| object.get("text").and_then(Value::as_str))
                    .flatten()
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    (!text.trim().is_empty()).then(|| crate::redaction::redact_excerpt(&text))
}

fn project_file_changes(
    changes: Option<&Value>,
    repository_id: &str,
    session_id: &str,
    repository_root: &Path,
    coverage: &mut CoverageV1,
) -> Vec<FileChangeV1> {
    let mut projected = Vec::new();
    let mut rejected = 0_usize;
    for change in changes.and_then(Value::as_array).into_iter().flatten() {
        let Some(raw_path) = change.get("path").and_then(Value::as_str) else {
            rejected += 1;
            continue;
        };
        let Some(path) = crate::git::validated_repository_relative_path(raw_path) else {
            rejected += 1;
            continue;
        };
        if path != raw_path {
            rejected += 1;
            continue;
        }
        let previous_path = match change.get("previousPath").and_then(Value::as_str) {
            Some(raw_previous) => {
                let Some(previous) = crate::git::validated_repository_relative_path(raw_previous)
                else {
                    rejected += 1;
                    continue;
                };
                if previous != raw_previous {
                    rejected += 1;
                    continue;
                }
                Some(previous)
            }
            None => None,
        };
        // The lexical validation above is the durable storage boundary. Joining it to the
        // supplied repository root must remain contained even when the imported item refers to a
        // file that no longer exists.
        let joined = repository_root.join(&path);
        if !joined.starts_with(repository_root) {
            rejected += 1;
            continue;
        }
        let status = match change
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "add" | "added" | "create" | "created" => ChangeStatus::Added,
            "delete" | "deleted" | "remove" | "removed" => ChangeStatus::Deleted,
            "rename" | "renamed" | "move" | "moved" => ChangeStatus::Renamed,
            "update" | "updated" | "modify" | "modified" => ChangeStatus::Modified,
            _ => ChangeStatus::Unknown,
        };
        projected.push(FileChangeV1 {
            schema_version: SCHEMA_VERSION_V1,
            repository_id: repository_id.to_string(),
            session_id: session_id.to_string(),
            task_id: None,
            path,
            previous_path,
            status,
            additions: None,
            deletions: None,
            attribution: ChangeAttribution::ObservedChangedIn,
            before_head: None,
            after_head: None,
        });
    }
    if rejected > 0 {
        degrade(
            coverage,
            "safe repository-relative file change path",
            format!(
                "ignored {rejected} imported file change path(s) that were sensitive, absolute, parent-traversing, or non-normal"
            ),
        );
    }
    projected
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppServerRpcError {
    pub method: String,
    pub code: Option<i64>,
    pub message: String,
    pub data: Option<Value>,
}

impl fmt::Display for AppServerRpcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.code {
            Some(code) => write!(
                formatter,
                "Codex app-server {} error {code}: {}",
                self.method, self.message
            ),
            None => write!(
                formatter,
                "Codex app-server {} error: {}",
                self.method, self.message
            ),
        }
    }
}

impl std::error::Error for AppServerRpcError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadImportDisposition {
    Imported,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadImportNoticeV1 {
    pub thread_id: Option<String>,
    pub disposition: ThreadImportDisposition,
    pub coverage: CoverageV1,
    pub message: String,
    pub rpc_error: Option<AppServerRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadListReportV1 {
    pub schema_version: u16,
    pub threads: Vec<AppServerThreadSummary>,
    pub coverage: CoverageV1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadImportReportV1 {
    pub schema_version: u16,
    pub threads: Vec<ImportedThreadV1>,
    pub notices: Vec<ThreadImportNoticeV1>,
    pub coverage: CoverageV1,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadPage {
    data: Vec<Value>,
    next_cursor: Option<String>,
}

pub struct AppServerClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    initialize_result: Value,
    token_usage_notifications: BTreeMap<String, AppServerTokenUsageNotificationV1>,
    collector_warnings: Vec<String>,
    experimental_api: bool,
}

impl AppServerClient {
    pub async fn connect() -> Result<Self> {
        Self::connect_with_program("codex").await
    }

    pub async fn connect_with_program(program: impl AsRef<Path>) -> Result<Self> {
        Self::connect_with_program_and_experimental(program, false).await
    }

    pub async fn connect_experimental() -> Result<Self> {
        Self::connect_with_program_and_experimental("codex", true).await
    }

    pub async fn connect_with_program_experimental(program: impl AsRef<Path>) -> Result<Self> {
        Self::connect_with_program_and_experimental(program, true).await
    }

    async fn connect_with_program_and_experimental(
        program: impl AsRef<Path>,
        experimental_api: bool,
    ) -> Result<Self> {
        let mut command = Command::new(program.as_ref());
        command
            .arg("app-server")
            .arg("--stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if experimental_api {
            for (name, _) in std::env::vars_os() {
                if is_sensitive_environment_name(&name) {
                    command.env_remove(name);
                }
            }
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("start {} app-server", program.as_ref().display()))?;
        let stdin = child
            .stdin
            .take()
            .context("Codex app-server stdin unavailable")?;
        let stdout = child
            .stdout
            .take()
            .context("Codex app-server stdout unavailable")?;
        let mut client = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            initialize_result: Value::Null,
            token_usage_notifications: BTreeMap::new(),
            collector_warnings: Vec::new(),
            experimental_api,
        };
        let initialized = client
            .request_timed(
                "initialize",
                json!({
                    "clientInfo": {
                        "name": "previously-on",
                        "title": "PreviouslyOn",
                        "version": APP_VERSION
                    },
                    "capabilities": {
                        "experimentalApi": experimental_api,
                        "requestAttestation": false
                    }
                }),
            )
            .await
            .context("initialize Codex app-server")?;
        client.initialize_result = initialized;
        client.notify("initialized", json!({})).await?;
        Ok(client)
    }

    pub fn experimental_api(&self) -> bool {
        self.experimental_api
    }

    pub fn initialize_result(&self) -> &Value {
        &self.initialize_result
    }

    pub async fn capability_report(&mut self) -> AppServerCapabilityReport {
        let detected_codex_version = codex_version().await.ok();
        let app_server_user_agent = self
            .initialize_result
            .get("userAgent")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut warnings = Vec::new();
        let status = match detected_codex_version.as_deref() {
            Some(version) if SUPPORTED_CODEX_VERSIONS.contains(&version) => {
                AppServerCapabilityStatus::Complete
            }
            Some(version) => {
                warnings.push(format!(
                    "Codex {version} differs from supported schemas {}",
                    SUPPORTED_CODEX_VERSIONS.join(", ")
                ));
                AppServerCapabilityStatus::Degraded
            }
            None => {
                warnings.push("unable to determine Codex version".to_string());
                AppServerCapabilityStatus::Degraded
            }
        };
        AppServerCapabilityReport {
            schema_version: 1,
            status,
            tested_codex_version: TESTED_CODEX_VERSION.to_string(),
            detected_codex_version,
            app_server_user_agent,
            supported_methods: vec![
                "initialize".to_string(),
                "initialized".to_string(),
                "thread/list".to_string(),
                "thread/read".to_string(),
                "thread/start".to_string(),
                "thread/resume".to_string(),
                "thread/name/set".to_string(),
                "turn/start".to_string(),
            ],
            warnings,
        }
    }

    pub async fn list_threads(
        &mut self,
        cwd: Option<&Path>,
    ) -> Result<Vec<AppServerThreadSummary>> {
        Ok(self.list_threads_report(cwd).await?.threads)
    }

    /// Lists all readable summaries while isolating schema and pagination faults.
    ///
    /// A malformed later page or repeated cursor stops pagination and marks coverage degraded;
    /// summaries already validated remain usable. This prevents one bad page from contaminating
    /// or discarding the rest of an import.
    pub async fn list_threads_report(&mut self, cwd: Option<&Path>) -> Result<ThreadListReportV1> {
        self.list_threads_report_with_sources(cwd, None).await
    }

    pub async fn list_lineage_threads_report(
        &mut self,
        cwd: Option<&Path>,
    ) -> Result<ThreadListReportV1> {
        if !self.experimental_api {
            bail!("agent lineage requires experimentalApi capability");
        }
        self.list_threads_report_with_sources(
            cwd,
            Some(&[
                "cli",
                "vscode",
                "exec",
                "appServer",
                "subAgent",
                "subAgentReview",
                "subAgentCompact",
                "subAgentThreadSpawn",
                "subAgentOther",
            ]),
        )
        .await
    }

    async fn list_threads_report_with_sources(
        &mut self,
        cwd: Option<&Path>,
        source_kinds: Option<&[&str]>,
    ) -> Result<ThreadListReportV1> {
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();
        let mut threads = Vec::new();
        let mut coverage = complete_coverage(["thread/list"]);
        for _ in 0..MAX_PAGES {
            let mut params = json!({
                "limit": 100,
                "sortKey": "created_at",
                "sortDirection": "desc",
                "useStateDbOnly": false
            });
            if let Some(cursor) = &cursor {
                params["cursor"] = json!(cursor);
            }
            if let Some(cwd) = cwd {
                params["cwd"] = json!(cwd.to_string_lossy());
            }
            if let Some(source_kinds) = source_kinds {
                params["sourceKinds"] = json!(source_kinds);
            }
            let response = match self.request_timed("thread/list", params).await {
                Ok(response) => response,
                Err(error) => {
                    degrade(
                        &mut coverage,
                        "thread/list page",
                        format!("Codex thread/list stopped safely: {error}"),
                    );
                    break;
                }
            };
            let page: ThreadPage = match serde_json::from_value(response) {
                Ok(page) => page,
                Err(error) => {
                    degrade(
                        &mut coverage,
                        "thread/list page",
                        format!("invalid Codex thread/list response; pagination stopped: {error}"),
                    );
                    break;
                }
            };
            for thread in page.data {
                match parse_thread_summary(thread) {
                    Ok(thread) => threads.push(thread),
                    Err(error) => degrade(
                        &mut coverage,
                        "thread/list item",
                        format!("malformed thread summary skipped: {error}"),
                    ),
                }
            }
            match page.next_cursor {
                None => {
                    self.apply_collector_warnings(&mut coverage);
                    return Ok(ThreadListReportV1 {
                        schema_version: 1,
                        threads,
                        coverage,
                    });
                }
                Some(next) if !seen_cursors.insert(next.clone()) => {
                    degrade(
                        &mut coverage,
                        "thread/list pagination",
                        format!("Codex thread/list repeated cursor `{next}`; pagination stopped"),
                    );
                    break;
                }
                Some(next) => cursor = Some(next),
            }
        }
        if coverage.status == CoverageStatus::Complete {
            degrade(
                &mut coverage,
                "thread/list pagination",
                format!("Codex thread/list exceeded {MAX_PAGES} pages"),
            );
        }
        self.apply_collector_warnings(&mut coverage);
        Ok(ThreadListReportV1 {
            schema_version: 1,
            threads,
            coverage,
        })
    }

    pub async fn list_permission_profiles(
        &mut self,
        cwd: &Path,
    ) -> Result<PermissionProfileListV1> {
        if !self.experimental_api {
            bail!("permissionProfile/list requires experimentalApi capability");
        }
        if !cwd.is_absolute() {
            bail!("permissionProfile/list requires an absolute cwd");
        }
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        let mut profiles = Vec::new();
        for _ in 0..MAX_PAGES {
            let mut params = json!({ "cwd": cwd.to_string_lossy(), "limit": 100 });
            if let Some(value) = cursor.as_ref() {
                params["cursor"] = json!(value);
            }
            let result = self.request_timed("permissionProfile/list", params).await?;
            let page: ThreadPage = serde_json::from_value(result)
                .context("invalid permissionProfile/list response")?;
            for raw in page.data {
                let id = raw
                    .get("id")
                    .or_else(|| raw.get("name"))
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .context("permission profile omitted id")?
                    .to_string();
                let allowed = raw
                    .get("allowed")
                    .or_else(|| raw.get("allowedByRequirements"))
                    .or_else(|| raw.get("isAllowed"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                profiles.push(PermissionProfileV1 { id, allowed });
            }
            match page.next_cursor {
                None => {
                    profiles.sort_by(|left, right| left.id.cmp(&right.id));
                    profiles.dedup_by(|left, right| left.id == right.id);
                    return Ok(PermissionProfileListV1 {
                        schema_version: SCHEMA_VERSION_V1,
                        profiles,
                    });
                }
                Some(next) if !seen.insert(next.clone()) => {
                    bail!("permissionProfile/list repeated cursor `{next}`")
                }
                Some(next) => cursor = Some(next),
            }
        }
        bail!("permissionProfile/list exceeded {MAX_PAGES} pages")
    }

    pub async fn read_thread(&mut self, thread_id: &str) -> Result<Value> {
        let result = self
            .request_timed(
                "thread/read",
                json!({ "threadId": thread_id, "includeTurns": true }),
            )
            .await
            .with_context(|| format!("Codex thread/read failed for {thread_id}"))?;
        result
            .get("thread")
            .cloned()
            .context("Codex thread/read response omitted `thread`")
    }

    /// Starts a persisted Codex task through the documented App Server interface.
    ///
    /// Approval and sandbox fields are intentionally omitted so the new task inherits the user's
    /// effective Codex configuration instead of PreviouslyOn silently widening permissions.
    pub async fn start_thread(
        &mut self,
        cwd: &Path,
        model: Option<&str>,
    ) -> Result<AppServerStartedThreadV1> {
        if !cwd.is_absolute() {
            bail!("Codex thread/start requires an absolute cwd");
        }
        let mut params = json!({
            "cwd": cwd.to_string_lossy(),
            "ephemeral": false,
            "serviceName": "previously-on"
        });
        if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
            params["model"] = json!(model);
        }
        let result = self
            .request_timed("thread/start", params)
            .await
            .context("Codex thread/start failed")?;
        let thread = result
            .get("thread")
            .context("Codex thread/start response omitted `thread`")?;
        let id = thread
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .context("Codex thread/start response omitted `thread.id`")?
            .to_string();
        let session_id = thread
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or(&id)
            .to_string();
        Ok(AppServerStartedThreadV1 { id, session_id })
    }

    /// Starts the user-triggered AI fact refresh in an isolated cwd with the verified named
    /// permission profile. The legacy `sandbox` field is deliberately never present.
    pub async fn start_ephemeral_thread_with_permissions(
        &mut self,
        cwd: &Path,
        permission_profile: &str,
    ) -> Result<AppServerStartedThreadV1> {
        if !self.experimental_api {
            bail!("named permissions require experimentalApi capability");
        }
        if !cwd.is_absolute() || permission_profile.trim().is_empty() {
            bail!("named-permission thread/start requires absolute cwd and profile id");
        }
        let result = self
            .request_timed(
                "thread/start",
                json!({
                    "cwd": cwd.to_string_lossy(),
                    "ephemeral": true,
                    "serviceName": "previously-on-fact-refresh",
                    "permissions": permission_profile,
                    "approvalPolicy": "never"
                }),
            )
            .await
            .context("Codex named-permission thread/start failed")?;
        let thread = result
            .get("thread")
            .context("Codex thread/start response omitted `thread`")?;
        let id = thread
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .context("Codex thread/start response omitted `thread.id`")?
            .to_string();
        let session_id = thread
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or(&id)
            .to_string();
        Ok(AppServerStartedThreadV1 { id, session_id })
    }

    pub async fn resume_thread(&mut self, thread_id: &str) -> Result<AppServerStartedThreadV1> {
        if thread_id.is_empty() {
            bail!("Codex thread/resume requires a thread id");
        }
        let result = self
            .request_timed("thread/resume", json!({ "threadId": thread_id }))
            .await
            .context("Codex thread/resume failed")?;
        let thread = result
            .get("thread")
            .context("Codex thread/resume response omitted `thread`")?;
        let id = thread
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .context("Codex thread/resume response omitted `thread.id`")?
            .to_string();
        let session_id = thread
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or(&id)
            .to_string();
        Ok(AppServerStartedThreadV1 { id, session_id })
    }

    pub async fn set_thread_name(&mut self, thread_id: &str, name: &str) -> Result<()> {
        let name = crate::redaction::redact_excerpt(name.trim());
        if thread_id.is_empty() || name.is_empty() {
            bail!("Codex thread/name/set requires a thread id and non-empty name");
        }
        self.request_timed(
            "thread/name/set",
            json!({ "threadId": thread_id, "name": name }),
        )
        .await
        .context("Codex thread/name/set failed")?;
        Ok(())
    }

    pub async fn start_turn(
        &mut self,
        thread_id: &str,
        cwd: &Path,
        prompt: &str,
        model: Option<&str>,
        client_user_message_id: &str,
    ) -> Result<AppServerStartedTurnV1> {
        if thread_id.is_empty() || client_user_message_id.is_empty() {
            bail!("Codex turn/start requires stable thread and client message ids");
        }
        if !cwd.is_absolute() {
            bail!("Codex turn/start requires an absolute cwd");
        }
        if prompt.trim().is_empty() {
            bail!("Codex turn/start requires non-empty input");
        }
        let mut params = json!({
            "threadId": thread_id,
            "cwd": cwd.to_string_lossy(),
            "clientUserMessageId": client_user_message_id,
            "input": [{ "type": "text", "text": prompt }]
        });
        if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
            params["model"] = json!(model);
        }
        let result = self
            .request_timed("turn/start", params)
            .await
            .context("Codex turn/start failed")?;
        let id = result
            .get("turn")
            .and_then(|turn| turn.get("id"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .context("Codex turn/start response omitted `turn.id`")?
            .to_string();
        Ok(AppServerStartedTurnV1 { id })
    }

    pub async fn start_structured_fact_refresh_turn(
        &mut self,
        thread_id: &str,
        cwd: &Path,
        prompt: &str,
        client_user_message_id: &str,
        output_schema: Value,
    ) -> Result<AppServerStartedTurnV1> {
        if !self.experimental_api {
            bail!("structured fact refresh requires experimentalApi capability");
        }
        if thread_id.is_empty() || client_user_message_id.is_empty() || !cwd.is_absolute() {
            bail!("structured turn/start requires stable ids and absolute cwd");
        }
        if prompt.trim().is_empty() {
            bail!("structured turn/start requires non-empty input");
        }
        let result = self
            .request_timed(
                "turn/start",
                json!({
                    "threadId": thread_id,
                    "cwd": cwd.to_string_lossy(),
                    "clientUserMessageId": client_user_message_id,
                    "input": [{ "type": "text", "text": prompt }],
                    "effort": "medium",
                    "outputSchema": output_schema
                }),
            )
            .await
            .context("Codex structured turn/start failed")?;
        let id = result
            .get("turn")
            .and_then(|turn| turn.get("id"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .context("Codex turn/start response omitted `turn.id`")?
            .to_string();
        Ok(AppServerStartedTurnV1 { id })
    }

    pub async fn import_threads(&mut self, cwd: &Path) -> Result<Vec<ImportedThreadV1>> {
        Ok(self.import_threads_report(cwd).await?.threads)
    }

    pub async fn import_threads_report(&mut self, cwd: &Path) -> Result<ThreadImportReportV1> {
        let registered_repository = repository_identity(cwd).with_context(|| {
            format!(
                "identify registered Git repository before Codex App Server import: {}",
                cwd.display()
            )
        })?;
        let listed = self.list_threads_report(Some(cwd)).await?;
        let mut imported = Vec::with_capacity(listed.threads.len());
        let mut notices = Vec::new();
        let mut coverages = vec![listed.coverage.clone()];
        for summary in listed.threads {
            if let Err(warning) = validate_thread_repository(&summary.cwd, &registered_repository) {
                let mut coverage = summary.coverage.clone();
                degrade(
                    &mut coverage,
                    "matching repository cwd",
                    warning.to_string(),
                );
                coverages.push(coverage.clone());
                notices.push(ThreadImportNoticeV1 {
                    thread_id: Some(summary.id),
                    disposition: ThreadImportDisposition::Skipped,
                    message: coverage.warnings.join("; "),
                    coverage,
                    rpc_error: None,
                });
                continue;
            }
            let mut thread = match self.read_thread(&summary.id).await {
                Ok(thread) => thread,
                Err(error) => {
                    let rpc_error = error.downcast_ref::<AppServerRpcError>().cloned();
                    let deleted = rpc_error.as_ref().is_some_and(is_deleted_thread_error);
                    let mut coverage = complete_coverage(std::iter::empty::<&str>());
                    degrade(
                        &mut coverage,
                        "thread/read",
                        if deleted {
                            "thread was deleted before import and was skipped".to_string()
                        } else {
                            format!("thread/read failed; thread skipped: {error}")
                        },
                    );
                    self.apply_collector_warnings(&mut coverage);
                    coverages.push(coverage.clone());
                    notices.push(ThreadImportNoticeV1 {
                        thread_id: Some(summary.id),
                        disposition: ThreadImportDisposition::Skipped,
                        message: coverage.warnings.join("; "),
                        coverage,
                        rpc_error,
                    });
                    continue;
                }
            };
            if let Some(notification) = self.token_usage_notifications.remove(&summary.id) {
                if let Some(object) = thread.as_object_mut() {
                    object.insert(
                        "_previously_token_usage".to_string(),
                        serde_json::to_value(notification)?,
                    );
                }
            }
            let (thread, mut thread_coverage) =
                normalize_and_assess_thread(thread, &summary.cli_version);
            self.apply_collector_warnings(&mut thread_coverage);
            let coverage = CoverageV1::merge([&summary.coverage, &thread_coverage]);
            if coverage.status != CoverageStatus::Complete {
                notices.push(ThreadImportNoticeV1 {
                    thread_id: Some(summary.id.clone()),
                    disposition: ThreadImportDisposition::Imported,
                    message: coverage.warnings.join("; "),
                    coverage: coverage.clone(),
                    rpc_error: None,
                });
            }
            coverages.push(coverage.clone());
            imported.push(ImportedThreadV1 {
                schema_version: 1,
                id: summary.id,
                session_id: summary.session_id,
                cwd: summary.cwd,
                cli_version: summary.cli_version,
                created_at: summary.created_at,
                updated_at: summary.updated_at,
                coverage,
                thread,
            });
        }
        let coverage = CoverageV1::merge(coverages.iter());
        Ok(ThreadImportReportV1 {
            schema_version: 1,
            threads: imported,
            notices,
            coverage,
        })
    }

    pub async fn shutdown(mut self) -> Result<()> {
        self.stdin.shutdown().await.ok();
        match tokio::time::timeout(std::time::Duration::from_secs(2), self.child.wait()).await {
            Ok(result) => {
                result?;
            }
            Err(_) => {
                self.child.kill().await.ok();
            }
        }
        Ok(())
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))
        .await?;
        loop {
            let line = match crate::bounded_io::read_bounded_line_async(
                &mut self.stdout,
                MAX_APP_SERVER_RPC_BYTES,
                false,
            )
            .await?
            {
                crate::bounded_io::BoundedLine::Eof => {
                    bail!("Codex app-server closed stdout while waiting for {method}")
                }
                crate::bounded_io::BoundedLine::TooLong => bail!(
                    "Codex app-server JSON-RPC frame exceeds {MAX_APP_SERVER_RPC_BYTES} byte limit"
                ),
                crate::bounded_io::BoundedLine::Line(line) => line,
            };
            let message: Value =
                serde_json::from_slice(&line).context("invalid JSON from Codex app-server")?;
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                match parse_token_usage_notification(&message) {
                    Ok(Some(notification)) => {
                        self.token_usage_notifications
                            .insert(notification.thread_id.clone(), notification);
                    }
                    Ok(None) => {}
                    Err(_) => {
                        if !self
                            .collector_warnings
                            .iter()
                            .any(|warning| warning == MALFORMED_TOKEN_USAGE_WARNING)
                        {
                            self.collector_warnings
                                .push(MALFORMED_TOKEN_USAGE_WARNING.to_string());
                        }
                    }
                }
                continue;
            }
            if let Some(error) = message.get("error") {
                let rpc_error = AppServerRpcError {
                    method: method.to_string(),
                    code: error.get("code").and_then(Value::as_i64),
                    message: crate::redaction::redact_excerpt(
                        error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("unspecified JSON-RPC error"),
                    ),
                    data: error.get("data").map(crate::redaction::redact_value),
                };
                return Err(anyhow::Error::new(rpc_error));
            }
            return message
                .get("result")
                .cloned()
                .with_context(|| format!("Codex app-server {method} response omitted result"));
        }
    }

    async fn request_timed(&mut self, method: &str, params: Value) -> Result<Value> {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.request(method, params),
        )
        .await
        .with_context(|| format!("Codex app-server {method} timed out"))?
    }

    fn apply_collector_warnings(&self, coverage: &mut CoverageV1) {
        for warning in &self.collector_warnings {
            degrade(coverage, "valid token-usage notification", warning.clone());
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }))
        .await
    }

    async fn write_message(&mut self, message: &Value) -> Result<()> {
        let bytes = serde_json::to_vec(message)?;
        self.stdin.write_all(&bytes).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }
}

fn is_sensitive_environment_name(name: &std::ffi::OsStr) -> bool {
    let upper = name.to_string_lossy().to_ascii_uppercase();
    ["KEY", "SECRET", "TOKEN"]
        .iter()
        .any(|marker| upper.contains(marker))
}

fn validate_thread_repository(cwd: &Path, registered: &RepositoryIdentity) -> Result<()> {
    if !cwd.is_absolute() {
        bail!("Codex thread/list returned a non-absolute cwd; thread skipped")
    }
    let returned = repository_identity(cwd)
        .context("Codex thread/list cwd is not a readable Git worktree; thread skipped")?;
    if returned.common_dir != registered.common_dir {
        bail!(
            "Codex thread/list returned a cwd owned by a different logical Git repository; thread skipped"
        )
    }
    Ok(())
}

pub async fn inspect_capabilities() -> AppServerCapabilityReport {
    match AppServerClient::connect().await {
        Ok(mut client) => {
            let report = client.capability_report().await;
            client.shutdown().await.ok();
            report
        }
        Err(error) => AppServerCapabilityReport::unsupported(error.to_string()),
    }
}

/// Read-only, same-device observation of interactive and sub-agent thread lineage.
///
/// Parent/task association is made only from an already observed parent or an existing session's
/// explicit `source_thread_id`. Cross-repository and missing-parent rows are never guessed.
pub async fn collect_agent_lineage(
    client: &mut AppServerClient,
    store: &Store,
    repository_root: &Path,
    repository_id: &str,
) -> Result<Vec<AgentV1>> {
    let registered = repository_identity(repository_root)?;
    let report = client.list_lineage_threads_report(None).await?;
    let session_tasks = store
        .list_sessions(Some(repository_id))?
        .into_iter()
        .filter_map(|session| Some((session.source_thread_id?, session.task_id?)))
        .collect::<BTreeMap<_, _>>();
    let mut observed = Vec::new();
    for summary in report.threads {
        if validate_thread_repository(&summary.cwd, &registered).is_err() {
            continue;
        }
        let source_kind = match summary.source_kind.as_deref() {
            Some("subAgent") => AgentSourceKindV1::SubAgent,
            Some("subAgentReview") => AgentSourceKindV1::SubAgentReview,
            Some("subAgentCompact") => AgentSourceKindV1::SubAgentCompact,
            Some("subAgentThreadSpawn") => AgentSourceKindV1::SubAgentThreadSpawn,
            Some("subAgentOther") => AgentSourceKindV1::SubAgentOther,
            Some("cli" | "vscode" | "exec" | "appServer") => AgentSourceKindV1::Interactive,
            None => continue,
            Some(_) => continue,
        };
        let thread = client.read_thread(&summary.id).await.ok();
        let (output_summary, files, tests) = thread
            .as_ref()
            .map(extract_agent_observation)
            .unwrap_or_default();
        let task_id = session_tasks.get(&summary.id).cloned();
        let association_state = if task_id.is_some() {
            AgentAssociationStateV1::Linked
        } else if summary.parent_thread_id.is_some() {
            AgentAssociationStateV1::Unlinked
        } else {
            AgentAssociationStateV1::Degraded
        };
        let degraded_reason = match association_state {
            AgentAssociationStateV1::Linked => None,
            AgentAssociationStateV1::Unlinked => {
                Some("parent observed but task association not yet resolved".to_string())
            }
            AgentAssociationStateV1::Degraded => {
                Some("no verified session or parent association".to_string())
            }
        };
        let role = match source_kind {
            AgentSourceKindV1::Interactive => "interactive",
            AgentSourceKindV1::SubAgentReview => "review",
            AgentSourceKindV1::SubAgentCompact => "compact",
            AgentSourceKindV1::SubAgentThreadSpawn => "thread_spawn",
            AgentSourceKindV1::SubAgent | AgentSourceKindV1::SubAgentOther => "subagent",
        };
        observed.push(AgentV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: deterministic_id("agent", &[repository_id, &summary.id]),
            repository_id: repository_id.to_string(),
            thread_id: summary.id.clone(),
            session_id: Some(summary.session_id.clone()),
            parent_thread_id: summary.parent_thread_id.clone(),
            forked_from_id: summary.forked_from_id.clone(),
            task_id,
            name: crate::redaction::redact_excerpt(
                summary.name.as_deref().unwrap_or(summary.preview.as_str()),
            ),
            source_kind,
            role: role.to_string(),
            status: crate::redaction::redact_excerpt(
                summary.status.as_deref().unwrap_or("unknown"),
            ),
            association_state,
            output_summary,
            files,
            tests,
            observed_at: timestamp_seconds(summary.updated_at),
            degraded_reason,
        });
    }

    // Resolve children only through a parent that was itself observed and explicitly linked.
    for _ in 0..observed.len() {
        let linked = observed
            .iter()
            .filter_map(|agent| Some((agent.thread_id.clone(), agent.task_id.clone()?)))
            .collect::<BTreeMap<_, _>>();
        let mut changed = false;
        for agent in &mut observed {
            if agent.task_id.is_some() {
                continue;
            }
            let parent = agent.parent_thread_id.as_deref();
            if let Some(task_id) = parent.and_then(|parent| linked.get(parent)).cloned() {
                agent.task_id = Some(task_id);
                agent.association_state = AgentAssociationStateV1::Linked;
                agent.degraded_reason = None;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    observed.sort_by(|left, right| left.thread_id.cmp(&right.thread_id));
    for agent in &observed {
        store.append_agent_observation(agent)?;
    }
    Ok(observed)
}

fn extract_agent_observation(thread: &Value) -> (Option<String>, Vec<String>, Vec<String>) {
    let mut output_summary = None;
    let mut files = BTreeMap::<String, ()>::new();
    let mut tests = BTreeMap::<String, ()>::new();
    for turn in thread
        .get("turns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for item in turn
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            match item.get("type").and_then(Value::as_str) {
                Some("agentMessage") => {
                    let text = item
                        .get("text")
                        .or_else(|| item.get("content"))
                        .and_then(Value::as_str);
                    if let Some(text) = text {
                        output_summary = Some(crate::redaction::redact_excerpt(text));
                    }
                }
                Some("fileChange") => {
                    for change in item
                        .get("changes")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        if let Some(path) = change.get("path").and_then(Value::as_str) {
                            files.insert(crate::redaction::redact_excerpt(path), ());
                        }
                    }
                }
                Some("commandExecution") => {
                    if let Some(command) = item.get("command").and_then(Value::as_str) {
                        let lower = command.to_ascii_lowercase();
                        if [" test", "pytest", "cargo test", "npm test", "vitest"]
                            .iter()
                            .any(|marker| lower.contains(marker))
                        {
                            tests.insert(crate::redaction::redact_excerpt(command), ());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    (
        output_summary,
        files.into_keys().take(32).collect(),
        tests.into_keys().take(16).collect(),
    )
}

fn parse_thread_summary(raw: Value) -> Result<AppServerThreadSummary> {
    let required_string = |key: &str| -> Result<String> {
        raw.get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .with_context(|| format!("Codex thread/list item omitted non-empty `{key}`"))
    };
    let required_i64 = |key: &str| -> Result<i64> {
        raw.get(key)
            .and_then(Value::as_i64)
            .with_context(|| format!("Codex thread/list item omitted `{key}`"))
    };
    let mut coverage = complete_coverage(["thread summary"]);
    let session_id = match raw.get("sessionId").and_then(Value::as_str) {
        Some(session_id) if !session_id.is_empty() => session_id.to_string(),
        _ => {
            let generated = format!("app-import-{}", Uuid::now_v7());
            degrade(
                &mut coverage,
                "stable session source ID",
                format!(
                    "thread summary omitted stable sessionId; assigned UUID `{generated}` without payload-hash deduplication"
                ),
            );
            generated
        }
    };
    let optional_string = |key: &str| {
        raw.get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let status = raw.get("status").and_then(|status| {
        status
            .get("type")
            .and_then(Value::as_str)
            .or_else(|| status.as_str())
            .map(str::to_string)
    });
    Ok(AppServerThreadSummary {
        id: required_string("id")?,
        session_id,
        cwd: PathBuf::from(required_string("cwd")?),
        cli_version: required_string("cliVersion")?,
        created_at: required_i64("createdAt")?,
        updated_at: required_i64("updatedAt")?,
        preview: optional_string("preview").unwrap_or_default(),
        name: optional_string("name"),
        source_kind: optional_string("sourceKind").or_else(|| optional_string("source")),
        parent_thread_id: optional_string("parentThreadId"),
        forked_from_id: optional_string("forkedFromId"),
        status,
        coverage,
        raw,
    })
}

fn complete_coverage(items: impl IntoIterator<Item = impl Into<String>>) -> CoverageV1 {
    CoverageV1 {
        captured: items.into_iter().map(Into::into).collect(),
        ..CoverageV1::default()
    }
}

fn degrade(coverage: &mut CoverageV1, missing: impl Into<String>, warning: impl Into<String>) {
    coverage.status = CoverageStatus::Degraded;
    let missing = missing.into();
    if !coverage.missing.contains(&missing) {
        coverage.missing.push(missing);
    }
    let warning = warning.into();
    if !coverage.warnings.contains(&warning) {
        coverage.warnings.push(warning);
    }
}

fn is_deleted_thread_error(error: &AppServerRpcError) -> bool {
    let mut evidence = error.message.to_ascii_lowercase();
    if let Some(data) = &error.data {
        evidence.push(' ');
        evidence.push_str(&data.to_string().to_ascii_lowercase());
    }
    matches!(error.code, Some(-32001) | Some(-32004))
        || evidence.contains("not found")
        || evidence.contains("deleted")
        || evidence.contains("thread_not_found")
}

fn normalize_and_assess_thread(mut thread: Value, cli_version: &str) -> (Value, CoverageV1) {
    let mut coverage = complete_coverage(["thread/read"]);
    if !SUPPORTED_CODEX_VERSIONS.contains(&cli_version) {
        degrade(
            &mut coverage,
            "tested App Server version",
            format!(
                "thread was recorded by Codex {cli_version}; supported versions are {}",
                SUPPORTED_CODEX_VERSIONS.join(", ")
            ),
        );
    }

    if thread.get("compacted").and_then(Value::as_bool) == Some(true)
        || value_contains_marker(thread.get("status"), &["compact", "incomplete"])
    {
        degrade(
            &mut coverage,
            "complete thread history",
            "thread is compacted or incomplete; available turns were imported as untrusted data",
        );
    }

    let Some(turns) = thread.get_mut("turns").and_then(Value::as_array_mut) else {
        degrade(
            &mut coverage,
            "thread turns",
            "thread/read omitted a turns array",
        );
        return (thread, coverage);
    };

    for (turn_index, turn) in turns.iter_mut().enumerate() {
        let Some(turn_object) = turn.as_object_mut() else {
            degrade(
                &mut coverage,
                "well-formed turn",
                format!("turn {turn_index} was not an object and remains opaque"),
            );
            continue;
        };
        ensure_uuid_source_id(turn_object, "turn", turn_index, &mut coverage);
        if value_contains_marker(
            turn_object.get("status"),
            &["failed", "interrupt", "incomplete"],
        ) {
            degrade(
                &mut coverage,
                "completed turn",
                format!("turn {turn_index} is incomplete or interrupted"),
            );
        }
        let Some(items) = turn_object.get_mut("items").and_then(Value::as_array_mut) else {
            degrade(
                &mut coverage,
                "turn items",
                format!("turn {turn_index} omitted an items array"),
            );
            continue;
        };
        for (item_index, item) in items.iter_mut().enumerate() {
            let Some(item_object) = item.as_object_mut() else {
                degrade(
                    &mut coverage,
                    "well-formed thread item",
                    format!("turn {turn_index} item {item_index} was not an object"),
                );
                continue;
            };
            ensure_uuid_source_id(item_object, "item", item_index, &mut coverage);
            let item_type = item_object.get("type").and_then(Value::as_str);
            if !item_type.is_some_and(is_known_thread_item_type) {
                degrade(
                    &mut coverage,
                    "known thread item schema",
                    format!(
                        "turn {turn_index} item {item_index} has unknown type `{}` and remains opaque",
                        item_type.unwrap_or("<missing>")
                    ),
                );
            }
        }
    }
    (thread, coverage)
}

fn ensure_uuid_source_id(
    object: &mut serde_json::Map<String, Value>,
    kind: &str,
    index: usize,
    coverage: &mut CoverageV1,
) {
    let has_stable_id = object
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty());
    if has_stable_id {
        return;
    }
    let source_id = format!("app-import-{}", Uuid::now_v7());
    object.insert("id".to_string(), Value::String(source_id.clone()));
    degrade(
        coverage,
        format!("stable {kind} source ID"),
        format!(
            "{kind} {index} omitted a stable ID; assigned UUID `{source_id}` without payload-hash deduplication"
        ),
    );
}

fn value_contains_marker(value: Option<&Value>, markers: &[&str]) -> bool {
    let Some(value) = value else {
        return false;
    };
    let rendered = value.to_string().to_ascii_lowercase();
    markers.iter().any(|marker| rendered.contains(marker))
}

fn is_known_thread_item_type(item_type: &str) -> bool {
    matches!(
        item_type,
        "userMessage"
            | "agentMessage"
            | "plan"
            | "reasoning"
            | "commandExecution"
            | "fileChange"
            | "mcpToolCall"
            | "webSearch"
            | "imageView"
            | "enteredReviewMode"
            | "exitedReviewMode"
            | "contextCompaction"
    )
}

pub async fn codex_version() -> Result<String> {
    let output = Command::new("codex")
        .arg("--version")
        .output()
        .await
        .context("run codex --version")?;
    if !output.status.success() {
        bail!("codex --version exited with {}", output.status);
    }
    let stdout = String::from_utf8(output.stdout).context("codex --version was not UTF-8")?;
    stdout
        .split_whitespace()
        .last()
        .map(str::to_string)
        .context("codex --version returned no version")
}

#[cfg(test)]
mod environment_tests {
    use super::is_sensitive_environment_name;
    use std::ffi::OsStr;

    #[test]
    fn experimental_app_server_removes_key_secret_and_token_environment_names() {
        for name in [
            "OPENAI_API_KEY",
            "CODEX_ACCESS_TOKEN",
            "PRIVATE_SECRET_VALUE",
            "api_key_lowercase",
        ] {
            assert!(is_sensitive_environment_name(OsStr::new(name)), "{name}");
        }
        for name in ["PATH", "HOME", "CODEX_HOME", "TMPDIR"] {
            assert!(!is_sensitive_environment_name(OsStr::new(name)), "{name}");
        }
    }
}
