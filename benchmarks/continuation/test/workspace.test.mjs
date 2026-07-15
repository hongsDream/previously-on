import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { cp, lstat, mkdtemp, mkdir, readFile, rename, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import {
  directoryDigest,
  fixtureDigest,
  inspectWorkspaceChanges,
  prepareArmWorkspace,
  runFixtureTest,
  workspaceIdFor,
} from '../src/workspace.mjs';

const BENCHMARK_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const TEMPLATE_ROOT = join(BENCHMARK_ROOT, 'repositories');
const FIXTURE_ROOT = join(BENCHMARK_ROOT, 'fixtures');

async function loadFixture(id) {
  return JSON.parse(await readFile(join(FIXTURE_ROOT, `${id}.json`), 'utf8'));
}

async function temporaryCase(t) {
  const root = await mkdtemp(join(tmpdir(), 'previously-on-continuation-workspace-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  return root;
}

test('fixture and template digests are stable and workspace ids bind arm plus fixture', async (t) => {
  const root = await temporaryCase(t);
  const fixture = await loadFixture('synthetic-config-guard');
  const reordered = Object.fromEntries(Object.entries(fixture).reverse());
  assert.equal(fixtureDigest(fixture), fixtureDigest(reordered));

  const template = join(TEMPLATE_ROOT, fixture.id);
  const first = await directoryDigest(template);
  assert.equal(first, await directoryDigest(template));
  const copied = join(root, 'template-copy');
  await cp(template, copied, { recursive: true });
  await writeFile(join(copied, 'src/config.ts'), '// changed\n', 'utf8');
  assert.notEqual(first, await directoryDigest(copied));

  const fixtureSha256 = fixtureDigest(fixture);
  assert.equal(
    workspaceIdFor({ armKey: { repetition: 1, model: 'gpt-5.5' }, fixtureSha256 }),
    workspaceIdFor({ armKey: { model: 'gpt-5.5', repetition: 1 }, fixtureSha256 }),
  );
  assert.notEqual(
    workspaceIdFor({ armKey: 'first', fixtureSha256 }),
    workspaceIdFor({ armKey: 'second', fixtureSha256 }),
  );
});

test('synthetic arms are isolated, report bounded changes, run exact argv, and reset idempotently', async (t) => {
  const root = await temporaryCase(t);
  const benchmarkRoot = join(root, 'benchmark');
  const fixture = await loadFixture('synthetic-config-guard');
  const fixtureSha256 = fixtureDigest(fixture);
  const first = await prepareArmWorkspace({
    benchmarkRoot,
    fixture,
    fixtureSha256,
    armKey: 'gpt-5.5/config/same/0/1',
  });
  const second = await prepareArmWorkspace({
    benchmarkRoot,
    fixture,
    fixtureSha256,
    armKey: 'gpt-5.5/config/native/0/1',
  });
  assert.notEqual(first.repositoryRoot, second.repositoryRoot);
  assert.equal(first.templateSha256, await directoryDigest(join(TEMPLATE_ROOT, fixture.id)));

  const oraclePath = join(dirname(first.repositoryRoot), '.immutable-oracle', `${fixture.id}.v1.mjs`);
  await assert.rejects(lstat(oraclePath), { code: 'ENOENT' });

  const firstConfig = join(first.repositoryRoot, 'src/config.ts');
  const original = await readFile(firstConfig, 'utf8');
  await writeFile(firstConfig, `${original}\n// arm-one-only\n`, 'utf8');
  await writeFile(join(first.repositoryRoot, 'unexpected.txt'), 'not allowed\n', 'utf8');
  assert.equal((await readFile(join(second.repositoryRoot, 'src/config.ts'), 'utf8')), original);

  const inspection = await inspectWorkspaceChanges({ repositoryRoot: first.repositoryRoot, fixture });
  assert.deepEqual(inspection.changedFilePaths, ['src/config.ts', 'unexpected.txt']);
  assert.deepEqual(inspection.unexpectedFiles, ['unexpected.txt']);
  assert.match(inspection.changeDigestSha256, /^[0-9a-f]{64}$/);

  const testResult = await runFixtureTest({ repositoryRoot: first.repositoryRoot, fixture });
  assert.equal(testResult.declaredPassed, true, testResult.stderr);
  assert.equal(testResult.declaredTestCount, 2);
  assert.equal(testResult.executionCountPassed, true);
  assert.equal(testResult.oracle.passed, false);
  assert.deepEqual(testResult.oracle.violatedInvariantIds, ['bounded-retries']);
  assert.equal(testResult.passed, false);
  assert.deepEqual(testResult.argv, fixture.finalChallenge.requiredTestCommand);
  assert.match(testResult.stdout, /tests 2/);

  const oracleInfo = await lstat(oraclePath);
  assert.equal(oracleInfo.isFile(), true);
  assert.equal(oracleInfo.mode & 0o222, 0);
  assert.equal(first.oracleSha256, fixture.oracle.sha256);

  const reset = await prepareArmWorkspace({
    benchmarkRoot,
    fixture,
    fixtureSha256,
    armKey: 'gpt-5.5/config/same/0/1',
  });
  assert.equal(reset.repositoryRoot, first.repositoryRoot);
  await assert.rejects(lstat(oraclePath), { code: 'ENOENT' });
  assert.equal(await readFile(firstConfig, 'utf8'), original);
  assert.deepEqual(
    (await inspectWorkspaceChanges({ repositoryRoot: first.repositoryRoot, fixture })).changedFiles,
    [],
  );
});

test('all four synthetic templates run declared tests offline but cannot satisfy hidden challenge oracles', async (t) => {
  const root = await temporaryCase(t);
  const fixtureIds = [
    'synthetic-config-guard',
    'synthetic-tenant-cache',
    'synthetic-parser-rename',
    'synthetic-retry-budget',
  ];
  for (const fixtureId of fixtureIds) {
    const fixture = await loadFixture(fixtureId);
    const prepared = await prepareArmWorkspace({
      benchmarkRoot: join(root, fixtureId),
      fixture,
      fixtureSha256: fixtureDigest(fixture),
      armKey: `baseline/${fixtureId}`,
    });
    const result = await runFixtureTest({ repositoryRoot: prepared.repositoryRoot, fixture });
    assert.equal(result.declaredPassed, true, `${fixtureId}: ${result.stderr}`);
    assert.equal(result.executionCountPassed, true, `${fixtureId}: expected declared tests to execute`);
    assert.equal(result.oracle.passed, false, `${fixtureId}: incomplete baseline must fail hidden oracle`);
    assert.equal(result.passed, false);
    assert.deepEqual(result.argv, fixture.finalChallenge.requiredTestCommand);
    assert.deepEqual(
      (await inspectWorkspaceChanges({ repositoryRoot: prepared.repositoryRoot, fixture })).changedFiles,
      [],
      `${fixtureId} test artifacts must be ignored`,
    );
  }
});

test('change inspection preserves rename origin without treating it as an out-of-scope file', async (t) => {
  const root = await temporaryCase(t);
  const fixture = await loadFixture('synthetic-parser-rename');
  const prepared = await prepareArmWorkspace({
    benchmarkRoot: join(root, 'benchmark'),
    fixture,
    fixtureSha256: fixtureDigest(fixture),
    armKey: 'rename',
  });
  await rename(
    join(prepared.repositoryRoot, 'src/legacy_parser.rs'),
    join(prepared.repositoryRoot, 'src/parser.rs'),
  );
  const inspected = await inspectWorkspaceChanges({ repositoryRoot: prepared.repositoryRoot, fixture });
  assert.deepEqual(inspected.changedFiles, [{
    path: 'src/parser.rs',
    oldPath: 'src/legacy_parser.rs',
    status: 'renamed',
    porcelain: ' R',
  }]);
  assert.deepEqual(inspected.unexpectedFiles, []);
});

test('test capture redacts credentials, bounds output, and enforces timeout without a shell', async (t) => {
  const root = await temporaryCase(t);
  const fixture = await loadFixture('synthetic-config-guard');
  const prepared = await prepareArmWorkspace({
    benchmarkRoot: join(root, 'benchmark'),
    fixture,
    fixtureSha256: fixtureDigest(fixture),
    armKey: 'capture',
  });
  const noisy = structuredClone(fixture);
  delete noisy.oracle;
  noisy.finalChallenge.requiredTestCommand = [
    process.execPath,
    '-e',
    'console.log("token=" + "sk-" + "a".repeat(32)); console.error("x".repeat(2048))',
  ];
  const captured = await runFixtureTest({
    repositoryRoot: prepared.repositoryRoot,
    fixture: noisy,
    outputLimitBytes: 128,
  });
  assert.equal(captured.passed, true);
  assert.equal(captured.outputTruncated, true);
  assert.doesNotMatch(`${captured.stdout}${captured.stderr}`, /sk-a{16}/);
  assert.match(captured.stdout, /\[REDACTED_TOKEN\]/);

  const slow = structuredClone(fixture);
  delete slow.oracle;
  slow.finalChallenge.requiredTestCommand = [process.execPath, '-e', 'setTimeout(() => {}, 5000)'];
  const timed = await runFixtureTest({
    repositoryRoot: prepared.repositoryRoot,
    fixture: slow,
    timeoutMs: 50,
  });
  assert.equal(timed.passed, false);
  assert.equal(timed.timedOut, true);
});

test('hidden oracle rejects weakened synthetic tests even when the mutable command reports success', async (t) => {
  const root = await temporaryCase(t);
  const fixture = await loadFixture('synthetic-config-guard');
  const prepared = await prepareArmWorkspace({
    benchmarkRoot: join(root, 'benchmark'),
    fixture,
    fixtureSha256: fixtureDigest(fixture),
    armKey: 'weakened-tests',
  });
  await writeFile(
    join(prepared.repositoryRoot, 'tests/config.test.ts'),
    "import test from 'node:test';\ntest('weakened one', () => {});\ntest('weakened two', () => {});\n",
    'utf8',
  );
  const result = await runFixtureTest({ repositoryRoot: prepared.repositoryRoot, fixture });
  assert.equal(result.exitCode, 0);
  assert.equal(result.declaredPassed, true);
  assert.equal(result.declaredTestCount, 2);
  assert.equal(result.executionCountPassed, true);
  assert.equal(result.oracle.passed, false);
  assert.deepEqual(result.oracle.violatedInvariantIds, ['bounded-retries']);
  assert.equal(result.passed, false);
});

test('hidden oracle exposes an empty bounded invariant set on semantic success', async (t) => {
  const root = await temporaryCase(t);
  const fixture = await loadFixture('synthetic-config-guard');
  const prepared = await prepareArmWorkspace({
    benchmarkRoot: join(root, 'benchmark'),
    fixture,
    fixtureSha256: fixtureDigest(fixture),
    armKey: 'semantic-success',
  });
  await writeFile(join(prepared.repositoryRoot, 'src/config.ts'), `export class ConfigError extends Error {}
const ALLOWED_KEYS = new Set(['endpoint', 'safeMode', 'maxRetries']);
export function parseConfig(input = {}) {
  for (const key of Object.keys(input)) if (!ALLOWED_KEYS.has(key)) throw new ConfigError('unknown key');
  if ('safeMode' in input && typeof input.safeMode !== 'boolean') throw new ConfigError('bad safeMode');
  if ('endpoint' in input && typeof input.endpoint !== 'string') throw new ConfigError('bad endpoint');
  if ('maxRetries' in input && (!Number.isInteger(input.maxRetries) || input.maxRetries < 0 || input.maxRetries > 5)) throw new ConfigError('bad retries');
  return { endpoint: input.endpoint ?? 'https://service.invalid', safeMode: input.safeMode ?? true, maxRetries: input.maxRetries ?? 2 };
}
`, 'utf8');
  const result = await runFixtureTest({ repositoryRoot: prepared.repositoryRoot, fixture });
  assert.equal(result.declaredPassed, true);
  assert.equal(result.executionCountPassed, true);
  assert.equal(result.oracle.passed, true, result.stderr);
  assert.deepEqual(result.oracle.violatedInvariantIds, []);
  assert.equal(result.passed, true);
});

test('Rust exact filter with zero executed tests fails despite cargo exit zero', async (t) => {
  const root = await temporaryCase(t);
  const fixture = await loadFixture('synthetic-parser-rename');
  const prepared = await prepareArmWorkspace({
    benchmarkRoot: join(root, 'benchmark'),
    fixture,
    fixtureSha256: fixtureDigest(fixture),
    armKey: 'zero-rust-tests',
  });
  const filtered = structuredClone(fixture);
  filtered.finalChallenge.requiredTestCommand = [
    'cargo', 'test', '--test', 'parser', 'definitely_missing_test', '--', '--exact',
  ];
  filtered.oracle.requiredCommandTestCount = 1;
  const result = await runFixtureTest({ repositoryRoot: prepared.repositoryRoot, fixture: filtered });
  assert.equal(result.exitCode, 0);
  assert.equal(result.declaredPassed, true);
  assert.equal(result.declaredTestCount, 0);
  assert.equal(result.executionCountPassed, false);
  assert.equal(result.passed, false);
});

test('previously_on arms use a detached verified worktree without changing the source checkout', async (t) => {
  const root = await temporaryCase(t);
  const sourceRoot = join(root, 'source');
  await mkdir(sourceRoot, { recursive: true });
  git(sourceRoot, ['init', '--quiet']);
  await writeFile(join(sourceRoot, 'tracked.txt'), 'baseline\n', 'utf8');
  git(sourceRoot, ['add', 'tracked.txt']);
  git(sourceRoot, [
    '-c', 'user.name=Fixture Test',
    '-c', 'user.email=fixture@invalid.example',
    '-c', 'commit.gpgsign=false',
    'commit', '--quiet', '-m', 'fixture source',
  ]);
  const baseSha = git(sourceRoot, ['rev-parse', 'HEAD']).trim();
  const sourceHeadBefore = baseSha;
  const sourceStatusBefore = git(sourceRoot, ['status', '--porcelain']);
  const fixture = {
    schemaVersion: 1,
    id: 'previouslyon-workspace-test',
    repositorySnapshot: { kind: 'previously_on_merge', baseSha, source: 'test repository' },
    changedFiles: [{ path: 'tracked.txt', status: 'modified' }],
    finalChallenge: {
      allowedFiles: ['tracked.txt'],
      requiredTestCommand: [process.execPath, '-e', 'process.exit(0)'],
    },
  };
  const fixtureSha256 = fixtureDigest(fixture);
  const benchmarkRoot = join(root, 'benchmark');
  const prepared = await prepareArmWorkspace({
    benchmarkRoot,
    sourceRepositoryRoot: sourceRoot,
    fixture,
    fixtureSha256,
    armKey: 'previously-on/one',
  });
  assert.equal(git(prepared.repositoryRoot, ['rev-parse', 'HEAD']).trim(), baseSha);
  assert.equal(git(sourceRoot, ['rev-parse', 'HEAD']).trim(), sourceHeadBefore);
  assert.equal(git(sourceRoot, ['status', '--porcelain']), sourceStatusBefore);

  await writeFile(join(prepared.repositoryRoot, 'tracked.txt'), 'arm change\n', 'utf8');
  assert.equal((await inspectWorkspaceChanges({ repositoryRoot: prepared.repositoryRoot, fixture })).changedFiles.length, 1);
  await prepareArmWorkspace({
    benchmarkRoot,
    sourceRepositoryRoot: sourceRoot,
    fixture,
    fixtureSha256,
    armKey: 'previously-on/one',
  });
  assert.equal(await readFile(join(prepared.repositoryRoot, 'tracked.txt'), 'utf8'), 'baseline\n');
  assert.equal(await readFile(join(sourceRoot, 'tracked.txt'), 'utf8'), 'baseline\n');
});

function git(cwd, args) {
  return execFileSync('git', args, {
    cwd,
    encoding: 'utf8',
    env: { ...process.env, GIT_TERMINAL_PROMPT: '0' },
  });
}
