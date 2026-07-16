import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { chmodSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';

import { expectedAssetNames, verifyReleaseBundle } from './verify-release-bundle.mjs';

const VERSION = '0.1.0-test.1';

test('accepts the exact seven-file arm64 CycloneDX release contract', async (t) => {
  const fixture = releaseFixture(t);
  const result = await verifyReleaseBundle(fixture);
  assert.equal(result.files.length, 7);
});

test('exposes the required directory and version CLI', (t) => {
  const fixture = releaseFixture(t);
  const result = spawnSync(process.execPath, [
    fileURLToPath(new URL('./verify-release-bundle.mjs', import.meta.url)),
    '--directory', fixture.directory,
    '--version', VERSION,
  ], {
    encoding: 'utf8',
    env: { ...process.env, PATH: `${join(fixture.directory, '..')}:${process.env.PATH ?? ''}` },
  });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /verified release bundle:/);
});

test('rejects an unexpected release asset', async (t) => {
  const fixture = releaseFixture(t);
  writeFileSync(join(fixture.directory, 'unexpected.txt'), 'extra');
  await assert.rejects(() => verifyReleaseBundle(fixture), /release bundle files differ/);
});

test('rejects a checksum mismatch', async (t) => {
  const fixture = releaseFixture(t);
  writeFileSync(join(fixture.directory, 'NOTICE'), 'tampered');
  await assert.rejects(() => verifyReleaseBundle(fixture), /SHA256 mismatch: NOTICE/);
});

test('rejects an unsafe source archive path', async (t) => {
  const fixture = releaseFixture(t);
  const archive = join(fixture.directory, `previously-on-v${VERSION}-source.tar.gz`);
  writeFileSync(archive, tarGzip([{ path: '../escape', content: 'unsafe' }]));
  writeChecksums(fixture.directory);
  await assert.rejects(() => verifyReleaseBundle(fixture), /unsafe archive path/);
});

test('rejects a malformed CycloneDX document', async (t) => {
  const fixture = releaseFixture(t);
  writeFileSync(join(fixture.directory, `previously-on-v${VERSION}.cdx.json`), '{}\n');
  writeChecksums(fixture.directory);
  await assert.rejects(() => verifyReleaseBundle(fixture), /not a CycloneDX SBOM/);
});

test('rejects a universal or non-arm64 binary', async (t) => {
  const fixture = releaseFixture(t, 'x86_64 arm64');
  await assert.rejects(() => verifyReleaseBundle(fixture), /expected a single arm64 slice/);
});

function releaseFixture(t, architectures = 'arm64') {
  const root = mkdtempSync(join(tmpdir(), 'previously-on-release-contract-'));
  const directory = join(root, 'release');
  mkdirSync(directory);
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const macosBundle = `previously-on-v${VERSION}-macos-arm64`;
  const sourceBundle = `previously-on-${VERSION}`;
  const sbomName = `previously-on-v${VERSION}.cdx.json`;

  writeFileSync(join(directory, 'NOTICE'), 'notice\n');
  writeFileSync(join(directory, 'THIRD_PARTY_LICENSES.md'), 'licenses\n');
  writeFileSync(join(directory, `previously-v${VERSION}-macos-arm64`), 'mach-o fixture\n');
  writeFileSync(join(directory, sbomName), `${JSON.stringify({
    bomFormat: 'CycloneDX',
    specVersion: '1.6',
    version: 1,
    metadata: { component: { type: 'application', name: 'previously-on', version: VERSION } },
    components: [],
  })}\n`);
  writeFileSync(join(directory, `previously-on-v${VERSION}-macos-arm64.tar.gz`), tarGzip([
    { path: `${macosBundle}/previously`, content: 'binary' },
    { path: `${macosBundle}/LICENSE`, content: 'license' },
    { path: `${macosBundle}/NOTICE`, content: 'notice' },
    { path: `${macosBundle}/README.md`, content: 'readme' },
    { path: `${macosBundle}/CHANGELOG.md`, content: 'changes' },
    { path: `${macosBundle}/THIRD_PARTY_LICENSES.md`, content: 'licenses' },
    { path: `${macosBundle}/${sbomName}`, content: 'sbom' },
  ]));
  writeFileSync(join(directory, `previously-on-v${VERSION}-source.tar.gz`), tarGzip([
    { path: `${sourceBundle}/Cargo.toml`, content: '[package]' },
    { path: `${sourceBundle}/Cargo.lock`, content: 'version = 4' },
    { path: `${sourceBundle}/ui/dist/index.html`, content: '<main />' },
    { path: `${sourceBundle}/ui/dist/assets/app.js`, content: 'export {}' },
    { path: `${sourceBundle}/ui/dist/assets/app.css`, content: ':root{}' },
  ]));
  const lipoProgram = join(root, 'lipo');
  writeFileSync(lipoProgram, `#!/bin/sh\nprintf '%s\\n' '${architectures}'\n`);
  chmodSync(lipoProgram, 0o755);
  writeChecksums(directory);
  return { directory, version: VERSION, lipoProgram };
}

function writeChecksums(directory) {
  const assets = expectedAssetNames(VERSION).filter((name) => name !== 'SHA256SUMS');
  const lines = assets.map((name) => {
    const digest = createHash('sha256').update(readFileSync(join(directory, name))).digest('hex');
    return `${digest}  ${name}`;
  });
  writeFileSync(join(directory, 'SHA256SUMS'), `${lines.join('\n')}\n`);
}

function tarGzip(entries) {
  const blocks = [];
  for (const entry of entries) {
    const content = Buffer.from(entry.content);
    const header = Buffer.alloc(512);
    writeString(header, 0, 100, entry.path);
    writeOctal(header, 100, 8, 0o644);
    writeOctal(header, 108, 8, 0);
    writeOctal(header, 116, 8, 0);
    writeOctal(header, 124, 12, content.length);
    writeOctal(header, 136, 12, 0);
    header.fill(32, 148, 156);
    header[156] = '0'.charCodeAt(0);
    writeString(header, 257, 6, 'ustar');
    writeString(header, 263, 2, '00');
    const checksum = header.reduce((sum, byte) => sum + byte, 0);
    writeOctal(header, 148, 8, checksum);
    blocks.push(header, content, Buffer.alloc((512 - (content.length % 512)) % 512));
  }
  blocks.push(Buffer.alloc(1024));
  return gzipSync(Buffer.concat(blocks));
}

function writeString(buffer, offset, length, value) {
  const bytes = Buffer.from(value);
  assert.ok(bytes.length <= length, `fixture tar field is too long: ${value}`);
  bytes.copy(buffer, offset);
}

function writeOctal(buffer, offset, length, value) {
  const encoded = value.toString(8).padStart(length - 2, '0');
  writeString(buffer, offset, length, `${encoded}\0`);
}
