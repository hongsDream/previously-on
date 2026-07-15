import { spawnSync } from 'node:child_process';
import { readFile, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';

const fixtureId = 'previouslyon-app-server-compaction';
const root = resolve(process.argv[2]);
const sourcePath = join(root, 'src', 'app_server.rs');
const testName = 'compacted_thread_never_reports_complete_history';
const invariantChecks = {
  'degraded-compaction': [
    'normal_test_executed',
    'normal_test_passed',
    'compaction_mutant_applied',
    'compaction_mutant_test_executed',
    'compaction_mutant_rejected',
    'compaction_source_restored',
  ],
  'no-history-invention': [
    'normal_test_executed',
    'normal_test_passed',
    'compaction_mutant_applied',
    'compaction_mutant_test_executed',
    'compaction_mutant_rejected',
    'compaction_source_restored',
  ],
  'token-unavailable': [
    'normal_test_executed',
    'normal_test_passed',
    'token_mutant_applied',
    'token_mutant_test_executed',
    'token_mutant_rejected',
    'token_source_restored',
  ],
};

const normal = runNamedTest(root);
const compaction = await runMutation({
  sourcePath,
  mutate: removeCompactionDegradation,
});
const token = await runMutation({
  sourcePath,
  mutate: fabricateTokenUsage,
});
const evidence = {
  normal_test_executed: normal.executed,
  normal_test_passed: normal.passed,
  compaction_mutant_applied: compaction.applied,
  compaction_mutant_test_executed: compaction.executed,
  compaction_mutant_rejected: compaction.rejected,
  compaction_source_restored: compaction.restored,
  token_mutant_applied: token.applied,
  token_mutant_test_executed: token.executed,
  token_mutant_rejected: token.rejected,
  token_source_restored: token.restored,
};
const violations = new Set();
for (const [invariantId, checks] of Object.entries(invariantChecks)) {
  if (checks.some((check) => evidence[check] !== true)) violations.add(invariantId);
}
const violatedInvariantIds = [...violations].sort();
process.stdout.write(`PREVIOUSLY_ON_HIDDEN_ORACLE_V1 ${JSON.stringify({
  fixtureId,
  version: 1,
  assertions: Object.keys(evidence).length,
  violatedInvariantIds,
})}\n`);
if (violatedInvariantIds.length > 0) process.exitCode = 1;

function runNamedTest(cwd) {
  const result = spawnSync('cargo', [
    'test', '--locked', '--offline', '--test', 'integration_app_server',
    testName, '--', '--exact', '--nocapture',
  ], {
    cwd,
    encoding: 'utf8',
    env: { ...process.env, CARGO_NET_OFFLINE: 'true', CARGO_TERM_COLOR: 'never' },
    maxBuffer: 1024 * 1024,
    timeout: 180_000,
  });
  const output = `${result.stdout ?? ''}\n${result.stderr ?? ''}`;
  const executed = /running\s+1\s+test/.test(output)
    && new RegExp(`test ${testName.replaceAll('_', '[_]')} \\.\\.\\. (?:ok|FAILED)`).test(output);
  return {
    executed,
    passed: executed && result.status === 0 && !result.error && result.signal === null,
    rejected: executed
      && Number.isInteger(result.status)
      && result.status !== 0
      && !result.error
      && result.signal === null
      && new RegExp(`test ${testName.replaceAll('_', '[_]')} \\.\\.\\. FAILED`).test(output),
  };
}

async function runMutation({ sourcePath: path, mutate }) {
  const original = await readFile(path);
  const source = original.toString('utf8');
  const mutated = mutate(source);
  if (typeof mutated !== 'string' || mutated === source) {
    return { applied: false, executed: false, rejected: false, restored: true };
  }
  let result = { executed: false, rejected: false };
  try {
    await writeFile(path, mutated, 'utf8');
    result = runNamedTest(root);
  } catch (error) {
    process.stderr.write(`hidden mutation failed: ${error.message}\n`);
  } finally {
    await writeFile(path, original);
  }
  const restored = (await readFile(path)).equals(original);
  return { applied: true, executed: result.executed, rejected: result.rejected, restored };
}

function removeCompactionDegradation(source) {
  const needle = `    if thread.get("compacted").and_then(Value::as_bool) == Some(true)
        || value_contains_marker(thread.get("status"), &["compact", "incomplete"])
    {
        degrade(
            &mut coverage,
            "complete thread history",
            "thread is compacted or incomplete; available turns were imported as untrusted data",
        );
    }

`;
  if (source.split(needle).length !== 2) return null;
  return source.replace(needle, '    // hidden mutant: compacted history is incorrectly treated as complete\n\n');
}

function fabricateTokenUsage(source) {
  const needle = '    let mut saw_prompt = false;';
  if (source.split(needle).length !== 2) return null;
  const mutant = `    events.push(semantic_event(
        format!("codex-app-server:thread:{}:fabricated-token-usage", thread.id),
        repository_id,
        &thread.session_id,
        EventKind::ContextUsageUpdated,
        updated_at,
        2,
        json!({
            "thread_id": thread.id,
            "source_thread_id": thread.id,
            "turn_id": "fabricated",
            "session_id": thread.session_id,
            "repository_path": repository_path,
            "context_usage": { "total_tokens": 1, "model_context_window": 1 },
            "app_server_source": "fabricated",
            "untrusted_data": true,
            "raw_transcript_stored": false
        }),
        &coverage,
    ));

`;
  return source.replace(needle, `${mutant}${needle}`);
}
