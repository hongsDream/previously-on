export type Freshness = 'fresh' | 'stale' | 'broken';
export type TemporalStatus = 'unchanged' | 'changed' | 'diverged' | 'broken' | 'degraded';
export type FactStatus = 'candidate' | 'confirmed' | 'pinned' | 'invalid' | 'superseded';
export type FactKind = 'decision' | 'constraint' | 'open_item' | 'progress' | 'goal' | 'note';
export type TaskStatus = 'active' | 'completed' | 'abandoned';
export type ContinuationState = 'normal' | 'eligible' | 'suggested';
export type ContractStatus = 'active' | 'superseded';
export type ContractReadiness = 'ready' | 'contract_blocked';
export type RequiredTestStatus = 'passed' | 'failed' | 'missing' | 'stale';
export type TaskGroupingAction = 'move' | 'merge' | 'split' | 'undo';
export type AiRefreshCapabilityStatus = 'ready' | 'needs_setup' | 'unsupported' | 'blocked';
export type AiFactRefreshStatus = 'pending' | 'thread_created' | 'completed' | 'failed';
export type AiFactCandidateAction = 'add' | 'update' | 'deprecate';
export type AiFactCandidateStatus = 'pending' | 'accepted' | 'rejected';
export type AgentAssociationState = 'linked' | 'unlinked' | 'degraded';

export interface AiRefreshCapabilityV1 {
  status: AiRefreshCapabilityStatus;
  profileName: string;
  reasonCode: 'ready' | 'setup_required' | 'app_server_unsupported' | 'verification_blocked';
  technicalDetails: string[];
  checkedAt?: string | null;
}

export interface AiFactCandidateV1 {
  schemaVersion: 1;
  id: string;
  operationId: string;
  action: AiFactCandidateAction;
  factId?: string | null;
  kind: FactKind;
  content: string;
  reason: string;
  status: AiFactCandidateStatus;
}

