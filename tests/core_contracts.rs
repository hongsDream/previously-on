use chrono::Utc;
use previously_on::contracts::{
    check_contracts, freshness_from_success, init_contracts, load_active_contracts, load_contracts,
    match_contracts_for_changes, related_content_fingerprint_from_snapshot, resolve_merge_base,
    update_contract, write_contract, ChangedHunkV1, ContractChangedFileV1, ContractOriginV1,
    ContractReadinessV1, ContractStatusV1, ImpactPathSelectorV1, ImpactSelectorGroupV1,
    PathSelectorKindV1, RegressionContractV1, RequiredTestStateV1, RequiredTestV1,
    CONTRACTS_WORKFLOW, MAX_FINGERPRINT_FILE_BYTES,
};
use previously_on::domain::SCHEMA_VERSION_V1;
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn git(repository: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn repository() -> TempDir {
    let temp = TempDir::new().unwrap();
    git(temp.path(), &["init", "-q", "-b", "main"]);
    git(
        temp.path(),
        &["config", "user.email", "contracts@example.test"],
    );
    git(
        temp.path(),
        &["config", "user.name", "Regression Contracts"],
    );
    std::fs::create_dir_all(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/lib.rs"), "pub fn stable() {}\n").unwrap();
    git(temp.path(), &["add", "."]);
    git(temp.path(), &["commit", "-qm", "initial"]);
    temp
}

fn contract(repository: &Path, id: uuid::Uuid) -> RegressionContractV1 {
    RegressionContractV1 {
        schema_version: SCHEMA_VERSION_V1,
        id: id.to_string(),
        title: "Keep the stable path redacted".to_string(),
        invariant: "stable behavior remains unchanged".to_string(),
        status: ContractStatusV1::Active,
        superseded_by: None,
        impact_selectors: vec![ImpactSelectorGroupV1 {
            path: ImpactPathSelectorV1 {
                kind: PathSelectorKindV1::Exact,
                value: "src/lib.rs".to_string(),
            },
            symbols: vec!["stable".to_string()],
        }],
        required_tests: vec![RequiredTestV1 {
            id: "core-contracts".to_string(),
            name: "Core contracts".to_string(),
            program: "cargo".to_string(),
            args: vec![
                "test".to_string(),
                "--test".to_string(),
                "core_contracts".to_string(),
            ],
            working_directory: ".".to_string(),
            timeout_seconds: 60,
        }],
        origin: ContractOriginV1 {
            fixed_at_commit: git(repository, &["rev-parse", "HEAD"]),
            recorded_at: Utc::now(),
            evidence_sha256: "a".repeat(64),
        },
    }
}

fn write_raw(repository: &Path, file_name: &str, contract: &RegressionContractV1) {
    let directory = repository.join(".previously-on/contracts");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join(file_name),
        serde_json::to_vec_pretty(contract).unwrap(),
    )
    .unwrap();
}

#[test]
fn public_contract_schema_is_camel_case_and_rejects_unknown_fields() {
    let repository = repository();
    let contract = contract(repository.path(), uuid::Uuid::new_v4());
    let mut value = serde_json::to_value(&contract).unwrap();
    assert!(value.get("schemaVersion").is_some());
    assert!(value.get("impactSelectors").is_some());
    assert!(value.get("requiredTests").is_some());
    assert!(value.get("schema_version").is_none());
    assert_eq!(value["impactSelectors"][0]["path"]["kind"], "exact");
    assert_eq!(value["impactSelectors"][0]["path"]["value"], "src/lib.rs");
    value.as_object_mut().unwrap().insert(
        "rawPrompt".to_string(),
        Value::String("must not persist".into()),
    );
    assert!(serde_json::from_value::<RegressionContractV1>(value).is_err());
}

#[test]
fn approved_uncommitted_contract_is_active_in_the_current_checkout() {
    let repository = repository();
    let contract = contract(repository.path(), uuid::Uuid::new_v4());
    let path = write_contract(repository.path(), &contract).unwrap();

    assert!(path.exists());
    assert!(git(repository.path(), &["status", "--short"]).contains("?? .previously-on/"));
    assert_eq!(
        load_active_contracts(repository.path()).unwrap(),
        vec![contract]
    );
}

#[test]
fn loader_fails_closed_for_schema_file_id_and_duplicate_contracts() {
    let repository = repository();
    let id = uuid::Uuid::new_v4();
    let mut invalid_schema = contract(repository.path(), id);
    invalid_schema.schema_version = 2;
    write_raw(repository.path(), &format!("{id}.json"), &invalid_schema);
    assert!(load_contracts(repository.path())
        .unwrap_err()
        .to_string()
        .contains("schemaVersion"));

    std::fs::remove_dir_all(repository.path().join(".previously-on")).unwrap();
    let valid = contract(repository.path(), id);
    write_raw(repository.path(), "wrong-id.json", &valid);
    assert!(load_contracts(repository.path())
        .unwrap_err()
        .to_string()
        .contains("file name"));

    std::fs::remove_dir_all(repository.path().join(".previously-on")).unwrap();
    write_raw(repository.path(), &format!("{id}.json"), &valid);
    write_raw(repository.path(), "second.json", &valid);
    assert!(load_contracts(repository.path())
        .unwrap_err()
        .to_string()
        .contains("duplicate/conflicting"));
}

#[test]
fn superseded_contract_is_valid_without_an_active_replacement() {
    let repo = repository();
    let old_id = uuid::Uuid::new_v4();
    let replacement_id = uuid::Uuid::new_v4();
    let mut old = contract(repo.path(), old_id);
    old.status = ContractStatusV1::Superseded;
    old.superseded_by = Some(replacement_id.to_string());
    write_raw(repo.path(), &format!("{old_id}.json"), &old);
    assert_eq!(load_contracts(repo.path()).unwrap(), vec![old.clone()]);
    assert!(load_active_contracts(repo.path()).unwrap().is_empty());

    let replacement = contract(repo.path(), replacement_id);
    write_raw(repo.path(), &format!("{replacement_id}.json"), &replacement);
    let loaded = load_contracts(repo.path()).unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(
        load_active_contracts(repo.path()).unwrap(),
        vec![replacement]
    );

    let conflicting_repository = repository();
    let conflicting_id = uuid::Uuid::new_v4();
    let mut conflicting = contract(conflicting_repository.path(), conflicting_id);
    conflicting.superseded_by = Some(uuid::Uuid::new_v4().to_string());
    write_raw(
        conflicting_repository.path(),
        &format!("{conflicting_id}.json"),
        &conflicting,
    );
    assert!(load_contracts(conflicting_repository.path())
        .unwrap_err()
        .to_string()
        .contains("active Contract"));
}

#[test]
fn atomic_update_preserves_uuid_file_identity_and_validates_the_full_set() {
    let repository = repository();
    let old_id = uuid::Uuid::new_v4();
    let replacement_id = uuid::Uuid::new_v4();
    let mut old = contract(repository.path(), old_id);
    let replacement = contract(repository.path(), replacement_id);
    write_contract(repository.path(), &old).unwrap();
    write_contract(repository.path(), &replacement).unwrap();

    old.status = ContractStatusV1::Superseded;
    old.superseded_by = Some(replacement_id.to_string());
    let path = update_contract(repository.path(), &old).unwrap();
    assert_eq!(
        path.file_name().unwrap().to_string_lossy(),
        format!("{old_id}.json")
    );
    assert_eq!(load_contracts(repository.path()).unwrap().len(), 2);
    assert!(std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tmp-")));
}

#[test]
fn required_test_validation_rejects_shell_strings_and_environment_assignments() {
    let repository = repository();
    let mut invalid = contract(repository.path(), uuid::Uuid::new_v4());
    invalid.required_tests[0].program = "bash".to_string();
    invalid.required_tests[0].args = vec!["-lc".to_string(), "cargo test".to_string()];
    assert!(write_contract(repository.path(), &invalid)
        .unwrap_err()
        .to_string()
        .contains("inline shell/interpreter"));

    invalid.required_tests[0].program = "cargo".to_string();
    invalid.required_tests[0].args = vec!["MODE=test".to_string(), "test".to_string()];
    assert!(write_contract(repository.path(), &invalid)
        .unwrap_err()
        .to_string()
        .contains("environment assignments"));
}

#[test]
fn required_test_validation_rejects_split_secrets_without_echoing_them() {
    let argv_repository = repository();
    let raw_secret = "verifier-secret-value-12345";
    let mut invalid = contract(argv_repository.path(), uuid::Uuid::new_v4());
    invalid.required_tests[0].args = vec!["--api-key".to_string(), raw_secret.to_string()];
    let error = write_contract(argv_repository.path(), &invalid).unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("contains sensitive data"));
    assert!(!message.contains(raw_secret));
    assert!(!argv_repository
        .path()
        .join(".previously-on/contracts")
        .exists());

    invalid.required_tests[0].args = vec!["--api-key".to_string(), "[REDACTED]".to_string()];
    assert!(write_contract(argv_repository.path(), &invalid)
        .unwrap_err()
        .to_string()
        .contains("contains sensitive data"));

    invalid.required_tests[0].args = vec!["--api-key".to_string(), raw_secret.to_string()];
    write_raw(
        argv_repository.path(),
        &format!("{}.json", invalid.id),
        &invalid,
    );
    let error = load_contracts(argv_repository.path()).unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("contains sensitive data"));
    assert!(!message.contains(raw_secret));

    let metadata_repository = repository();
    let raw_metadata_secret = "raw-git-contract-secret-67890";
    let mut raw = contract(metadata_repository.path(), uuid::Uuid::new_v4());
    raw.title = format!("api_key={raw_metadata_secret}");
    write_raw(
        metadata_repository.path(),
        &format!("{}.json", raw.id),
        &raw,
    );
    let error = load_contracts(metadata_repository.path()).unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("title contains sensitive data"));
    assert!(!message.contains(raw_metadata_secret));
}

