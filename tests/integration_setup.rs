use std::fs;

use previously_on::setup::{
    install_codex, install_codex_with_options, uninstall_codex, uninstall_codex_detailed,
    SetupPaths, MANAGED_ID,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

fn create_private_dir(path: &std::path::Path) {
    fs::create_dir_all(path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
}

fn write_private_file(path: &std::path::Path, bytes: impl AsRef<[u8]>) {
    fs::write(path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn journal_target(path: &std::path::Path, bytes: &[u8]) -> Value {
    let hash = hex::encode(Sha256::digest(bytes));
    json!({
        "path": path,
        "originalExisted": true,
        "originalBytes": bytes,
        "originalSha256": hash,
        "desiredBytes": bytes,
        "desiredSha256": hash,
    })
}

fn tree_snapshot(root: &std::path::Path) -> Vec<(std::path::PathBuf, Option<Vec<u8>>)> {
    fn walk(
        root: &std::path::Path,
        path: &std::path::Path,
        entries: &mut Vec<(std::path::PathBuf, Option<Vec<u8>>)>,
    ) {
        let mut children = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        children.sort();
        for child in children {
            let relative = child.strip_prefix(root).unwrap().to_path_buf();
            let metadata = fs::symlink_metadata(&child).unwrap();
            if metadata.file_type().is_symlink() {
                entries.push((
                    relative,
                    Some(
                        format!("symlink:{}", fs::read_link(&child).unwrap().display())
                            .into_bytes(),
                    ),
                ));
            } else if metadata.is_dir() {
                entries.push((relative, None));
                walk(root, &child, entries);
            } else {
                entries.push((relative, Some(fs::read(&child).unwrap())));
            }
        }
    }

    let mut entries = Vec::new();
    walk(root, root, &mut entries);
    entries
}

fn valid_install_journal(paths: &SetupPaths) -> Value {
    let hooks = fs::read(paths.hooks_path()).unwrap();
    let config = fs::read(paths.config_path()).unwrap();
    let manifest = fs::read(paths.manifest_path()).unwrap();
    json!({
        "version": 1,
        "managedId": MANAGED_ID,
        "operation": "install",
        "targets": [
            journal_target(&paths.hooks_path(), &hooks),
            journal_target(&paths.config_path(), &config),
            journal_target(&paths.manifest_path(), &manifest),
        ]
    })
}

fn valid_uninstall_journal(paths: &SetupPaths) -> Value {
    let hooks = fs::read(paths.hooks_path()).unwrap();
    let config = fs::read(paths.config_path()).unwrap();
    let manifest = fs::read(paths.manifest_path()).unwrap();
    let manifest_hash = hex::encode(Sha256::digest(&manifest));
    let archive = paths.data_dir.join(format!(
        "setup-manifest.uninstalled-1-{}.json",
        uuid::Uuid::now_v7()
    ));
    json!({
        "version": 1,
        "managedId": MANAGED_ID,
        "operation": "uninstall",
        "targets": [
            journal_target(&paths.hooks_path(), &hooks),
            journal_target(&paths.config_path(), &config),
            {
                "path": archive,
                "originalExisted": false,
                "originalBytes": [],
                "originalSha256": null,
                "desiredBytes": manifest,
                "desiredSha256": manifest_hash,
            },
            {
                "path": paths.manifest_path(),
                "originalExisted": true,
                "originalBytes": manifest,
                "originalSha256": manifest_hash,
                "desiredBytes": null,
                "desiredSha256": null,
            },
        ]
    })
}

#[test]
fn ai_refresh_profile_is_explicit_input_only_and_never_changes_the_global_default() {
    let (_temp, paths, repository) = fixture();
    fs::write(
        paths.config_path(),
        "default_permissions = \"user-default\"\n",
    )
    .unwrap();

    let manifest = install_codex_with_options(&paths, &repository, true).unwrap();
    let config = fs::read_to_string(paths.config_path()).unwrap();

    assert!(manifest.ai_refresh_enabled);
    assert_eq!(
        manifest.ai_refresh_profile_sha256.as_deref().unwrap().len(),
        64
    );
    assert!(config.contains("default_permissions = \"user-default\""));
    assert!(config.contains("[permissions.previously-input-only.filesystem]"));
    assert!(config.contains("\":root\" = \"deny\""));
    assert!(config.contains("\":minimal\" = \"read\""));
    assert!(config.contains("\":tmpdir\" = \"deny\""));
    assert!(config.contains("\":slash_tmp\" = \"deny\""));
    assert!(config.contains("[permissions.previously-input-only.network]"));
    assert!(config.contains("enabled = false"));

    let result = uninstall_codex_detailed(&paths).unwrap();
    assert!(!result.degraded);
    assert_eq!(
        fs::read_to_string(paths.config_path()).unwrap(),
        "default_permissions = \"user-default\"\n"
    );
}

#[test]
fn ai_refresh_profile_collision_is_a_strict_no_mutation_failure() {
    let (temp, paths, repository) = fixture();
    fs::write(
        paths.config_path(),
        "[permissions.previously-input-only.network]\nenabled = true\n",
    )
    .unwrap();
    let before = tree_snapshot(temp.path());

    let error = install_codex_with_options(&paths, &repository, true).unwrap_err();

    assert!(error
        .to_string()
        .contains("already exists and is not managed"));
    assert_eq!(tree_snapshot(temp.path()), before);
}

#[test]
fn uninstall_preserves_a_user_modified_ai_refresh_profile() {
    let (_temp, paths, repository) = fixture();
    install_codex_with_options(&paths, &repository, true).unwrap();
    let mut config = fs::read_to_string(paths.config_path()).unwrap();
    config = config.replace("enabled = false", "enabled = true");
    fs::write(paths.config_path(), &config).unwrap();
    let manifest = previously_on::setup::read_manifest(&paths.manifest_path()).unwrap();
    assert!(!previously_on::setup::ai_refresh_profile_matches(&paths, &manifest).unwrap());

    let result = uninstall_codex_detailed(&paths).unwrap();
    let after = fs::read_to_string(paths.config_path()).unwrap();

    assert!(result.degraded);
    assert!(after.contains("[permissions.previously-input-only.network]"));
    assert!(after.contains("enabled = true"));
    assert!(!after.contains("mcp_servers.previously_on"));
}

#[cfg(unix)]
#[tokio::test]
async fn ai_refresh_capability_is_ready_only_for_an_unchanged_allowed_profile() {
    use previously_on::ai_refresh::{inspect_capability_with_program, AiRefreshCapabilityStatusV1};
    use std::os::unix::fs::PermissionsExt;

    fn fake_server(root: &std::path::Path, name: &str, allowed: bool) -> std::path::PathBuf {
        let path = root.join(name);
        fs::write(
            &path,
            format!(
                r#"#!/bin/sh
IFS= read -r initialize
case "$initialize" in *'"experimentalApi":true'*) ;; *) exit 10 ;; esac
printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"userAgent":"codex-cli/test"}}}}'
IFS= read -r initialized
IFS= read -r profiles
case "$profiles" in *'"method":"permissionProfile/list"'*) ;; *) exit 11 ;; esac
printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"data":[{{"id":"previously-input-only","allowed":{allowed}}}],"nextCursor":null}}}}'
"#
            ),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    let (temp, paths, repository) = fixture();
    install_codex_with_options(&paths, &repository, true).unwrap();
    let allowed = fake_server(temp.path(), "allowed-app-server", true);
    let denied = fake_server(temp.path(), "denied-app-server", false);

    let ready = inspect_capability_with_program(&paths, &repository, &allowed)
        .await
        .unwrap();
    assert_eq!(ready.status, AiRefreshCapabilityStatusV1::Ready);
    let blocked = inspect_capability_with_program(&paths, &repository, &denied)
        .await
        .unwrap();
    assert_eq!(blocked.status, AiRefreshCapabilityStatusV1::Blocked);

    let config = fs::read_to_string(paths.config_path())
        .unwrap()
        .replace("enabled = false", "enabled = true");
    fs::write(paths.config_path(), config).unwrap();
    let changed = inspect_capability_with_program(
        &paths,
        &repository,
        &temp.path().join("must-not-be-started"),
    )
    .await
    .unwrap();
    assert_eq!(changed.status, AiRefreshCapabilityStatusV1::Blocked);
    assert!(changed.reason.unwrap().contains("changed or is missing"));
}

