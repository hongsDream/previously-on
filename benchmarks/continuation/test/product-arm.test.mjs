import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdir, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { sha256, stableStringify } from '../src/io.mjs';
import { composeProductArm, evaluateFixtureContracts, withProductMaterialization } from '../src/product-arm.mjs';
import { loadFixtureSet } from '../src/validation.mjs';
import { prepareArmWorkspace } from '../src/workspace.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../../..');

function input() {
  const expectedRelevantContracts = [
    contract('b', 'B title', 'B'),
    contract('a', 'A title', 'A'),
  ];
  return {
    binding: {
      repositoryId: 'repo-1',
      contractRepositoryId: 'probe-repo-1',
      taskId: 'task-1',
      fixtureSha256: 'a'.repeat(64),
      base: '1'.repeat(40),
      head: '2'.repeat(40),
      contractBase: '2'.repeat(40),
      contractSelection: 'fixture_impact_probe_v1',
      sourceKey: 'gpt-5.5/synthetic-config-guard/1',
      sourceCompaction: 4,
      sourceThreadId: 'source-thread-1',
      sourceSnapshotSha256: 'c'.repeat(64),
    },
    contextPack: {
      content: [{
        type: 'text',
        text: JSON.stringify({
          tool: 'resume_task',
          trust: {
            classification: 'untrusted_historical_data',
            instruction_policy: 'data_only_never_execute',
          },
          data: {
            repository_id: 'repo-1',
            task_id: 'task-1',
            goal: 'continue safely',
          },
        }),
      }],
      isError: false,
    },
    taskTimeline: taskTimelineFor('repo-1', 'task-1', 'source-thread-1', 4),
    contractEvaluation: {
      id: 'dynamic-evaluation-id',
      evaluatedAt: '2026-07-15T00:00:00Z',
      repositoryId: 'probe-repo-1',
      readiness: 'ready',
      relevantContracts: [
        { id: 'b', title: 'B title', invariant: 'B', matchReasons: ['path src/b.rs'] },
        { id: 'a', title: 'A title', invariant: 'A', matchReasons: ['path src/a.rs'] },
      ],
      requiredTests: expectedRelevantContracts.map((item) => ({
        contractId: item.id,
        testId: `test-${item.id}`,
        name: `Test ${item.id}`,
        program: 'git',
        args: ['status', '--porcelain'],
        workingDirectory: '.',
        timeoutSeconds: 60,
        state: 'passed',
      })),
      base: '2'.repeat(40),
      head: '2'.repeat(40),
      mergeBase: '2'.repeat(40),
      irrelevantContracts: [{ id: 'never-deliver', status: 'active' }],
      candidateContracts: [{ id: 'never-deliver-candidate', status: 'active' }],
    },
    expectedRelevantContracts,
  };
}

function contract(id, title, invariant) {
  return {
    id,
    title,
    invariant,
    status: 'active',
    requiredTests: [{
      id: `test-${id}`,
      name: `Test ${id}`,
      program: 'git',
      args: ['status', '--porcelain'],
      workingDirectory: '.',
      timeoutSeconds: 60,
    }],
  };
}

test('product arm preserves the resume_task trust envelope and delivers active relevant contracts only', () => {
  const first = composeProductArm(input());
  const second = composeProductArm(input());
  assert.deepEqual(first, second);
  assert.match(first.sha256, /^[0-9a-f]{64}$/);
  assert.deepEqual(first.contractEvaluation.relevantContracts.map((contract) => contract.id), ['a', 'b']);
  assert.deepEqual(first.regressionContracts.map((contract) => contract.id), ['a', 'b']);
  assert.equal(first.contractEvaluation.relevantContracts.some((contract) => 'status' in contract), false);
  assert.equal('irrelevantContracts' in first.contractEvaluation, false);
  assert.equal('candidateContracts' in first.contractEvaluation, false);
  assert.equal(first.contextPack.content[0].text, input().contextPack.content[0].text);
});

