import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const fixtureId = 'synthetic-parser-rename';
const repositoryRoot = resolve(process.argv[2]);
const violations = new Set();
let assertions = 0;
const check = async (id, assertion) => {
  assertions += 1;
  try { await assertion(); } catch { violations.add(id); }
};
const temporary = await mkdtemp(join(tmpdir(), 'previously-on-parser-oracle-'));
let cargo = { status: null, stdout: '', stderr: '' };
try {
  const cargoPath = repositoryRoot.replaceAll('\\', '\\\\').replaceAll('"', '\\"');
  await writeFile(join(temporary, 'Cargo.toml'), `[package]\nname = "hidden-parser-oracle"\nversion = "0.0.0"\nedition = "2021"\n\n[dependencies.target]\npackage = "continuation-parser-fixture"\npath = "${cargoPath}"\n`, 'utf8');
  await mkdir(join(temporary, 'src'));
  await writeFile(join(temporary, 'src/lib.rs'), '', 'utf8');
  await mkdir(join(temporary, 'tests'));
  await writeFile(join(temporary, 'tests/oracle.rs'), `#[test]\nfn both_paths_and_errors_are_stable() {\n    let current = target::parser::parse_frame("event:ready").unwrap();\n    let legacy = target::legacy_parser::parse_frame("event:ready").unwrap();\n    assert_eq!(current, legacy);\n    assert_eq!(current.kind, "event");\n    assert_eq!(current.value, "ready");\n    assert_eq!(target::parser::parse_frame("missing"), Err("frame must contain ':'"));\n    assert_eq!(target::parser::parse_frame(":"), Err("frame fields must not be empty"));\n}\n`, 'utf8');
  cargo = spawnSync('cargo', ['test', '--offline', '--test', 'oracle'], {
    cwd: temporary,
    encoding: 'utf8',
    env: { ...process.env, CARGO_NET_OFFLINE: 'true', NO_COLOR: '1' },
  });
} finally {
  await rm(temporary, { recursive: true, force: true });
}
const output = `${cargo.stdout}\n${cargo.stderr}`;
await check('compat-alias', () => assert.equal(cargo.status, 0));
await check('compat-alias', () => assert.match(output, /running 1 test/));
await check('no-format-change', () => assert.match(output, /1 passed/));
const lib = await readFile(join(repositoryRoot, 'src/lib.rs'), 'utf8').catch(() => '');
await check('compat-alias', () => assert.match(lib, /pub\s+mod\s+parser\s*;/));
await check('compat-alias', () => assert.match(lib, /legacy_parser/));
const parser = await readFile(join(repositoryRoot, 'src/parser.rs'), 'utf8').catch(() => '');
const legacy = await readFile(join(repositoryRoot, 'src/legacy_parser.rs'), 'utf8').catch(() => '');
await check('single-implementation', () => assert.equal((`${parser}\n${legacy}`.match(/\bfn\s+parse_frame\b/g) ?? []).length, 1));
await check('single-implementation', () => assert.equal(legacy, ''));

const violatedInvariantIds = [...violations].sort();
process.stdout.write(`PREVIOUSLY_ON_HIDDEN_ORACLE_V1 ${JSON.stringify({ fixtureId, version: 1, assertions, violatedInvariantIds })}\n`);
if (violatedInvariantIds.length > 0) process.exitCode = 1;