#[cfg(unix)]
#[tokio::test]
async fn ai_refresh_capability_reports_unsupported_app_server_without_enabling_refresh() {
    use previously_on::ai_refresh::{inspect_capability_with_program, AiRefreshCapabilityStatusV1};
    use std::os::unix::fs::PermissionsExt;

    let (temp, paths, repository) = fixture();
    install_codex_with_options(&paths, &repository, true).unwrap();
    let fake = temp.path().join("unsupported-app-server");
    fs::write(&fake, "#!/bin/sh\nexit 7\n").unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).unwrap();

    let report = inspect_capability_with_program(&paths, &repository, &fake)
        .await
        .unwrap();
    assert_eq!(report.status, AiRefreshCapabilityStatusV1::Unsupported);
}

fn fixture() -> (TempDir, SetupPaths, std::path::PathBuf) {
    let temp = TempDir::new().unwrap();
    let codex_home = temp.path().join("codex");
    let data_dir = temp.path().join("previously-on");
    let repository = temp.path().join("repo");
    fs::create_dir_all(repository.join(".git")).unwrap();
    fs::create_dir_all(&codex_home).unwrap();
    let paths = SetupPaths {
        codex_home,
        data_dir,
        executable: std::path::PathBuf::from("/Applications/PreviouslyOn/previously"),
    };
    (temp, paths, repository)
}

