use clap::Parser;
use previously_on::config::{
    Cli, Commands, ContractsTarget, ImportTarget, RunTarget, SetupTarget, UninstallTarget,
};

#[test]
fn parses_public_cli_surface() {
    let setup =
        Cli::try_parse_from(["previously", "setup", "codex", "--repo", "/tmp/repo"]).unwrap();
    assert!(matches!(
        setup.command,
        Commands::Setup {
            target: SetupTarget::Codex { .. }
        }
    ));
    let setup_ai = Cli::try_parse_from([
        "previously",
        "setup",
        "codex",
        "--repo",
        "/tmp/repo",
        "--enable-ai-refresh",
    ])
    .unwrap();
    assert!(matches!(
        setup_ai.command,
        Commands::Setup {
            target: SetupTarget::Codex {
                enable_ai_refresh: true,
                ..
            }
        }
    ));
    for args in [
        vec!["previously", "status"],
        vec!["previously", "doctor"],
        vec!["previously", "diagnostics", "--repo", "/tmp/repo"],
        vec!["previously", "ui"],
        vec!["previously", "export", "--format", "json"],
        vec!["previously", "purge", "--repo", "/tmp/repo"],
    ] {
        Cli::try_parse_from(args).unwrap();
    }
    assert!(Cli::try_parse_from(["previously", "ui", "--enable-ai-refresh"]).is_err());
    let uninstall = Cli::try_parse_from(["previously", "uninstall", "codex"]).unwrap();
    assert!(matches!(
        uninstall.command,
        Commands::Uninstall {
            target: UninstallTarget::Codex
        }
    ));
    let import =
        Cli::try_parse_from(["previously", "import", "codex", "--repo", "/tmp/repo"]).unwrap();
    assert!(matches!(
        import.command,
        Commands::Import {
            target: ImportTarget::Codex { .. }
        }
    ));
    let run = Cli::try_parse_from([
        "previously",
        "run",
        "codex",
        "--repo",
        "/tmp/repo",
        "--",
        "--model",
        "gpt-test",
    ])
    .unwrap();
    assert!(matches!(
        run.command,
        Commands::Run {
            target: RunTarget::Codex { codex_args, .. }
        } if codex_args == ["--model", "gpt-test"]
    ));
    let contracts = Cli::try_parse_from([
        "previously",
        "contracts",
        "check",
        "--base",
        "origin/main",
        "--execute",
        "--json",
    ])
    .unwrap();
    assert!(matches!(
        contracts.command,
        Commands::Contracts {
            target: ContractsTarget::Check {
                base,
                execute: true,
                json: true,
            }
        } if base == "origin/main"
    ));
    Cli::try_parse_from(["previously", "contracts", "init", "--github-actions"]).unwrap();
    Cli::try_parse_from(["previously", "contracts", "validate"]).unwrap();
}

