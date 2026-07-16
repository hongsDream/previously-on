#!/usr/bin/env node

import { createHash } from 'node:crypto';
import { createReadStream, lstatSync, readFileSync, readdirSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { gunzipSync } from 'node:zlib';
import { basename, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { TextDecoder } from 'node:util';

const TAR_BLOCK_SIZE = 512;
const decoder = new TextDecoder('utf-8', { fatal: true });

export async function verifyReleaseBundle({ directory, version, lipoProgram = 'lipo' }) {
  validateVersion(version);
  const root = resolve(directory);
  const directoryStat = lstatSync(root);
  if (!directoryStat.isDirectory() || directoryStat.isSymbolicLink()) {
    throw new Error(`release directory must be a real directory: ${root}`);
  }

  const expected = expectedAssetNames(version);
  const actualEntries = readdirSync(root, { withFileTypes: true });
  const actual = actualEntries.map((entry) => entry.name).sort();
  if (!sameStrings(actual, expected)) {
    throw new Error(`release bundle files differ\nexpected: ${expected.join(', ')}\nactual: ${actual.join(', ')}`);
  }
  for (const entry of actualEntries) {
    if (!entry.isFile() || entry.isSymbolicLink()) {
      throw new Error(`release bundle entry is not a regular file: ${entry.name}`);
    }
  }

  await verifyChecksums(root, expected.filter((name) => name !== 'SHA256SUMS'));
  verifyMacosArchive(root, version);
  verifySourceArchive(root, version);
  verifyCycloneDx(root, version);
  verifyArchitecture(root, version, lipoProgram);

  return { directory: root, version, files: expected };
}

export function expectedAssetNames(version) {
  validateVersion(version);
  return [
    'NOTICE',
    'SHA256SUMS',
    'THIRD_PARTY_LICENSES.md',
    `previously-on-v${version}-macos-arm64.tar.gz`,
    `previously-on-v${version}-source.tar.gz`,
    `previously-on-v${version}.cdx.json`,
    `previously-v${version}-macos-arm64`,
  ].sort();
}

async function verifyChecksums(directory, expectedAssets) {
  const checksumPath = resolve(directory, 'SHA256SUMS');
  const source = readFileSync(checksumPath, 'utf8');
  if (!source.endsWith('\n')) throw new Error('SHA256SUMS must end with a newline');
  const entries = new Map();
  for (const line of source.trimEnd().split('\n')) {
    const match = /^([0-9a-fA-F]{64})  ([^/\\\0]+)$/.exec(line);
    if (!match) throw new Error(`invalid SHA256SUMS line: ${line}`);
    const [, digest, name] = match;
    if (entries.has(name)) throw new Error(`duplicate SHA256SUMS entry: ${name}`);
    entries.set(name, digest.toLowerCase());
  }
  const names = [...entries.keys()].sort();
  if (!sameStrings(names, [...expectedAssets].sort())) {
    throw new Error(`SHA256SUMS entries differ from release assets: ${names.join(', ')}`);
  }
  for (const name of names) {
    const actual = await sha256(resolve(directory, name));
    if (actual !== entries.get(name)) throw new Error(`SHA256 mismatch: ${name}`);
  }
}

function verifyMacosArchive(directory, version) {
  const archiveName = `previously-on-v${version}-macos-arm64.tar.gz`;
  const bundle = `previously-on-v${version}-macos-arm64`;
  const sbom = `previously-on-v${version}.cdx.json`;
  const entries = inspectTarGzip(resolve(directory, archiveName));
  requireSafeRoot(entries, bundle, archiveName);
  for (const path of [
    `${bundle}/previously`,
    `${bundle}/LICENSE`,
    `${bundle}/NOTICE`,
    `${bundle}/README.md`,
    `${bundle}/CHANGELOG.md`,
    `${bundle}/THIRD_PARTY_LICENSES.md`,
    `${bundle}/${sbom}`,
  ]) {
    requireRegularFile(entries, path, archiveName);
  }
}

function verifySourceArchive(directory, version) {
  const archiveName = `previously-on-v${version}-source.tar.gz`;
  const bundle = `previously-on-${version}`;
  const entries = inspectTarGzip(resolve(directory, archiveName));
  requireSafeRoot(entries, bundle, archiveName);
  for (const path of [
    `${bundle}/Cargo.toml`,
    `${bundle}/Cargo.lock`,
    `${bundle}/ui/dist/index.html`,
  ]) {
    requireRegularFile(entries, path, archiveName);
  }
  const files = entries.filter((entry) => entry.type === 'file').map((entry) => entry.path);
  if (!files.some((path) => path.startsWith(`${bundle}/ui/dist/assets/`) && path.endsWith('.js'))) {
    throw new Error(`${archiveName} omitted a built UI JavaScript asset`);
  }
  if (!files.some((path) => path.startsWith(`${bundle}/ui/dist/assets/`) && path.endsWith('.css'))) {
    throw new Error(`${archiveName} omitted a built UI CSS asset`);
  }
}

function verifyCycloneDx(directory, version) {
  const name = `previously-on-v${version}.cdx.json`;
  const value = JSON.parse(readFileSync(resolve(directory, name), 'utf8'));
  if (value.bomFormat !== 'CycloneDX') throw new Error(`${name} is not a CycloneDX SBOM`);
  if (!/^1\.[0-9]+$/.test(value.specVersion)) throw new Error(`${name} has an unsupported CycloneDX specVersion`);
  if (!Number.isInteger(value.version) || value.version < 1) throw new Error(`${name} has an invalid BOM version`);
  if (value.metadata?.component?.type !== 'application'
    || value.metadata?.component?.name !== 'previously-on'
    || value.metadata?.component?.version !== version) {
    throw new Error(`${name} does not describe previously-on ${version}`);
  }
  if (!Array.isArray(value.components)) throw new Error(`${name} omitted the CycloneDX components array`);
}

function verifyArchitecture(directory, version, lipoProgram) {
  const binary = resolve(directory, `previously-v${version}-macos-arm64`);
  let output;
  try {
    output = execFileSync(lipoProgram, ['-archs', binary], { encoding: 'utf8' }).trim();
  } catch (error) {
    throw new Error(`lipo failed for ${basename(binary)}: ${error.message}`);
  }
  const architectures = output.split(/\s+/).filter(Boolean);
  if (!sameStrings(architectures, ['arm64'])) {
    throw new Error(`expected a single arm64 slice, received: ${architectures.join(' ') || '<none>'}`);
  }
}

export function inspectTarGzip(path) {
  const tar = gunzipSync(readFileSync(path));
  if (tar.length % TAR_BLOCK_SIZE !== 0) throw new Error(`${basename(path)} has a truncated tar payload`);
  const entries = [];
  let offset = 0;
  let zeroBlocks = 0;
  while (offset < tar.length) {
    const header = tar.subarray(offset, offset + TAR_BLOCK_SIZE);
    if (header.every((byte) => byte === 0)) {
      zeroBlocks += 1;
      offset += TAR_BLOCK_SIZE;
      continue;
    }
    if (zeroBlocks > 0) throw new Error(`${basename(path)} contains data after a tar end marker`);
    verifyTarChecksum(header, path);
    const magic = decodeField(header.subarray(257, 263));
    if (!magic.startsWith('ustar')) throw new Error(`${basename(path)} is not a ustar archive`);
    const name = decodeField(header.subarray(0, 100));
    const prefix = decodeField(header.subarray(345, 500));
    const entryPath = prefix ? `${prefix}/${name}` : name;
    validateArchivePath(entryPath, path);
    const size = parseOctal(header.subarray(124, 136), 'size', path);
    const typeFlag = String.fromCharCode(header[156] || 48);
    const type = typeFlag === '0' ? 'file' : typeFlag === '5' ? 'directory' : null;
    if (!type) throw new Error(`${basename(path)} contains unsupported tar entry type ${typeFlag}: ${entryPath}`);
    const dataStart = offset + TAR_BLOCK_SIZE;
    const dataEnd = dataStart + size;
    if (dataEnd > tar.length) throw new Error(`${basename(path)} has a truncated entry: ${entryPath}`);
    if (entries.some((entry) => entry.path === entryPath)) throw new Error(`${basename(path)} contains a duplicate entry: ${entryPath}`);
    entries.push({ path: entryPath, type, size });
    offset = dataStart + Math.ceil(size / TAR_BLOCK_SIZE) * TAR_BLOCK_SIZE;
  }
  if (zeroBlocks < 2) throw new Error(`${basename(path)} omitted the two-block tar end marker`);
  return entries;
}

function verifyTarChecksum(header, path) {
  const expected = parseOctal(header.subarray(148, 156), 'checksum', path);
  let actual = 0;
  for (let index = 0; index < header.length; index += 1) {
    actual += index >= 148 && index < 156 ? 32 : header[index];
  }
  if (actual !== expected) throw new Error(`${basename(path)} has an invalid tar header checksum`);
}

function parseOctal(bytes, field, path) {
  const value = decodeField(bytes).trim();
  if (!/^[0-7]+$/.test(value)) throw new Error(`${basename(path)} has an invalid tar ${field}`);
  const parsed = Number.parseInt(value, 8);
  if (!Number.isSafeInteger(parsed)) throw new Error(`${basename(path)} has an oversized tar ${field}`);
  return parsed;
}

function decodeField(bytes) {
  const end = bytes.indexOf(0);
  return decoder.decode(end === -1 ? bytes : bytes.subarray(0, end));
}

function validateArchivePath(path, archive) {
  if (!path || path.includes('\\') || path.startsWith('/') || /^[A-Za-z]:\//.test(path)) {
    throw new Error(`${basename(archive)} contains an unsafe archive path: ${path}`);
  }
  const normalized = path.endsWith('/') ? path.slice(0, -1) : path;
  const parts = normalized.split('/');
  if (parts.some((part) => !part || part === '.' || part === '..')) {
    throw new Error(`${basename(archive)} contains an unsafe archive path: ${path}`);
  }
}

function requireSafeRoot(entries, root, archive) {
  if (entries.length === 0) throw new Error(`${archive} is empty`);
  for (const entry of entries) {
    if (entry.path !== root && !entry.path.startsWith(`${root}/`)) {
      throw new Error(`${archive} contains an entry outside ${root}: ${entry.path}`);
    }
  }
}

function requireRegularFile(entries, path, archive) {
  if (!entries.some((entry) => entry.path === path && entry.type === 'file')) {
    throw new Error(`${archive} omitted required file: ${path}`);
  }
}

function validateVersion(version) {
  if (typeof version !== 'string' || !/^[0-9A-Za-z][0-9A-Za-z.-]*$/.test(version)) {
    throw new Error(`invalid release version: ${version ?? '<missing>'}`);
  }
}

function sameStrings(left, right) {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

function sha256(path) {
  return new Promise((resolveDigest, reject) => {
    const hash = createHash('sha256');
    const stream = createReadStream(path);
    stream.on('error', reject);
    stream.on('data', (chunk) => hash.update(chunk));
    stream.on('end', () => resolveDigest(hash.digest('hex')));
  });
}

function parseArguments(argv) {
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const name = argv[index];
    const value = argv[index + 1];
    if (!['--directory', '--version'].includes(name) || value === undefined || values.has(name)) {
      throw new Error('usage: verify-release-bundle.mjs --directory <path> --version <version>');
    }
    values.set(name, value);
  }
  if (values.size !== 2) throw new Error('usage: verify-release-bundle.mjs --directory <path> --version <version>');
  return { directory: values.get('--directory'), version: values.get('--version') };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const result = await verifyReleaseBundle(parseArguments(process.argv.slice(2)));
    process.stdout.write(`verified release bundle: ${result.directory} (${result.version}, ${result.files.length} files)\n`);
  } catch (error) {
    process.stderr.write(`error: ${error.message}\n`);
    process.exitCode = 1;
  }
}
