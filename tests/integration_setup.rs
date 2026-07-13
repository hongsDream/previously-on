use std::fs;

use previously_on::setup::{
    install_codex, uninstall_codex, uninstall_codex_detailed, SetupPaths, MANAGED_ID,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

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
            if child.is_dir() {
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
    fs::create_dir_all(&paths.data_dir).unwrap();
    let marker = paths.data_dir.join("recovered-marker");
    let desired = b"durably recovered".to_vec();
    let journal = json!({
        "version": 1,
        "managedId": MANAGED_ID,
        "operation": "install",
        "targets": [{
            "path": marker,
            "originalExisted": false,
            "originalBytes": [],
            "originalSha256": null,
            "desiredBytes": desired,
            "desiredSha256": hex::encode(Sha256::digest(b"durably recovered")),
        }]
    });
    let journal_path = paths.data_dir.join("setup-recovery-journal.json");
    fs::write(&journal_path, serde_json::to_vec(&journal).unwrap()).unwrap();

    install_codex(&paths, &repository).unwrap();

    assert_eq!(
        fs::read(paths.data_dir.join("recovered-marker")).unwrap(),
        b"durably recovered"
    );
    assert!(!journal_path.exists());
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
        fs::create_dir_all(&paths.data_dir).unwrap();
        fs::write(paths.hooks_path(), b"{\"userOwned\":true}\n").unwrap();
        fs::write(paths.config_path(), b"model = \"user-owned\"\n").unwrap();
        fs::write(paths.manifest_path(), manifest_bytes).unwrap();
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
    fs::create_dir_all(&paths.data_dir).unwrap();
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
    fs::write(
        paths.manifest_path(),
        serde_json::to_vec_pretty(&foreign_manifest).unwrap(),
    )
    .unwrap();
    let before = tree_snapshot(temp.path());

    let error = install_codex(&paths, &repository).unwrap_err();

    assert!(error.to_string().contains("different repository or Codex"));
    assert_eq!(tree_snapshot(temp.path()), before);
}