#[test]
fn setup_is_idempotent_and_preserves_existing_entries() {
    let (_temp, paths, repository) = fixture();
    fs::write(
        paths.hooks_path(),
        r#"{
          "hooks": {
            "UserPromptSubmit": [{"hooks":[{"type":"command","command":"user-hook"}]}],
            "PreToolUse": [{"hooks":[{"type":"command","command":"unrelated"}]}]
          },
          "userField": true
        }"#,
    )
    .unwrap();
    fs::write(
        paths.config_path(),
        r#"model = "gpt-test"

[mcp_servers.user_server]
command = "user-mcp"
"#,
    )
    .unwrap();

    let first = install_codex(&paths, &repository).unwrap();
    let second = install_codex(&paths, &repository).unwrap();
    assert_eq!(first.hooks_backup.sha256, second.hooks_backup.sha256);
    assert_eq!(first.config_backup.sha256, second.config_backup.sha256);

    let hooks: Value = serde_json::from_slice(&fs::read(paths.hooks_path()).unwrap()).unwrap();
    assert_eq!(hooks["userField"], true);
    assert!(hooks.to_string().contains("user-hook"));
    assert_eq!(hooks.to_string().matches(MANAGED_ID).count(), 6);
    assert!(hooks["hooks"]["PreToolUse"]
        .to_string()
        .contains(MANAGED_ID));
    assert_eq!(
        hooks["hooks"]["SessionStart"][0]["matcher"],
        "startup|resume|clear|compact"
    );

    let config = fs::read_to_string(paths.config_path()).unwrap();
    assert!(config.contains("user_server"));
    assert!(config.contains("user-mcp"));
    assert_eq!(config.matches("[mcp_servers.previously_on]").count(), 1);
    assert_eq!(config.matches(MANAGED_ID).count(), 1);
    assert!(config.contains("hooks = true"));
    let reserve = paths.data_dir.join("queue/disk-reserve.bin");
    assert_eq!(fs::metadata(reserve).unwrap().len(), 4 * 1024 * 1024);
}

#[test]
fn uninstall_removes_only_managed_entries_and_keeps_backups() {
    let (_temp, paths, repository) = fixture();
    fs::write(
        paths.hooks_path(),
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"keep-me"}]}]}}"#,
    )
    .unwrap();
    fs::write(
        paths.config_path(),
        "[mcp_servers.keep]\ncommand = \"keep-me\"\n",
    )
    .unwrap();
    let manifest = install_codex(&paths, &repository).unwrap();
    let hooks_backup = manifest.hooks_backup.backup_path.clone().unwrap();
    let config_backup = manifest.config_backup.backup_path.clone().unwrap();

    assert!(uninstall_codex(&paths).unwrap());
    assert!(!uninstall_codex(&paths).unwrap());

    let hooks = fs::read_to_string(paths.hooks_path()).unwrap();
    assert!(hooks.contains("keep-me"));
    assert!(!hooks.contains(MANAGED_ID));
    let config = fs::read_to_string(paths.config_path()).unwrap();
    assert!(config.contains("mcp_servers.keep"));
    assert!(!config.contains("previously_on"));
    assert!(!config.contains("hooks = true"));
    assert!(hooks_backup.exists());
    assert!(config_backup.exists());
}

