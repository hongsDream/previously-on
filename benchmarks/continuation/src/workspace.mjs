import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  chmod,
  lstat,
  mkdir,
  open,
  readFile,
  readdir,
  rename,
  rm,
  writeFile,
} from 'node:fs/promises';
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { sha256, stableStringify } from './io.mjs';
import { installIgnoredBuildInputs } from './product-arm.mjs';

const MODULE_ROOT = dirname(fileURLToPath(import.meta.url));
const DEFAULT_TEMPLATE_ROOT = resolve(MODULE_ROOT, '../repositories');
const ORACLE_ROOT = resolve(MODULE_ROOT, '../oracles');
const WORKSPACE_SCHEMA_VERSION = 1;
const DEFAULT_TIMEOUT_MS = 180_000;
const DEFAULT_OUTPUT_LIMIT_BYTES = 256 * 1024;
const INTERNAL_OUTPUT_LIMIT_BYTES = 1024 * 1024;
const FIXED_GIT_DATE = '2000-01-01T00:00:00Z';

export function fixtureDigest(fixture) {
  return sha256(stableStringify(fixture));
}

export function workspaceIdFor({ armKey, fixtureSha256 }) {
  const key = typeof armKey === 'string' ? armKey : stableStringify(armKey);
  assertText(key, 'armKey');
  assertSha256(fixtureSha256, 'fixtureSha256');
  return sha256(`continuation-workspace-v1\0${fixtureSha256}\0${key}`).slice(0, 32);
}

export async function directoryDigest(directory) {
  const root = resolve(directory);
  const paths = await listRegularFiles(root);
  const digest = createHash('sha256');
  for (const path of paths) {
    const name = relative(root, path).split(sep).join('/');
    digest.update(name);
    digest.update('\0');
    digest.update(await readFile(path));
    digest.update('\0');
  }
  return digest.digest('hex');
}

export async function prepareArmWorkspace({
  benchmarkRoot,
  sourceRepositoryRoot,
  fixture,
  fixtureSha256,
  armKey,
  templateRoot = DEFAULT_TEMPLATE_ROOT,
}) {
  assertFixture(fixture);
  assertSha256(fixtureSha256, 'fixtureSha256');
  const actualFixtureSha256 = fixtureDigest(fixture);
  if (actualFixtureSha256 !== fixtureSha256) {
    throw new Error(`fixture digest mismatch for ${fixture.id}`);
  }

  const root = resolve(benchmarkRoot);
  const workspaceBase = join(root, 'results', 'workspaces');
  const workspaceId = workspaceIdFor({ armKey, fixtureSha256 });
  const armRoot = join(workspaceBase, workspaceId);
  const repositoryRoot = join(armRoot, 'repository');
  assertStrictDescendant(workspaceBase, armRoot, 'arm workspace');
  assertStrictDescendant(armRoot, repositoryRoot, 'arm repository');
  await mkdir(workspaceBase, { recursive: true, mode: 0o700 });

  const common = {
    schemaVersion: WORKSPACE_SCHEMA_VERSION,
    workspaceId,
    armKeySha256: sha256(typeof armKey === 'string' ? armKey : stableStringify(armKey)),
    fixtureId: fixture.id,
    fixtureSha256,
    requestedBaseSha: fixture.repositorySnapshot.baseSha,
  };

  let prepared;
  if (fixture.repositorySnapshot.kind === 'synthetic_template') {
    prepared = await prepareSynthetic({
      armRoot,
      repositoryRoot,
      fixture,
      fixtureSha256,
      templateRoot: resolve(templateRoot),
      common,
    });
  } else if (fixture.repositorySnapshot.kind === 'previously_on_merge') {
    prepared = await preparePreviouslyOn({
      armRoot,
      repositoryRoot,
      workspaceBase,
      sourceRepositoryRoot,
      fixture,
      fixtureSha256,
      common,
    });
  } else {
    throw new Error(`unsupported repository snapshot kind ${fixture.repositorySnapshot.kind}`);
  }

  await removeHiddenOracle(armRoot);
  const oracle = fixture.oracle ? await validateHiddenOracleSource(fixture) : null;

  return {
    workspaceId,
    armRoot,
    repositoryRoot,
    fixtureSha256,
    fixtureId: fixture.id,
    snapshotKind: fixture.repositorySnapshot.kind,
    requestedBaseSha: fixture.repositorySnapshot.baseSha,
    headSha: prepared.baselineCommit,
    templateSha256: prepared.templateSha256 ?? null,
    oracleSha256: oracle?.sha256 ?? null,
  };
}