export interface AiFactRefreshOperationV1 {
  schemaVersion: 1;
  operationId: string;
  repositoryId: string;
  taskId: string;
  status: AiFactRefreshStatus;
  requestFingerprint: string;
  threadId?: string | null;
  candidates: AiFactCandidateV1[];
  modelId?: string | null;
  inputTokens?: number | null;
  outputTokens?: number | null;
  latencyMs?: number | null;
  error?: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface AgentV1 {
  schemaVersion: 1;
  id: string;
  repositoryId: string;
  threadId: string;
  sessionId?: string | null;
  parentThreadId?: string | null;
  forkedFromId?: string | null;
  taskId?: string | null;
  sourceKind: 'interactive' | 'subAgent' | 'subAgentReview' | 'subAgentCompact' | 'subAgentThreadSpawn' | 'subAgentOther';
  role: string;
  status: string;
  name: string;
  outputSummary?: string | null;
  files: string[];
  tests: string[];
  observedAt: string;
  associationState: AgentAssociationState;
  degradedReason?: string | null;
}

export interface TaskTitleSuggestionV1 {
  value: string;
  source: 'goal' | 'branch' | 'touched_area';
}

export interface TaskUpdateV1 {
  title?: string;
  goal?: string;
  status?: TaskStatus;
}

export interface TaskGroupingRequestV1 {
  operationId: string;
  action: Exclude<TaskGroupingAction, 'undo'>;
  sessionIds: string[];
  fromTaskId: string;
  targetTaskId?: string;
  newTaskTitle?: string;
  newTaskGoal?: string;
}

export interface SessionMoveV1 {
  sessionId: string;
  fromTaskId: string;
  toTaskId: string;
}

export interface TaskLifecycleSnapshotV1 {
  taskId: string;
  before?: TaskStatus | null;
  after?: TaskStatus | null;
}

export interface FactGroupingImpactV1 {
  factId: string;
  fromTaskId: string;
  toTaskId?: string | null;
  mixedProvenance: boolean;
  sessionIds: string[];
}

export interface TaskGroupingOperationV1 {
  schemaVersion: 1;
  operationId: string;
  repositoryId: string;
  action: TaskGroupingAction;
  sessionMoves: SessionMoveV1[];
  taskLifecycle: TaskLifecycleSnapshotV1[];
  factImpacts: FactGroupingImpactV1[];
  createdTask?: { id: string; title: string } | null;
  inverseOf?: string | null;
  requestFingerprint: string;
  occurredAt: string;
}

export interface TaskGroupingPreviewV1 {
  operation: TaskGroupingOperationV1;
  affectedSessions: SessionMoveV1[];
  affectedFacts: FactGroupingImpactV1[];
  counts: {
    sessions: number;
    factsMoved: number;
    factsMixed: number;
  };
}

export type GraphNodeKindV1 = 'task' | 'session' | 'commit' | 'file' | 'regression_contract' | 'verified_symbol' | 'test' | 'agent';
export type GraphEdgeKindV1 =
  | 'task-has-session'
  | 'session-observed-commit'
  | 'session-changed-file'
  | 'contract-covers-file'
  | 'contract-declares-symbol'
  | 'contract-requires-test'
  | 'task-relevant-contract'
  | 'agent-parent'
  | 'agent-worked-on-task';
export type GraphSourceKindV1 = 'canonical_event' | 'projection' | 'regression_contract' | 'contract_evaluation' | 'agent_observation';

export interface GraphNodeV1 {
  id: string;
  kind: GraphNodeKindV1;
  label: string;
  taskId?: string | null;
}

export interface GraphEdgeV1 {
  id: string;
  kind: GraphEdgeKindV1;
  from: string;
  to: string;
  provenanceIds: string[];
  sourceKind: GraphSourceKindV1;
  observedAt: string;
  /** @deprecated V1 compatibility field. Use provenanceIds and sourceKind. */
  verified: boolean;
}

export interface RelationshipGraphV1 {
  schemaVersion: 1;
  repositoryId: string;
  taskFilter?: string | null;
  nodes: GraphNodeV1[];
  edges: GraphEdgeV1[];
}

export interface RelationshipGraphSummaryV1 {
  nodeCount: number;
  edgeCount: number;
  /** @deprecated V1 compatibility field. It mirrors evidence-backed edges in V1. */
  verifiedEdgeCount: number;
}

export interface ContractPathSelectorV1 {
  kind: 'exact' | 'prefix';
  value: string;
}

export interface ContractImpactSelectorV1 {
  path: ContractPathSelectorV1;
  symbols: string[];
}

export interface ContractRequiredTestV1 {
  id: string;
  name: string;
  program: string;
  args: string[];
  workingDirectory: string;
  timeoutSeconds: number;
}

export interface ContractOriginV1 {
  fixedAtCommit: string;
  recordedAt: string;
  evidenceSha256: string;
}

export interface RegressionContractV1 {
  schemaVersion: 1;
  id: string;
  title: string;
  invariant: string;
  status: ContractStatus;
  supersededBy?: string | null;
  impactSelectors: ContractImpactSelectorV1[];
  requiredTests: ContractRequiredTestV1[];
  origin: ContractOriginV1;
}

export interface RegressionCandidateV1 {
  schemaVersion: 1;
  id: string;
  repositoryId: string;
  taskId?: string | null;
  title: string;
  invariant: string;
  status: 'pending' | 'approved' | 'rejected';
  impactSelectors: ContractImpactSelectorV1[];
  requiredTests: ContractRequiredTestV1[];
  origin: ContractOriginV1;
  evidenceKind: 'failure_edit_pass' | 'test_file_edit_pass' | 'manual';
  evidenceSha256: string;
  createdAt: string;
  updatedAt: string;
}

export type RegressionCandidateDraftV1 = Pick<
  RegressionCandidateV1,
  'title' | 'invariant' | 'impactSelectors' | 'requiredTests'
>;

export interface RelevantContractV1 {
  id: string;
  title: string;
  invariant: string;
  matchReasons: string[];
}

export interface RequiredTestEvaluationV1 {
  contractId: string;
  testId: string;
  name: string;
  program: string;
  args: string[];
  workingDirectory: string;
  timeoutSeconds: number;
  state: RequiredTestStatus;
  detail?: string | null;
}

export interface ContractEvaluationV1 {
  schemaVersion: 1;
  id: string;
  repositoryId: string;
  taskId?: string | null;
  readiness: ContractReadiness;
  evaluatedAt: string;
  relevantContracts: RelevantContractV1[];
  requiredTests: RequiredTestEvaluationV1[];
  warnings: string[];
  contentFingerprint: string;
  continuationIssued: boolean;
  base?: string | null;
  head?: string | null;
  mergeBase?: string | null;
}

export interface ContextUsage {
  totalTokens: number;
  modelContextWindow: number;
  observedAt?: string;
}

export interface ContinuationAdvice {
  action: 'same_thread' | 'new_thread';
  reasons: Array<'compaction_limit' | 'context_usage_limit' | 'old_session_code_changed' | string>;
  taskId?: string;
  taskTitle?: string;
  lastActivityAt?: string;
  compactionCount?: number;
  contextUsage?: ContextUsage;
  message?: string;
  suggestedAt?: string;
}

export interface RelatedChange {
  path: string;
  previousPath?: string;
  status: 'added' | 'modified' | 'renamed' | 'deleted' | 'copied' | 'type_changed' | 'unmerged' | 'unknown';
  additions?: number;
  deletions?: number;
}

export interface TemporalRevalidation {
  status: TemporalStatus;
  baselineSha?: string;
  currentSha?: string;
  validatedAt?: string;
  changes?: RelatedChange[];
  warnings?: string[];
}

export interface FileChange {
  path: string;
  additions: number;
  deletions: number;
}

export interface Checkpoint {
  id: string;
  sequence: number;
  sessionTitle: string;
  capturedAt: string;
  branch: string;
  sha: string;
  filesChanged: number;
  additions: number;
  deletions: number;
  testsPassed: number;
  testsFailed: number;
  coverage: number;
  coverageDelta: number;
  freshness: Freshness;
  state: 'confirmed' | 'draft';
  sourceThreadId?: string;
  lastActivityAt?: string;
  turnCount?: number;
  compactionCount?: number;
  contextUsage?: ContextUsage;
  continuationState?: ContinuationState;
  continuationAdvice?: ContinuationAdvice;
  temporalRevalidation?: TemporalRevalidation;
}

export interface Evidence {
  id: string;
  checkpointId: string;
  factId: string;
  sessionId: string;
  sessionLabel: string;
  turnLabel: string;
  capturedAt: string;
  source: string;
  excerpt: string;
  code: string;
  freshness: Freshness;
  selectionReason: string;
  excludedSession: boolean;
  relatedFiles: FileChange[];
}

export interface Fact {
  id: string;
  taskId: string;
  kind: FactKind;
  text: string;
  status: FactStatus;
  confirmedAt?: string;
  updatedAt: string;
  evidenceIds: string[];
  selectionReason?: string;
  relatedFiles: string[];
  deprecatedAfterCommit?: string;
  mixedProvenance: boolean;
  provenanceSessionIds: string[];
}

export interface Session {
  id: string;
  taskId: string;
  sourceThreadId?: string;
  startedAt: string;
  lastActivityAt?: string;
  turnCount: number;
  compactionCount: number;
  contextUsage?: ContextUsage;
  continuationState: ContinuationState;
  excluded: boolean;
}

export interface AutomaticRollover {
  operationId?: string;
  status: 'pending' | 'thread_created' | 'started' | 'failed';
  sourceSessionId?: string;
  newThreadId?: string;
  newTurnId?: string;
  startedAt?: string;
  message?: string;
  warnings?: string[];
}

export interface TaskCodebaseConnection {
  repositoryName: string;
  registeredRoot: string;
  worktreeRoot: string;
  branch: string;
  baselineSha?: string;
  currentSha?: string;
  status: TemporalStatus;
  sourceThreadIds: string[];
  sessionCount: number;
}

export interface Task {
  id: string;
  repositoryId: string;
  title: string;
  status: TaskStatus;
  updatedAt: string;
  checkpointIds: string[];
  goal: string;
  decisions: { confirmed: number; proposed: number };
  openItems: { risks: number; questions: number; actions: number };
  files: { path: string; count: number }[];
  tests: { passing: number; failing: number; skipped: number };
  codebase: TaskCodebaseConnection;
  rollover?: AutomaticRollover;
  titleSuggestion?: TaskTitleSuggestionV1;
}

export interface ResumeCandidate {
  id: string;
  taskId: string;
  uncompletedSessions: number;
  reason: string;
  score: number;
  lastActivityAt?: string;
  continuationAdvice?: ContinuationAdvice;
}

export interface ContextPackFact {
  id: string;
  kind: 'decision' | 'constraint' | 'open_item' | 'progress' | 'goal' | 'note';
  lifecycle: FactStatus;
  freshness: Freshness;
  content: string;
  selection_reason: string;
}

export interface ContextPack {
  task_id: string;
  token_count: number;
  token_budget: number;
  goal?: string;
  facts: ContextPackFact[];
  unresolved_items: ContextPackFact[];
  files: Array<{
    path: string;
    previous_path?: string;
    status: string;
    attribution: string;
  }>;
  tests: Array<{
    name: string;
    status: 'passed' | 'failed' | 'skipped' | 'unknown';
  }>;
  coverage: {
    status: 'complete' | 'degraded' | 'unsupported';
    missing: string[];
    warnings: string[];
  };
  temporal_revalidation?: {
    status: TemporalStatus;
    baseline_head?: string;
    current_head?: string;
    merge_base?: string;
    related_changes?: Array<{
      path: string;
      previous_path?: string;
      status: RelatedChange['status'];
      additions?: number;
      deletions?: number;
    }>;
    checked_paths?: string[];
    warnings?: string[];
  };
  current_validation?: {
    status: TemporalStatus;
    current_head?: string;
    verified_paths?: string[];
    warnings?: string[];
  };
}

export interface BootstrapData {
  trust: {
    classification: 'untrusted_historical_data';
    instructionPolicy: 'display_only_never_execute';
    source: 'previously_on_local_history';
  };
  repository: {
    name: string;
    path: string;
    branch: string;
    connected: boolean;
    state: 'unregistered' | 'registered-empty' | 'active' | 'degraded';
    captureHealth: 'good' | 'degraded' | 'offline';
  };
  tasks: Task[];
  checkpoints: Checkpoint[];
  facts: Fact[];
  evidence: Evidence[];
  sessions: Session[];
  contracts: RegressionContractV1[];
  contractCandidates: RegressionCandidateV1[];
  contractEvaluation: ContractEvaluationV1 | null;
  contractEvaluations: ContractEvaluationV1[];
  taskGroupingOperations: TaskGroupingOperationV1[];
  graphSummary: RelationshipGraphSummaryV1;
  aiRefreshCapability: AiRefreshCapabilityV1;
  factRefreshOperations: AiFactRefreshOperationV1[];
  agents: AgentV1[];
  resumeCandidate?: ResumeCandidate;
  contextPacks: Record<string, ContextPack>;
}

export type CodexImportStatus = 'complete' | 'degraded' | 'unsupported';
export type CodexImportReasonCode = 'synchronized' | 'partial_import' | 'app_server_unsupported';

export interface CoverageV1 {
  status: 'complete' | 'degraded';
  captured: string[];
  missing: string[];
  warnings: string[];
}

export interface CodexImportReportV1 {
  schemaVersion: number;
  repositoryId: string;
  status: CodexImportStatus;
  reasonCode: CodexImportReasonCode;
  importedTaskCount: number;
  semanticEventCount: number;
  duplicateCount: number;
  missingOrUnknownItems: string[];
  lastSyncedAt: string;
  capability: {
    status: CodexImportStatus;
    testedCodexVersion: string;
    detectedCodexVersion?: string;
    warnings: string[];
  };
  coverage: CoverageV1;
  semanticCoverage: CoverageV1;
  notices: Array<{ threadId?: string; message: string }>;
  observedAgentCount: number;
  technicalDetails: string[];
}

export interface RepositoryOverviewV1 {
  repositoryId: string;
  primaryRoot: string;
  taskCount: number;
  recentActivityAt?: string;
  recordStatus: 'empty' | 'ready' | 'degraded';
}