#[test]
fn parses_managed_hidden_runtime_commands() {
    for args in [
        vec!["previously", "hook", "UserPromptSubmit"],
        vec!["previously", "hook", "PreToolUse"],
        vec!["previously", "daemon"],
        vec!["previously", "server"],
        vec!["previously", "mcp"],
        vec!["previously", "reconcile"],
    ] {
        Cli::try_parse_from(args).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn run_codex_preserves_exit_status_and_executes_in_repository() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command as StdCommand;

    use assert_cmd::cargo::cargo_bin_cmd;
    use tempfile::TempDir;

    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    fs::create_dir(&repo).unwrap();
    assert!(StdCommand::new("git")
        .args(["init", "--initial-branch=main"])
        .current_dir(&repo)
        .status()
        .unwrap()
        .success());
    let bin_dir = temp.path().join("bin");
    fs::create_dir(&bin_dir).unwrap();
    let fake_codex = bin_dir.join("codex");
    fs::write(
        &fake_codex,
        r#"#!/bin/sh
if [ "${1:-}" = "app-server" ]; then
  exit 9
fi
printf '%s\n' "$PWD" > "$FAKE_CODEX_RECORD"
printf '%s\n' "$@" >> "$FAKE_CODEX_RECORD"
exit 42
"#,
    )
    .unwrap();
    fs::set_permissions(&fake_codex, fs::Permissions::from_mode(0o700)).unwrap();
    let record = temp.path().join("record.txt");
    let path = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    cargo_bin_cmd!("previously")
        .args([
            "--data-dir",
            temp.path().join("data").to_str().unwrap(),
            "run",
            "codex",
            "--repo",
            repo.to_str().unwrap(),
            "--",
            "--model",
            "gpt-test",
        ])
        .env("PATH", path)
        .env("FAKE_CODEX_RECORD", &record)
        .assert()
        .code(42)
        .stderr(predicates::str::contains(
            "best-effort App Server import failed",
        ));

    let recorded = fs::read_to_string(record).unwrap();
    let mut lines = recorded.lines();
    let recorded_cwd = std::path::Path::new(lines.next().unwrap())
        .canonicalize()
        .unwrap();
    assert_eq!(recorded_cwd, repo.canonicalize().unwrap());
    assert_eq!(lines.collect::<Vec<_>>(), ["--model", "gpt-test"]);
}

#[test]
fn contracts_check_json_reports_plan_blocking_with_a_nonzero_exit() {
    use assert_cmd::cargo::cargo_bin_cmd;
    use predicates::prelude::PredicateBooleanExt;
    use predicates::str::contains;
    use tempfile::TempDir;

    let temp = TempDir::new().unwrap();
    assert!(std::process::Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(temp.path())
        .status()
        .unwrap()
        .success());
    for args in [
        ["config", "user.email", "cli@example.test"],
        ["config", "user.name", "CLI Contracts"],
    ] {
        assert!(std::process::Command::new("git")
            .args(args)
            .current_dir(temp.path())
            .status()
            .unwrap()
            .success());
    }
    std::fs::create_dir_all(temp.path().join("src")).unwrap();
    std::fs::write(temp.path().join("src/lib.rs"), "fn protected() {}\n").unwrap();
    assert!(std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(temp.path())
        .status()
        .unwrap()
        .success());
    assert!(std::process::Command::new("git")
        .args(["commit", "-qm", "base"])
        .current_dir(temp.path())
        .status()
        .unwrap()
        .success());
    let base = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let base = String::from_utf8(base.stdout).unwrap().trim().to_string();
    let id = uuid::Uuid::new_v4().to_string();
    let directory = temp.path().join(".previously-on/contracts");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join(format!("{id}.json")),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schemaVersion": 1,
            "id": id.clone(),
            "title": "Protect CLI",
            "invariant": "protected stays correct",
            "status": "active",
            "supersededBy": null,
            "impactSelectors": [{
                "path": {"kind": "exact", "value": "src/lib.rs"},
                "symbols": []
            }],
            "requiredTests": [{
                "id": "cli-test",
                "name": "CLI test",
                "program": "git",
                "args": ["status", "--porcelain"],
                "workingDirectory": ".",
                "timeoutSeconds": 60
            }],
            "origin": {
                "fixedAtCommit": base.clone(),
                "recordedAt": chrono::Utc::now(),
                "evidenceSha256": "b".repeat(64)
            }
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        temp.path().join("src/lib.rs"),
        "fn protected_changed() {}\n",
    )
    .unwrap();

    cargo_bin_cmd!("previously")
        .current_dir(temp.path())
        .args(["contracts", "check", "--base", &base, "--json"])
        .assert()
        .failure()
        .stdout(
            contains("\"readiness\": \"contract_blocked\"").and(contains("\"program\": \"git\"")),
        );

    cargo_bin_cmd!("previously")
        .current_dir(temp.path())
        .args(["contracts", "check", "--base", &base, "--execute", "--json"])
        .assert()
        .success()
        .stdout(contains("\"readiness\": \"ready\"").and(contains("\"state\": \"passed\"")));

    let contract_path = directory.join(format!("{id}.json"));
    let mut invalid: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&contract_path).unwrap()).unwrap();
    invalid["schemaVersion"] = serde_json::json!(2);
    std::fs::write(&contract_path, serde_json::to_vec_pretty(&invalid).unwrap()).unwrap();
    cargo_bin_cmd!("previously")
        .current_dir(temp.path())
        .args(["contracts", "check", "--base", &base, "--json"])
        .assert()
        .failure()
        .stdout(
            contains("\"readiness\": \"contract_blocked\"")
                .and(contains("unsupported schemaVersion 2")),
        );

    let raw_secret = "cli-secret-value-must-not-escape";
    invalid["schemaVersion"] = serde_json::json!(1);
    invalid["requiredTests"][0]["args"] = serde_json::json!(["--api-key", raw_secret]);
    std::fs::write(&contract_path, serde_json::to_vec_pretty(&invalid).unwrap()).unwrap();
    cargo_bin_cmd!("previously")
        .current_dir(temp.path())
        .args(["contracts", "validate"])
        .assert()
        .failure()
        .stderr(contains("contains sensitive data").and(contains(raw_secret).not()));
    cargo_bin_cmd!("previously")
        .current_dir(temp.path())
        .args(["contracts", "check", "--base", &base, "--json"])
        .assert()
        .failure()
        .stdout(contains("\"readiness\": \"contract_blocked\"").and(contains(raw_secret).not()));
}
