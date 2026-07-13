#!/usr/bin/env node

import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { dirname } from 'node:path';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = new Map();
for (let index = 2; index < process.argv.length; index += 2) {
  args.set(process.argv[index], process.argv[index + 1]);
}
const sbomPath = resolve(args.get('--sbom') ?? `${root}/previously-on.cdx.json`);
const licensesPath = resolve(args.get('--licenses') ?? `${root}/THIRD_PARTY_LICENSES.md`);

const cargo = JSON.parse(execFileSync('cargo', [
  'metadata', '--format-version', '1', '--locked', '--manifest-path', `${root}/Cargo.toml`,
], { encoding: 'utf8' }));
const cargoPackages = new Map(cargo.packages.map((pkg) => [pkg.id, pkg]));
const cargoNodes = new Map(cargo.resolve.nodes.map((node) => [node.id, node]));
const reachable = new Set();
const queue = [cargo.resolve.root];
while (queue.length > 0) {
  const id = queue.shift();
  if (!id || reachable.has(id)) continue;
  reachable.add(id);
  for (const dependency of cargoNodes.get(id)?.deps ?? []) {
    const production = dependency.dep_kinds.some((kind) => kind.kind === null || kind.kind === 'build');
    if (production) queue.push(dependency.pkg);
  }
}

const rootPackage = cargoPackages.get(cargo.resolve.root);
if (!rootPackage) throw new Error('Cargo metadata did not contain the root package');

const components = [];
const licenseRows = [];
for (const id of [...reachable].sort()) {
  if (id === cargo.resolve.root) continue;
  const pkg = cargoPackages.get(id);
  if (!pkg) continue;
  const license = pkg.license ?? 'UNKNOWN';
  components.push(component('cargo', pkg.name, pkg.version, license));
  licenseRows.push({ ecosystem: 'Rust', name: pkg.name, version: pkg.version, license });
}

const npmLock = JSON.parse(readFileSync(`${root}/ui/package-lock.json`, 'utf8'));
for (const [path, pkg] of Object.entries(npmLock.packages)) {
  if (!path || pkg.dev || !pkg.version) continue;
  const name = path.slice(path.lastIndexOf('node_modules/') + 'node_modules/'.length);
  const license = pkg.license ?? 'UNKNOWN';
  components.push(component('npm', name, pkg.version, license));
  licenseRows.push({ ecosystem: 'npm', name, version: pkg.version, license });
}

components.sort((a, b) => a.purl.localeCompare(b.purl));
licenseRows.sort((a, b) => `${a.ecosystem}/${a.name}/${a.version}`.localeCompare(`${b.ecosystem}/${b.name}/${b.version}`));

const serialSeed = `${rootPackage.name}@${rootPackage.version}:${components.map((item) => item.purl).join(',')}`;
const serial = execFileSync('shasum', ['-a', '256'], { input: serialSeed, encoding: 'utf8' }).split(/\s+/)[0];
const sbom = {
  bomFormat: 'CycloneDX',
  specVersion: '1.6',
  serialNumber: `urn:uuid:${serial.slice(0, 8)}-${serial.slice(8, 12)}-4${serial.slice(13, 16)}-a${serial.slice(17, 20)}-${serial.slice(20, 32)}`,
  version: 1,
  metadata: {
    component: component('cargo', rootPackage.name, rootPackage.version, rootPackage.license ?? 'Apache-2.0', 'application'),
  },
  components,
};
writeFileSync(sbomPath, `${JSON.stringify(sbom, null, 2)}\n`);

const rows = licenseRows.map(({ ecosystem, name, version, license }) =>
  `| ${escapeCell(ecosystem)} | ${escapeCell(name)} | ${escapeCell(version)} | ${escapeCell(license)} |`).join('\n');
const inventory = `# Third-party licenses

This inventory covers production Rust dependencies and packages embedded in the production UI for
PreviouslyOn ${rootPackage.version}. It is generated from locked dependency metadata by
\`scripts/generate-release-metadata.mjs\`. A declared \`UNKNOWN\` value blocks release review.

| Ecosystem | Package | Version | Declared license |
| --- | --- | --- | --- |
${rows}

Complete license texts and source are available in each package's source distribution. This
inventory is informational and does not alter the terms of any third-party license.
`;
writeFileSync(licensesPath, inventory);

if (licenseRows.some((item) => item.license === 'UNKNOWN')) {
  throw new Error('one or more production dependencies do not declare a license');
}

function component(ecosystem, name, version, license, type = 'library') {
  const namespaceAndName = name.startsWith('@') ? name.slice(1).split('/').map(encodeURIComponent).join('/') : encodeURIComponent(name);
  return {
    type,
    name,
    version,
    licenses: license === 'UNKNOWN' || license.includes('/')
      ? [{ license: { name: license } }]
      : [{ expression: license }],
    purl: `pkg:${ecosystem}/${namespaceAndName}@${encodeURIComponent(version)}`,
  };
}

function escapeCell(value) {
  return String(value).replaceAll('|', '\\|').replaceAll('\n', ' ');
}