#[test]
fn interpreter_eval_flags_are_rejected_without_blocking_script_options() {
    let rejected = [
        (
            "bash",
            vec!["--noprofile", "-O", "extglob", "-c", "echo ok"],
        ),
        ("bash", vec!["-lc", "echo ok"]),
        ("dash", vec!["-c", "echo ok"]),
        ("python3", vec!["-I", "-c", "print('ok')"]),
        ("python2.7", vec!["-E", "-c", "print('ok')"]),
        ("node", vec!["--trace-warnings", "-e", "console.log('ok')"]),
        ("nodejs", vec!["--eval=console.log('ok')"]),
    ];
    for (program, args) in rejected {
        let repository = repository();
        let mut invalid = contract(repository.path(), uuid::Uuid::new_v4());
        invalid.required_tests[0].program = program.to_string();
        invalid.required_tests[0].args = args.into_iter().map(str::to_string).collect();
        assert!(write_contract(repository.path(), &invalid)
            .unwrap_err()
            .to_string()
            .contains("inline shell/interpreter"));
    }

    for (program, args) in [
        ("bash", vec!["--noprofile", "scripts/test.sh", "-c"]),
        ("python3", vec!["-m", "pytest"]),
        ("node", vec!["--check", "scripts/test.js"]),
    ] {
        let repository = repository();
        let mut allowed = contract(repository.path(), uuid::Uuid::new_v4());
        allowed.required_tests[0].program = program.to_string();
        allowed.required_tests[0].args = args.into_iter().map(str::to_string).collect();
        write_contract(repository.path(), &allowed).unwrap();
    }
}

