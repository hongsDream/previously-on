use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA_VERSION_V1: u16 = 1;
pub const MAX_EVIDENCE_EXCERPT_CHARS: usize = 500;
pub const MAX_CONTEXT_TEMPORAL_ITEMS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    #[default]
    Complete,
    Degraded,
    Unsupported,
}

impl CoverageStatus {
    pub fn worst(self, other: Self) -> Self {
        use CoverageStatus::*;
        match (self, other) {
            (Unsupported, _) | (_, Unsupported) => Unsupported,
            (Degraded, _) | (_, Degraded) => Degraded,
            _ => Complete,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageV1 {
    pub schema_version: u16,
    pub status: CoverageStatus,
    #[serde(default)]
    pub captured: Vec<String>,
    #[serde(default)]
    pub missing: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl Default for CoverageV1 {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION_V1,
            status: CoverageStatus::Complete,
            captured: Vec::new(),
            missing: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

impl CoverageV1 {
    pub fn merge<'a>(items: impl IntoIterator<Item = &'a CoverageV1>) -> Self {
        let mut status = CoverageStatus::Complete;
        let mut captured = BTreeSet::new();
        let mut missing = BTreeSet::new();
        let mut warnings = BTreeSet::new();
        for item in items {
            status = status.worst(item.status);
            captured.extend(item.captured.iter().cloned());
            missing.extend(item.missing.iter().cloned());
            warnings.extend(item.warnings.iter().cloned());
        }
        Self {
            schema_version: SCHEMA_VERSION_V1,
            status,
            captured: captured.into_iter().collect(),
            missing: missing.into_iter().collect(),
            warnings: warnings.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskLifecycle {
    #[default]
    Active,
    Completed,
    Abandoned,
}

pub type TaskStatus = TaskLifecycle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionLifecycle {
    #[default]
    Active,
    Completed,
    Interrupted,
}

pub type SessionStatus = SessionLifecycle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChangeAttribution {
    ModifiedBy,
    #[default]
    ObservedChangedIn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Unmerged,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TestStatus {
    Passed,
    Failed,
    Skipped,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    #[default]
    Fresh,
    Stale,
    Broken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TemporalStatusV1 {
    #[default]
    Unchanged,
    Changed,
    Diverged,
    Broken,
    Degraded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContinuationStateV1 {
    #[default]
    Normal,
    Eligible,
    Suggested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuationReasonV1 {
    CompactionLimit,
    ContextUsageLimit,
    OldSessionCodeChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FactLifecycle {
    #[default]
    Candidate,
    Confirmed,
    Pinned,
    Invalid,
    Superseded,
}

pub type FactStatus = FactLifecycle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FactKind {
    #[default]
    Decision,
    Constraint,
    OpenItem,
    Progress,
    Goal,
    Note,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceIntegrity {
    #[default]
    Verified,
    Missing,
    Mismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    SessionStarted,
    UserPrompt,
    AssistantFinal,
    ToolStarted,
    ToolFinished,
    GitSnapshot,
    FactCandidate,
    FactConfirmed,
    Checkpoint,
    ContextCompaction,
    ContextUsageUpdated,
    ContinuationSuggested,
    #[default]
    SessionStopped,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelopeV1 {
    pub schema_version: u16,
    pub event_id: String,
    pub dedupe_key: String,
    pub source_id: String,
    pub repository_id: String,
    pub session_id: String,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub sequence: Option<i64>,
    pub occurred_at: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    pub kind: EventKind,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub coverage: CoverageV1,
}

impl EventEnvelopeV1 {
    pub fn new(
        source_id: impl Into<String>,
        repository_id: impl Into<String>,
        session_id: impl Into<String>,
        kind: EventKind,
        occurred_at: DateTime<Utc>,
        payload: Value,
    ) -> Self {
        let source_id = source_id.into();
        let repository_id = repository_id.into();
        let session_id = session_id.into();
        let payload_bytes = serde_json::to_vec(&payload).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(source_id.as_bytes());
        hasher.update([0]);
        hasher.update(repository_id.as_bytes());
        hasher.update([0]);
        hasher.update(session_id.as_bytes());
        hasher.update([0]);
        hasher.update(format!("{kind:?}").as_bytes());
        hasher.update([0]);
        hasher.update(occurred_at.timestamp_micros().to_be_bytes());
        hasher.update(payload_bytes);
        let digest = hex::encode(hasher.finalize());
        Self {
            schema_version: SCHEMA_VERSION_V1,
            event_id: format!("evt-{}", &digest[..24]),
            dedupe_key: digest,
            source_id,
            repository_id,
            session_id,
            task_id: None,
            sequence: None,
            occurred_at,
            received_at: Utc::now(),
            kind,
            payload,
            coverage: CoverageV1::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryV1 {
    pub schema_version: u16,
    pub id: String,
    pub path: String,
    #[serde(default)]
    pub remote_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskV1 {
    pub schema_version: u16,
    pub id: String,
    pub repository_id: String,
    pub title: String,
    #[serde(default)]
    pub goal: Option<String>,
    pub lifecycle: TaskLifecycle,
    #[serde(default)]
    pub branch: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionV1 {
    pub schema_version: u16,
    pub id: String,
    pub repository_id: String,
    #[serde(default)]
    pub task_id: Option<String>,
    pub lifecycle: SessionLifecycle,
    pub started_at: DateTime<Utc>,
    #[serde(default)]
    pub ended_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub source_thread_id: Option<String>,
    #[serde(default)]
    pub last_activity_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub turn_count: u32,
    #[serde(default)]
    pub compaction_count: u32,
    #[serde(default)]
    pub context_usage: Option<ContextUsageV1>,
    #[serde(default)]
    pub continuation_state: ContinuationStateV1,
    #[serde(default)]
    pub coverage: CoverageV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextUsageV1 {
    pub total_tokens: u64,
    pub model_context_window: u64,
    #[serde(default)]
    pub observed_at: Option<DateTime<Utc>>,
}

impl ContextUsageV1 {
    pub fn utilization(&self) -> Option<f64> {
        (self.model_context_window > 0)
            .then(|| self.total_tokens as f64 / self.model_context_window as f64)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContinuationAdviceV1 {
    pub action: String,
    #[serde(default)]
    pub reasons: Vec<ContinuationReasonV1>,
    pub task_id: String,
    pub task_title: String,
    pub last_activity_at: DateTime<Utc>,
    pub compaction_count: u32,
    #[serde(default)]
    pub context_usage: Option<ContextUsageV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSnapshotV1 {
    pub schema_version: u16,
    pub repository_id: String,
    pub root: String,
    #[serde(default)]
    pub remote_url: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub head: Option<String>,
    pub captured_at: DateTime<Utc>,
    #[serde(default)]
    pub dirty_files: Vec<String>,
    #[serde(default)]
    pub working_tree_changes: Vec<FileChangeV1>,
    /// SHA-256 fingerprints of dirty working-tree content at capture time.
    /// `None` records that the path was intentionally absent (for example, a
    /// deletion or the source side of a rename). Raw file content is never
    /// persisted.
    #[serde(default)]
    pub content_fingerprints: BTreeMap<String, Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChangeV1 {
    pub schema_version: u16,
    pub repository_id: String,
    pub session_id: String,
    #[serde(default)]
    pub task_id: Option<String>,
    pub path: String,
    #[serde(default)]
    pub previous_path: Option<String>,
    pub status: ChangeStatus,
    #[serde(default)]
    pub additions: Option<u64>,
    #[serde(default)]
    pub deletions: Option<u64>,
    pub attribution: ChangeAttribution,
    #[serde(default)]
    pub before_head: Option<String>,
    #[serde(default)]
    pub after_head: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestResultV1 {
    pub schema_version: u16,
    pub id: String,
    pub repository_id: String,
    pub session_id: String,
    #[serde(default)]
    pub task_id: Option<String>,
    pub name: String,
    pub command: String,
    pub status: TestStatus,
    #[serde(default)]
    pub summary: Option<String>,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckpointV1 {
    pub schema_version: u16,
    pub id: String,
    pub repository_id: String,
    pub task_id: String,
    pub session_id: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub goal_hint: Option<String>,
    #[serde(default)]
    pub git_before: Option<GitSnapshotV1>,
    pub git_after: GitSnapshotV1,
    #[serde(default)]
    pub changed_files: Vec<FileChangeV1>,
    #[serde(default)]
    pub tests: Vec<TestResultV1>,
    #[serde(default)]
    pub failures: Vec<String>,
    #[serde(default)]
    pub unresolved_items: Vec<String>,
    #[serde(default)]
    pub coverage: CoverageV1,
}

impl CheckpointV1 {
    pub fn project(
        events: &[EventEnvelopeV1],
        git_before: Option<GitSnapshotV1>,
        git_after: GitSnapshotV1,
        mut changed_files: Vec<FileChangeV1>,
        mut tests: Vec<TestResultV1>,
    ) -> Self {
        let mut ordered = events.to_vec();
        ordered.sort_by(|a, b| {
            (a.occurred_at, a.sequence.unwrap_or(i64::MAX), &a.event_id).cmp(&(
                b.occurred_at,
                b.sequence.unwrap_or(i64::MAX),
                &b.event_id,
            ))
        });
        let repository_id = ordered
            .first()
            .map(|event| event.repository_id.clone())
            .unwrap_or_else(|| git_after.repository_id.clone());
        let session_id = ordered
            .first()
            .map(|event| event.session_id.clone())
            .unwrap_or_else(|| "unknown-session".to_string());
        let task_id = ordered
            .iter()
            .find_map(|event| event.task_id.clone())
            .unwrap_or_else(|| deterministic_id("task", &[&repository_id, &session_id]));
        let goal_hint = ordered.iter().rev().find_map(|event| {
            if event.kind != EventKind::UserPrompt {
                return None;
            }
            payload_text(&event.payload).map(|text| first_nonempty_line(&text))
        });
        let mut unresolved_items = ordered
            .iter()
            .flat_map(|event| payload_string_array(&event.payload, "unresolved_items"))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        unresolved_items.sort();
        changed_files.sort_by(|a, b| (&a.path, &a.previous_path).cmp(&(&b.path, &b.previous_path)));
        tests.sort_by(|a, b| {
            (&a.name, &a.command, a.occurred_at).cmp(&(&b.name, &b.command, b.occurred_at))
        });
        let failures = tests
            .iter()
            .filter(|test| test.status == TestStatus::Failed)
            .map(|test| test.summary.clone().unwrap_or_else(|| test.name.clone()))
            .collect::<Vec<_>>();
        let coverage = CoverageV1::merge(ordered.iter().map(|event| &event.coverage));
        let latest_at = ordered
            .last()
            .map(|event| event.occurred_at)
            .unwrap_or(git_after.captured_at);
        let id = deterministic_id(
            "checkpoint",
            &[
                &repository_id,
                &task_id,
                &session_id,
                &latest_at.timestamp_micros().to_string(),
            ],
        );
        Self {
            schema_version: SCHEMA_VERSION_V1,
            id,
            repository_id,
            task_id,
            session_id,
            created_at: latest_at,
            goal_hint,
            git_before,
            git_after,
            changed_files,
            tests,
            failures,
            unresolved_items,
            coverage,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactV1 {
    pub schema_version: u16,
    pub id: String,
    pub repository_id: String,
    pub task_id: String,
    pub kind: FactKind,
    pub lifecycle: FactLifecycle,
    pub freshness: Freshness,
    pub content: String,
    #[serde(default)]
    pub evidence_ids: Vec<String>,
    #[serde(default)]
    pub superseded_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl FactV1 {
    pub fn is_pack_eligible(&self) -> bool {
        matches!(
            self.lifecycle,
            FactLifecycle::Confirmed | FactLifecycle::Pinned
        ) && self.freshness == Freshness::Fresh
            && self.superseded_by.is_none()
            && !self.evidence_ids.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceV1 {
    pub schema_version: u16,
    pub id: String,
    pub repository_id: String,
    pub task_id: String,
    pub session_id: String,
    #[serde(default)]
    pub fact_id: Option<String>,
    pub source_id: String,
    #[serde(default)]
    pub turn_index: Option<u32>,
    #[serde(default)]
    pub item_index: Option<u32>,
    pub excerpt: String,
    pub excerpt_sha256: String,
    pub integrity: EvidenceIntegrity,
    pub created_at: DateTime<Utc>,
}

impl EvidenceV1 {
    pub fn new(
        id: impl Into<String>,
        repository_id: impl Into<String>,
        task_id: impl Into<String>,
        session_id: impl Into<String>,
        source_id: impl Into<String>,
        excerpt: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Self {
        let excerpt = excerpt
            .into()
            .chars()
            .take(MAX_EVIDENCE_EXCERPT_CHARS)
            .collect::<String>();
        let excerpt_sha256 = hex::encode(Sha256::digest(excerpt.as_bytes()));
        Self {
            schema_version: SCHEMA_VERSION_V1,
            id: id.into(),
            repository_id: repository_id.into(),
            task_id: task_id.into(),
            session_id: session_id.into(),
            fact_id: None,
            source_id: source_id.into(),
            turn_index: None,
            item_index: None,
            excerpt,
            excerpt_sha256,
            integrity: EvidenceIntegrity::Verified,
            created_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextFactV1 {
    pub id: String,
    pub kind: FactKind,
    pub lifecycle: FactLifecycle,
    pub freshness: Freshness,
    pub content: String,
    pub evidence: Vec<EvidenceV1>,
    pub selection_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemporalRevalidationV1 {
    pub schema_version: u16,
    pub status: TemporalStatusV1,
    #[serde(default)]
    pub baseline_head: Option<String>,
    #[serde(default)]
    pub current_head: Option<String>,
    #[serde(default)]
    pub merge_base: Option<String>,
    #[serde(default)]
    pub related_changes: Vec<FileChangeV1>,
    #[serde(default)]
    pub checked_paths: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl TemporalRevalidationV1 {
    /// Keep task-scoped temporal metadata deterministic and bounded without
    /// removing it from a context pack. Omitted counts use the existing
    /// warnings extension point so the V1 wire shape remains compatible.
    pub fn bounded_for_context_pack(mut self) -> Self {
        self.checked_paths.sort();
        self.checked_paths.dedup();
        let omitted_paths = self
            .checked_paths
            .len()
            .saturating_sub(MAX_CONTEXT_TEMPORAL_ITEMS);
        self.checked_paths.truncate(MAX_CONTEXT_TEMPORAL_ITEMS);

        self.related_changes.sort_by(|a, b| {
            (
                &a.path,
                &a.previous_path,
                a.status as u8,
                a.attribution as u8,
                &a.repository_id,
                &a.session_id,
                &a.task_id,
                a.additions,
                a.deletions,
                &a.before_head,
                &a.after_head,
            )
                .cmp(&(
                    &b.path,
                    &b.previous_path,
                    b.status as u8,
                    b.attribution as u8,
                    &b.repository_id,
                    &b.session_id,
                    &b.task_id,
                    b.additions,
                    b.deletions,
                    &b.before_head,
                    &b.after_head,
                ))
        });
        self.related_changes.dedup();
        let omitted_changes = self
            .related_changes
            .len()
            .saturating_sub(MAX_CONTEXT_TEMPORAL_ITEMS);
        self.related_changes.truncate(MAX_CONTEXT_TEMPORAL_ITEMS);

        self.warnings.sort();
        self.warnings.dedup();
        if omitted_paths > 0 {
            self.warnings.push(format!(
                "checked_paths_omitted_count={omitted_paths}; limit={MAX_CONTEXT_TEMPORAL_ITEMS}"
            ));
        }
        if omitted_changes > 0 {
            self.warnings.push(format!(
                "related_changes_omitted_count={omitted_changes}; limit={MAX_CONTEXT_TEMPORAL_ITEMS}"
            ));
        }
        self.warnings.sort();
        self.warnings.dedup();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentValidationV1 {
    pub schema_version: u16,
    pub status: TemporalStatusV1,
    #[serde(default)]
    pub current_head: Option<String>,
    #[serde(default)]
    pub verified_paths: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl CurrentValidationV1 {
    /// Remove fields already represented by temporal revalidation while
    /// retaining the public V1 object and any independently supplied values.
    pub fn minimized_against(mut self, temporal: Option<&TemporalRevalidationV1>) -> Self {
        if let Some(temporal) = temporal {
            if self.current_head == temporal.current_head {
                self.current_head = None;
            }
            self.verified_paths
                .retain(|path| !temporal.checked_paths.contains(path));
            self.warnings
                .retain(|warning| !temporal.warnings.contains(warning));
        }
        self.verified_paths.sort();
        self.verified_paths.dedup();
        let omitted_paths = self
            .verified_paths
            .len()
            .saturating_sub(MAX_CONTEXT_TEMPORAL_ITEMS);
        self.verified_paths.truncate(MAX_CONTEXT_TEMPORAL_ITEMS);
        self.warnings.sort();
        self.warnings.dedup();
        if omitted_paths > 0 {
            self.warnings.push(format!(
                "verified_paths_omitted_count={omitted_paths}; limit={MAX_CONTEXT_TEMPORAL_ITEMS}"
            ));
        }
        self.warnings.sort();
        self.warnings.dedup();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextPackV1 {
    pub schema_version: u16,
    pub repository_id: String,
    pub task_id: String,
    pub generated_at: DateTime<Utc>,
    pub token_budget: u32,
    pub token_count: u32,
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub facts: Vec<ContextFactV1>,
    #[serde(default)]
    pub unresolved_items: Vec<ContextFactV1>,
    #[serde(default)]
    pub files: Vec<FileChangeV1>,
    #[serde(default)]
    pub tests: Vec<TestResultV1>,
    #[serde(default)]
    pub temporal_revalidation: Option<TemporalRevalidationV1>,
    #[serde(default)]
    pub current_validation: Option<CurrentValidationV1>,
    pub coverage: CoverageV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskTimelineV1 {
    pub task: TaskV1,
    #[serde(default)]
    pub sessions: Vec<SessionV1>,
    #[serde(default)]
    pub checkpoints: Vec<CheckpointV1>,
    #[serde(default)]
    pub facts: Vec<FactV1>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskSuggestionV1 {
    pub task_id: String,
    pub title: String,
    pub score: f64,
    pub last_activity_at: DateTime<Utc>,
    pub matching_reasons: Vec<String>,
    #[serde(default)]
    pub continuation_advice: Option<ContinuationAdviceV1>,
}

pub fn deterministic_id(prefix: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    let digest = hex::encode(hasher.finalize());
    format!("{prefix}-{}", &digest[..24])
}

fn payload_text(payload: &Value) -> Option<String> {
    payload
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            payload
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            payload
                .get("prompt")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            payload
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn first_nonempty_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .chars()
        .take(240)
        .collect()
}

fn payload_string_array(payload: &Value, key: &str) -> Vec<String> {
    payload
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}
