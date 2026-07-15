import { spawn } from 'node:child_process';
import { appendFile, lstat, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { extname, join, resolve, sep } from 'node:path';
import { performance } from 'node:perf_hooks';
import { createInterface } from 'node:readline';
import { assertSanitized, sha256, stableStringify } from './io.mjs';

export async function materializeProductArm({
  previouslyBin,
  dataDir,
  repository,
  repositoryId,
  taskId,
  fixtureSha256,
  fixture,
  base,
  head,
  sourceKey,
  sourceCompaction,
  sourceThreadId,
  sourceSnapshotSha256,
  tokenBudget = 1_200,
  timeoutMs = 15 * 60 * 1000,
  monotonicNow = () => performance.now(),
}) {
  const materializationStartedAt = monotonicNow();
  const { contracts, contractEvaluation, sourceRepositoryId } = await evaluateFixtureContracts({
    previouslyBin,
    dataDir,
    repository,
    fixture,
    fixtureSha256,
    base,
    head,
    timeoutMs,
  });
  if (sourceRepositoryId !== repositoryId) {
    throw new Error('product arm repositoryId does not match the bound repository common directory');
  }
  const { contextPack, taskTimeline } = await callProductHistory({
    previouslyBin,
    dataDir,
    taskId,
    tokenBudget,
    timeoutMs,
  });
  const product = composeProductArm({
    binding: {
      repositoryId,
      contractRepositoryId: contractEvaluation.repositoryId,
      taskId,
      fixtureSha256,
      base,
      head,
      contractBase: head,
      contractSelection: 'fixture_impact_probe_v1',
      sourceKey,
      sourceCompaction,
      sourceThreadId,
      sourceSnapshotSha256,
    },
    contextPack,
    taskTimeline,
    contractEvaluation,
    expectedRelevantContracts: contracts.relevant,
  });
  return withProductMaterialization(product, Math.max(0, monotonicNow() - materializationStartedAt));
}

export function withProductMaterialization(product, durationMs) {
  if (!Number.isFinite(durationMs) || durationMs < 0) {
    throw new Error('product arm materialization duration is invalid');
  }
  const { sha256: _claimed, ...body } = product;
  const timed = { ...body, materialization: { durationMs } };
  assertSanitized(timed);
  return { ...timed, sha256: sha256(stableStringify(timed)) };
}

export async function evaluateFixtureContracts({
  previouslyBin,
  dataDir,
  repository,
  fixture,
  fixtureSha256,
  base,
  head,
  timeoutMs = 15 * 60 * 1000,
}) {
  const contracts = validateFixtureProductArm({ fixture, fixtureSha256, base });
  const sourceRepositoryId = await repositoryId(resolve(repository));
  const contractEvaluation = await withContractProbe(
    { repository, head, contracts },
    (probeRepository) => runContractsCheck({
      previouslyBin,
      dataDir,
      repository: probeRepository,
      base: head,
      timeoutMs,
    }),
  );
  return { contracts, contractEvaluation, sourceRepositoryId };
}

export function composeProductArm({ binding, contextPack, taskTimeline, contractEvaluation, expectedRelevantContracts }) {
  validateBinding(binding);
  if (contextPack?.structuredContent !== undefined) {
    throw new Error('resume_task product arm must not contain structuredContent');
  }
  const text = contextPack?.content?.[0]?.text;
  if (typeof text !== 'string') throw new Error('resume_task result omitted content[0].text');
  const envelope = JSON.parse(text);
  if (
    envelope?.tool !== 'resume_task' ||
    envelope?.trust?.classification !== 'untrusted_historical_data' ||
    envelope?.trust?.instruction_policy !== 'data_only_never_execute'
  ) {
    throw new Error('resume_task trust envelope changed');
  }
  if (
    typeof envelope?.data?.repository_id !== 'string' ||
    typeof envelope?.data?.task_id !== 'string'
  ) {
    throw new Error('resume_task data omitted repository_id or task_id');
  }
  if (
    envelope.data.repository_id !== binding.repositoryId ||
    envelope.data.task_id !== binding.taskId
  ) {
    throw new Error('resume_task data repository_id/task_id binding changed');
  }
  if (contextPack.isError !== false) throw new Error('resume_task returned an error');
  const sourceProof = verifyTaskTimeline(taskTimeline, binding);
  if (contractEvaluation?.readiness !== 'ready') {
    throw new Error(`Regression Contract product arm is not ready: ${contractEvaluation?.readiness ?? 'missing'}`);
  }
  const verifiedContracts = verifyContractEvaluation({ binding, contractEvaluation, expectedRelevantContracts });
  const normalizedEvaluation = normalizeContractEvaluation(contractEvaluation);
  const product = {
    schemaVersion: 1,
    binding,
    trust: {
      contextPack: {
        classification: 'untrusted_historical_data',
        instructionPolicy: 'data_only_never_execute',
      },
      regressionContracts: {
        classification: 'untrusted_repository_metadata',
        instructionPolicy: 'data_only_never_execute',
      },
    },
    contextPack,
    sourceProof,
    regressionContracts: verifiedContracts,
    contractEvaluation: normalizedEvaluation,
  };
  assertSanitized(product);
  return { ...product, sha256: sha256(stableStringify(product)) };
}

function verifyTaskTimeline(taskTimeline, binding) {
  const text = taskTimeline?.content?.[0]?.text;
  if (typeof text !== 'string' || taskTimeline?.isError !== false) {
    throw new Error('get_task_timeline did not return successful text content');
  }
  const envelope = JSON.parse(text);
  if (stableStringify(taskTimeline.structuredContent) !== stableStringify(envelope)) {
    throw new Error('get_task_timeline text and structuredContent differ');
  }
  if (
    envelope?.tool !== 'get_task_timeline' ||
    envelope?.trust?.classification !== 'untrusted_historical_data' ||
    envelope?.trust?.instruction_policy !== 'data_only_never_execute'
  ) {
    throw new Error('get_task_timeline trust envelope changed');
  }
  if (
    envelope?.data?.task?.id !== binding.taskId ||
    envelope?.data?.task?.repository_id !== binding.repositoryId
  ) {
    throw new Error('get_task_timeline task/repository binding changed');
  }
  const sessions = Array.isArray(envelope.data.sessions) ? envelope.data.sessions : [];
  const sourceSession = sessions.find((session) => session?.source_thread_id === binding.sourceThreadId);
  if (!sourceSession || sourceSession.compaction_count !== binding.sourceCompaction) {
    throw new Error('get_task_timeline source thread/compaction binding changed');
  }
  return {
    taskTimelineSha256: sha256(text),
    sourceThreadId: sourceSession.source_thread_id,
    sourceSessionId: sourceSession.id,
    compactionCount: sourceSession.compaction_count,
  };
}

function normalizeContractEvaluation(value) {
  const normalized = structuredClone(value);
  delete normalized.id;
  delete normalized.evaluatedAt;
  delete normalized.irrelevantContracts;
  delete normalized.candidateContracts;
  normalized.relevantContracts = normalized.relevantContracts
    .sort((left, right) => String(left.id).localeCompare(String(right.id)));
  normalized.requiredTests = [...(normalized.requiredTests ?? [])]
    .sort((left, right) => `${left.contractId}\0${left.testId}`.localeCompare(`${right.contractId}\0${right.testId}`));
  return normalized;
}

function verifyContractEvaluation({ binding, contractEvaluation, expectedRelevantContracts }) {
  if (!Array.isArray(expectedRelevantContracts) || expectedRelevantContracts.length === 0) {
    throw new Error('product arm omitted the fixture relevant Contract catalog');
  }
  if (expectedRelevantContracts.some((contract) => contract?.status !== 'active')) {
    throw new Error('fixture relevant Contract catalog contains a non-active Contract');
  }
  const expected = [...expectedRelevantContracts].sort((left, right) => String(left.id).localeCompare(String(right.id)));
  const actual = contractEvaluation.relevantContracts;
  if (!Array.isArray(actual) || actual.length === 0) {
    throw new Error('Regression Contract product arm has no relevant active contract');
  }
  const actualById = new Map(actual.map((contract) => [contract?.id, contract]));
  if (actualById.size !== actual.length) throw new Error('Regression Contract evaluation contains duplicate relevant Contract ids');
  const expectedIds = expected.map((contract) => contract.id);
  const actualIds = [...actualById.keys()].sort();
  if (stableStringify(actualIds) !== stableStringify(expectedIds)) {
    throw new Error('Regression Contract evaluation does not match the fixture active Contract set');
  }
  for (const contract of expected) {
    const summary = actualById.get(contract.id);
    if (
      summary?.title !== contract.title ||
      summary?.invariant !== contract.invariant ||
      !Array.isArray(summary?.matchReasons) ||
      summary.matchReasons.length === 0
    ) {
      throw new Error(`Regression Contract evaluation summary changed for ${contract.id}`);
    }
  }
  if (contractEvaluation.repositoryId !== binding.contractRepositoryId) {
    throw new Error('Regression Contract evaluation repository binding changed');
  }
  if (
    contractEvaluation.base !== binding.contractBase ||
    contractEvaluation.head !== binding.head ||
    contractEvaluation.mergeBase !== binding.contractBase
  ) {
    throw new Error('Regression Contract evaluation base/head binding changed');
  }
  verifyRequiredTests(contractEvaluation.requiredTests, expected);
  return expected;
}

function verifyRequiredTests(actualTests, contracts) {
  if (!Array.isArray(actualTests)) throw new Error('Regression Contract evaluation omitted required test results');
  const expected = contracts.flatMap((contract) => contract.requiredTests.map((test) => ({
    contractId: contract.id,
    testId: test.id,
    name: test.name,
    program: test.program,
    args: test.args,
    workingDirectory: test.workingDirectory,
    timeoutSeconds: test.timeoutSeconds,
  })));
  const key = (test) => `${test.contractId}\0${test.testId}`;
  const actualByKey = new Map(actualTests.map((test) => [key(test), test]));
  if (actualByKey.size !== actualTests.length || actualByKey.size !== expected.length) {
    throw new Error('Regression Contract evaluation required test set changed');
  }
  for (const expectedTest of expected) {
    const actual = actualByKey.get(key(expectedTest));
    const comparable = actual && {
      contractId: actual.contractId,
      testId: actual.testId,
      name: actual.name,
      program: actual.program,
      args: actual.args,
      workingDirectory: actual.workingDirectory,
      timeoutSeconds: actual.timeoutSeconds,
    };
    if (stableStringify(comparable) !== stableStringify(expectedTest) || actual?.state !== 'passed') {
      throw new Error(`Regression Contract required test did not pass exactly: ${key(expectedTest)}`);
    }
  }
}

function validateFixtureProductArm({ fixture, fixtureSha256, base }) {
  if (!fixture || sha256(stableStringify(fixture)) !== fixtureSha256) {
    throw new Error('product arm fixture content does not match fixtureSha256');
  }
  if (fixture.repositorySnapshot?.baseSha !== base || fixture.productArm?.fixtureBaseSha !== base) {
    throw new Error('product arm logical base does not match the fixture');
  }
  const relevant = fixture.productArm?.relevantContracts;
  const irrelevant = fixture.productArm?.irrelevantContracts;
  if (!Array.isArray(relevant) || relevant.length === 0 || !Array.isArray(irrelevant)) {
    throw new Error('fixture product arm Contract catalog is incomplete');
  }
  if (relevant.some((contract) => contract?.status !== 'active')) {
    throw new Error('fixture relevant Contract catalog contains a non-active Contract');
  }
  const relevantIds = new Set(relevant.map((contract) => contract.id));
  if (relevantIds.size !== relevant.length || irrelevant.some((contract) => relevantIds.has(contract.id))) {
    throw new Error('fixture product arm Contract catalog overlaps or contains duplicate ids');
  }
  return {
    relevant: structuredClone(relevant),
    active: structuredClone([...relevant, ...irrelevant].filter((contract) => contract?.status === 'active')),
  };
}

async function withContractProbe({ repository, head, contracts }, callback) {
  if (!/^[0-9a-f]{40}$/.test(head ?? '')) throw new Error('product arm head must be a 40-hex commit');
  const root = resolve(repository);
  const observedHead = (await runGit(root, ['rev-parse', '--verify', 'HEAD^{commit}'])).trim();
  if (observedHead !== head) throw new Error('product arm repository HEAD does not match the requested head');
  const temporary = await mkdtemp(join(tmpdir(), 'previously-continuation-contract-probe-'));
  const probeRepository = join(temporary, 'repository');
  try {
    await runCommand('git', ['clone', '--quiet', '--no-hardlinks', '--no-checkout', root, probeRepository], 60_000);
    await runGit(probeRepository, ['checkout', '--quiet', '--detach', head]);
    await installIgnoredBuildInputs(probeRepository);
    await installContractCatalog(probeRepository, contracts.active);
    await installImpactProbe(probeRepository, contracts.relevant);
    return await callback(probeRepository);
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

export async function installIgnoredBuildInputs(repository) {
  const embeddedUiSource = join(repository, 'src', 'server.rs');
  const embeddedUiDirectory = join(repository, 'ui', 'dist');
  const needsEmbeddedUi = await readFile(embeddedUiSource, 'utf8')
    .then((source) => source.includes('#[folder = "ui/dist/"]'), () => false);
  const hasEmbeddedUi = await lstat(embeddedUiDirectory).then((info) => info.isDirectory(), () => false);
  if (needsEmbeddedUi && !hasEmbeddedUi) {
    await mkdir(embeddedUiDirectory, { recursive: true, mode: 0o700 });
    await writeFile(
      join(embeddedUiDirectory, 'index.html'),
      '<!doctype html><meta charset="utf-8"><title>Continuation contract probe</title>\n',
      { encoding: 'utf8', mode: 0o600 },
    );
  }
}

async function installContractCatalog(repository, contracts) {
  const directory = join(repository, '.previously-on', 'contracts');
  await mkdir(directory, { recursive: true, mode: 0o700 });
  for (const contract of contracts) {
    await writeFile(join(directory, `${contract.id}.json`), `${stableStringify(contract)}\n`, { encoding: 'utf8', mode: 0o600 });
  }
}

async function installImpactProbe(repository, contracts) {
  const probes = new Map();
  for (const contract of contracts) {
    const selector = contract.impactSelectors?.[0];
    const path = selector?.path?.value;
    if (selector?.path?.kind !== 'exact' || typeof path !== 'string') {
      throw new Error(`product arm Contract ${contract.id} needs an exact impact selector for deterministic materialization`);
    }
    const absolute = safeRepositoryPath(repository, path);
    const info = await lstat(absolute).catch((error) => {
      throw new Error(`product arm impact probe path is unavailable for ${contract.id}: ${error.message}`);
    });
    if (!info.isFile() || info.isSymbolicLink()) throw new Error(`product arm impact probe path is not a regular file: ${path}`);
    const symbols = Array.isArray(selector.symbols) ? selector.symbols : [];
    const existing = probes.get(absolute) ?? { path, symbols: new Set(), contractIds: [] };
    symbols.forEach((symbol) => existing.symbols.add(symbol));
    existing.contractIds.push(contract.id);
    probes.set(absolute, existing);
  }
  for (const [absolute, probe] of probes) {
    const content = await readFile(absolute, 'utf8');
    const prefix = extname(probe.path).toLowerCase() === '.py' ? '#' : '//';
    const literals = [...probe.symbols];
    const marker = `${prefix} continuation-contract-probe ${probe.contractIds.join(' ')} ${literals.join(' ')}`.trimEnd();
    await appendFile(absolute, `${content.endsWith('\n') ? '' : '\n'}${marker}\n`, 'utf8');
  }
}

function safeRepositoryPath(repository, path) {
  if (path.includes('\0')) throw new Error('product arm impact probe path contains NUL');
  const root = resolve(repository);
  const absolute = resolve(root, path);
  if (absolute === root || !absolute.startsWith(`${root}${sep}`) || path.split(/[\\/]/).includes('..')) {
    throw new Error(`product arm impact probe path escapes the repository: ${path}`);
  }
  return absolute;
}

async function runGit(repository, args) {
  return runCommand('git', ['-C', repository, ...args], 30_000);
}

async function runCommand(program, args, timeoutMs) {
  const result = await runCaptured(program, args, { timeoutMs });
  if (result.code !== 0) throw new Error(`git ${args[0]} failed: ${result.stderr.trim() || `exit ${result.code}`}`);
  return result.stdout;
}

async function repositoryId(repository) {
  const commonDirectory = (await runGit(repository, ['rev-parse', '--git-common-dir'])).trim();
  return realpath(resolve(repository, commonDirectory));
}

async function callProductHistory({ previouslyBin, dataDir, taskId, tokenBudget, timeoutMs }) {
  const rpc = new LineRpcProcess(previouslyBin, ['--data-dir', dataDir, 'mcp'], timeoutMs);
  try {
    await rpc.request('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'previously-on-continuation-benchmark', version: '1.0.0' } });
    rpc.notify('notifications/initialized', {});
    const contextPack = await rpc.request('tools/call', { name: 'resume_task', arguments: { task_id: taskId, token_budget: tokenBudget } });
    const taskTimeline = await rpc.request('tools/call', { name: 'get_task_timeline', arguments: { task_id: taskId } });
    return { contextPack, taskTimeline };
  } finally {
    await rpc.close();
  }
}

async function runContractsCheck({ previouslyBin, dataDir, repository, base, timeoutMs }) {
  const result = await runCaptured(
    previouslyBin,
    ['--data-dir', dataDir, 'contracts', 'check', '--base', base, '--execute', '--json'],
    { cwd: repository, timeoutMs },
  );
  let parsed;
  try {
    parsed = JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`contracts check did not return JSON: ${error.message}`);
  }
  if (result.code !== 0 && parsed.readiness !== 'contract_blocked') {
    throw new Error(`contracts check failed without a semantic blocked result (exit ${result.code})`);
  }
  return parsed;
}

class LineRpcProcess {
  constructor(binary, args, timeoutMs) {
    this.timeoutMs = timeoutMs;
    this.nextId = 1;
    this.pending = new Map();
    this.child = spawn(binary, args, { stdio: ['pipe', 'pipe', 'pipe'] });
    this.stderr = '';
    this.child.stderr.on('data', (chunk) => {
      this.stderr = `${this.stderr}${chunk.toString('utf8')}`.slice(-256 * 1024);
    });
    createInterface({ input: this.child.stdout, crlfDelay: Infinity }).on('line', (line) => {
      const message = JSON.parse(line);
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      clearTimeout(pending.timer);
      if (message.error) pending.reject(new Error(message.error.message ?? 'MCP error'));
      else pending.resolve(message.result);
    });
  }

  request(method, params) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`PreviouslyOn MCP ${method} timed out`));
      }, this.timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      this.child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
    });
  }

  notify(method, params) {
    this.child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method, params })}\n`);
  }

  async close() {
    this.child.stdin.end();
    if (!this.child.killed) this.child.kill('SIGTERM');
  }
}

function runCaptured(binary, args, { cwd, timeoutMs }) {
  return new Promise((resolve, reject) => {
    const child = spawn(binary, args, { cwd, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    const limit = 8 * 1024 * 1024;
    const timer = setTimeout(() => {
      child.kill('SIGTERM');
      setTimeout(() => child.kill('SIGKILL'), 2_000).unref();
    }, timeoutMs);
    child.stdout.on('data', (chunk) => {
      stdout += chunk.toString('utf8');
      if (Buffer.byteLength(stdout) > limit) child.kill('SIGKILL');
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk.toString('utf8');
      if (Buffer.byteLength(stderr) > limit) child.kill('SIGKILL');
    });
    child.once('error', reject);
    child.once('exit', (code, signal) => {
      clearTimeout(timer);
      if (signal) reject(new Error(`process exited by ${signal}`));
      else resolve({ code, stdout, stderr });
    });
  });
}

function validateBinding(binding) {
  for (const key of [
    'repositoryId',
    'contractRepositoryId',
    'taskId',
    'fixtureSha256',
    'base',
    'head',
    'contractBase',
    'sourceKey',
    'sourceThreadId',
    'sourceSnapshotSha256',
  ]) {
    if (typeof binding?.[key] !== 'string' || binding[key].trim() === '') throw new Error(`product arm binding omitted ${key}`);
  }
  if (!/^[0-9a-f]{64}$/.test(binding.fixtureSha256)) throw new Error('product arm fixtureSha256 is invalid');
  if (!/^[0-9a-f]{64}$/.test(binding.sourceSnapshotSha256)) throw new Error('product arm sourceSnapshotSha256 is invalid');
  if (!Number.isInteger(binding.sourceCompaction) || binding.sourceCompaction < 0) {
    throw new Error('product arm sourceCompaction is invalid');
  }
  for (const key of ['base', 'head', 'contractBase']) {
    if (!/^[0-9a-f]{40}$/.test(binding[key])) throw new Error(`product arm binding ${key} is invalid`);
  }
  if (binding.contractBase !== binding.head) throw new Error('product arm contractBase must equal the isolated workspace head');
  if (binding.contractSelection !== 'fixture_impact_probe_v1') throw new Error('product arm contract selection method changed');
}
