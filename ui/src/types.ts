export type Freshness = 'fresh' | 'stale' | 'broken';
export type TemporalStatus = 'unchanged' | 'changed' | 'diverged' | 'broken' | 'degraded';
export type FactStatus = 'candidate' | 'confirmed' | 'pinned' | 'invalid' | 'superseded';
export type TaskStatus = 'active' | 'completed' | 'abandoned';
export type ContinuationState = 'normal' | 'eligible' | 'suggested';
export type ContractStatus = 'active' | 'superseded';
export type ContractReadiness = 'ready' | 'contract_blocked';
export type RequiredTestStatus = 'passed' | 'failed' | 'missing' | 'stale';

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
  sessionLabel: string;
  turnLabel: string;
  capturedAt: string;
  source: string;
  excerpt: string;
  code: string;
  freshness: Freshness;
  selectionReason: string;
  relatedFiles: FileChange[];
}

export interface Fact {
  id: string;
  text: string;
  status: FactStatus;
  confirmedAt?: string;
  updatedAt: string;
  evidenceIds: string[];
}

export interface Task {
  id: string;
  title: string;
  status: TaskStatus;
  updatedAt: string;
  checkpointIds: string[];
  goal: string;
  decisions: { confirmed: number; proposed: number };
  openItems: { risks: number; questions: number; actions: number };
  files: { path: string; count: number }[];
  tests: { passing: number; failing: number; skipped: number };
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
    captureHealth: 'good' | 'degraded' | 'offline';
  };
  tasks: Task[];
  checkpoints: Checkpoint[];
  facts: Fact[];
  evidence: Evidence[];
  contracts: RegressionContractV1[];
  contractCandidates: RegressionCandidateV1[];
  contractEvaluation: ContractEvaluationV1 | null;
  contractEvaluations: ContractEvaluationV1[];
  resumeCandidate?: ResumeCandidate;
  contextPacks: Record<string, ContextPack>;
}