#[test]
fn atomic_write_redacts_secret_corpus_before_git_persistence() {
    let repository = repository();
    let mut secret = contract(repository.path(), uuid::Uuid::new_v4());
    secret.title = "password=contract-secret-value".to_string();
    secret.invariant = "Authorization: Bearer sk-contract-secret-token".to_string();
    let path = write_contract(repository.path(), &secret).unwrap();
    let bytes = std::fs::read_to_string(path).unwrap();

    for raw in [
        "contract-secret-value",
        "sk-contract-secret-token",
        "Bearer sk-",
    ] {
        assert!(!bytes.contains(raw), "Git Contract leaked `{raw}`");
    }
    assert!(bytes.contains("[REDACTED]"));
}

fn with_selector(
    mut contract: RegressionContractV1,
    kind: PathSelectorKindV1,
    value: &str,
    symbols: &[&str],
) -> RegressionContractV1 {
    contract.impact_selectors = vec![ImpactSelectorGroupV1 {
        path: ImpactPathSelectorV1 {
            kind,
            value: value.to_string(),
        },
        symbols: symbols.iter().map(|value| (*value).to_string()).collect(),
    }];
    contract
}

#[test]
fn matcher_handles_exact_prefix_literal_rename_delete_and_fallbacks() {
    let repository = repository();
    let exact = with_selector(
        contract(repository.path(), uuid::Uuid::new_v4()),
        PathSelectorKindV1::Exact,
        "src/lib.rs",
        &["stable"],
    );
    let prefix = with_selector(
        contract(repository.path(), uuid::Uuid::new_v4()),
        PathSelectorKindV1::Prefix,
        "src/",
        &[],
    );
    let rename = with_selector(
        contract(repository.path(), uuid::Uuid::new_v4()),
        PathSelectorKindV1::Exact,
        "src/old.rs",
        &["renamed_token"],
    );
    let deleted = with_selector(
        contract(repository.path(), uuid::Uuid::new_v4()),
        PathSelectorKindV1::Exact,
        "src/deleted.rs",
        &["deleted_token"],
    );
    let binary = with_selector(
        contract(repository.path(), uuid::Uuid::new_v4()),
        PathSelectorKindV1::Exact,
        "assets/logo.bin",
        &["binary_symbol"],
    );
    let oversized = with_selector(
        contract(repository.path(), uuid::Uuid::new_v4()),
        PathSelectorKindV1::Exact,
        "generated/huge.rs",
        &["huge_symbol"],
    );
    let false_positive = with_selector(
        contract(repository.path(), uuid::Uuid::new_v4()),
        PathSelectorKindV1::Exact,
        "src/lib.rs",
        &["table"],
    );
    let changes = vec![
        ContractChangedFileV1 {
            path: "src/lib.rs".to_string(),
            previous_path: None,
            changed_hunk: ChangedHunkV1::Available(
                "pub fn stable() {}\npub fn unstable_only() {}".to_string(),
            ),
        },
        ContractChangedFileV1 {
            path: "src/new.rs".to_string(),
            previous_path: Some("src/old.rs".to_string()),
            changed_hunk: ChangedHunkV1::Available("fn renamed_token() {}".to_string()),
        },
        ContractChangedFileV1 {
            path: "src/deleted.rs".to_string(),
            previous_path: None,
            changed_hunk: ChangedHunkV1::Available("fn deleted_token() {}".to_string()),
        },
        ContractChangedFileV1 {
            path: "assets/logo.bin".to_string(),
            previous_path: None,
            changed_hunk: ChangedHunkV1::Unavailable("binary diff".to_string()),
        },
        ContractChangedFileV1 {
            path: "generated/huge.rs".to_string(),
            previous_path: None,
            changed_hunk: ChangedHunkV1::Unavailable("oversized diff".to_string()),
        },
    ];
    let matches = match_contracts_for_changes(
        &[
            exact,
            prefix,
            rename,
            deleted,
            binary,
            oversized,
            false_positive,
        ],
        &changes,
    );

    assert_eq!(matches.relevant_contracts.len(), 6);
    assert!(matches.summaries.iter().any(|item| item
        .match_reasons
        .iter()
        .any(|reason| reason.contains("renamed_token"))));
    assert!(matches
        .warnings
        .iter()
        .any(|warning| warning.contains("binary diff")));
    assert!(matches
        .warnings
        .iter()
        .any(|warning| warning.contains("oversized diff")));
    assert!(!matches.summaries.iter().any(|item| item
        .match_reasons
        .iter()
        .any(|reason| reason.contains("`table`"))));
}