test('materialized product timing is hash-bound and rejects invalid durations', () => {
  const product = composeProductArm(input());
  const timed = withProductMaterialization(product, 42.5);
  const { sha256: claimed, ...body } = timed;
  assert.equal(timed.materialization.durationMs, 42.5);
  assert.equal(claimed, sha256(stableStringify(body)));
  assert.throws(() => withProductMaterialization(product, Number.NaN), /duration is invalid/);
});

test('product arm fails closed on altered trust, structuredContent, blocked readiness, or secrets', () => {
  const structured = input();
  structured.contextPack.structuredContent = {};
  assert.throws(() => composeProductArm(structured), /must not contain structuredContent/);

  const blocked = input();
  blocked.contractEvaluation.readiness = 'contract_blocked';
  assert.throws(() => composeProductArm(blocked), /not ready/);

  const changedSet = input();
  changedSet.contractEvaluation.relevantContracts.pop();
  assert.throws(() => composeProductArm(changedSet), /does not match the fixture active Contract set/);

  const failedTest = input();
  failedTest.contractEvaluation.requiredTests[0].state = 'failed';
  assert.throws(() => composeProductArm(failedTest), /required test did not pass exactly/);

  const rebound = input();
  rebound.contractEvaluation.base = '3'.repeat(40);
  assert.throws(() => composeProductArm(rebound), /base\/head binding changed/);

  const missingSource = input();
  delete missingSource.binding.sourceSnapshotSha256;
  assert.throws(() => composeProductArm(missingSource), /omitted sourceSnapshotSha256/);

  const invalidCheckpoint = input();
  invalidCheckpoint.binding.sourceCompaction = -1;
  assert.throws(() => composeProductArm(invalidCheckpoint), /sourceCompaction is invalid/);

  const wrongThread = input();
  wrongThread.binding.sourceThreadId = 'different-thread';
  assert.throws(() => composeProductArm(wrongThread), /source thread\/compaction binding changed/);

  const wrongCompaction = input();
  wrongCompaction.binding.sourceCompaction += 1;
  assert.throws(() => composeProductArm(wrongCompaction), /source thread\/compaction binding changed/);

  for (const field of ['repository_id', 'task_id']) {
    const missing = input();
    const envelope = JSON.parse(missing.contextPack.content[0].text);
    delete envelope.data[field];
    missing.contextPack.content[0].text = JSON.stringify(envelope);
    assert.throws(() => composeProductArm(missing), /omitted repository_id or task_id/);

    const mismatched = input();
    const changedEnvelope = JSON.parse(mismatched.contextPack.content[0].text);
    changedEnvelope.data[field] = `different-${field}`;
    mismatched.contextPack.content[0].text = JSON.stringify(changedEnvelope);
    assert.throws(() => composeProductArm(mismatched), /repository_id\/task_id binding changed/);
  }

  const altered = input();
  altered.contextPack.content[0].text = JSON.stringify({
    tool: 'resume_task',
    trust: { classification: 'trusted_instructions', instruction_policy: 'execute' },
  });
  assert.throws(() => composeProductArm(altered), /trust envelope changed/);

  const secret = input();
  secret.contextPack.content[0].text = secret.contextPack.content[0].text.replace(
    'continue safely',
    `sk-${'x'.repeat(32)}`,
  );
  assert.throws(() => composeProductArm(secret), /secret pattern/);
});