export async function inspectWorkspaceChanges({ repositoryRoot, fixture }) {
  assertFixture(fixture);
  const root = resolve(repositoryRoot);
  await assertGitRepository(root);
  const statusResult = await runGit(['-C', root, 'status', '--porcelain=v1', '-z', '--untracked-files=all']);
  const changes = collapseFixtureRenames(parsePorcelainZ(statusResult.stdout), fixture);
  const allowed = new Set([
    ...(fixture.finalChallenge.allowedFiles ?? []),
    ...(fixture.changedFiles ?? []).map((entry) => entry.previousPath).filter(Boolean),
  ]);
  const digestEntries = [];
  for (const change of changes) {
    const absolute = safeRepositoryPath(root, change.path);
    let contentSha256 = null;
    try {
      const info = await lstat(absolute);
      if (info.isSymbolicLink()) throw new Error(`workspace change is a symbolic link: ${change.path}`);
      if (info.isFile()) contentSha256 = await hashFile(absolute);
    } catch (error) {
      if (error?.code !== 'ENOENT') throw error;
    }
    digestEntries.push({ ...change, contentSha256 });
  }
  return {
    changedFiles: changes,
    changedFilePaths: changes.map((entry) => entry.path),
    unexpectedFiles: changes
      .filter((entry) => !allowed.has(entry.path) && !(entry.oldPath && allowed.has(entry.oldPath)))
      .map((entry) => entry.path),
    changeDigestSha256: sha256(stableStringify(digestEntries)),
  };
}

export function fixtureTestArgv(fixture) {
  assertFixture(fixture);
  const argv = fixture.finalChallenge.requiredTestCommand;
  if (!Array.isArray(argv) || argv.length === 0 || argv.some((value) => typeof value !== 'string' || value.includes('\0'))) {
    throw new Error(`fixture ${fixture.id} has invalid test argv`);
  }
  assertText(argv[0], 'test program');
  const program = basename(argv[0]).toLowerCase();
  if (new Set(['sh', 'bash', 'zsh', 'fish', 'cmd', 'cmd.exe', 'powershell', 'pwsh']).has(program)) {
    throw new Error(`fixture ${fixture.id} test argv may not invoke a shell`);
  }
  return [...argv];
}

export async function runFixtureTest({
  repositoryRoot,
  fixture,
  timeoutMs = DEFAULT_TIMEOUT_MS,
  outputLimitBytes = DEFAULT_OUTPUT_LIMIT_BYTES,
}) {
  const root = resolve(repositoryRoot);
  await assertGitRepository(root);
  const argv = fixtureTestArgv(fixture);
  const result = await runProcess(argv, {
    cwd: root,
    timeoutMs,
    outputLimitBytes,
    env: safeExecutionEnvironment(),
  });
  const declaredTestCount = observedTestCount(argv, `${result.stdout}\n${result.stderr}`);
  const requiredTestCount = fixture.oracle?.requiredCommandTestCount ?? null;
  const declaredPassed = result.exitCode === 0 && !result.timedOut;
  const executionCountPassed = requiredTestCount === null
    || (Number.isInteger(declaredTestCount) && declaredTestCount >= requiredTestCount);
  let oracle = null;
  if (fixture.oracle) {
    const armRoot = dirname(root);
    await removeHiddenOracle(armRoot);
    await installHiddenOracle({ armRoot, fixture });
    oracle = await runHiddenOracle({ repositoryRoot: root, fixture, timeoutMs, outputLimitBytes });
  }
  const passed = declaredPassed && executionCountPassed && (oracle?.passed ?? true);
  const oracleDiagnostic = oracle && !oracle.passed
    ? `\n[hidden-oracle] ${oracle.diagnostic}`
    : '';
  return {
    argv,
    cwd: root,
    passed,
    declaredPassed,
    declaredTestCount,
    requiredTestCount,
    executionCountPassed,
    oracle: oracle && {
      version: fixture.oracle.version,
      sha256: fixture.oracle.sha256,
      passed: oracle.passed,
      assertionCount: oracle.assertionCount,
      violatedInvariantIds: oracle.violatedInvariantIds,
      minimumAssertionCount: fixture.oracle.minimumAssertionCount,
      exitCode: oracle.exitCode,
      timedOut: oracle.timedOut,
    },
    exitCode: result.exitCode,
    signal: result.signal,
    timedOut: result.timedOut,
    durationMs: result.durationMs,
    stdout: redactOutput(result.stdout),
    stderr: redactOutput(`${result.stderr}${oracleDiagnostic}`),
    stdoutBytes: result.stdoutBytes,
    stderrBytes: result.stderrBytes,
    outputTruncated: result.outputTruncated,
  };
}

