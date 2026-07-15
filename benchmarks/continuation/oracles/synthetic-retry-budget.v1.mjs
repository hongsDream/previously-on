import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';

const fixtureId = 'synthetic-retry-budget';
const repositoryRoot = resolve(process.argv[2]);
const violations = new Set();
let assertions = 0;
const program = `
import json
result = {}
try:
    from client.retry import TOTAL_BUDGET_SECONDS, selected_delay
    result["constant"] = TOTAL_BUDGET_SECONDS == 30.0
    result["retry_after"] = selected_delay(2, "7", 0) == 7.0
    result["clamped_header"] = selected_delay(2, "25", 10) == 20.0
    result["invalid_fallback"] = selected_delay(2, "invalid", 28) == 2.0
    result["negative_fallback"] = selected_delay(2, -1, 29) == 1.0
    result["exhausted"] = selected_delay(5, None, 30) == 0.0
    result["cumulative"] = sum([selected_delay(4, "20", 0), selected_delay(4, "20", 20)]) == 30.0
except Exception:
    pass
print(json.dumps(result, sort_keys=True))
`;
const execution = spawnSync('python3', ['-c', program], {
  cwd: repositoryRoot,
  encoding: 'utf8',
  env: { ...process.env, PYTHONDONTWRITEBYTECODE: '1', PIP_NO_INDEX: '1' },
});
let result = {};
try { result = JSON.parse(execution.stdout); } catch {}
const check = (id, key) => {
  assertions += 1;
  try { assert.equal(result[key], true); } catch { violations.add(id); }
};
check('total-budget', 'constant');
check('retry-after', 'retry_after');
check('retry-after', 'clamped_header');
check('invalid-header', 'invalid_fallback');
check('invalid-header', 'negative_fallback');
check('total-budget', 'exhausted');
check('total-budget', 'cumulative');

const violatedInvariantIds = [...violations].sort();
process.stdout.write(`PREVIOUSLY_ON_HIDDEN_ORACLE_V1 ${JSON.stringify({ fixtureId, version: 1, assertions, violatedInvariantIds })}\n`);
if (violatedInvariantIds.length > 0) process.exitCode = 1;