#[test]
fn uninstall_restores_an_explicit_hooks_feature_value() {
    let (_temp, paths, repository) = fixture();
    fs::write(paths.config_path(), "[features]\nhooks = false\n").unwrap();
    install_codex(&paths, &repository).unwrap();
    assert!(fs::read_to_string(paths.config_path())
        .unwrap()
        .contains("hooks = true"));
    uninstall_codex(&paths).unwrap();
    assert!(fs::read_to_string(paths.config_path())
        .unwrap()
        .contains("hooks = false"));
}

#[test]
fn setup_refuses_to_overwrite_a_user_owned_mcp_with_the_same_name() {
    let (_temp, paths, repository) = fixture();
    fs::write(
        paths.config_path(),
        "[mcp_servers.previously_on]\ncommand = \"my-own-server\"\n",
    )
    .unwrap();
    let error = install_codex(&paths, &repository).unwrap_err();
    assert!(error.to_string().contains("not managed"));
    let config = fs::read_to_string(paths.config_path()).unwrap();
    assert!(config.contains("my-own-server"));
}

#[test]
fn uninstall_restores_original_bytes_when_managed_files_are_unchanged() {
    let (_temp, paths, repository) = fixture();
    let original_hooks = b"{\n  \"userField\": true,\n  \"hooks\": {}\n}\n";
    let original_config = b"# preserve exact whitespace\nmodel   =   \"gpt-test\"\n";
    fs::write(paths.hooks_path(), original_hooks).unwrap();
    fs::write(paths.config_path(), original_config).unwrap();

    install_codex(&paths, &repository).unwrap();
    let result = uninstall_codex_detailed(&paths).unwrap();

    assert!(result.removed);
    assert!(!result.degraded);
    assert_eq!(fs::read(paths.hooks_path()).unwrap(), original_hooks);
    assert_eq!(fs::read(paths.config_path()).unwrap(), original_config);
}

#[test]
fn uninstall_preserves_external_changes_and_reports_degraded_result() {
    let (_temp, paths, repository) = fixture();
    install_codex(&paths, &repository).unwrap();

    let mut hooks: Value = serde_json::from_slice(&fs::read(paths.hooks_path()).unwrap()).unwrap();
    hooks["externalAfterSetup"] = Value::Bool(true);
    fs::write(
        paths.hooks_path(),
        serde_json::to_vec_pretty(&hooks).unwrap(),
    )
    .unwrap();
    let mut config = fs::read_to_string(paths.config_path()).unwrap();
    config.push_str("\n[mcp_servers.added_later]\ncommand = \"keep-me\"\n");
    fs::write(paths.config_path(), config).unwrap();

    let result = uninstall_codex_detailed(&paths).unwrap();
    assert!(result.removed);
    assert!(result.degraded);
    assert_eq!(result.warnings.len(), 2);
    let hooks = fs::read_to_string(paths.hooks_path()).unwrap();
    assert!(hooks.contains("externalAfterSetup"));
    assert!(!hooks.contains(MANAGED_ID));
    let config = fs::read_to_string(paths.config_path()).unwrap();
    assert!(config.contains("added_later"));
    assert!(!config.contains("previously_on"));
}

#[test]
fn setup_finishes_an_interrupted_durable_journal_before_reinstalling() {
    let (_temp, paths, repository) = fixture();
    install_codex(&paths, &repository).unwrap();
    let hooks = fs::read(paths.hooks_path()).unwrap();
    let journal = valid_install_journal(&paths);
    let journal_path = paths.data_dir.join("setup-recovery-journal.json");
    write_private_file(&journal_path, serde_json::to_vec(&journal).unwrap());

    install_codex(&paths, &repository).unwrap();

    assert_eq!(fs::read(paths.hooks_path()).unwrap(), hooks);
    assert!(!journal_path.exists());
}

