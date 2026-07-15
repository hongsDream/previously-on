use crate::domain::{FileChangeV1, GitSnapshotV1, SCHEMA_VERSION_V1};
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

pub const CONTRACTS_DIRECTORY: &str = ".previously-on/contracts";
pub const CONTRACTS_WORKFLOW: &str = ".github/workflows/previously-contracts.yml";
pub const DEFAULT_TEST_TIMEOUT_SECONDS: u64 = 900;
pub const MAX_TEST_TIMEOUT_SECONDS: u64 = 3600;
pub const MAX_SYMBOL_DIFF_BYTES: usize = 1024 * 1024;
pub const MAX_FINGERPRINT_FILE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractStatusV1 {
    Active,
    Superseded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathSelectorKindV1 {
    Exact,
    Prefix,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImpactPathSelectorV1 {
    pub kind: PathSelectorKindV1,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImpactSelectorGroupV1 {
    pub path: ImpactPathSelectorV1,
    #[serde(default)]
    pub symbols: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequiredTestV1 {
    pub id: String,
    pub name: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub working_directory: String,
    #[serde(default = "default_test_timeout_seconds")]
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContractOriginV1 {
    pub fixed_at_commit: String,
    pub recorded_at: DateTime<Utc>,
    pub evidence_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegressionContractV1 {
    pub schema_version: u16,
    pub id: String,
    pub title: String,
    pub invariant: String,
    pub status: ContractStatusV1,
    #[serde(default)]
    pub superseded_by: Option<String>,
    pub impact_selectors: Vec<ImpactSelectorGroupV1>,
    pub required_tests: Vec<RequiredTestV1>,
    pub origin: ContractOriginV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegressionCandidateStatusV1 {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateEvidenceKindV1 {
    FailureEditPass,
    TestFileEditPass,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegressionCandidateV1 {
    pub schema_version: u16,
    pub id: String,
    pub repository_id: String,
    #[serde(default)]
    pub task_id: Option<String>,
    pub title: String,
    pub invariant: String,
    pub status: RegressionCandidateStatusV1,
    pub impact_selectors: Vec<ImpactSelectorGroupV1>,
    pub required_tests: Vec<RequiredTestV1>,
    pub origin: ContractOriginV1,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub evidence_kind: CandidateEvidenceKindV1,
    pub evidence_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractReadinessV1 {
    Ready,
    ContractBlocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequiredTestStateV1 {
    Passed,
    Failed,
    Missing,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelevantContractV1 {
    pub id: String,
    pub title: String,
    pub invariant: String,
    pub match_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequiredTestEvaluationV1 {
    pub contract_id: String,
    pub test_id: String,
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub working_directory: String,
    pub timeout_seconds: u64,
    pub state: RequiredTestStateV1,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContractEvaluationV1 {
    pub schema_version: u16,
    pub id: String,
    pub repository_id: String,
    #[serde(default)]
    pub task_id: Option<String>,
    pub readiness: ContractReadinessV1,
    pub evaluated_at: DateTime<Utc>,
    pub relevant_contracts: Vec<RelevantContractV1>,
    pub required_tests: Vec<RequiredTestEvaluationV1>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub content_fingerprint: String,
    #[serde(default)]
    pub continuation_issued: bool,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub merge_base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractMatchV1 {
    pub relevant_contracts: Vec<RegressionContractV1>,
    pub summaries: Vec<RelevantContractV1>,
    pub matched_paths: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractChangedFileV1 {
    pub path: String,
    pub previous_path: Option<String>,
    pub changed_hunk: ChangedHunkV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangedHunkV1 {
    Available(String),
    Unavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractValidationReportV1 {
    pub schema_version: u16,
    pub contract_count: usize,
    pub active_count: usize,
    pub superseded_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractInitReportV1 {
    pub schema_version: u16,
    pub contracts_directory: String,
    pub contracts_directory_created: bool,
    pub workflow: Option<String>,
    pub workflow_created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LoadedContract {
    file_name: String,
    contract: RegressionContractV1,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TestCommandKey {
    program: String,
    args: Vec<String>,
    working_directory: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandOutcome {
    state: RequiredTestStateV1,
    detail: Option<String>,
}

pub fn default_test_timeout_seconds() -> u64 {
    DEFAULT_TEST_TIMEOUT_SECONDS
}

pub fn contracts_directory(repository: impl AsRef<Path>) -> PathBuf {
    repository.as_ref().join(CONTRACTS_DIRECTORY)
}

pub fn load_contracts(repository: impl AsRef<Path>) -> Result<Vec<RegressionContractV1>> {
    let repository = crate::git::repository_identity(repository)?.root;
    let directory = contracts_directory(&repository);
    if !directory.exists() {
        return Ok(Vec::new());
    }
    if !directory.is_dir() {
        bail!("{} is not a directory", directory.display());
    }
    let mut loaded = Vec::new();
    let mut entries = fs::read_dir(&directory)
        .with_context(|| format!("read Contract directory {}", directory.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        if !entry.file_type()?.is_file() {
            bail!("Contract entry {} is not a regular file", path.display());
        }
        let bytes = fs::read(&path).with_context(|| format!("read Contract {}", path.display()))?;
        let contract: RegressionContractV1 = serde_json::from_slice(&bytes)
            .with_context(|| format!("invalid Contract JSON {}", path.display()))?;
        loaded.push(LoadedContract {
            file_name: entry.file_name().to_string_lossy().into_owned(),
            contract,
        });
    }
    validate_loaded_contracts(&loaded)?;
    Ok(loaded.into_iter().map(|item| item.contract).collect())
}

pub fn load_active_contracts(repository: impl AsRef<Path>) -> Result<Vec<RegressionContractV1>> {
    Ok(load_contracts(repository)?
        .into_iter()
        .filter(|contract| contract.status == ContractStatusV1::Active)
        .collect())
}

pub fn validate_contracts(repository: impl AsRef<Path>) -> Result<ContractValidationReportV1> {
    let contracts = load_contracts(repository)?;
    let active_count = contracts
        .iter()
        .filter(|contract| contract.status == ContractStatusV1::Active)
        .count();
    Ok(ContractValidationReportV1 {
        schema_version: SCHEMA_VERSION_V1,
        contract_count: contracts.len(),
        active_count,
        superseded_count: contracts.len() - active_count,
    })
}

/// Atomically create or update one Git-backed Contract in the current checkout.
///
/// Validation runs against the complete post-write Contract set before the
/// target file is replaced, so an invalid supersede relationship or duplicate
/// active identity never becomes visible to readers.
pub fn write_contract(
    repository: impl AsRef<Path>,
    contract: &RegressionContractV1,
) -> Result<PathBuf> {
    let root = crate::git::repository_identity(repository)?.root;
    if contract
        .required_tests
        .iter()
        .any(|test| argv_contains_sensitive_data(&test.program, &test.args))
    {
        bail!("required test argv contains sensitive data; use secret-free repository scripts");
    }
    let contract = sanitized_contract(contract)?;
    validate_contract(&contract)?;
    let directory = contracts_directory(&root);
    if directory.exists() && !directory.is_dir() {
        bail!("{} is not a directory", directory.display());
    }
    fs::create_dir_all(&directory)
        .with_context(|| format!("create Contract directory {}", directory.display()))?;

    let file_name = format!("{}.json", contract.id);
    let target = directory.join(&file_name);
    if let Ok(metadata) = fs::symlink_metadata(&target) {
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            bail!("Contract target {} is not a regular file", target.display());
        }
    }

    let mut loaded = read_loaded_contracts(&directory)?;
    validate_loaded_contracts(&loaded)?;
    loaded.retain(|item| item.contract.id != contract.id);
    loaded.push(LoadedContract {
        file_name,
        contract: contract.clone(),
    });
    validate_loaded_contracts(&loaded)?;

    let mut bytes = serde_json::to_vec_pretty(&contract)?;
    bytes.push(b'\n');
    write_atomic_file(&target, &bytes)?;
    Ok(target)
}

fn sanitized_contract(contract: &RegressionContractV1) -> Result<RegressionContractV1> {
    let value = serde_json::to_value(contract)?;
    serde_json::from_value(crate::redaction::redact_value(&value))
        .context("redacted Contract no longer satisfies the strict schema")
}

pub fn validate_contract_document(contract: &RegressionContractV1) -> Result<()> {
    validate_contract(contract)
}

pub fn validate_candidate(candidate: &RegressionCandidateV1) -> Result<()> {
    validate_nonempty("repositoryId", &candidate.repository_id)?;
    validate_git_sha("evidenceSha256", &candidate.evidence_sha256, 64)?;
    if candidate.origin.evidence_sha256 != candidate.evidence_sha256 {
        bail!("candidate evidenceSha256 must match origin.evidenceSha256");
    }
    validate_contract(&RegressionContractV1 {
        schema_version: candidate.schema_version,
        id: candidate.id.clone(),
        title: candidate.title.clone(),
        invariant: candidate.invariant.clone(),
        status: ContractStatusV1::Active,
        superseded_by: None,
        impact_selectors: candidate.impact_selectors.clone(),
        required_tests: candidate.required_tests.clone(),
        origin: candidate.origin.clone(),
    })
}

pub fn approve_candidate(
    repository: impl AsRef<Path>,
    candidate: &RegressionCandidateV1,
) -> Result<PathBuf> {
    validate_candidate(candidate)?;
    write_contract(
        repository,
        &RegressionContractV1 {
            schema_version: candidate.schema_version,
            id: candidate.id.clone(),
            title: candidate.title.clone(),
            invariant: candidate.invariant.clone(),
            status: ContractStatusV1::Active,
            superseded_by: None,
            impact_selectors: candidate.impact_selectors.clone(),
            required_tests: candidate.required_tests.clone(),
            origin: candidate.origin.clone(),
        },
    )
}

/// Update an existing Contract while retaining the same UUID/file identity.
pub fn update_contract(
    repository: impl AsRef<Path>,
    contract: &RegressionContractV1,
) -> Result<PathBuf> {
    let root = crate::git::repository_identity(&repository)?.root;
    let target = contracts_directory(&root).join(format!("{}.json", contract.id));
    if !target.is_file() {
        bail!("Contract `{}` does not exist in this checkout", contract.id);
    }
    write_contract(root, contract)
}

fn read_loaded_contracts(directory: &Path) -> Result<Vec<LoadedContract>> {
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut entries = fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut loaded = Vec::new();
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        if !entry.file_type()?.is_file() {
            bail!("Contract entry {} is not a regular file", path.display());
        }
        let contract = serde_json::from_slice(&fs::read(&path)?)
            .with_context(|| format!("invalid Contract JSON {}", path.display()))?;
        loaded.push(LoadedContract {
            file_name: entry.file_name().to_string_lossy().into_owned(),
            contract,
        });
    }
    Ok(loaded)
}

fn write_atomic_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Contract path has no parent")?;
    let temporary = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|value| value.to_str())
            .context("Contract file name is not UTF-8")?,
        uuid::Uuid::new_v4()
    ));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    let write_result = (|| -> Result<()> {
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

fn validate_loaded_contracts(loaded: &[LoadedContract]) -> Result<()> {
    let mut ids = BTreeMap::new();
    for item in loaded {
        validate_contract(&item.contract)?;
        if let Some(previous) = ids.insert(item.contract.id.clone(), item.file_name.clone()) {
            bail!(
                "duplicate/conflicting Contract id `{}` in `{previous}` and `{}`",
                item.contract.id,
                item.file_name
            );
        }
    }
    for item in loaded {
        let expected = format!("{}.json", item.contract.id);
        if item.file_name != expected {
            bail!(
                "Contract file name `{}` must match Contract id `{}`",
                item.file_name,
                item.contract.id
            );
        }
    }
    for item in loaded {
        match item.contract.status {
            ContractStatusV1::Active if item.contract.superseded_by.is_some() => {
                bail!(
                    "active Contract `{}` cannot set supersededBy",
                    item.contract.id
                );
            }
            ContractStatusV1::Superseded => {
                let replacement_id = item.contract.superseded_by.as_deref().ok_or_else(|| {
                    anyhow!(
                        "superseded Contract `{}` must set supersededBy",
                        item.contract.id
                    )
                })?;
                if replacement_id == item.contract.id {
                    bail!("Contract `{}` cannot supersede itself", item.contract.id);
                }
                uuid::Uuid::parse_str(replacement_id).with_context(|| {
                    format!(
                        "superseded Contract `{}` has a non-UUID supersededBy",
                        item.contract.id
                    )
                })?;
            }
            ContractStatusV1::Active => {}
        }
    }
    Ok(())
}

fn validate_contract(contract: &RegressionContractV1) -> Result<()> {
    if contract.schema_version != SCHEMA_VERSION_V1 {
        bail!(
            "Contract `{}` has unsupported schemaVersion {}",
            contract.id,
            contract.schema_version
        );
    }
    let parsed_id = uuid::Uuid::parse_str(&contract.id)
        .with_context(|| format!("Contract id `{}` is not a UUID", contract.id))?;
    if parsed_id.to_string() != contract.id {
        bail!("Contract id `{}` must use canonical UUID form", contract.id);
    }
    validate_nonempty("title", &contract.title)?;
    validate_nonempty("invariant", &contract.invariant)?;
    validate_secret_free("title", &contract.title)?;
    validate_secret_free("invariant", &contract.invariant)?;
    if contract.impact_selectors.is_empty() {
        bail!("Contract `{}` has no impactSelectors", contract.id);
    }
    for (group_index, group) in contract.impact_selectors.iter().enumerate() {
        let selector = &group.path;
        validate_secret_free("impactSelectors.path.value", &selector.value)?;
        let validation_value = if selector.kind == PathSelectorKindV1::Prefix {
            selector.value.trim_end_matches('/')
        } else {
            selector.value.as_str()
        };
        if validation_value.is_empty()
            || crate::git::validated_repository_relative_path(validation_value).as_deref()
                != Some(validation_value)
        {
            bail!(
                "Contract `{}` impactSelectors[{group_index}] has invalid case-sensitive Git path selector `{}`",
                contract.id,
                selector.value
            );
        }
        for symbol in &group.symbols {
            validate_nonempty("symbol", symbol)?;
            validate_secret_free("impactSelectors.symbols", symbol)?;
            if symbol.chars().any(char::is_whitespace) || symbol.contains('\0') {
                bail!(
                    "Contract `{}` symbol `{symbol}` must be a literal identifier/token",
                    contract.id
                );
            }
        }
    }
    if contract.required_tests.is_empty() {
        bail!("Contract `{}` has no requiredTests", contract.id);
    }
    let mut test_ids = BTreeSet::new();
    for test in &contract.required_tests {
        validate_required_test(test)?;
        if !test_ids.insert(&test.id) {
            bail!(
                "Contract `{}` has duplicate required test id `{}`",
                contract.id,
                test.id
            );
        }
    }
    validate_git_sha("fixedAtCommit", &contract.origin.fixed_at_commit, 40)?;
    validate_git_sha("evidenceSha256", &contract.origin.evidence_sha256, 64)?;
    Ok(())
}

fn validate_required_test(test: &RequiredTestV1) -> Result<()> {
    if argv_contains_sensitive_data(&test.program, &test.args) {
        bail!("required test argv contains sensitive data; use secret-free repository scripts");
    }
    validate_nonempty("requiredTests.id", &test.id)?;
    validate_nonempty("requiredTests.name", &test.name)?;
    validate_nonempty("requiredTests.program", &test.program)?;
    validate_secret_free("requiredTests.id", &test.id)?;
    validate_secret_free("requiredTests.name", &test.name)?;
    validate_secret_free("requiredTests.program", &test.program)?;
    validate_secret_free("requiredTests.workingDirectory", &test.working_directory)?;
    if test.program.contains('\0')
        || test.program.chars().any(char::is_whitespace)
        || test.args.iter().any(|arg| arg.contains('\0'))
    {
        bail!("required test argv cannot contain NUL bytes");
    }
    let program_path = Path::new(&test.program);
    if program_path.is_absolute()
        || program_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("requiredTests.program must be a program name or repository-relative script");
    }
    let program_name = program_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(&test.program);
    if interpreter_uses_inline_evaluation(program_name, &test.args) {
        bail!(
            "inline shell/interpreter evaluation is forbidden; use a repository script as program"
        );
    }
    if program_name == "env"
        || looks_like_environment_assignment(&test.program)
        || test
            .args
            .iter()
            .any(|arg| looks_like_environment_assignment(arg))
    {
        bail!("environment assignments are forbidden in requiredTests argv");
    }
    validate_relative_directory(&test.working_directory)?;
    if !(1..=MAX_TEST_TIMEOUT_SECONDS).contains(&test.timeout_seconds) {
        bail!(
            "required test `{}` timeoutSeconds must be in 1..={MAX_TEST_TIMEOUT_SECONDS}",
            test.id
        );
    }
    Ok(())
}

fn argv_contains_sensitive_data(program: &str, args: &[String]) -> bool {
    let individual = std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .any(|value| {
            value.contains(crate::redaction::REDACTED)
                || crate::redaction::redact_text(value) != value
        });
    if individual {
        return true;
    }
    let joined = std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    crate::redaction::redact_text(&joined) != joined
}

fn validate_secret_free(field: &str, value: &str) -> Result<()> {
    if crate::redaction::redact_text(value) != value {
        bail!("{field} contains sensitive data; store only redacted Contract metadata");
    }
    Ok(())
}

fn interpreter_uses_inline_evaluation(program_name: &str, args: &[String]) -> bool {
    let normalized = program_name.to_ascii_lowercase();
    let (inline_flags, options_with_values): (&[&str], &[&str]) = match normalized.as_str() {
        "sh" | "dash" | "bash" | "zsh" | "ksh" | "fish" => (
            &["-c", "--command"],
            &["-o", "+o", "-O", "+O", "--rcfile", "--init-file"],
        ),
        name if name.starts_with("python") => (&["-c"], &["-W", "-X", "--check-hash-based-pycs"]),
        "node" | "nodejs" => (
            &["-e", "--eval"],
            &["-r", "--require", "--loader", "--import"],
        ),
        "ruby" | "perl" => (&["-e"], &[]),
        "php" => (&["-r"], &[]),
        "powershell" | "pwsh" => (&["-command", "--command", "-encodedcommand"], &[]),
        _ => return false,
    };

    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if argument == "--" {
            return false;
        }
        if inline_flags.iter().any(|flag| {
            argument == flag
                || argument
                    .strip_prefix(flag)
                    .is_some_and(|suffix| suffix.starts_with('='))
        }) {
            return true;
        }
        if is_shell_short_option_cluster(&normalized, argument) {
            return true;
        }
        if options_with_values.contains(&argument.as_str()) {
            index += 2;
            continue;
        }
        if argument.starts_with('-') || argument.starts_with('+') {
            index += 1;
            continue;
        }
        return false;
    }
    false
}

fn is_shell_short_option_cluster(program_name: &str, argument: &str) -> bool {
    matches!(
        program_name,
        "sh" | "dash" | "bash" | "zsh" | "ksh" | "fish"
    ) && argument.starts_with('-')
        && !argument.starts_with("--")
        && argument[1..].contains('c')
}

fn looks_like_environment_assignment(value: &str) -> bool {
    let Some((name, _)) = value.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name.chars().enumerate().all(|(index, character)| {
            character == '_'
                || character.is_ascii_alphabetic()
                || (index > 0 && character.is_ascii_digit())
        })
}

fn validate_relative_directory(path: &str) -> Result<()> {
    validate_nonempty("requiredTests.workingDirectory", path)?;
    let path = Path::new(path);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        bail!("requiredTests.workingDirectory must stay inside the repository");
    }
    Ok(())
}

fn validate_nonempty(name: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{name} cannot be empty");
    }
    Ok(())
}

fn validate_git_sha(name: &str, value: &str, length: usize) -> Result<()> {
    if value.len() != length || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{name} must be a {length}-character hexadecimal SHA-256/Git digest");
    }
    Ok(())
}

pub fn match_contracts_for_file_changes(
    contracts: &[RegressionContractV1],
    changes: &[FileChangeV1],
) -> ContractMatchV1 {
    let conservative = changes
        .iter()
        .map(|change| ContractChangedFileV1 {
            path: change.path.clone(),
            previous_path: change.previous_path.clone(),
            changed_hunk: ChangedHunkV1::Unavailable(
                "changed hunk is unavailable from hook file metadata".to_string(),
            ),
        })
        .collect::<Vec<_>>();
    match_contracts_for_changes(contracts, &conservative)
}

/// Match hook-observed file changes using real Git hunks whenever the checkout can still
/// reconstruct them. This preserves literal-symbol semantics at Stop while retaining the
/// conservative path fallback for binary, unreadable, or oversized diffs.
pub fn match_contracts_for_repository_file_changes(
    repository: impl AsRef<Path>,
    contracts: &[RegressionContractV1],
    changes: &[FileChangeV1],
) -> Result<ContractMatchV1> {
    let root = crate::git::repository_identity(repository)?.root;
    let current_head = git_text(&root, &["rev-parse", "--verify", "HEAD"])
        .ok()
        .map(|head| head.trim().to_string());
    let inspectable = changes
        .iter()
        .map(|change| {
            let shell = ContractChangedFileV1 {
                path: change.path.clone(),
                previous_path: change.previous_path.clone(),
                changed_hunk: ChangedHunkV1::Available(String::new()),
            };
            let changed_hunk = match (change.before_head.as_deref(), change.after_head.as_deref()) {
                (Some(before), Some(after)) if before != after => {
                    read_git_hunk(&root, Some(before), Some(after), &shell)
                }
                _ => {
                    let dirty = read_git_hunk(&root, Some("HEAD"), None, &shell);
                    match (
                        &dirty,
                        change.before_head.as_deref(),
                        current_head.as_deref(),
                    ) {
                        (ChangedHunkV1::Available(hunk), Some(before), Some(current))
                            if hunk.is_empty() && before != current =>
                        {
                            read_git_hunk(&root, Some(before), Some(current), &shell)
                        }
                        _ => dirty,
                    }
                }
            };
            ContractChangedFileV1 {
                changed_hunk,
                ..shell
            }
        })
        .collect::<Vec<_>>();
    Ok(match_contracts_for_changes(contracts, &inspectable))
}

pub fn match_contracts_for_changes(
    contracts: &[RegressionContractV1],
    changes: &[ContractChangedFileV1],
) -> ContractMatchV1 {
    let mut relevant_contracts = Vec::new();
    let mut summaries = Vec::new();
    let mut matched_paths = BTreeSet::new();
    let mut warnings = BTreeSet::new();
    for contract in contracts
        .iter()
        .filter(|contract| contract.status == ContractStatusV1::Active)
    {
        let mut reasons = BTreeSet::new();
        let mut contract_paths = BTreeSet::new();
        let mut matched = false;
        for (group_index, group) in contract.impact_selectors.iter().enumerate() {
            for change in changes {
                let matching_path = [Some(change.path.as_str()), change.previous_path.as_deref()]
                    .into_iter()
                    .flatten()
                    .find(|path| path_matches(&group.path, path));
                let Some(path) = matching_path else {
                    continue;
                };
                if group.symbols.is_empty() {
                    matched = true;
                    contract_paths.insert(path.to_string());
                    reasons.insert(format!(
                        "impactSelectors[{group_index}] path matched `{path}`"
                    ));
                    continue;
                }
                match &change.changed_hunk {
                    ChangedHunkV1::Available(hunk) => {
                        if let Some(symbol) = group
                            .symbols
                            .iter()
                            .find(|symbol| contains_literal_token(hunk, symbol))
                        {
                            matched = true;
                            contract_paths.insert(path.to_string());
                            reasons.insert(format!(
                                "impactSelectors[{group_index}] path `{path}` and literal symbol `{symbol}` matched"
                            ));
                        }
                    }
                    ChangedHunkV1::Unavailable(reason) => {
                        matched = true;
                        contract_paths.insert(path.to_string());
                        reasons.insert(format!(
                            "impactSelectors[{group_index}] path matched `{path}`; symbol inspection unavailable"
                        ));
                        warnings.insert(format!(
                            "Contract `{}` treated `{path}` as relevant conservatively: {reason}",
                            contract.id
                        ));
                    }
                }
            }
        }
        if matched {
            matched_paths.extend(contract_paths);
            relevant_contracts.push(contract.clone());
            summaries.push(RelevantContractV1 {
                id: contract.id.clone(),
                title: contract.title.clone(),
                invariant: contract.invariant.clone(),
                match_reasons: reasons.into_iter().collect(),
            });
        }
    }
    ContractMatchV1 {
        relevant_contracts,
        summaries,
        matched_paths: matched_paths.into_iter().collect(),
        warnings: warnings.into_iter().collect(),
    }
}

fn path_matches(selector: &ImpactPathSelectorV1, candidate: &str) -> bool {
    match selector.kind {
        PathSelectorKindV1::Exact => candidate == selector.value,
        PathSelectorKindV1::Prefix => candidate.starts_with(&selector.value),
    }
}

fn contains_literal_token(text: &str, symbol: &str) -> bool {
    text.match_indices(symbol).any(|(start, _)| {
        let before = text[..start].chars().next_back();
        let end = start + symbol.len();
        let after = text[end..].chars().next();
        !before.is_some_and(is_identifier_character) && !after.is_some_and(is_identifier_character)
    })
}

fn is_identifier_character(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

pub fn resolve_merge_base(repository: impl AsRef<Path>, base: &str) -> Result<(String, String)> {
    let root = crate::git::repository_identity(repository)?.root;
    let head = git_text(&root, &["rev-parse", "--verify", "HEAD^{commit}"])
        .context("current HEAD is not a resolvable commit")?;
    let head = head.trim().to_string();
    let base_commit = format!("{base}^{{commit}}");
    git_text(&root, &["rev-parse", "--verify", &base_commit]).with_context(|| {
        format!(
            "base ref `{base}` is not resolvable; fetch enough history for merge-base calculation"
        )
    })?;
    let merge_base = git_text(&root, &["merge-base", base, &head]).with_context(|| {
        format!(
            "cannot compute merge-base for `{base}` and HEAD; the checkout may be shallow or missing base history"
        )
    })?;
    let merge_base = merge_base.trim().to_string();
    if merge_base.is_empty() {
        bail!("git merge-base returned an empty commit");
    }
    Ok((head, merge_base))
}

fn changed_files_for_check(
    repository: &Path,
    merge_base: &str,
    head: &str,
) -> Result<Vec<ContractChangedFileV1>> {
    let committed = git_name_status(repository, Some(merge_base), Some(head))?;
    let dirty = git_name_status(repository, Some("HEAD"), None)?;
    let mut merged = BTreeMap::new();
    for mut change in committed {
        change.changed_hunk = read_git_hunk(repository, Some(merge_base), Some(head), &change);
        insert_changed_file(&mut merged, change);
    }
    for mut change in dirty {
        change.changed_hunk = read_git_hunk(repository, Some("HEAD"), None, &change);
        insert_changed_file(&mut merged, change);
    }
    for path in git_untracked_paths(repository)? {
        let change = ContractChangedFileV1 {
            changed_hunk: read_untracked_hunk(repository, &path),
            path: path.clone(),
            previous_path: None,
        };
        merged.insert((path, None), change);
    }
    Ok(merged.into_values().collect())
}

fn insert_changed_file(
    changes: &mut BTreeMap<(String, Option<String>), ContractChangedFileV1>,
    incoming: ContractChangedFileV1,
) {
    let key = (incoming.path.clone(), incoming.previous_path.clone());
    changes
        .entry(key)
        .and_modify(|existing| {
            existing.changed_hunk = merge_changed_hunks(
                std::mem::replace(
                    &mut existing.changed_hunk,
                    ChangedHunkV1::Available(String::new()),
                ),
                incoming.changed_hunk.clone(),
            );
        })
        .or_insert(incoming);
}

fn merge_changed_hunks(left: ChangedHunkV1, right: ChangedHunkV1) -> ChangedHunkV1 {
    match (left, right) {
        (ChangedHunkV1::Available(left), ChangedHunkV1::Available(right)) => {
            ChangedHunkV1::Available(match (left.is_empty(), right.is_empty()) {
                (true, _) => right,
                (_, true) => left,
                (false, false) => format!("{left}\n{right}"),
            })
        }
        (ChangedHunkV1::Unavailable(left), ChangedHunkV1::Unavailable(right)) => {
            ChangedHunkV1::Unavailable(format!("{left}; {right}"))
        }
        (ChangedHunkV1::Unavailable(reason), _) | (_, ChangedHunkV1::Unavailable(reason)) => {
            ChangedHunkV1::Unavailable(reason)
        }
    }
}

fn git_name_status(
    repository: &Path,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Vec<ContractChangedFileV1>> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(repository)
        .args(["diff", "--name-status", "-z", "--find-renames"]);
    if let Some(from) = from {
        command.arg(from);
    }
    if let Some(to) = to {
        command.arg(to);
    }
    command.arg("--");
    let output = command.output().context("run git diff --name-status")?;
    if !output.status.success() {
        bail!(
            "git diff --name-status failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let fields = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| String::from_utf8_lossy(field).into_owned())
        .collect::<Vec<_>>();
    let mut changes = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let status = &fields[index];
        index += 1;
        let path = fields
            .get(index)
            .ok_or_else(|| anyhow!("truncated git name-status output"))?
            .clone();
        index += 1;
        let (previous_path, path) = if status.starts_with('R') || status.starts_with('C') {
            let new_path = fields
                .get(index)
                .ok_or_else(|| anyhow!("truncated git rename output"))?
                .clone();
            index += 1;
            (Some(path), new_path)
        } else {
            (None, path)
        };
        let Some(path) = crate::git::validated_repository_relative_path(&path) else {
            continue;
        };
        let previous_path = previous_path
            .as_deref()
            .and_then(crate::git::validated_repository_relative_path);
        changes.push(ContractChangedFileV1 {
            path,
            previous_path,
            changed_hunk: ChangedHunkV1::Available(String::new()),
        });
    }
    Ok(changes)
}

fn git_untracked_paths(repository: &Path) -> Result<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["ls-files", "--others", "--exclude-standard", "-z"])
        .output()
        .context("run git ls-files for untracked files")?;
    if !output.status.success() {
        bail!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .filter_map(|field| String::from_utf8(field.to_vec()).ok())
        .filter_map(|path| crate::git::validated_repository_relative_path(&path))
        .collect())
}

fn read_git_hunk(
    repository: &Path,
    from: Option<&str>,
    to: Option<&str>,
    change: &ContractChangedFileV1,
) -> ChangedHunkV1 {
    let mut args = vec!["diff", "--unified=0", "--no-ext-diff", "--no-textconv"];
    if let Some(from) = from {
        args.push(from);
    }
    if let Some(to) = to {
        args.push(to);
    }
    args.push("--");
    let mut owned = args
        .iter()
        .map(|value| (*value).to_string())
        .collect::<Vec<_>>();
    if let Some(previous) = &change.previous_path {
        owned.push(previous.clone());
    }
    owned.push(change.path.clone());
    match git_bounded(repository, &owned, MAX_SYMBOL_DIFF_BYTES) {
        Ok(bytes)
            if bytes
                .windows(b"Binary files".len())
                .any(|window| window == b"Binary files") =>
        {
            ChangedHunkV1::Unavailable(
                "binary diff cannot be inspected for literal symbols".to_string(),
            )
        }
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(diff) => ChangedHunkV1::Available(extract_changed_lines(&diff)),
            Err(_) => ChangedHunkV1::Unavailable("diff is not valid UTF-8".to_string()),
        },
        Err(error) => ChangedHunkV1::Unavailable(error.to_string()),
    }
}

fn read_untracked_hunk(repository: &Path, relative_path: &str) -> ChangedHunkV1 {
    let path = repository.join(relative_path);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) => return ChangedHunkV1::Unavailable(format!("cannot inspect file: {error}")),
    };
    if !metadata.is_file() {
        return ChangedHunkV1::Unavailable("path is not a regular file".to_string());
    }
    if metadata.len() > MAX_SYMBOL_DIFF_BYTES as u64 {
        return ChangedHunkV1::Unavailable(format!(
            "diff exceeds {MAX_SYMBOL_DIFF_BYTES} byte symbol inspection limit"
        ));
    }
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => return ChangedHunkV1::Unavailable(format!("cannot read file: {error}")),
    };
    if bytes.contains(&0) {
        return ChangedHunkV1::Unavailable(
            "binary file cannot be inspected for literal symbols".to_string(),
        );
    }
    match String::from_utf8(bytes) {
        Ok(text) => ChangedHunkV1::Available(text),
        Err(_) => ChangedHunkV1::Unavailable("file is not valid UTF-8".to_string()),
    }
}

fn git_bounded(repository: &Path, args: &[String], limit: usize) -> Result<Vec<u8>> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("run git {}", args.join(" ")))?;
    let mut output = Vec::new();
    child
        .stdout
        .take()
        .context("capture git diff stdout")?
        .take((limit + 1) as u64)
        .read_to_end(&mut output)?;
    if output.len() > limit {
        let _ = child.kill();
        let _ = child.wait();
        bail!("diff exceeds {limit} byte symbol inspection limit");
    }
    let status = child.wait()?;
    if !status.success() {
        bail!("git diff failed while reading changed hunks");
    }
    Ok(output)
}

fn extract_changed_lines(diff: &str) -> String {
    diff.lines()
        .filter(|line| {
            (line.starts_with('+') && !line.starts_with("+++"))
                || (line.starts_with('-') && !line.starts_with("---"))
        })
        .map(|line| &line[1..])
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn related_content_fingerprint(
    repository: impl AsRef<Path>,
    paths: &[String],
) -> Result<String> {
    let root = crate::git::repository_identity(repository)?.root;
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("canonicalize repository root {}", root.display()))?;
    let mut paths = paths
        .iter()
        .filter_map(|path| crate::git::validated_repository_relative_path(path))
        .collect::<BTreeSet<_>>();
    let mut hasher = Sha256::new();
    for path in std::mem::take(&mut paths) {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        let full_path = canonical_root.join(&path);
        let metadata = match fs::symlink_metadata(&full_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                hasher.update(b"missing\0");
                continue;
            }
            Err(error) => {
                return Err(error).with_context(|| format!("inspect relevant path {path}"));
            }
        };
        if metadata.file_type().is_symlink() {
            let mut content = Sha256::new();
            content.update(b"symlink\0");
            content.update(
                fs::read_link(&full_path)
                    .with_context(|| format!("read relevant symlink {path}"))?
                    .as_os_str()
                    .as_encoded_bytes(),
            );
            update_content_state(&mut hasher, Some(&hex::encode(content.finalize())));
            continue;
        }
        if !metadata.is_file() {
            bail!("relevant path `{path}` is not a regular file or symlink");
        }
        if metadata.len() > MAX_FINGERPRINT_FILE_BYTES {
            bail!(
                "relevant path `{path}` exceeds the {MAX_FINGERPRINT_FILE_BYTES} byte fingerprint limit"
            );
        }
        let canonical_path = full_path
            .canonicalize()
            .with_context(|| format!("canonicalize relevant path {path}"))?;
        if !canonical_path.starts_with(&canonical_root) {
            bail!("relevant path `{path}` resolves outside the repository");
        }
        match fs::File::open(&canonical_path) {
            Ok(mut file) => {
                let mut content = Sha256::new();
                content.update(b"file\0");
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    content.update((metadata.permissions().mode() & 0o111).to_le_bytes());
                }
                let mut buffer = [0_u8; 64 * 1024];
                loop {
                    let read = file
                        .read(&mut buffer)
                        .with_context(|| format!("hash relevant path {path}"))?;
                    if read == 0 {
                        break;
                    }
                    content.update(&buffer[..read]);
                }
                update_content_state(&mut hasher, Some(&hex::encode(content.finalize())));
            }
            Err(error) => return Err(error).with_context(|| format!("open relevant path {path}")),
        }
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Reconstruct the same related-content fingerprint from the immutable Git snapshot captured by
/// the hook process when a test command finished. Dirty paths use stored SHA-256 states; clean
/// paths are read from that snapshot's commit, never from the later replay-time checkout.
pub fn related_content_fingerprint_from_snapshot(
    repository: impl AsRef<Path>,
    paths: &[String],
    snapshot: &GitSnapshotV1,
) -> Result<String> {
    let identity = crate::git::repository_identity(repository)?;
    if snapshot.repository_id != identity.id {
        bail!("test execution snapshot belongs to a different repository");
    }
    let paths = paths
        .iter()
        .filter_map(|path| crate::git::validated_repository_relative_path(path))
        .collect::<BTreeSet<_>>();
    let mut hasher = Sha256::new();
    for path in paths {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        if let Some(state) = snapshot.content_fingerprints.get(&path) {
            update_content_state(&mut hasher, state.as_deref());
            continue;
        }
        if snapshot.dirty_files.iter().any(|dirty| dirty == &path) {
            bail!("test execution snapshot could not fingerprint dirty relevant path `{path}`");
        }
        let state = snapshot
            .head
            .as_deref()
            .map(|head| git_tree_content_state(&identity.root, head, &path))
            .transpose()?
            .flatten();
        update_content_state(&mut hasher, state.as_deref());
    }
    Ok(hex::encode(hasher.finalize()))
}

fn update_content_state(hasher: &mut Sha256, fingerprint: Option<&str>) {
    match fingerprint {
        Some(fingerprint) => {
            hasher.update(b"sha256\0");
            hasher.update(fingerprint.as_bytes());
        }
        None => hasher.update(b"missing\0"),
    }
}

fn git_tree_content_state(repository: &Path, head: &str, path: &str) -> Result<Option<String>> {
    let entry = git_bounded(
        repository,
        &[
            "ls-tree".to_string(),
            "-z".to_string(),
            head.to_string(),
            "--".to_string(),
            path.to_string(),
        ],
        8 * 1024,
    )?;
    if entry.is_empty() {
        return Ok(None);
    }
    let entry = entry.split(|byte| *byte == 0).next().unwrap_or_default();
    let entry = std::str::from_utf8(entry).context("Git tree entry was not UTF-8")?;
    let metadata = entry.split('\t').next().unwrap_or_default();
    let mut fields = metadata.split_whitespace();
    let mode = fields
        .next()
        .context("Git tree entry is missing its mode")?;
    let kind = fields
        .next()
        .context("Git tree entry is missing its type")?;
    let object = fields
        .next()
        .context("Git tree entry is missing its object id")?;
    if kind != "blob" {
        bail!("relevant path `{path}` is not a Git blob in the execution snapshot");
    }
    let bytes = git_bounded(
        repository,
        &[
            "cat-file".to_string(),
            "blob".to_string(),
            object.to_string(),
        ],
        MAX_FINGERPRINT_FILE_BYTES as usize,
    )?;
    let mut content = Sha256::new();
    if mode == "120000" {
        content.update(b"symlink\0");
    } else {
        content.update(b"file\0");
        #[cfg(unix)]
        content.update(if mode == "100755" { 0o111_u32 } else { 0_u32 }.to_le_bytes());
    }
    content.update(bytes);
    Ok(Some(hex::encode(content.finalize())))
}

pub fn freshness_from_success(
    current_content_fingerprint: &str,
    successful_content_fingerprint: Option<&str>,
) -> RequiredTestStateV1 {
    match successful_content_fingerprint {
        None => RequiredTestStateV1::Missing,
        Some(success) if success == current_content_fingerprint => RequiredTestStateV1::Passed,
        Some(_) => RequiredTestStateV1::Stale,
    }
}

pub fn evaluation_from_match(
    repository_id: impl Into<String>,
    task_id: Option<String>,
    matches: &ContractMatchV1,
    content_fingerprint: String,
    continuation_issued: bool,
) -> ContractEvaluationV1 {
    let repository_id = repository_id.into();
    let evaluated_at = Utc::now();
    let required_tests = matches
        .relevant_contracts
        .iter()
        .flat_map(|contract| {
            contract
                .required_tests
                .iter()
                .map(|test| RequiredTestEvaluationV1 {
                    contract_id: contract.id.clone(),
                    test_id: test.id.clone(),
                    name: test.name.clone(),
                    program: test.program.clone(),
                    args: test.args.clone(),
                    working_directory: test.working_directory.clone(),
                    timeout_seconds: test.timeout_seconds,
                    state: RequiredTestStateV1::Missing,
                    detail: None,
                })
        })
        .collect::<Vec<_>>();
    let readiness = readiness_for_tests(&required_tests);
    let id = evaluation_id(
        &repository_id,
        task_id.as_deref(),
        &content_fingerprint,
        evaluated_at,
    );
    ContractEvaluationV1 {
        schema_version: SCHEMA_VERSION_V1,
        id,
        repository_id,
        task_id,
        readiness,
        evaluated_at,
        relevant_contracts: matches.summaries.clone(),
        required_tests,
        warnings: matches.warnings.clone(),
        content_fingerprint,
        continuation_issued,
        base: None,
        head: None,
        merge_base: None,
    }
}

fn evaluation_id(
    repository_id: &str,
    task_id: Option<&str>,
    fingerprint: &str,
    evaluated_at: DateTime<Utc>,
) -> String {
    let mut hasher = Sha256::new();
    for part in [
        repository_id,
        task_id.unwrap_or_default(),
        fingerprint,
        &evaluated_at.timestamp_micros().to_string(),
    ] {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    format!("evaluation-{}", &hex::encode(hasher.finalize())[..24])
}

fn readiness_for_tests(tests: &[RequiredTestEvaluationV1]) -> ContractReadinessV1 {
    if tests
        .iter()
        .all(|test| test.state == RequiredTestStateV1::Passed)
    {
        ContractReadinessV1::Ready
    } else {
        ContractReadinessV1::ContractBlocked
    }
}

pub async fn check_contracts(
    repository: impl AsRef<Path>,
    base: &str,
    execute: bool,
) -> Result<ContractEvaluationV1> {
    let identity = crate::git::repository_identity(repository)?;
    let contracts = load_active_contracts(&identity.root)?;
    if contracts.is_empty() {
        let matches = ContractMatchV1 {
            relevant_contracts: Vec::new(),
            summaries: Vec::new(),
            matched_paths: Vec::new(),
            warnings: Vec::new(),
        };
        let content_fingerprint = related_content_fingerprint(&identity.root, &[])?;
        let mut evaluation =
            evaluation_from_match(identity.id, None, &matches, content_fingerprint, false);
        evaluation.base = Some(base.to_string());
        return Ok(evaluation);
    }
    let (head, merge_base) = resolve_merge_base(&identity.root, base)?;
    let changes = changed_files_for_check(&identity.root, &merge_base, &head)?;
    let matches = match_contracts_for_changes(&contracts, &changes);
    let content_fingerprint = related_content_fingerprint(&identity.root, &matches.matched_paths)?;
    let mut evaluation =
        evaluation_from_match(identity.id, None, &matches, content_fingerprint, false);
    evaluation.base = Some(base.to_string());
    evaluation.head = Some(head);
    evaluation.merge_base = Some(merge_base);
    if execute {
        execute_evaluation_tests(&identity.root, &matches.relevant_contracts, &mut evaluation)
            .await;
    }
    evaluation.readiness = readiness_for_tests(&evaluation.required_tests);
    Ok(evaluation)
}

async fn execute_evaluation_tests(
    repository: &Path,
    contracts: &[RegressionContractV1],
    evaluation: &mut ContractEvaluationV1,
) {
    let mut unique = BTreeMap::<TestCommandKey, RequiredTestV1>::new();
    for test in contracts
        .iter()
        .flat_map(|contract| &contract.required_tests)
    {
        let key = TestCommandKey {
            program: test.program.clone(),
            args: test.args.clone(),
            working_directory: test.working_directory.clone(),
        };
        unique
            .entry(key)
            .and_modify(|existing| {
                existing.timeout_seconds = existing.timeout_seconds.min(test.timeout_seconds)
            })
            .or_insert_with(|| test.clone());
    }
    let mut outcomes = BTreeMap::new();
    for (key, test) in unique {
        outcomes.insert(key, execute_required_test(repository, &test).await);
    }
    for test in &mut evaluation.required_tests {
        let key = TestCommandKey {
            program: test.program.clone(),
            args: test.args.clone(),
            working_directory: test.working_directory.clone(),
        };
        if let Some(outcome) = outcomes.get(&key) {
            test.state = outcome.state;
            test.detail = outcome.detail.clone();
        }
    }
}

async fn execute_required_test(repository: &Path, test: &RequiredTestV1) -> CommandOutcome {
    let working_directory = repository.join(&test.working_directory);
    let canonical_repository = match repository.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            return CommandOutcome {
                state: RequiredTestStateV1::Failed,
                detail: Some(format!("cannot resolve repository: {error}")),
            };
        }
    };
    let canonical_working_directory = match working_directory.canonicalize() {
        Ok(path) if path.starts_with(&canonical_repository) => path,
        Ok(_) => {
            return CommandOutcome {
                state: RequiredTestStateV1::Failed,
                detail: Some("workingDirectory escapes the repository".to_string()),
            };
        }
        Err(error) => {
            return CommandOutcome {
                state: RequiredTestStateV1::Failed,
                detail: Some(format!("workingDirectory is unavailable: {error}")),
            };
        }
    };
    let program = if test.program.contains('/') {
        canonical_working_directory.join(&test.program)
    } else {
        PathBuf::from(&test.program)
    };
    let mut command = tokio::process::Command::new(program);
    command
        .args(&test.args)
        .current_dir(canonical_working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let detail = if error.kind() == ErrorKind::NotFound {
                format!("missing executable `{}`", test.program)
            } else {
                format!("cannot start `{}`: {error}", test.program)
            };
            return CommandOutcome {
                state: RequiredTestStateV1::Failed,
                detail: Some(detail),
            };
        }
    };
    match tokio::time::timeout(
        Duration::from_secs(test.timeout_seconds),
        child.wait_with_output(),
    )
    .await
    {
        Err(_) => CommandOutcome {
            state: RequiredTestStateV1::Failed,
            detail: Some(format!("timed out after {} seconds", test.timeout_seconds)),
        },
        Ok(Err(error)) => CommandOutcome {
            state: RequiredTestStateV1::Failed,
            detail: Some(format!("test process failed: {error}")),
        },
        Ok(Ok(output)) if output.status.success() => CommandOutcome {
            state: RequiredTestStateV1::Passed,
            detail: None,
        },
        Ok(Ok(output)) => CommandOutcome {
            state: RequiredTestStateV1::Failed,
            detail: Some(format!("exited with status {}", output.status)),
        },
    }
}

pub fn init_contracts(
    repository: impl AsRef<Path>,
    github_actions: bool,
) -> Result<ContractInitReportV1> {
    let root = crate::git::repository_identity(repository)?.root;
    let directory = contracts_directory(&root);
    let directory_created = !directory.exists();
    fs::create_dir_all(&directory)
        .with_context(|| format!("create Contract directory {}", directory.display()))?;
    let mut workflow = None;
    let mut workflow_created = false;
    if github_actions {
        let workflow_path = root.join(CONTRACTS_WORKFLOW);
        workflow = Some(CONTRACTS_WORKFLOW.to_string());
        if !workflow_path.exists() {
            if let Some(parent) = workflow_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let contents = github_workflow();
            let mut options = OpenOptions::new();
            options.create_new(true).write(true);
            match options.open(&workflow_path) {
                Ok(mut file) => {
                    file.write_all(contents.as_bytes())?;
                    file.sync_all()?;
                    workflow_created = true;
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(ContractInitReportV1 {
        schema_version: SCHEMA_VERSION_V1,
        contracts_directory: CONTRACTS_DIRECTORY.to_string(),
        contracts_directory_created: directory_created,
        workflow,
        workflow_created,
    })
}

fn github_workflow() -> String {
    r#"name: PreviouslyOn Regression Contracts

on:
  pull_request:
  push:
    branches: [main]

permissions:
  contents: read

jobs:
  regression-contracts:
    runs-on: macos-14
    timeout-minutes: 60
    steps:
      - name: Check out source with merge-base history
        uses: actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5 # v4
        with:
          persist-credentials: false
          fetch-depth: 0
      - name: Install pinned PreviouslyOn CLI
        run: cargo install previously-on --version '=__PREVIOUSLY_ON_VERSION__' --locked --root "$RUNNER_TEMP/previously-on-cli"
      - name: Enforce Regression Contracts
        env:
          PREVIOUSLY_BASE: ${{ github.event.pull_request.base.sha || github.event.before }}
        run: '"$RUNNER_TEMP/previously-on-cli/bin/previously" contracts check --base "$PREVIOUSLY_BASE" --execute --json'
"#
    .replace("__PREVIOUSLY_ON_VERSION__", env!("CARGO_PKG_VERSION"))
}

fn git_text(repository: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .with_context(|| format!("run git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).context("git output was not UTF-8")
}