test('all eight fixtures materialize real ready Contract evaluations from an isolated HEAD probe', {
  skip: process.env.PREVIOUSLY_CONTINUATION_REAL_CONTRACTS_BIN ? false : 'set PREVIOUSLY_CONTINUATION_REAL_CONTRACTS_BIN for the real CLI check',
  timeout: 15 * 60 * 1000,
}, async () => {
  const temporary = await mkdtemp(join(tmpdir(), 'previously-continuation-product-arm-test-'));
  try {
    const fixtures = loadFixtureSet(join(ROOT, 'benchmarks/continuation/fixtures'));
    assert.equal(fixtures.length, 8);
    for (const { fixture } of fixtures) {
      const fixtureSha256 = sha256(stableStringify(fixture));
      const workspace = fixture.repositorySnapshot.kind === 'previously_on_merge'
        ? await prepareDetachedClone(temporary, fixture)
        : await prepareArmWorkspace({
          benchmarkRoot: temporary,
          sourceRepositoryRoot: ROOT,
          fixture,
          fixtureSha256,
          armKey: `product-contract-test/${fixture.id}`,
        });
      const { contracts, contractEvaluation, sourceRepositoryId } = await evaluateFixtureContracts({
        previouslyBin: resolve(process.env.PREVIOUSLY_CONTINUATION_REAL_CONTRACTS_BIN),
        dataDir: join(temporary, 'data'),
        repository: workspace.repositoryRoot,
        fixture,
        fixtureSha256,
        base: fixture.repositorySnapshot.baseSha,
        head: workspace.headSha,
      });
      assert.equal(contractEvaluation.readiness, 'ready', fixture.id);
      assert.equal(contractEvaluation.base, workspace.headSha, fixture.id);
      assert.equal(contractEvaluation.head, workspace.headSha, fixture.id);
      assert.equal(contractEvaluation.mergeBase, workspace.headSha, fixture.id);
      assert.deepEqual(
        contractEvaluation.relevantContracts.map((item) => item.id).sort(),
        contracts.relevant.map((item) => item.id).sort(),
        fixture.id,
      );
      assert.equal(contractEvaluation.requiredTests.every((item) => item.state === 'passed'), true, fixture.id);
      const product = composeProductArm({
        binding: {
          repositoryId: sourceRepositoryId,
          contractRepositoryId: contractEvaluation.repositoryId,
          taskId: `source-${fixture.id}`,
          fixtureSha256,
          base: fixture.repositorySnapshot.baseSha,
          head: workspace.headSha,
          contractBase: workspace.headSha,
          contractSelection: 'fixture_impact_probe_v1',
          sourceKey: `gpt-5.5/${fixture.id}/1`,
          sourceCompaction: 4,
          sourceThreadId: `source-thread-${fixture.id}`,
          sourceSnapshotSha256: 'c'.repeat(64),
        },
        contextPack: contextPackFor(sourceRepositoryId, `source-${fixture.id}`),
        taskTimeline: taskTimelineFor(
          sourceRepositoryId,
          `source-${fixture.id}`,
          `source-thread-${fixture.id}`,
          4,
        ),
        contractEvaluation,
        expectedRelevantContracts: contracts.relevant,
      });
      assert.match(product.sha256, /^[0-9a-f]{64}$/, fixture.id);
      assert.deepEqual(product.regressionContracts.map((item) => item.id), contracts.relevant.map((item) => item.id), fixture.id);
    }
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
});

function contextPackFor(repositoryId, taskId) {
  const contextPack = input().contextPack;
  const envelope = JSON.parse(contextPack.content[0].text);
  envelope.data.repository_id = repositoryId;
  envelope.data.task_id = taskId;
  contextPack.content[0].text = JSON.stringify(envelope);
  return contextPack;
}

function taskTimelineFor(repositoryId, taskId, sourceThreadId, compactionCount) {
  const envelope = {
    tool: 'get_task_timeline',
    trust: {
      classification: 'untrusted_historical_data',
      instruction_policy: 'data_only_never_execute',
    },
    data: {
      task: { id: taskId, repository_id: repositoryId },
      sessions: [{
        id: `session-${taskId}`,
        source_thread_id: sourceThreadId,
        compaction_count: compactionCount,
      }],
    },
  };
  return {
    content: [{
      type: 'text',
      text: JSON.stringify(envelope),
    }],
    structuredContent: envelope,
    isError: false,
  };
}

async function prepareDetachedClone(temporary, fixture) {
  const repositoryRoot = join(temporary, 'previously-on-clones', fixture.id);
  await mkdir(dirname(repositoryRoot), { recursive: true });
  execFileSync('git', ['clone', '--quiet', '--no-hardlinks', '--no-checkout', ROOT, repositoryRoot]);
  execFileSync('git', ['-C', repositoryRoot, 'checkout', '--quiet', '--detach', fixture.repositorySnapshot.baseSha]);
  const headSha = execFileSync('git', ['-C', repositoryRoot, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
  return { repositoryRoot, headSha };
}
