import { spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { rm, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';

const fixtureId = 'previouslyon-mcp-trust';
const root = resolve(process.argv[2]);
const target = `continuation_hidden_mcp_trust_${randomUUID().replaceAll('-', '')}`;
const testPath = join(root, 'tests', `${target}.rs`);
const invariantChecks = {
  'text-only': [
    'content_kind',
    'structured_absent',
    'text_payload',
    'non_resume_unchanged',
    'executable_completed',
  ],
  untrusted: [
    'classification',
    'instruction_policy',
    'meta_classification',
    'meta_instruction_policy',
    'executable_completed',
  ],
  'read-only': ['tool_names', 'read_only_hints', 'non_destructive_hints', 'executable_completed'],
};

const hiddenTest = String.raw`
use anyhow::Result;
use previously_on::mcp::{handle_request, tool_definitions, McpBackend};
use serde_json::{json, Value};

struct Fixture;

impl McpBackend for Fixture {
    fn suggest_resume(&self, query: &str) -> Result<Value> {
        Ok(json!({"query": query}))
    }
    fn resume_task(&self, task_id: &str, token_budget: Option<u32>) -> Result<Value> {
        Ok(json!({"task_id": task_id, "token_budget": token_budget}))
    }
    fn search_tasks(&self, query: &str) -> Result<Value> {
        Ok(json!({"query": query}))
    }
    fn explain_fact(&self, fact_id: &str) -> Result<Value> {
        Ok(json!({"fact_id": fact_id}))
    }
    fn get_task_timeline(&self, task_id: &str) -> Result<Value> {
        Ok(json!({"task_id": task_id}))
    }
}

#[test]
fn hidden_mcp_trust_semantics() {
    let response = handle_request(
        &Fixture,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": "resume_task", "arguments": {"task_id": "task-7"}}
        }),
    )
    .unwrap();
    let result = &response["result"];
    let envelope: Value = result["content"][0]["text"]
        .as_str()
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or(Value::Null);
    let search = handle_request(
        &Fixture,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {"name": "search_tasks", "arguments": {"query": "auth"}}
        }),
    )
    .unwrap();
    let tools = tool_definitions();
    let names = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect::<Vec<_>>();

    let content_kind = result["_meta"]["previous_on/content_kind"] == "context_pack";
    let structured_absent = result.get("structuredContent").is_none();
    let text_payload = result["content"][0]["type"] == "text"
        && envelope["tool"] == "resume_task"
        && envelope["data"]["task_id"] == "task-7";
    let non_resume_unchanged = search["result"].get("structuredContent").is_some()
        && search["result"]["_meta"].get("previous_on/content_kind").is_none();
    let classification = envelope["trust"]["classification"] == "untrusted_historical_data";
    let instruction_policy = envelope["trust"]["instruction_policy"] == "data_only_never_execute";
    let meta_classification = result["_meta"]["previously_on/trust"]["classification"]
        == "untrusted_historical_data";
    let meta_instruction_policy = result["_meta"]["previously_on/trust"]["instruction_policy"]
        == "data_only_never_execute";
    let tool_names = names
        == [
            "suggest_resume",
            "resume_task",
            "search_tasks",
            "explain_fact",
            "get_task_timeline",
        ];
    let read_only_hints = tools
        .iter()
        .all(|tool| tool["annotations"]["readOnlyHint"] == true);
    let non_destructive_hints = tools
        .iter()
        .all(|tool| tool["annotations"]["destructiveHint"] == false);

    println!(
        "PREVIOUSLY_ON_RUST_ORACLE_V1 {{\"content_kind\":{content_kind},\"structured_absent\":{structured_absent},\"text_payload\":{text_payload},\"non_resume_unchanged\":{non_resume_unchanged},\"classification\":{classification},\"instruction_policy\":{instruction_policy},\"meta_classification\":{meta_classification},\"meta_instruction_policy\":{meta_instruction_policy},\"tool_names\":{tool_names},\"read_only_hints\":{read_only_hints},\"non_destructive_hints\":{non_destructive_hints}}}"
    );
    assert!(content_kind
        && structured_absent
        && text_payload
        && non_resume_unchanged
        && classification
        && instruction_policy
        && meta_classification
        && meta_instruction_policy
        && tool_names
        && read_only_hints
        && non_destructive_hints);
}
`;

const evidence = await executeHiddenTest({
  root,
  target,
  testName: 'hidden_mcp_trust_semantics',
  testPath,
  source: hiddenTest,
});
const assertions = evidence ? Object.keys(evidence).length : 0;
const violations = new Set();
for (const [invariantId, checks] of Object.entries(invariantChecks)) {
  if (!evidence || checks.some((check) => evidence[check] !== true)) violations.add(invariantId);
}
const violatedInvariantIds = [...violations].sort();
process.stdout.write(`PREVIOUSLY_ON_HIDDEN_ORACLE_V1 ${JSON.stringify({ fixtureId, version: 1, assertions, violatedInvariantIds })}\n`);
if (violatedInvariantIds.length > 0) process.exitCode = 1;

async function executeHiddenTest({ root: cwd, target: name, testName, testPath: path, source }) {
  let result;
  let created = false;
  try {
    await writeFile(path, source, { encoding: 'utf8', flag: 'wx', mode: 0o400 });
    created = true;
    result = spawnSync('cargo', [
      'test', '--locked', '--offline', '--test', name, testName, '--', '--exact', '--nocapture',
    ], {
      cwd,
      encoding: 'utf8',
      env: { ...process.env, CARGO_NET_OFFLINE: 'true', CARGO_TERM_COLOR: 'never' },
      maxBuffer: 1024 * 1024,
      timeout: 150_000,
    });
  } catch (error) {
    process.stderr.write(`hidden executable oracle setup failed: ${error.message}\n`);
    return null;
  } finally {
    if (created) await rm(path, { force: true }).catch(() => {});
  }
  const output = `${result.stdout ?? ''}\n${result.stderr ?? ''}`;
  const records = output.split('\n').filter((line) => line.startsWith('PREVIOUSLY_ON_RUST_ORACLE_V1 '));
  if (records.length !== 1) {
    process.stderr.write(`hidden executable oracle failed before evidence (status=${result.status}, signal=${result.signal ?? 'none'})\n`);
    return null;
  }
  try {
    return {
      ...JSON.parse(records[0].slice('PREVIOUSLY_ON_RUST_ORACLE_V1 '.length)),
      executable_completed: result.status === 0 && !result.error && result.signal === null,
    };
  } catch {
    process.stderr.write('hidden executable oracle emitted invalid evidence\n');
    return null;
  }
}