async function installHiddenOracle({ armRoot, fixture }) {
  if (!fixture.oracle) return null;
  const { bytes, sha256: actualSha256 } = await validateHiddenOracleSource(fixture);
  const oracleDirectory = join(armRoot, '.immutable-oracle');
  const destination = join(oracleDirectory, basename(fixture.oracle.path));
  assertStrictDescendant(armRoot, oracleDirectory, 'hidden oracle directory');
  assertStrictDescendant(oracleDirectory, destination, 'hidden oracle file');
  await mkdir(oracleDirectory, { recursive: true, mode: 0o700 });
  await chmod(oracleDirectory, 0o700);
  const temporary = `${destination}.tmp`;
  await writeFile(temporary, bytes, { mode: 0o400 });
  await chmod(temporary, 0o400);
  await rename(temporary, destination);
  await chmod(destination, 0o400);
  return { path: destination, sha256: actualSha256 };
}

async function validateHiddenOracleSource(fixture) {
  validateOracleBinding(fixture);
  const source = resolve(MODULE_ROOT, '..', fixture.oracle.path);
  assertStrictDescendant(ORACLE_ROOT, source, 'oracle source');
  const bytes = await readFile(source);
  const actualSha256 = sha256(bytes);
  if (actualSha256 !== fixture.oracle.sha256) {
    throw new Error(`oracle digest mismatch for ${fixture.id}`);
  }
  return { bytes, sha256: actualSha256 };
}

async function removeHiddenOracle(armRoot) {
  const oracleDirectory = join(armRoot, '.immutable-oracle');
  assertStrictDescendant(armRoot, oracleDirectory, 'hidden oracle directory');
  await rm(oracleDirectory, { recursive: true, force: true });
}

async function runHiddenOracle({ repositoryRoot, fixture, timeoutMs, outputLimitBytes }) {
  validateOracleBinding(fixture);
  const armRoot = dirname(repositoryRoot);
  const path = join(armRoot, '.immutable-oracle', basename(fixture.oracle.path));
  assertStrictDescendant(armRoot, path, 'hidden oracle');
  const info = await lstat(path);
  if (!info.isFile() || info.isSymbolicLink() || (info.mode & 0o222) !== 0) {
    throw new Error(`hidden oracle is not an immutable regular file for ${fixture.id}`);
  }
  const actualSha256 = await hashFile(path);
  if (actualSha256 !== fixture.oracle.sha256) {
    throw new Error(`hidden oracle digest mismatch for ${fixture.id}`);
  }
  const result = await runProcess([process.execPath, path, repositoryRoot], {
    cwd: armRoot,
    timeoutMs,
    outputLimitBytes,
    env: safeExecutionEnvironment(),
  });
  let assertionCount = null;
  let violatedInvariantIds = null;
  let diagnostic = result.stderr || result.stdout || `exit ${result.exitCode ?? result.signal}`;
  const records = result.stdout
    .split('\n')
    .filter((line) => line.startsWith('PREVIOUSLY_ON_HIDDEN_ORACLE_V1 '));
  if (records.length === 1) {
    try {
      const record = JSON.parse(records[0].slice('PREVIOUSLY_ON_HIDDEN_ORACLE_V1 '.length));
      if (record.fixtureId === fixture.id && record.version === fixture.oracle.version) {
        assertionCount = record.assertions;
        violatedInvariantIds = boundedInvariantIds(record.violatedInvariantIds, fixture);
      } else {
        diagnostic = 'hidden oracle emitted a mismatched identity';
      }
    } catch {
      diagnostic = 'hidden oracle emitted invalid evidence';
    }
  } else if (result.exitCode === 0 && !result.timedOut) {
    diagnostic = 'hidden oracle omitted unique execution evidence';
  }
  const passed = result.exitCode === 0
    && !result.timedOut
    && Number.isInteger(assertionCount)
    && assertionCount >= fixture.oracle.minimumAssertionCount
    && Array.isArray(violatedInvariantIds)
    && violatedInvariantIds.length === 0;
  return {
    passed,
    assertionCount,
    violatedInvariantIds: violatedInvariantIds ?? [...fixture.oracle.invariantIds],
    exitCode: result.exitCode,
    timedOut: result.timedOut,
    diagnostic: redactOutput(diagnostic).slice(0, 4096),
  };
}