#[tokio::test]
async fn checkout_and_dirty_binary_changes_are_matched_conservatively() {
    let repository = repository();
    std::fs::create_dir_all(repository.path().join("assets")).unwrap();
    std::fs::write(repository.path().join("assets/logo.bin"), [0_u8, 1, 2]).unwrap();
    git(repository.path(), &["add", "."]);
    git(repository.path(), &["commit", "-qm", "add binary"]);
    let base = git(repository.path(), &["rev-parse", "HEAD"]);
    let binary_contract = with_selector(
        contract(repository.path(), uuid::Uuid::new_v4()),
        PathSelectorKindV1::Exact,
        "assets/logo.bin",
        &["cannot_inspect"],
    );
    write_contract(repository.path(), &binary_contract).unwrap();
    std::fs::write(repository.path().join("assets/logo.bin"), [0_u8, 255, 2]).unwrap();

    let evaluation = check_contracts(repository.path(), &base, false)
        .await
        .unwrap();
    assert_eq!(evaluation.readiness, ContractReadinessV1::ContractBlocked);
    assert_eq!(evaluation.relevant_contracts.len(), 1);
    assert!(evaluation
        .warnings
        .iter()
        .any(|warning| warning.contains("binary")));
    assert_eq!(
        evaluation.required_tests[0].state,
        RequiredTestStateV1::Missing
    );
}

