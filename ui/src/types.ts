export type Freshness = 'fresh' | 'stale' | 'broken';
export type FactStatus = 'candidate' | 'confirmed' | 'pinned' | 'invalid' | 'superseded';
export type TaskStatus = 'active' | 'completed' | 'abandoned';

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
  resumeCandidate?: ResumeCandidate;
  contextPacks: Record<string, ContextPack>;
}
