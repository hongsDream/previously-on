import { spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { readFile, rm, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';

const fixtureId = 'previouslyon-contract-freshness';
const root = resolve(process.argv[2]);
const target = `continuation_hidden_contract_freshness_${randomUUID().replaceAll('-', '')}`;
const testPath = join(root, 'tests', `${target}.rs`);
const invariantChecks = {
  'literal-boundary': [
    'substring_rejected',
    'identifier_prefix_rejected',
    'executable_completed',
    'substring_mutant_applied',
    'named_test_executed',
    'named_test_rejected_mutant',
    'mutated_source_restored',
  ],
  'path-and-symbol': [
    'same_path_without_symbol_rejected',
    'wrong_path_with_symbol_rejected',
    'same_path_with_symbol_selected',
    'previous_path_with_symbol_selected',
    'executable_completed',
  ],
  'conservative-only-when-unavailable': [
    'unavailable_hunk_selected',
    'unavailable_hunk_warned',
    'inspectable_miss_not_warned',
    'executable_completed',
  ],
};

const hiddenTest = String.raw`
use chrono::{DateTime, Utc};
use previously_on::contracts::{
    match_contracts_for_changes, ChangedHunkV1, ContractChangedFileV1, ContractOriginV1,
    ContractStatusV1, ImpactPathSelectorV1, ImpactSelectorGroupV1, PathSelectorKindV1,
    RegressionContractV1,
};
use previously_on::domain::SCHEMA_VERSION_V1;

fn contract() -> RegressionContractV1 {
    RegressionContractV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: "11111111-1111-4111-8111-111111111111".into(),
        title: "Literal symbol boundary".into(),
        invariant: "table is selected only as a complete changed-hunk token".into(),
        status: ContractStatusV1::Active,
        superseded_by: None,
        impact_selectors: vec![ImpactSelectorGroupV1 {
            path: ImpactPathSelectorV1 {
                kind: PathSelectorKindV1::Exact,
                value: "src/lib.rs".into(),
            },
            symbols: vec!["table".into()],
        }],
        required_tests: vec![],
        origin: ContractOriginV1 {
            fixed_at_commit: "1".repeat(40),
            recorded_at: DateTime::<Utc>::UNIX_EPOCH,
            evidence_sha256: "2".repeat(64),
        },
    }
}

fn changed(path: &str, previous_path: Option<&str>, hunk: ChangedHunkV1) -> ContractChangedFileV1 {
    ContractChangedFileV1 {
        path: path.into(),
        previous_path: previous_path.map(str::to_owned),
        changed_hunk: hunk,
    }
}

#[test]
fn hidden_contract_freshness_semantics() {
    let contract = contract();
    let run = |change| match_contracts_for_changes(std::slice::from_ref(&contract), &[change]);

    let substring = run(changed(
        "src/lib.rs",
        None,
        ChangedHunkV1::Available("pub fn unstable_only() {}".into()),
    ));
    let identifier_prefix = run(changed(
        "src/lib.rs",
        None,
        ChangedHunkV1::Available("let tableau = 1;".into()),
    ));
    let wrong_path = run(changed(
        "src/other.rs",
        None,
        ChangedHunkV1::Available("fn table() {}".into()),
    ));
    let exact = run(changed(
        "src/lib.rs",
        None,
        ChangedHunkV1::Available("fn table() {}".into()),
    ));
    let renamed = run(changed(
        "src/renamed.rs",
        Some("src/lib.rs"),
        ChangedHunkV1::Available("fn table() {}".into()),
    ));
    let unavailable = run(changed(
        "src/lib.rs",
        None,
        ChangedHunkV1::Unavailable("binary diff".into()),
    ));

    let substring_rejected = substring.relevant_contracts.is_empty();
    let identifier_prefix_rejected = identifier_prefix.relevant_contracts.is_empty();
    let same_path_without_symbol_rejected = substring.summaries.is_empty();
    let wrong_path_with_symbol_rejected = wrong_path.relevant_contracts.is_empty();
    let same_path_with_symbol_selected = exact.relevant_contracts.len() == 1
        && exact.summaries.len() == 1;
    let previous_path_with_symbol_selected = renamed.relevant_contracts.len() == 1
        && renamed.matched_paths == vec!["src/lib.rs".to_string()];
    let unavailable_hunk_selected = unavailable.relevant_contracts.len() == 1;
    let unavailable_hunk_warned = unavailable
        .warnings
        .iter()
        .any(|warning| warning.contains("binary diff"));
    let inspectable_miss_not_warned = substring.warnings.is_empty();

    println!(
        "PREVIOUSLY_ON_RUST_ORACLE_V1 {{\"substring_rejected\":{substring_rejected},\"identifier_prefix_rejected\":{identifier_prefix_rejected},\"same_path_without_symbol_rejected\":{same_path_without_symbol_rejected},\"wrong_path_with_symbol_rejected\":{wrong_path_with_symbol_rejected},\"same_path_with_symbol_selected\":{same_path_with_symbol_selected},\"previous_path_with_symbol_selected\":{previous_path_with_symbol_selected},\"unavailable_hunk_selected\":{unavailable_hunk_selected},\"unavailable_hunk_warned\":{unavailable_hunk_warned},\"inspectable_miss_not_warned\":{inspectable_miss_not_warned}}}"
    );
    assert!(substring_rejected
        && identifier_prefix_rejected
        && same_path_without_symbol_rejected
        && wrong_path_with_symbol_rejected
        && same_path_with_symbol_selected
        && previous_path_with_symbol_selected
        && unavailable_hunk_selected
        && unavailable_hunk_warned
        && inspectable_miss_not_warned);
}
`;

const evidence = await executeHiddenTest({
  root,
  target,
  testName: 'hidden_contract_freshness_semantics',
  testPath,
  source: hiddenTest,
});
if (evidence) {
  Object.assign(evidence, await namedTestKillsSubstringMutant(root));
}
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

async function namedTestKillsSubstringMutant(cwd) {
  const sourcePath = join(cwd, 'src', 'contracts.rs');
  const original = await readFile(sourcePath);
  const source = original.toString('utf8');
  const needle = 'contains_literal_token(hunk, symbol)';
  if (source.split(needle).length !== 2) {
    return {
      substring_mutant_applied: false,
      named_test_executed: false,
      named_test_rejected_mutant: false,
      mutated_source_restored: true,
    };
  }
  const mutant = source.replace(needle, 'hunk.contains(symbol.as_str())');
  let result;
  try {
    await writeFile(sourcePath, mutant, 'utf8');
    result = spawnSync('cargo', [
      'test', '--locked', '--offline', '--test', 'core_contracts',
      'symbol_scoped_contract_ignores_unrelated_literal_changes', '--', '--exact', '--nocapture',
    ], {
      cwd,
      encoding: 'utf8',
      env: { ...process.env, CARGO_NET_OFFLINE: 'true', CARGO_TERM_COLOR: 'never' },
      maxBuffer: 1024 * 1024,
      timeout: 150_000,
    });
  } finally {
    await writeFile(sourcePath, original);
  }
  const output = `${result?.stdout ?? ''}\n${result?.stderr ?? ''}`;
  const namedTestExecuted = /running\s+1\s+test/.test(output)
    && /test symbol_scoped_contract_ignores_unrelated_literal_changes \.\.\. (?:ok|FAILED)/.test(output);
  return {
    substring_mutant_applied: true,
    named_test_executed: namedTestExecuted,
    named_test_rejected_mutant: namedTestExecuted
      && Number.isInteger(result?.status)
      && result.status !== 0
      && !result.error
      && result.signal === null
      && /test symbol_scoped_contract_ignores_unrelated_literal_changes \.\.\. FAILED/.test(output),
    mutated_source_restored: (await readFile(sourcePath)).equals(original),
  };
}