#[tokio::test]
async fn committed_and_dirty_hunks_for_the_same_path_are_both_preserved() {
    let repository = repository();
    let base = git(repository.path(), &["rev-parse", "HEAD"]);
    let item = with_selector(
        contract(repository.path(), uuid::Uuid::new_v4()),
        PathSelectorKindV1::Exact,
        "src/lib.rs",
        &["regression_symbol"],
    );
    write_contract(repository.path(), &item).unwrap();
    std::fs::write(
        repository.path().join("src/lib.rs"),
        "pub fn regression_symbol() {}\n",
    )
    .unwrap();
    git(repository.path(), &["add", "src/lib.rs"]);
    git(
        repository.path(),
        &["commit", "-qm", "add regression symbol"],
    );
    std::fs::write(
        repository.path().join("src/lib.rs"),
        "pub fn regression_symbol() {}\npub fn unrelated_dirty() {}\n",
    )
    .unwrap();

    let evaluation = check_contracts(repository.path(), &base, false)
        .await
        .unwrap();
    assert_eq!(evaluation.relevant_contracts.len(), 1);
    assert!(evaluation.relevant_contracts[0]
        .match_reasons
        .iter()
        .any(|reason| reason.contains("regression_symbol")));
}

#[cfg(unix)]
fn executable(path: &Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, contents).unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(path, permissions).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn argv_execution_deduplicates_identical_commands() {
    let repository = repository();
    let base = git(repository.path(), &["rev-parse", "HEAD"]);
    executable(
        &repository.path().join("test-pass.sh"),
        "#!/bin/sh\nprintf 'run\\n' >> count.txt\nexit 0\n",
    );
    for index in 0..2 {
        let mut item = with_selector(
            contract(repository.path(), uuid::Uuid::new_v4()),
            PathSelectorKindV1::Exact,
            "src/lib.rs",
            &[],
        );
        item.required_tests[0] = RequiredTestV1 {
            id: format!("dedupe-{index}"),
            name: format!("Dedupe {index}"),
            program: "./test-pass.sh".to_string(),
            args: Vec::new(),
            working_directory: ".".to_string(),
            timeout_seconds: 10,
        };
        write_contract(repository.path(), &item).unwrap();
    }
    std::fs::write(
        repository.path().join("src/lib.rs"),
        "pub fn stable_changed() {}\n",
    )
    .unwrap();

    let evaluation = check_contracts(repository.path(), &base, true)
        .await
        .unwrap();
    assert_eq!(evaluation.readiness, ContractReadinessV1::Ready);
    assert_eq!(evaluation.required_tests.len(), 2);
    assert!(evaluation
        .required_tests
        .iter()
        .all(|test| test.state == RequiredTestStateV1::Passed));
    assert_eq!(
        std::fs::read_to_string(repository.path().join("count.txt")).unwrap(),
        "run\n"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn argv_execution_reports_failure_timeout_and_missing_executable() {
    let repository = repository();
    let base = git(repository.path(), &["rev-parse", "HEAD"]);
    executable(
        &repository.path().join("test-fail.sh"),
        "#!/bin/sh\nexit 7\n",
    );
    executable(
        &repository.path().join("test-timeout.sh"),
        "#!/bin/sh\nsleep 3\n",
    );
    for (id, program, timeout) in [
        ("failure", "./test-fail.sh", 10),
        ("timeout", "./test-timeout.sh", 1),
        ("missing", "definitely-missing-previously-program", 10),
    ] {
        let mut item = with_selector(
            contract(repository.path(), uuid::Uuid::new_v4()),
            PathSelectorKindV1::Exact,
            "src/lib.rs",
            &[],
        );
        item.required_tests[0] = RequiredTestV1 {
            id: id.to_string(),
            name: id.to_string(),
            program: program.to_string(),
            args: Vec::new(),
            working_directory: ".".to_string(),
            timeout_seconds: timeout,
        };
        write_contract(repository.path(), &item).unwrap();
    }
    std::fs::write(
        repository.path().join("src/lib.rs"),
        "pub fn changed() {}\n",
    )
    .unwrap();

    let evaluation = check_contracts(repository.path(), &base, true)
        .await
        .unwrap();
    assert_eq!(evaluation.readiness, ContractReadinessV1::ContractBlocked);
    assert!(evaluation.required_tests.iter().any(|test| test
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("status"))));
    assert!(evaluation.required_tests.iter().any(|test| test
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("timed out"))));
    assert!(evaluation.required_tests.iter().any(|test| test
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("missing executable"))));
}