#[test]
fn setup_recovery_rejects_external_duplicate_and_missing_targets_without_mutation() {
    for case in ["external", "duplicate", "missing"] {
        let (temp, paths, repository) = fixture();
        install_codex(&paths, &repository).unwrap();
        let external = temp.path().join("must-not-change");
        fs::write(&external, b"outside-safe").unwrap();
        let mut journal = valid_install_journal(&paths);
        match case {
            "external" => journal["targets"][0]["path"] = json!(external),
            "duplicate" => journal["targets"][1]["path"] = json!(paths.hooks_path()),
            "missing" => {
                journal["targets"].as_array_mut().unwrap().pop();
            }
            _ => unreachable!(),
        }
        let journal_path = paths.data_dir.join("setup-recovery-journal.json");
        write_private_file(&journal_path, serde_json::to_vec(&journal).unwrap());
        let before = tree_snapshot(temp.path());

        let error = install_codex(&paths, &repository).unwrap_err();

        assert!(
            error.to_string().contains("exact allowlist"),
            "{case}: unexpected error: {error:#}"
        );
        assert_eq!(fs::read(&external).unwrap(), b"outside-safe");
        assert_eq!(tree_snapshot(temp.path()), before, "{case} mutated state");
    }
}

#[test]
fn uninstall_recovery_requires_one_exact_direct_child_archive_without_mutation() {
    for case in ["external", "duplicate", "missing", "bad_name"] {
        let (temp, paths, repository) = fixture();
        install_codex(&paths, &repository).unwrap();
        let external = temp.path().join("must-not-be-archive.json");
        fs::write(&external, b"outside-safe").unwrap();
        let mut journal = valid_uninstall_journal(&paths);
        match case {
            "external" => journal["targets"][2]["path"] = json!(external),
            "duplicate" => {
                let archive = journal["targets"][2].clone();
                journal["targets"].as_array_mut().unwrap().push(archive);
            }
            "missing" => {
                journal["targets"].as_array_mut().unwrap().remove(2);
            }
            "bad_name" => {
                journal["targets"][2]["path"] = json!(paths.data_dir.join("archive.json"));
            }
            _ => unreachable!(),
        }
        let journal_path = paths.data_dir.join("setup-recovery-journal.json");
        write_private_file(&journal_path, serde_json::to_vec(&journal).unwrap());
        let before = tree_snapshot(temp.path());

        let error = uninstall_codex_detailed(&paths).unwrap_err();

        assert!(
            error.to_string().contains("exact allowlist"),
            "{case}: unexpected error: {error:#}"
        );
        assert_eq!(fs::read(&external).unwrap(), b"outside-safe");
        assert_eq!(tree_snapshot(temp.path()), before, "{case} mutated state");
    }
}

#[cfg(unix)]
#[test]
fn setup_recovery_rejects_symlinked_or_overpermissive_journals_without_mutation() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    {
        let (temp, paths, repository) = fixture();
        install_codex(&paths, &repository).unwrap();
        let external_journal = temp.path().join("attacker-journal.json");
        write_private_file(
            &external_journal,
            serde_json::to_vec(&valid_install_journal(&paths)).unwrap(),
        );
        let journal_path = paths.data_dir.join("setup-recovery-journal.json");
        symlink(&external_journal, &journal_path).unwrap();
        let before = tree_snapshot(temp.path());

        let error = install_codex(&paths, &repository).unwrap_err();

        assert!(format!("{error:#}").contains("regular file"));
        assert_eq!(tree_snapshot(temp.path()), before);
    }

    {
        let (temp, paths, repository) = fixture();
        install_codex(&paths, &repository).unwrap();
        let journal_path = paths.data_dir.join("setup-recovery-journal.json");
        write_private_file(
            &journal_path,
            serde_json::to_vec(&valid_install_journal(&paths)).unwrap(),
        );
        fs::set_permissions(&journal_path, fs::Permissions::from_mode(0o644)).unwrap();
        let before = tree_snapshot(temp.path());

        let error = install_codex(&paths, &repository).unwrap_err();

        assert!(format!("{error:#}").contains("private 0600 boundary"));
        assert_eq!(tree_snapshot(temp.path()), before);
    }
}