function observedTestCount(argv, output) {
  const program = basename(argv[0]).toLowerCase();
  const matches = [];
  const collect = (pattern) => {
    for (const match of output.matchAll(pattern)) matches.push(Number(match[1]));
  };
  if (program === 'cargo') collect(/^running\s+(\d+)\s+tests?$/gm);
  else if (program === 'npm' || program === 'node') collect(/^#\s*tests\s+(\d+)\s*$/gm);
  else if (program.startsWith('python')) {
    collect(/^(\d+)\s+passed(?:,|\s|$)/gm);
    const explicit = output.match(/^(?:PASSED|FAILED)\s+/gm)?.length ?? 0;
    if (explicit > 0) matches.push(explicit);
  }
  return matches.length > 0 ? Math.max(...matches) : null;
}

function validateOracleBinding(fixture) {
  const oracle = fixture.oracle;
  const fixtureInvariantIds = (fixture.invariants ?? []).map((item) => item.id).sort();
  const boundInvariantIds = Array.isArray(oracle?.invariantIds) ? [...oracle.invariantIds].sort() : [];
  if (
    oracle?.version !== 1
    || oracle.path !== `oracles/${fixture.id}.v1.mjs`
    || !/^[0-9a-f]{64}$/.test(oracle.sha256 ?? '')
    || !Number.isInteger(oracle.requiredCommandTestCount)
    || oracle.requiredCommandTestCount < 1
    || !Number.isInteger(oracle.minimumAssertionCount)
    || oracle.minimumAssertionCount < 1
    || stableStringify(boundInvariantIds) !== stableStringify(fixtureInvariantIds)
  ) {
    throw new Error(`fixture ${fixture.id} has an invalid hidden oracle binding`);
  }
}

function boundedInvariantIds(value, fixture) {
  if (!Array.isArray(value) || value.length > fixture.oracle.invariantIds.length) {
    throw new Error('hidden oracle emitted invalid invariant evidence');
  }
  const allowed = new Set(fixture.oracle.invariantIds);
  const result = [];
  for (const id of value) {
    if (typeof id !== 'string' || id.length > 64 || !allowed.has(id) || result.includes(id)) {
      throw new Error('hidden oracle emitted invalid invariant evidence');
    }
    result.push(id);
  }
  return result.sort();
}

async function prepareSynthetic({ armRoot, repositoryRoot, fixture, fixtureSha256, templateRoot, common }) {
  const template = join(templateRoot, fixture.id);
  assertStrictDescendant(templateRoot, template, 'synthetic template');
  await validateTemplateMetadata(template, fixture);
  const templateSha256 = await directoryDigest(template);
  const existing = await readMetadata(armRoot);
  if (
    existing?.snapshotKind === 'synthetic_template'
    && existing.fixtureSha256 === fixtureSha256
    && existing.templateSha256 === templateSha256
    && await isGitRepository(repositoryRoot)
  ) {
    await resetRepository(repositoryRoot, existing.baselineCommit);
    return existing;
  }

  await removeArmRoot(armRoot, dirname(armRoot));
  await mkdir(repositoryRoot, { recursive: true, mode: 0o700 });
  await copyTree(template, repositoryRoot);
  await runGit(['-C', repositoryRoot, 'init', '--quiet']);
  await runGit(['-C', repositoryRoot, 'add', '--all']);
  await runGit(
    [
      '-C', repositoryRoot,
      '-c', 'user.name=PreviouslyOn Benchmark',
      '-c', 'user.email=benchmark@invalid.example',
      '-c', 'commit.gpgsign=false',
      'commit', '--quiet', '-m', 'benchmark fixture baseline',
    ],
    {
      env: {
        ...safeExecutionEnvironment(),
        GIT_AUTHOR_DATE: FIXED_GIT_DATE,
        GIT_COMMITTER_DATE: FIXED_GIT_DATE,
      },
    },
  );
  const baselineCommit = (await runGit(['-C', repositoryRoot, 'rev-parse', 'HEAD'])).stdout.trim();
  const metadata = {
    ...common,
    snapshotKind: 'synthetic_template',
    templateSha256,
    baselineCommit,
  };
  await writeMetadata(armRoot, metadata);
  return metadata;
}

async function validateTemplateMetadata(template, fixture) {
  let metadata;
  try {
    metadata = JSON.parse(await readFile(join(template, '.continuation-template.v1.json'), 'utf8'));
  } catch (error) {
    throw new Error(`synthetic template metadata is unavailable for ${fixture.id}: ${error.message}`);
  }
  if (metadata.schemaVersion !== 1 || metadata.fixtureId !== fixture.id || metadata.networkInstallRequired !== false) {
    throw new Error(`synthetic template metadata is invalid for ${fixture.id}`);
  }
  if (stableStringify(metadata.baselineTestArgv) !== stableStringify(fixtureTestArgv(fixture))) {
    throw new Error(`synthetic template test argv differs from fixture ${fixture.id}`);
  }
}

async function preparePreviouslyOn({
  armRoot,
  repositoryRoot,
  workspaceBase,
  sourceRepositoryRoot,
  fixture,
  fixtureSha256,
  common,
}) {
  assertText(sourceRepositoryRoot, 'sourceRepositoryRoot');
  const sourceRoot = resolve(sourceRepositoryRoot);
  const baseSha = fixture.repositorySnapshot.baseSha;
  if (!/^[0-9a-f]{40}$/.test(baseSha)) throw new Error(`fixture ${fixture.id} has invalid base SHA`);
  await assertGitRepository(sourceRoot);
  await runGit(['-C', sourceRoot, 'cat-file', '-e', `${baseSha}^{commit}`]);
  const sourceBefore = await sourceWorktreeFingerprint(sourceRoot);
  const existing = await readMetadata(armRoot);
  if (
    existing?.snapshotKind === 'previously_on_merge'
    && existing.fixtureSha256 === fixtureSha256
    && existing.baselineCommit === baseSha
    && await isGitRepository(repositoryRoot)
  ) {
    await resetRepository(repositoryRoot, baseSha);
  } else {
    await removeRegisteredWorktree({ sourceRoot, repositoryRoot, workspaceBase });
    await removeArmRoot(armRoot, workspaceBase);
    await mkdir(armRoot, { recursive: true, mode: 0o700 });
    await runGit(['-C', sourceRoot, 'worktree', 'add', '--detach', repositoryRoot, baseSha]);
    await resetRepository(repositoryRoot, baseSha);
  }
  const head = (await runGit(['-C', repositoryRoot, 'rev-parse', 'HEAD'])).stdout.trim();
  if (head !== baseSha) throw new Error(`worktree HEAD ${head} does not match fixture base ${baseSha}`);
  await installIgnoredBuildInputs(repositoryRoot);
  const sourceAfter = await sourceWorktreeFingerprint(sourceRoot);
  if (sourceAfter !== sourceBefore) throw new Error('source worktree changed while preparing benchmark workspace');
  const metadata = {
    ...common,
    snapshotKind: 'previously_on_merge',
    templateSha256: null,
    baselineCommit: baseSha,
  };
  await writeMetadata(armRoot, metadata);
  return metadata;
}

async function resetRepository(repositoryRoot, baselineCommit) {
  if (!/^[0-9a-f]{40}$/.test(baselineCommit ?? '')) throw new Error('invalid workspace baseline commit');
  await runGit(['-C', repositoryRoot, 'reset', '--hard', baselineCommit]);
  await runGit(['-C', repositoryRoot, 'clean', '-ffdx']);
  const head = (await runGit(['-C', repositoryRoot, 'rev-parse', 'HEAD'])).stdout.trim();
  if (head !== baselineCommit) throw new Error('workspace reset did not restore the bound baseline');
}

async function sourceWorktreeFingerprint(sourceRoot) {
  const [head, unstaged, staged] = await Promise.all([
    runGit(['-C', sourceRoot, 'rev-parse', 'HEAD']),
    runGit(['-C', sourceRoot, 'diff', '--binary', '--no-ext-diff', 'HEAD']),
    runGit(['-C', sourceRoot, 'diff', '--binary', '--no-ext-diff', '--cached', 'HEAD']),
  ]);
  return sha256(`${head.stdout.trim()}\0${unstaged.stdout}\0${staged.stdout}`);
}

async function removeRegisteredWorktree({ sourceRoot, repositoryRoot, workspaceBase }) {
  assertStrictDescendant(workspaceBase, repositoryRoot, 'registered worktree');
  const listing = await runGit(['-C', sourceRoot, 'worktree', 'list', '--porcelain']);
  const registered = listing.stdout
    .split('\n')
    .filter((line) => line.startsWith('worktree '))
    .map((line) => resolve(line.slice('worktree '.length)))
    .includes(repositoryRoot);
  if (registered) await runGit(['-C', sourceRoot, 'worktree', 'remove', '--force', repositoryRoot]);
}

async function removeArmRoot(armRoot, boundary) {
  assertStrictDescendant(boundary, armRoot, 'arm workspace');
  await rm(armRoot, { recursive: true, force: true });
}

async function writeMetadata(armRoot, metadata) {
  await mkdir(armRoot, { recursive: true, mode: 0o700 });
  const path = join(armRoot, 'workspace.v1.json');
  const temporary = `${path}.tmp`;
  await writeFile(temporary, `${JSON.stringify(metadata, null, 2)}\n`, { mode: 0o600 });
  const descriptor = await open(temporary, 'r');
  await descriptor.sync();
  await descriptor.close();
  await rename(temporary, path);
}

async function readMetadata(armRoot) {
  try {
    const value = JSON.parse(await readFile(join(armRoot, 'workspace.v1.json'), 'utf8'));
    if (value?.schemaVersion !== WORKSPACE_SCHEMA_VERSION) return null;
    return value;
  } catch (error) {
    if (error?.code === 'ENOENT' || error instanceof SyntaxError) return null;
    throw error;
  }
}

async function copyTree(source, destination) {
  const entries = await readdir(source, { withFileTypes: true });
  entries.sort((left, right) => left.name.localeCompare(right.name));
  for (const entry of entries) {
    const from = join(source, entry.name);
    const to = join(destination, entry.name);
    if (entry.isSymbolicLink()) throw new Error(`synthetic template contains symbolic link: ${from}`);
    if (entry.isDirectory()) {
      await mkdir(to, { recursive: true, mode: 0o700 });
      await copyTree(from, to);
    } else if (entry.isFile()) {
      await writeFile(to, await readFile(from), { mode: 0o600 });
    } else {
      throw new Error(`synthetic template contains unsupported entry: ${from}`);
    }
  }
}

async function listRegularFiles(root) {
  const files = [];
  const visit = async (directory) => {
    const entries = await readdir(directory, { withFileTypes: true });
    entries.sort((left, right) => left.name.localeCompare(right.name));
    for (const entry of entries) {
      const path = join(directory, entry.name);
      if (entry.isSymbolicLink()) throw new Error(`directory digest rejects symbolic link: ${path}`);
      if (entry.isDirectory()) await visit(path);
      else if (entry.isFile()) files.push(path);
      else throw new Error(`directory digest rejects special file: ${path}`);
    }
  };
  await visit(root);
  return files;
}

async function hashFile(path) {
  const descriptor = await open(path, 'r');
  try {
    const digest = createHash('sha256');
    const buffer = Buffer.allocUnsafe(64 * 1024);
    let position = 0;
    while (true) {
      const { bytesRead } = await descriptor.read(buffer, 0, buffer.length, position);
      if (bytesRead === 0) break;
      digest.update(buffer.subarray(0, bytesRead));
      position += bytesRead;
    }
    return digest.digest('hex');
  } finally {
    await descriptor.close();
  }
}

function parsePorcelainZ(output) {
  const fields = output.split('\0');
  const changes = [];
  for (let index = 0; index < fields.length;) {
    const field = fields[index];
    index += 1;
    if (!field) continue;
    if (field.length < 4 || field[2] !== ' ') throw new Error('invalid git status porcelain output');
    const code = field.slice(0, 2);
    const path = normalizeGitPath(field.slice(3));
    let oldPath = null;
    if (code.includes('R') || code.includes('C')) {
      oldPath = normalizeGitPath(fields[index] ?? '');
      index += 1;
    }
    changes.push({ path, oldPath, status: statusName(code), porcelain: code });
  }
  return changes.sort((left, right) => left.path.localeCompare(right.path));
}

function collapseFixtureRenames(changes, fixture) {
  const remaining = [...changes];
  const renames = [];
  for (const expected of fixture.changedFiles ?? []) {
    if (!expected.previousPath) continue;
    const deletedIndex = remaining.findIndex((entry) => entry.path === expected.previousPath && entry.status === 'deleted');
    const addedIndex = remaining.findIndex((entry) => entry.path === expected.path && entry.status === 'added');
    if (deletedIndex === -1 || addedIndex === -1) continue;
    const indexes = [deletedIndex, addedIndex].sort((left, right) => right - left);
    for (const index of indexes) remaining.splice(index, 1);
    renames.push({ path: expected.path, oldPath: expected.previousPath, status: 'renamed', porcelain: ' R' });
  }
  return [...remaining, ...renames].sort((left, right) => left.path.localeCompare(right.path));
}

function statusName(code) {
  if (code === '??' || code.includes('A')) return 'added';
  if (code.includes('R')) return 'renamed';
  if (code.includes('C')) return 'copied';
  if (code.includes('D')) return 'deleted';
  if (code.includes('U')) return 'unmerged';
  return 'modified';
}

function normalizeGitPath(path) {
  if (!path || path.includes('\0') || path.startsWith('/') || path.split('/').includes('..')) {
    throw new Error('git reported an unsafe workspace path');
  }
  return path.split('\\').join('/');
}

function safeRepositoryPath(root, path) {
  const absolute = resolve(root, path);
  assertStrictDescendant(root, absolute, 'changed file');
  return absolute;
}

async function assertGitRepository(root) {
  if (!await isGitRepository(root)) throw new Error(`not a git repository: ${root}`);
}

async function isGitRepository(root) {
  try {
    const result = await runGit(['-C', root, 'rev-parse', '--is-inside-work-tree'], { allowFailure: true });
    return result.exitCode === 0 && result.stdout.trim() === 'true';
  } catch {
    return false;
  }
}

async function runGit(args, options = {}) {
  const result = await runProcess(['git', ...args], {
    cwd: options.cwd ?? process.cwd(),
    timeoutMs: options.timeoutMs ?? 60_000,
    outputLimitBytes: INTERNAL_OUTPUT_LIMIT_BYTES,
    env: options.env ?? safeExecutionEnvironment(),
  });
  if (result.exitCode !== 0 && !options.allowFailure) {
    throw new Error(`git command failed (${result.exitCode ?? result.signal}): ${redactOutput(result.stderr || result.stdout)}`);
  }
  return result;
}

function runProcess(argv, { cwd, timeoutMs, outputLimitBytes, env }) {
  if (!Number.isInteger(timeoutMs) || timeoutMs < 1) throw new Error('timeoutMs must be a positive integer');
  if (!Number.isInteger(outputLimitBytes) || outputLimitBytes < 1) throw new Error('outputLimitBytes must be a positive integer');
  return new Promise((resolvePromise, reject) => {
    const started = process.hrtime.bigint();
    const child = spawn(argv[0], argv.slice(1), {
      cwd,
      env,
      shell: false,
      windowsHide: true,
      detached: process.platform !== 'win32',
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    const stdout = [];
    const stderr = [];
    let stdoutBytes = 0;
    let stderrBytes = 0;
    let capturedBytes = 0;
    let outputTruncated = false;
    let timedOut = false;
    let forceTimer = null;

    const capture = (target, chunk, stream) => {
      if (stream === 'stdout') stdoutBytes += chunk.length;
      else stderrBytes += chunk.length;
      const remaining = Math.max(0, outputLimitBytes - capturedBytes);
      if (remaining > 0) {
        const part = chunk.subarray(0, remaining);
        target.push(part);
        capturedBytes += part.length;
      }
      if (chunk.length > remaining) outputTruncated = true;
    };
    child.stdout.on('data', (chunk) => capture(stdout, chunk, 'stdout'));
    child.stderr.on('data', (chunk) => capture(stderr, chunk, 'stderr'));

    const terminate = (signal) => {
      try {
        if (process.platform !== 'win32' && child.pid) process.kill(-child.pid, signal);
        else child.kill(signal);
      } catch (error) {
        if (error?.code !== 'ESRCH') throw error;
      }
    };
    const timeout = setTimeout(() => {
      timedOut = true;
      terminate('SIGTERM');
      forceTimer = setTimeout(() => terminate('SIGKILL'), 250);
      forceTimer.unref();
    }, timeoutMs);
    timeout.unref();

    child.once('error', (error) => {
      clearTimeout(timeout);
      if (forceTimer) clearTimeout(forceTimer);
      reject(error);
    });
    child.once('close', (exitCode, signal) => {
      clearTimeout(timeout);
      if (forceTimer) clearTimeout(forceTimer);
      const durationMs = Number(process.hrtime.bigint() - started) / 1_000_000;
      resolvePromise({
        exitCode,
        signal,
        timedOut,
        durationMs,
        stdout: Buffer.concat(stdout).toString('utf8'),
        stderr: Buffer.concat(stderr).toString('utf8'),
        stdoutBytes,
        stderrBytes,
        outputTruncated,
      });
    });
  });
}

function safeExecutionEnvironment() {
  const allow = ['PATH', 'HOME', 'TMPDIR', 'LANG', 'LC_ALL', 'CARGO_HOME', 'RUSTUP_HOME'];
  const env = {};
  for (const name of allow) if (process.env[name]) env[name] = process.env[name];
  return {
    ...env,
    CI: '1',
    NO_COLOR: '1',
    npm_config_offline: 'true',
    npm_config_audit: 'false',
    npm_config_fund: 'false',
    CARGO_NET_OFFLINE: 'true',
    PIP_NO_INDEX: '1',
    PYTHONDONTWRITEBYTECODE: '1',
    GIT_TERMINAL_PROMPT: '0',
  };
}

function redactOutput(value) {
  return String(value)
    .replace(/\bBearer\s+\S+/gi, 'Bearer [REDACTED]')
    .replace(/\b(?:sk|ghp|gho|ghu|ghs|github_pat)-?[A-Za-z0-9_-]{16,}\b/g, '[REDACTED_TOKEN]')
    .replace(/\b(api[_-]?key|access[_-]?token|password|secret)\s*[:=]\s*[^\s]+/gi, '$1=[REDACTED]');
}

function assertFixture(fixture) {
  if (!fixture || typeof fixture !== 'object' || !/^[a-z0-9-]+$/.test(fixture.id ?? '')) {
    throw new Error('fixture must have a lowercase slug id');
  }
  if (!fixture.repositorySnapshot || !fixture.finalChallenge) throw new Error(`fixture ${fixture.id} lacks workspace metadata`);
}

function assertSha256(value, label) {
  if (!/^[0-9a-f]{64}$/.test(value ?? '')) throw new Error(`${label} must be a lowercase SHA-256`);
}

function assertText(value, label) {
  if (typeof value !== 'string' || value.trim() === '') throw new Error(`${label} must be nonempty text`);
}

function assertStrictDescendant(parent, child, label) {
  const parentPath = resolve(parent);
  const childPath = resolve(child);
  const path = relative(parentPath, childPath);
  if (!path || path.startsWith(`..${sep}`) || path === '..' || isAbsolute(path)) {
    throw new Error(`${label} must stay inside ${parentPath}`);
  }
}