#[tokio::test]
async fn no_matching_contract_is_ready_without_execution() {
    let repository = repository();
    let base = git(repository.path(), &["rev-parse", "HEAD"]);
    let item = with_selector(
        contract(repository.path(), uuid::Uuid::new_v4()),
        PathSelectorKindV1::Exact,
        "unrelated/path.rs",
        &[],
    );
    write_contract(repository.path(), &item).unwrap();

    let evaluation = check_contracts(repository.path(), &base, true)
        .await
        .unwrap();
    assert_eq!(evaluation.readiness, ContractReadinessV1::Ready);
    assert!(evaluation.relevant_contracts.is_empty());
    assert!(evaluation.required_tests.is_empty());
}

#[tokio::test]
async fn no_active_contracts_skip_invalid_merge_base_and_remain_ready() {
    let repository = repository();
    let evaluation = check_contracts(repository.path(), "definitely-invalid-base", true)
        .await
        .unwrap();
    assert_eq!(evaluation.readiness, ContractReadinessV1::Ready);
    assert!(evaluation.relevant_contracts.is_empty());
    assert!(evaluation.required_tests.is_empty());
    assert_eq!(evaluation.base.as_deref(), Some("definitely-invalid-base"));
    assert!(evaluation.merge_base.is_none());

    let superseded_id = uuid::Uuid::new_v4();
    let mut superseded = contract(repository.path(), superseded_id);
    superseded.status = ContractStatusV1::Superseded;
    superseded.superseded_by = Some(uuid::Uuid::new_v4().to_string());
    write_raw(
        repository.path(),
        &format!("{superseded_id}.json"),
        &superseded,
    );
    let evaluation = check_contracts(repository.path(), "still-invalid-base", true)
        .await
        .unwrap();
    assert_eq!(evaluation.readiness, ContractReadinessV1::Ready);
    assert!(evaluation.relevant_contracts.is_empty());
    assert!(evaluation.merge_base.is_none());
}

#[cfg(unix)]
#[test]
fn fingerprints_hash_symlink_identity_without_following_external_targets() {
    use std::os::unix::fs::symlink;

    let repository = repository();
    let external = TempDir::new().unwrap();
    let target = external.path().join("external.txt");
    std::fs::write(&target, "external-before\n").unwrap();
    symlink(&target, repository.path().join("src/link.rs")).unwrap();
    let paths = vec!["src/link.rs".to_string()];
    let before =
        previously_on::contracts::related_content_fingerprint(repository.path(), &paths).unwrap();
    std::fs::write(&target, "external-after-with-different-content\n").unwrap();
    let after =
        previously_on::contracts::related_content_fingerprint(repository.path(), &paths).unwrap();
    assert_eq!(before, after);

    symlink(external.path(), repository.path().join("linked-external")).unwrap();
    let nested = vec!["linked-external/external.txt".to_string()];
    let error = previously_on::contracts::related_content_fingerprint(repository.path(), &nested)
        .unwrap_err();
    assert!(error.to_string().contains("outside the repository"));
}