#[cfg(unix)]
#[test]
fn setup_rejects_symlinked_or_overpermissive_data_and_manifest_boundaries() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    {
        let (temp, paths, repository) = fixture();
        fs::create_dir_all(&paths.data_dir).unwrap();
        fs::set_permissions(&paths.data_dir, fs::Permissions::from_mode(0o755)).unwrap();
        let before = tree_snapshot(temp.path());

        let error = install_codex(&paths, &repository).unwrap_err();

        assert!(error.to_string().contains("private 0700 boundary"));
        assert_eq!(tree_snapshot(temp.path()), before);
    }

    {
        let (temp, paths, repository) = fixture();
        let external_dir = temp.path().join("attacker-data");
        create_private_dir(&external_dir);
        symlink(&external_dir, &paths.data_dir).unwrap();
        let before = tree_snapshot(temp.path());

        let error = install_codex(&paths, &repository).unwrap_err();

        assert!(error.to_string().contains("regular directory"));
        assert_eq!(tree_snapshot(temp.path()), before);
    }

    {
        let (temp, paths, repository) = fixture();
        install_codex(&paths, &repository).unwrap();
        let external_manifest = temp.path().join("attacker-manifest.json");
        write_private_file(&external_manifest, fs::read(paths.manifest_path()).unwrap());
        fs::remove_file(paths.manifest_path()).unwrap();
        symlink(&external_manifest, paths.manifest_path()).unwrap();
        let before = tree_snapshot(temp.path());

        let error = install_codex(&paths, &repository).unwrap_err();

        assert!(format!("{error:#}").contains("regular file"));
        assert_eq!(tree_snapshot(temp.path()), before);
    }

    {
        let (temp, paths, repository) = fixture();
        install_codex(&paths, &repository).unwrap();
        fs::set_permissions(paths.manifest_path(), fs::Permissions::from_mode(0o644)).unwrap();
        let before = tree_snapshot(temp.path());

        let error = install_codex(&paths, &repository).unwrap_err();

        assert!(format!("{error:#}").contains("private 0600 boundary"));
        assert_eq!(tree_snapshot(temp.path()), before);
    }
}

#[test]
fn uninstall_rejects_foreign_manifest_and_backup_paths_without_mutation() {
    for case in ["hooks", "backup"] {
        let (temp, paths, repository) = fixture();
        fs::write(paths.hooks_path(), b"{\"hooks\":{}}\n").unwrap();
        fs::write(paths.config_path(), b"model = \"user\"\n").unwrap();
        install_codex(&paths, &repository).unwrap();
        let external = temp.path().join(format!("foreign-{case}"));
        fs::write(&external, b"outside-safe").unwrap();
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(paths.manifest_path()).unwrap()).unwrap();
        match case {
            "hooks" => manifest["hooksPath"] = json!(external),
            "backup" => manifest["hooksBackup"]["backupPath"] = json!(external),
            _ => unreachable!(),
        }
        write_private_file(
            &paths.manifest_path(),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        );
        let before = tree_snapshot(temp.path());

        let error = uninstall_codex_detailed(&paths).unwrap_err();

        let message = error.to_string();
        assert!(
            message.contains("different repository or Codex")
                || message.contains("invalid hooks.json backup path"),
            "{case}: unexpected error: {error:#}"
        );
        assert_eq!(fs::read(&external).unwrap(), b"outside-safe");
        assert_eq!(tree_snapshot(temp.path()), before, "{case} mutated state");
    }
}

