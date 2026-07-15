import { spawnSync } from 'node:child_process';
import { readFile, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';

const fixtureId = 'previouslyon-linked-worktree';
const root = resolve(process.argv[2]);
const sourcePath = join(root, 'src', 'mcp.rs');
const invariantChecks = {
  'checkpoint-root': [
    'latest_checkpoint_selected',
    'earlier_checkpoint_not_selected',
    'empty_latest_falls_back',
    'missing_checkpoint_falls_back',
    'helper_call_mutant_applied',
    'named_test_executed',
    'named_test_rejected_mutant',
    'mutated_source_restored',
    'executable_completed',
    'injected_source_restored',
  ],
  'logical-repository': [
    'logical_repository_rejected',
    'executable_completed',
    'injected_source_restored',
  ],
  'stale-fail-closed': [
    'freshness_error_is_stale',
    'executable_completed',
    'injected_source_restored',
  ],
};

const hiddenModule = String.raw`
#[cfg(test)]
mod continuation_hidden_checkpoint_path_oracle {
    use super::{checkpoint_repository_path, fact_freshness, McpBackend, StoreMcpBackend};
    use crate::domain::{
        CheckpointV1, CoverageV1, FactKind, FactLifecycle, FactV1, Freshness, GitSnapshotV1,
        RepositoryV1, TaskLifecycle, TaskV1, SCHEMA_VERSION_V1,
    };
    use crate::store::Store;
    use chrono::{DateTime, Utc};
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn snapshot(root: &str) -> GitSnapshotV1 {
        GitSnapshotV1 {
            schema_version: SCHEMA_VERSION_V1,
            repository_id: "repo-1".into(),
            root: root.into(),
            remote_url: None,
            branch: Some("feature".into()),
            head: Some("1".repeat(40)),
            captured_at: DateTime::<Utc>::UNIX_EPOCH,
            dirty_files: Vec::new(),
            working_tree_changes: Vec::new(),
            content_fingerprints: BTreeMap::new(),
        }
    }

    fn checkpoint(id: &str, root: &str) -> CheckpointV1 {
        CheckpointV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: id.into(),
            repository_id: "repo-1".into(),
            task_id: "task-1".into(),
            session_id: format!("session-{id}"),
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            goal_hint: None,
            git_before: None,
            git_after: snapshot(root),
            changed_files: Vec::new(),
            tests: Vec::new(),
            failures: Vec::new(),
            unresolved_items: Vec::new(),
            coverage: CoverageV1::default(),
        }
    }

    fn fact() -> FactV1 {
        FactV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: "fact-1".into(),
            repository_id: "repo-1".into(),
            task_id: "task-1".into(),
            kind: FactKind::Decision,
            lifecycle: FactLifecycle::Confirmed,
            freshness: Freshness::Fresh,
            content: "checkpoint root remains authoritative".into(),
            evidence_ids: Vec::new(),
            superseded_by: None,
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            updated_at: DateTime::<Utc>::UNIX_EPOCH,
        }
    }

    #[test]
    fn continuation_hidden_checkpoint_repository_path_semantics() {
        let checkpoints = vec![checkpoint("first", "/checkpoint-first"), checkpoint("last", "/checkpoint-last")];
        let latest = checkpoint_repository_path(&checkpoints, "/registered");
        let latest_checkpoint_selected = latest == "/checkpoint-last";
        let earlier_checkpoint_not_selected = latest != "/checkpoint-first";
        let empty_latest = vec![checkpoint("first", "/checkpoint-first"), checkpoint("last", "")];
        let empty_latest_falls_back = checkpoint_repository_path(&empty_latest, "/registered") == "/registered";
        let missing_checkpoint_falls_back = checkpoint_repository_path(&[], "/registered") == "/registered";
        let freshness_error_is_stale = fact_freshness(
            "/definitely/missing/previously-on-hidden-oracle",
            &fact(),
            &[],
            &[],
            &[],
        ) == Freshness::Stale;

        let temp = TempDir::new().unwrap();
        let database = temp.path().join("previously.sqlite3");
        let store = Store::open(&database).unwrap();
        let now = DateTime::<Utc>::UNIX_EPOCH;
        for id in ["registered-repo", "foreign-repo"] {
            store.upsert_repository(&RepositoryV1 {
                schema_version: SCHEMA_VERSION_V1,
                id: id.into(),
                path: temp.path().to_string_lossy().into_owned(),
                remote_url: None,
                created_at: now,
                updated_at: now,
            }).unwrap();
        }
        store.upsert_task(&TaskV1 {
            schema_version: SCHEMA_VERSION_V1,
            id: "foreign-task".into(),
            repository_id: "foreign-repo".into(),
            title: "Foreign task".into(),
            goal: None,
            lifecycle: TaskLifecycle::Active,
            branch: None,
            created_at: now,
            updated_at: now,
        }).unwrap();
        drop(store);
        let backend = StoreMcpBackend::open(&database, "registered-repo".into()).unwrap();
        let logical_repository_rejected = backend
            .resume_task("foreign-task", Some(1_200))
            .unwrap_err()
            .to_string()
            .contains("does not belong to the registered repository");

        println!(
            "PREVIOUSLY_ON_RUST_ORACLE_V1 {}",
            serde_json::json!({
                "latest_checkpoint_selected": latest_checkpoint_selected,
                "earlier_checkpoint_not_selected": earlier_checkpoint_not_selected,
                "empty_latest_falls_back": empty_latest_falls_back,
                "missing_checkpoint_falls_back": missing_checkpoint_falls_back,
                "freshness_error_is_stale": freshness_error_is_stale,
                "logical_repository_rejected": logical_repository_rejected,
            })
        );
        assert!(latest_checkpoint_selected
            && earlier_checkpoint_not_selected
            && empty_latest_falls_back
            && missing_checkpoint_falls_back
            && freshness_error_is_stale
            && logical_repository_rejected);
    }
}
`;

const semanticEvidence = await executeInjectedModule();
const mutationEvidence = await namedTestKillsFallbackMutant();
const evidence = semanticEvidence ? { ...semanticEvidence, ...mutationEvidence } : null;
const assertions = evidence ? Object.keys(evidence).length : 0;
const violations = new Set();
for (const [invariantId, checks] of Object.entries(invariantChecks)) {
  if (!evidence || checks.some((check) => evidence[check] !== true)) violations.add(invariantId);
}
const violatedInvariantIds = [...violations].sort();
process.stdout.write(`PREVIOUSLY_ON_HIDDEN_ORACLE_V1 ${JSON.stringify({ fixtureId, version: 1, assertions, violatedInvariantIds })}\n`);
if (violatedInvariantIds.length > 0) process.exitCode = 1;

async function executeInjectedModule() {
  const original = await readFile(sourcePath);
  let result;
  try {
    await writeFile(sourcePath, `${original.toString('utf8')}\n${hiddenModule}`, 'utf8');
    result = spawnSync('cargo', [
      'test', '--locked', '--offline', '--lib',
      'mcp::continuation_hidden_checkpoint_path_oracle::continuation_hidden_checkpoint_repository_path_semantics',
      '--', '--exact', '--nocapture',
    ], {
      cwd: root,
      encoding: 'utf8',
      env: { ...process.env, CARGO_NET_OFFLINE: 'true', CARGO_TERM_COLOR: 'never' },
      maxBuffer: 1024 * 1024,
      timeout: 180_000,
    });
  } catch (error) {
    process.stderr.write(`hidden executable oracle setup failed: ${error.message}\n`);
    return null;
  } finally {
    await writeFile(sourcePath, original);
  }
  const restored = (await readFile(sourcePath)).equals(original);
  const output = `${result.stdout ?? ''}\n${result.stderr ?? ''}`;
  const records = output.split('\n').filter((line) => line.startsWith('PREVIOUSLY_ON_RUST_ORACLE_V1 '));
  if (records.length !== 1) {
    process.stderr.write(`hidden executable oracle failed before evidence (status=${result.status}, signal=${result.signal ?? 'none'})\n`);
    return null;
  }
  try {
    return {
      ...JSON.parse(records[0].slice('PREVIOUSLY_ON_RUST_ORACLE_V1 '.length)),
      executable_completed: result.status === 0 && !result.error && result.signal === null,
      injected_source_restored: restored,
    };
  } catch {
    process.stderr.write('hidden executable oracle emitted invalid evidence\n');
    return null;
  }
}

async function namedTestKillsFallbackMutant() {
  const original = await readFile(sourcePath);
  const source = original.toString('utf8');
  const callPattern = /checkpoint_repository_path\(\s*&checkpoints\s*,\s*&registered_repository_path\s*\)/g;
  const matches = [...source.matchAll(callPattern)];
  if (matches.length !== 1) {
    return {
      helper_call_mutant_applied: false,
      named_test_executed: false,
      named_test_rejected_mutant: false,
      mutated_source_restored: true,
    };
  }
  const mutant = source.replace(callPattern, 'registered_repository_path.as_str()');
  let result;
  try {
    await writeFile(sourcePath, mutant, 'utf8');
    result = spawnSync('cargo', [
      'test', '--locked', '--offline', '--test', 'integration_mcp',
      'store_resume_task_revalidates_the_checkpoint_worktree_not_the_registered_sibling',
      '--', '--exact', '--nocapture',
    ], {
      cwd: root,
      encoding: 'utf8',
      env: { ...process.env, CARGO_NET_OFFLINE: 'true', CARGO_TERM_COLOR: 'never' },
      maxBuffer: 1024 * 1024,
      timeout: 180_000,
    });
  } finally {
    await writeFile(sourcePath, original);
  }
  const output = `${result?.stdout ?? ''}\n${result?.stderr ?? ''}`;
  const namedTestExecuted = /running\s+1\s+test/.test(output)
    && /test store_resume_task_revalidates_the_checkpoint_worktree_not_the_registered_sibling \.\.\. (?:ok|FAILED)/.test(output);
  return {
    helper_call_mutant_applied: true,
    named_test_executed: namedTestExecuted,
    named_test_rejected_mutant: namedTestExecuted
      && Number.isInteger(result?.status)
      && result.status !== 0
      && !result.error
      && result.signal === null
      && /test store_resume_task_revalidates_the_checkpoint_worktree_not_the_registered_sibling \.\.\. FAILED/.test(output),
    mutated_source_restored: (await readFile(sourcePath)).equals(original),
  };
}