#[test]
fn oversized_fingerprint_input_fails_closed_without_reading_the_file() {
    let repository = repository();
    let path = repository.path().join("src/oversized.bin");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(MAX_FINGERPRINT_FILE_BYTES + 1).unwrap();
    let error = previously_on::contracts::related_content_fingerprint(
        repository.path(),
        &["src/oversized.bin".to_string()],
    )
    .unwrap_err();
    assert!(error.to_string().contains("fingerprint limit"));
}

#[test]
fn execution_snapshot_fingerprint_matches_then_becomes_stale_after_a_related_edit() {
    let repository = repository();
    let paths = vec!["src/lib.rs".to_string()];
    let execution_snapshot = previously_on::git::capture_snapshot(repository.path()).unwrap();
    let execution =
        related_content_fingerprint_from_snapshot(repository.path(), &paths, &execution_snapshot)
            .unwrap();
    let current =
        previously_on::contracts::related_content_fingerprint(repository.path(), &paths).unwrap();
    assert_eq!(execution, current);

    std::fs::write(
        repository.path().join("src/lib.rs"),
        "pub fn stable() {}\npub fn changed_after_test() {}\n",
    )
    .unwrap();
    let changed =
        previously_on::contracts::related_content_fingerprint(repository.path(), &paths).unwrap();
    assert_ne!(execution, changed);
}

#[test]
fn freshness_changes_after_related_content_changes() {
    assert_eq!(
        freshness_from_success("current", None),
        RequiredTestStateV1::Missing
    );
    assert_eq!(
        freshness_from_success("current", Some("older")),
        RequiredTestStateV1::Stale
    );
    assert_eq!(
        freshness_from_success("current", Some("current")),
        RequiredTestStateV1::Passed
    );
}

#[test]
fn merge_base_errors_explain_missing_or_shallow_history() {
    let repository = repository();
    let error = resolve_merge_base(repository.path(), "missing-base-ref").unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("fetch enough history"));
    assert!(message.contains("missing-base-ref"));
}

#[test]
fn shallow_checkout_error_requests_more_history() {
    let source = repository();
    let first_commit = git(source.path(), &["rev-parse", "HEAD"]);
    std::fs::write(source.path().join("src/lib.rs"), "pub fn second() {}\n").unwrap();
    git(source.path(), &["add", "."]);
    git(source.path(), &["commit", "-qm", "second"]);
    let clone_parent = TempDir::new().unwrap();
    let shallow = clone_parent.path().join("shallow");
    let source_url = format!("file://{}", source.path().display());
    let output = Command::new("git")
        .args([
            "clone",
            "-q",
            "--depth",
            "1",
            &source_url,
            shallow.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());

    let error = resolve_merge_base(&shallow, &first_commit).unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("fetch enough history") || message.contains("shallow"));
}

#[test]
fn init_is_version_pinned_fetches_history_and_never_overwrites() {
    let repository = repository();
    let first = init_contracts(repository.path(), true).unwrap();
    assert!(first.contracts_directory_created);
    assert!(first.workflow_created);
    let workflow_path = repository.path().join(CONTRACTS_WORKFLOW);
    let workflow = std::fs::read_to_string(&workflow_path).unwrap();
    assert!(workflow.contains("runs-on: macos-14"));
    assert!(
        workflow.contains("uses: actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5 # v4")
    );
    assert!(workflow.contains("fetch-depth: 0"));
    assert!(workflow.contains(&format!(
        "cargo install previously-on --version '={}' --locked --root \"$RUNNER_TEMP/previously-on-cli\"",
        env!("CARGO_PKG_VERSION")
    )));
    assert!(workflow.contains(
        "\"$RUNNER_TEMP/previously-on-cli/bin/previously\" contracts check --base \"$PREVIOUSLY_BASE\" --execute --json"
    ));
    assert!(!workflow.contains("cargo build --locked --release --bin previously"));

    std::fs::write(&workflow_path, "user-owned-workflow\n").unwrap();
    let second = init_contracts(repository.path(), true).unwrap();
    assert!(!second.contracts_directory_created);
    assert!(!second.workflow_created);
    assert_eq!(
        std::fs::read_to_string(workflow_path).unwrap(),
        "user-owned-workflow\n"
    );
}
