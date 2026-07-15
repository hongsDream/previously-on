import { createHash } from 'node:crypto';
import {
  closeSync,
  existsSync,
  fsyncSync,
  mkdirSync,
  openSync,
  readFileSync,
  renameSync,
  writeFileSync,
  writeSync,
} from 'node:fs';
import { dirname } from 'node:path';

const SECRET_PATTERNS = [
  /-----BEGIN [A-Z ]*PRIVATE KEY-----/i,
  /\b(?:sk|sess|pat|ghp|github_pat|xox[baprs])-[-_A-Za-z0-9]{12,}\b/i,
  /\bBearer\s+[-._~+/=A-Za-z0-9]{12,}\b/i,
  /\b(?:api[_-]?key|access[_-]?token|refresh[_-]?token|password|secret)\s*[:=]\s*["']?[^\s,"']{8,}/i,
  /https?:\/\/[^\s/:]+:[^\s/@]+@/i,
];

const RAW_FIELD_NAMES = new Set([
  'rawPrompt',
  'rawToolOutput',
  'rawTranscript',
  'rawJsonl',
  'sourceCode',
  'credentials',
  'authToken',
]);

export function sha256(value) {
  const bytes = Buffer.isBuffer(value) ? value : Buffer.from(String(value));
  return createHash('sha256').update(bytes).digest('hex');
}

export function stableStringify(value) {
  return JSON.stringify(sortValue(value));
}

function sortValue(value) {
  if (Array.isArray(value)) return value.map(sortValue);
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, item]) => [key, sortValue(item)]),
    );
  }
  return value;
}

export function readJson(path) {
  return JSON.parse(readFileSync(path, 'utf8'));
}

export function readJsonLines(path) {
  if (!existsSync(path)) return [];
  return readFileSync(path, 'utf8')
    .split('\n')
    .filter(Boolean)
    .map((line, index) => {
      try {
        return JSON.parse(line);
      } catch (error) {
        throw new Error(`${path}:${index + 1} is not valid JSON: ${error.message}`);
      }
    });
}

export function appendJsonLine(path, value) {
  assertSanitized(value);
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const payload = `${JSON.stringify(value)}\n`;
  const descriptor = openSync(path, 'a', 0o600);
  try {
    writeSync(descriptor, payload, null, 'utf8');
    fsyncSync(descriptor);
  } finally {
    closeSync(descriptor);
  }
}

export function writeJsonAtomic(path, value) {
  assertSanitized(value);
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const temporary = `${path}.tmp-${process.pid}`;
  writeFileSync(temporary, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
  const descriptor = openSync(temporary, 'r');
  try {
    fsyncSync(descriptor);
  } finally {
    closeSync(descriptor);
  }
  renameSync(temporary, path);
  const directory = openSync(dirname(path), 'r');
  try {
    fsyncSync(directory);
  } finally {
    closeSync(directory);
  }
}

export function assertSanitized(value) {
  walk(value, []);
  const serialized = JSON.stringify(value);
  for (const pattern of SECRET_PATTERNS) {
    if (pattern.test(serialized)) {
      throw new Error(`retained benchmark data matched secret pattern ${pattern}`);
    }
  }
}

function walk(value, path) {
  if (Array.isArray(value)) {
    value.forEach((item, index) => walk(item, [...path, String(index)]));
    return;
  }
  if (!value || typeof value !== 'object') return;
  for (const [key, item] of Object.entries(value)) {
    if (RAW_FIELD_NAMES.has(key)) {
      throw new Error(`retained benchmark data contains forbidden field ${[...path, key].join('.')}`);
    }
    walk(item, [...path, key]);
  }
}

export function unavailable(reason = 'provider_did_not_expose_metric') {
  return { status: 'unavailable', reason };
}