#[cfg(unix)]
#[test]
fn uninstall_rejects_symlinked_or_overpermissive_backups_without_mutation() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    for case in ["symlink", "permissions"] {
        let (temp, paths, repository) = fixture();
        fs::write(paths.hooks_path(), b"{\"hooks\":{}}\n").unwrap();
        fs::write(paths.config_path(), b"model = \"user\"\n").unwrap();
        let manifest = install_codex(&paths, &repository).unwrap();
        let backup = manifest.hooks_backup.backup_path.unwrap();
        match case {
            "symlink" => {
                let external = temp.path().join("foreign-backup");
                write_private_file(&external, b"outside-safe");
                fs::remove_file(&backup).unwrap();
                symlink(&external, &backup).unwrap();
            }
            "permissions" => {
                fs::set_permissions(&backup, fs::Permissions::from_mode(0o644)).unwrap();
            }
            _ => unreachable!(),
        }
        let before = tree_snapshot(temp.path());

        let error = uninstall_codex_detailed(&paths).unwrap_err();

        let message = format!("{error:#}");
        assert!(
            message.contains("regular file") || message.contains("private 0600 boundary"),
            "{case}: unexpected error: {error:#}"
        );
        assert_eq!(tree_snapshot(temp.path()), before, "{case} mutated state");
    }
}

#[test]
fn setup_fails_closed_without_mutations_for_invalid_existing_manifests() {
    let valid_manifest = |version: u32, managed_id: &str| {
        serde_json::to_vec_pretty(&json!({
            "version": version,
            "managedId": managed_id,
            "installedAt": "2026-07-13T00:00:00Z",
            "repository": "/tmp/original-repository",
            "executable": "/tmp/previously",
            "hooksPath": "/tmp/hooks.json",
            "configPath": "/tmp/config.toml",
            "hooksBackup": {"existed": false, "backupPath": null, "sha256": null},
            "configBackup": {"existed": false, "backupPath": null, "sha256": null},
            "installedHooksSha256": "hooks-hash",
            "installedConfigSha256": "config-hash"
        }))
        .unwrap()
    };
    let cases = [
        ("malformed", b"{not-json".to_vec(), "parse setup manifest"),
        (
            "unsupported",
            valid_manifest(2, MANAGED_ID),
            "unsupported or foreign setup manifest",
        ),
        (
            "foreign",
            valid_manifest(1, "someone-else-v1"),
            "unsupported or foreign setup manifest",
        ),
    ];

    for (name, manifest_bytes, expected_error) in cases {
        let (temp, paths, repository) = fixture();
        create_private_dir(&paths.data_dir);
        fs::write(paths.hooks_path(), b"{\"userOwned\":true}\n").unwrap();
        fs::write(paths.config_path(), b"model = \"user-owned\"\n").unwrap();
        write_private_file(&paths.manifest_path(), manifest_bytes);
        let before = tree_snapshot(temp.path());

        let error = install_codex(&paths, &repository).unwrap_err();

        assert!(
            error.to_string().contains(expected_error),
            "{name}: unexpected error: {error:#}"
        );
        assert_eq!(
            tree_snapshot(temp.path()),
            before,
            "{name} mutated setup state"
        );
    }
}

#[test]
fn setup_rejects_a_manifest_that_claims_ownership_of_foreign_paths() {
    let (temp, paths, repository) = fixture();
    create_private_dir(&paths.data_dir);
    fs::write(paths.hooks_path(), b"{\"userOwned\":true}\n").unwrap();
    fs::write(paths.config_path(), b"model = \"user-owned\"\n").unwrap();
    let foreign_manifest = json!({
        "version": 1,
        "managedId": MANAGED_ID,
        "installedAt": "2026-07-13T00:00:00Z",
        "repository": repository.clone(),
        "executable": paths.executable.clone(),
        "hooksPath": temp.path().join("other-codex/hooks.json"),
        "configPath": paths.config_path(),
        "hooksBackup": {"existed": false, "backupPath": null, "sha256": null},
        "configBackup": {"existed": false, "backupPath": null, "sha256": null},
        "installedHooksSha256": "0".repeat(64),
        "installedConfigSha256": "0".repeat(64)
    });
    write_private_file(
        &paths.manifest_path(),
        serde_json::to_vec_pretty(&foreign_manifest).unwrap(),
    );
    let before = tree_snapshot(temp.path());

    let error = install_codex(&paths, &repository).unwrap_err();

    assert!(error.to_string().contains("different repository or Codex"));
    assert_eq!(tree_snapshot(temp.path()), before);
}
