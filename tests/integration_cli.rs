use clap::Parser;
use previously_on::config::{Cli, Commands, ImportTarget, RunTarget, SetupTarget, UninstallTarget};

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
    for args in [
        vec!["previously", "status"],
        vec!["previously", "doctor"],
        vec!["previously", "ui"],
        vec!["previously", "export", "--format", "json"],
        vec!["previously", "purge", "--repo", "/tmp/repo"],
    ] {
        Cli::try_parse_from(args).unwrap();
    }
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
