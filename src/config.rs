use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, ExitCode, Stdio};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use directories::BaseDirs;
use serde::Serialize;
use serde_json::json;

use crate::app_server::{codex_version, inspect_capabilities, AppServerCapabilityStatus};
use crate::domain::EventEnvelopeV1;
use crate::hook::{HookEvent, HookIngressConfig};
use crate::setup::{self, SetupPaths, MANAGED_ID};
use crate::store::Store;

#[derive(Debug, Parser)]
#[command(
    name = "previously",
    version,
    about = "Verifiable local Codex work handoffs"
)]
pub struct Cli {
    /// Override ~/.previously-on (primarily for testing and portable installations).
    #[arg(long, env = "PREVIOUSLY_ON_DATA_DIR", global = true)]
    pub data_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Install a PreviouslyOn integration.
    Setup {
        #[command(subcommand)]
        target: SetupTarget,
    },
    /// Show repository, queue, and database state.
    Status,
    /// Check local prerequisites and installation integrity.
    Doctor,
    /// Print a privacy-bounded, read-only pilot report as schemaVersion 1 JSON.
    Diagnostics {
        #[arg(long)]
        repo: PathBuf,
    },
    /// Open the local review interface.
    Ui {
        #[arg(long, default_value = "127.0.0.1:43129")]
        bind: SocketAddr,
        #[arg(long)]
        no_open: bool,
    },
    /// Export all local project lineage data as JSON.
    Export {
        #[arg(long, value_enum, default_value_t = ExportFormat::Json)]
        format: ExportFormat,
    },
    /// Permanently purge one repository from local storage.
    Purge {
        #[arg(long)]
        repo: PathBuf,
    },
    /// Stop capture for one logical repository while preserving its local history.
    Unregister {
        #[arg(long)]
        repo: PathBuf,
    },
    /// Remove a PreviouslyOn integration without touching user-owned config.
    Uninstall {
        #[command(subcommand)]
        target: UninstallTarget,
    },
    /// Explicitly import Codex data through a supported integration.
    Import {
        #[command(subcommand)]
        target: ImportTarget,
    },
    /// Run a tool under explicit PreviouslyOn capture and repair its import afterward.
    Run {
        #[command(subcommand)]
        target: RunTarget,
    },
    /// Manage Git-backed Regression Contracts for this repository.
    Contracts {
        #[command(subcommand)]
        target: ContractsTarget,
    },
    /// Rebuild SQLite projections from the canonical event log.
    #[command(hide = true)]
    Reconcile,
    /// Codex hook ingress. This command is managed by `previously setup codex`.
    #[command(hide = true)]
    Hook { event: String },
    /// Run the local Unix-socket ingestion daemon.
    #[command(hide = true, alias = "server")]
    Daemon,
    /// Run the MCP server over stdio. Its single local write requires Codex approval.
    #[command(hide = true)]
    Mcp,
    /// Create a verified fresh-task continuation. Invoked only after MCP user approval.
    #[command(hide = true)]
    AutoRollover,
}

#[derive(Debug, Subcommand)]
pub enum SetupTarget {
    Codex {
        #[arg(long)]
        repo: PathBuf,
        /// Explicitly install the beta input-only permission profile for user-triggered fact refresh.
        #[arg(long)]
        enable_ai_refresh: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum UninstallTarget {
    Codex,
}

#[derive(Debug, Subcommand)]
pub enum ImportTarget {
    Codex {
        #[arg(long)]
        repo: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
pub enum RunTarget {
    Codex {
        #[arg(long)]
        repo: PathBuf,
        /// Arguments passed verbatim to Codex after `--`.
        #[arg(last = true)]
        codex_args: Vec<OsString>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ContractsTarget {
    /// Create the Contract directory and optional pinned GitHub Actions gate.
    Init {
        #[arg(long)]
        github_actions: bool,
    },
    /// Validate every Contract in the current checkout.
    Validate,
    /// Select relevant Contracts and optionally execute their required tests.
    Check {
        #[arg(long)]
        base: String,
        #[arg(long)]
        execute: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ExportFormat {
    Json,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub data_dir: PathBuf,
    pub database_path: PathBuf,
    pub socket_path: PathBuf,
    pub queue_path: PathBuf,
    pub codex_home: PathBuf,
    pub executable: PathBuf,
}

impl AppConfig {
    pub fn resolve(data_dir: Option<PathBuf>) -> Result<Self> {
        let base = BaseDirs::new().context("home directory is unavailable")?;
        let data_dir = data_dir.unwrap_or_else(|| base.home_dir().join(".previously-on"));
        let codex_home = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| base.home_dir().join(".codex"));
        let executable = std::env::current_exe().context("resolve PreviouslyOn executable path")?;
        Ok(Self {
            database_path: data_dir.join("previously.sqlite3"),
            socket_path: data_dir.join("previously.sock"),
            queue_path: data_dir.join("queue/events.jsonl"),
            data_dir,
            codex_home,
            executable,
        })
    }

    pub fn setup_paths(&self) -> SetupPaths {
        SetupPaths {
            codex_home: self.codex_home.clone(),
            data_dir: self.data_dir.clone(),
            executable: self.executable.clone(),
        }
    }

    pub fn hook_config(&self) -> HookIngressConfig {
        HookIngressConfig {
            socket_path: self.socket_path.clone(),
            queue_path: self.queue_path.clone(),
            registered_repositories: setup::read_manifest(&self.setup_paths().manifest_path())
                .ok()
                .map(|manifest| {
                    manifest
                        .projects
                        .into_iter()
                        .flat_map(|project| project.known_worktree_roots)
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    pub fn codex_import_service(&self) -> crate::codex_import::CodexImportService {
        crate::codex_import::CodexImportService::new(
            &self.database_path,
            self.setup_paths().manifest_path(),
        )
    }
}

pub async fn run(cli: Cli) -> Result<ExitCode> {
    let config = AppConfig::resolve(cli.data_dir)?;
    match cli.command {
        Commands::Setup {
            target:
                SetupTarget::Codex {
                    repo,
                    enable_ai_refresh,
                },
        } => {
            let manifest =
                setup::install_codex_with_options(&config.setup_paths(), &repo, enable_ai_refresh)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
        }
        Commands::Status => {
            let store = Store::open(&config.database_path)?;
            let manifest = setup::read_manifest(&config.setup_paths().manifest_path()).ok();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "dataDir": config.data_dir,
                    "database": store.health()?,
                    "projects": manifest.as_ref().map(|manifest| &manifest.projects),
                    "repository": manifest.as_ref().and_then(|manifest| {
                        (manifest.projects.len() == 1)
                            .then(|| &manifest.projects[0].primary_root)
                    }),
                    "queuedEvents": queue_line_count(&config.queue_path)?
                }))?
            );
        }
        Commands::Doctor => {
            let report = doctor(&config).await;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if !report.healthy {
                bail!("one or more doctor checks failed");
            }
        }
        Commands::Diagnostics { repo } => {
            let report = diagnostics(&config, &repo).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Commands::Ui { bind, no_open } => {
            crate::server::serve_ui(config.data_dir, bind, !no_open).await?;
        }
        Commands::Export { format } => match format {
            ExportFormat::Json => {
                let store = Store::open(&config.database_path)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&store.export_json(None)?)?
                );
            }
        },
        Commands::Purge { repo } => {
            let repository = crate::git::repository_identity(&repo)?;
            let repository_id = repository.id;
            let _ = crate::hook::stop_daemon(&config.socket_path);
            let store = Store::open(&config.database_path)?;
            store.purge_repository_with(&repository_id, || {
                for queue in [
                    config.queue_path.clone(),
                    config.queue_path.with_extension("replay.jsonl"),
                    config.queue_path.with_extension("corrupt.jsonl"),
                ] {
                    purge_queue(&queue, &repository_id)?;
                }
                Ok(())
            })?;
            println!("purged {}", repository.root.display());
        }
        Commands::Unregister { repo } => {
            let result = setup::unregister_repository(&config.setup_paths(), &repo)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Commands::Uninstall {
            target: UninstallTarget::Codex,
        } => {
            let _ = crate::hook::stop_daemon(&config.socket_path);
            let result = setup::uninstall_codex_detailed(&config.setup_paths())?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Commands::Import {
            target: ImportTarget::Codex { repo },
        } => import_codex_threads(&config, &repo).await?,
        Commands::Run {
            target: RunTarget::Codex { repo, codex_args },
        } => return run_codex(&config, &repo, &codex_args).await,
        Commands::Contracts { target } => {
            let repository = std::env::current_dir().context("resolve current repository")?;
            match target {
                ContractsTarget::Init { github_actions } => {
                    let report = crate::contracts::init_contracts(&repository, github_actions)?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                }
                ContractsTarget::Validate => {
                    let report = crate::contracts::validate_contracts(&repository)?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                }
                ContractsTarget::Check {
                    base,
                    execute,
                    json,
                } => {
                    return run_contract_check(&repository, &base, execute, json).await;
                }
            }
        }
        Commands::Reconcile => {
            let store = Store::open(&config.database_path)?;
            store.rebuild_projections()?;
            println!("projections rebuilt");
        }
        Commands::Hook { event } => {
            let event = event.parse::<HookEvent>()?;
            crate::hook::run_hook(
                event,
                &config.hook_config(),
                &mut std::io::stdin().lock(),
                &mut std::io::stdout().lock(),
            )?;
        }
        Commands::Daemon => crate::hook::run_daemon(config.data_dir).await?,
        Commands::Mcp => {
            let manifest = setup::read_manifest(&config.setup_paths().manifest_path())?;
            let backend = crate::mcp::StoreMcpBackend::open_registry(
                &config.database_path,
                &manifest.projects,
            )?;
            crate::mcp::run_stdio(&backend, tokio::io::stdin(), tokio::io::stdout()).await?;
        }
        Commands::AutoRollover => {
            let mut input = Vec::new();
            std::io::stdin()
                .take(crate::hook::MAX_HOOK_PAYLOAD_BYTES as u64 + 1)
                .read_to_end(&mut input)?;
            if input.len() > crate::hook::MAX_HOOK_PAYLOAD_BYTES {
                bail!("automatic rollover request exceeds the input limit");
            }
            let request =
                serde_json::from_slice::<crate::continuation::AutomaticRolloverRequestV1>(&input)
                    .context("parse automatic rollover request")?;
            let result =
                crate::continuation::execute_automatic_rollover(&config.database_path, request)
                    .await?;
            println!("{}", serde_json::to_string(&result)?);
        }
    }
    Ok(ExitCode::SUCCESS)
}

async fn diagnostics(
    config: &AppConfig,
    repo: &Path,
) -> Result<crate::diagnostics::PilotDiagnosticsV1> {
    let requested = crate::git::repository_identity(repo)?;
    let manifest = setup::read_manifest(&config.setup_paths().manifest_path())?;
    let project = manifest
        .projects
        .iter()
        .find(|project| project.repository_id == requested.id)
        .context("diagnostics repository is not registered")?;
    let setup_at = DateTime::parse_from_rfc3339(&project.registered_at)
        .context("parse project registration timestamp")?
        .with_timezone(&Utc);
    let store = Store::open_read_only(&config.database_path)?;
    let sessions = store.list_sessions(Some(&requested.id))?;
    let tasks = store.list_tasks(Some(&requested.id))?;
    let mut checkpoints = Vec::new();
    for task in tasks {
        checkpoints.extend(store.list_checkpoints(&task.id)?);
    }
    let events = store.list_events(Some(&requested.id))?;
    let contract_evaluations = store.list_contract_evaluations(Some(&requested.id))?;
    let codex_version = codex_version().await.ok();
    Ok(crate::diagnostics::build_diagnostics(
        crate::diagnostics::DiagnosticsInputV1 {
            app_version: crate::APP_VERSION.to_string(),
            codex_version,
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            setup_at,
            sessions,
            checkpoints,
            events,
            contract_evaluations,
        },
    ))
}

async fn run_contract_check(
    repository: &Path,
    base: &str,
    execute: bool,
    json_output: bool,
) -> Result<ExitCode> {
    let evaluation = match crate::contracts::check_contracts(repository, base, execute).await {
        Ok(evaluation) => evaluation,
        Err(error) if json_output => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "schemaVersion": crate::domain::SCHEMA_VERSION_V1,
                    "readiness": "contract_blocked",
                    "error": crate::redaction::redact_excerpt(&format!("{error:#}"))
                }))?
            );
            return Ok(ExitCode::FAILURE);
        }
        Err(error) => return Err(error),
    };
    if json_output {
        println!("{}", serde_json::to_string_pretty(&evaluation)?);
    } else if evaluation.relevant_contracts.is_empty() {
        println!("No relevant Regression Contracts.");
    } else {
        println!("Regression Contract readiness: {:?}", evaluation.readiness);
        for contract in &evaluation.relevant_contracts {
            println!("- {}: {}", contract.id, contract.invariant);
        }
        for test in &evaluation.required_tests {
            let args = test
                .args
                .iter()
                .map(|arg| format!(" {:?}", arg))
                .collect::<String>();
            println!(
                "  {:?}: (cd {:?} && {:?}{})",
                test.state, test.working_directory, test.program, args
            );
        }
        for warning in &evaluation.warnings {
            eprintln!("warning: {warning}");
        }
    }
    Ok(
        if evaluation.readiness == crate::contracts::ContractReadinessV1::Ready {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        },
    )
}

async fn run_codex(config: &AppConfig, repo: &Path, codex_args: &[OsString]) -> Result<ExitCode> {
    let repository = crate::git::repository_identity(repo)?;
    let status = tokio::process::Command::new("codex")
        .args(codex_args)
        .current_dir(&repository.root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .context("start Codex through `previously run codex`")?;

    let replay_result = Store::open(&config.database_path)
        .and_then(|store| crate::hook::replay_fallback(&store, &config.queue_path));
    if let Err(error) = replay_result {
        eprintln!(
            "PreviouslyOn warning: Codex exited, but replaying the redacted fallback queue failed: {error:#}"
        );
    }
    if let Err(error) = import_codex_threads(config, &repository.root).await {
        eprintln!(
            "PreviouslyOn warning: Codex exited, but the best-effort App Server import failed: {error:#}"
        );
    }

    Ok(status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .map(ExitCode::from)
        .unwrap_or(ExitCode::FAILURE))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DoctorReport {
    pub(crate) healthy: bool,
    pub(crate) checks: Vec<DoctorCheck>,
}

#[derive(Debug, Serialize)]
pub(crate) struct DoctorCheck {
    pub(crate) name: &'static str,
    pub(crate) ok: bool,
    pub(crate) detail: String,
}

async fn doctor(config: &AppConfig) -> DoctorReport {
    let mut checks = Vec::new();
    let git = StdCommand::new("git").arg("--version").output();
    checks.push(command_check("git", git));

    match codex_version().await {
        Ok(version) => checks.push(DoctorCheck {
            name: "codex",
            ok: true,
            detail: version,
        }),
        Err(error) => checks.push(DoctorCheck {
            name: "codex",
            ok: false,
            detail: error.to_string(),
        }),
    }

    let app_server = inspect_capabilities().await;
    let warnings = if app_server.warnings.is_empty() {
        String::new()
    } else {
        format!("; {}", app_server.warnings.join("; "))
    };
    let capability_status = |status| match status {
        AppServerCapabilityStatus::Complete => "complete",
        AppServerCapabilityStatus::Degraded => "degraded",
        AppServerCapabilityStatus::Unsupported => "unsupported",
    };
    checks.push(DoctorCheck {
        name: "Codex core import",
        ok: app_server.capabilities.core_import == AppServerCapabilityStatus::Complete,
        detail: format!(
            "{}{warnings}",
            capability_status(app_server.capabilities.core_import)
        ),
    });
    checks.push(DoctorCheck {
        name: "Codex continuation",
        ok: app_server.capabilities.continuation == AppServerCapabilityStatus::Complete,
        detail: format!(
            "{}{warnings}",
            capability_status(app_server.capabilities.continuation)
        ),
    });
    checks.push(DoctorCheck {
        name: "Codex experimental refresh",
        ok: true,
        detail: format!(
            "{} (optional){warnings}",
            capability_status(app_server.capabilities.experimental_refresh)
        ),
    });

    let setup = setup::read_manifest(&config.setup_paths().manifest_path());
    checks.push(match setup {
        Ok(manifest) => DoctorCheck {
            name: "repository registration",
            ok: !manifest.projects.is_empty()
                && manifest.projects.iter().all(|project| {
                    project
                        .known_worktree_roots
                        .iter()
                        .any(|root| root.exists())
                }),
            detail: manifest
                .projects
                .iter()
                .map(|project| project.primary_root.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        },
        Err(error) => DoctorCheck {
            name: "repository registration",
            ok: false,
            detail: error.to_string(),
        },
    });

    let hooks = fs::read_to_string(config.setup_paths().hooks_path());
    checks.push(content_check("Codex hooks", hooks, MANAGED_ID));
    let codex_config = fs::read_to_string(config.setup_paths().config_path());
    checks.push(content_check("Codex MCP", codex_config, MANAGED_ID));
    checks.push(permission_check(
        "data permissions",
        &config.data_dir,
        0o700,
    ));
    checks.push(
        match Store::open(&config.database_path).and_then(|store| store.health()) {
            Ok(health) => DoctorCheck {
                name: "database",
                ok: health.integrity_check == "ok",
                detail: serde_json::to_string(&health).unwrap_or_else(|_| "healthy".to_string()),
            },
            Err(error) => DoctorCheck {
                name: "database",
                ok: false,
                detail: error.to_string(),
            },
        },
    );
    checks.push(permission_check(
        "database permissions",
        &config.database_path,
        0o600,
    ));
    checks.push(permission_check(
        "setup manifest permissions",
        &config.setup_paths().manifest_path(),
        0o600,
    ));
    if config.queue_path.exists() {
        checks.push(permission_check(
            "queue permissions",
            &config.queue_path,
            0o600,
        ));
    }
    if let Some(base) = BaseDirs::new() {
        let legacy = base.home_dir().join(".lineage");
        if legacy.exists() {
            checks.push(DoctorCheck {
                name: "legacy data directory",
                ok: true,
                detail: format!(
                    "{} exists but is intentionally ignored; PreviouslyOn does not migrate or delete unreleased Context Lineage data",
                    legacy.display()
                ),
            });
        }
    }
    DoctorReport {
        healthy: checks.iter().all(|check| check.ok),
        checks,
    }
}

pub(crate) async fn doctor_for_setup_paths(paths: &SetupPaths) -> DoctorReport {
    let config = AppConfig {
        database_path: paths.data_dir.join("previously.sqlite3"),
        socket_path: paths.data_dir.join("previously.sock"),
        queue_path: paths.data_dir.join("queue/events.jsonl"),
        data_dir: paths.data_dir.clone(),
        codex_home: paths.codex_home.clone(),
        executable: paths.executable.clone(),
    };
    doctor(&config).await
}

async fn import_codex_threads(config: &AppConfig, repo: &Path) -> Result<()> {
    let repository = crate::git::repository_identity(repo)?;
    let report = config
        .codex_import_service()
        .sync_repository(&repository.id)
        .await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn queue_line_count(path: &Path) -> Result<usize> {
    match crate::store::read_private_file(path, "fallback queue")? {
        Some(bytes) => {
            let contents = String::from_utf8(bytes).context("fallback queue is not UTF-8")?;
            Ok(contents
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count())
        }
        None => Ok(0),
    }
}

pub(crate) fn purge_queue(path: &Path, repository_id: &str) -> Result<()> {
    let parent = path.parent().context("queue path has no parent")?;
    match fs::symlink_metadata(parent) {
        Ok(_) => crate::store::validate_private_directory(parent, "fallback queue directory")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    if !crate::store::validate_private_regular_file(path, "fallback queue")? {
        return Ok(());
    }
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.contains("corrupt"))
    {
        match fs::remove_file(path) {
            Ok(()) => {
                if let Some(parent) = path.parent() {
                    fs::File::open(parent)?.sync_all()?;
                }
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        }
    }
    let contents = match crate::store::read_private_file(path, "fallback queue")? {
        Some(bytes) => String::from_utf8(bytes).context("fallback queue is not UTF-8")?,
        None => return Ok(()),
    };
    let mut kept_lines = Vec::new();
    for (index, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event: EventEnvelopeV1 = serde_json::from_str(line).with_context(|| {
            format!(
                "cannot prove repository deletion because {} contains malformed record {}",
                path.display(),
                index + 1
            )
        })?;
        if event.repository_id != repository_id {
            kept_lines.push(line);
        }
    }
    let kept = kept_lines.join("\n");
    let replacement = if kept.is_empty() {
        kept
    } else {
        format!("{kept}\n")
    };
    let temporary = path.with_extension(format!("purge-{}.tmp", uuid::Uuid::now_v7()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    let mut file =
        crate::store::open_private_file(&temporary, "temporary fallback queue", &mut options)?;
    file.write_all(replacement.as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)?;
    if let Some(parent) = path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn command_check(name: &'static str, result: std::io::Result<std::process::Output>) -> DoctorCheck {
    match result {
        Ok(output) => DoctorCheck {
            name,
            ok: output.status.success(),
            detail: String::from_utf8_lossy(&output.stdout).trim().to_string(),
        },
        Err(error) => DoctorCheck {
            name,
            ok: false,
            detail: error.to_string(),
        },
    }
}

fn content_check(name: &'static str, result: std::io::Result<String>, needle: &str) -> DoctorCheck {
    match result {
        Ok(content) => DoctorCheck {
            name,
            ok: content.contains(needle),
            detail: if content.contains(needle) {
                "managed entry present".to_string()
            } else {
                "managed entry missing".to_string()
            },
        },
        Err(error) => DoctorCheck {
            name,
            ok: false,
            detail: error.to_string(),
        },
    }
}

#[cfg(unix)]
fn permission_check(name: &'static str, path: &Path, expected: u32) -> DoctorCheck {
    use std::os::unix::fs::PermissionsExt;
    match fs::metadata(path) {
        Ok(metadata) => {
            let mode = metadata.permissions().mode() & 0o777;
            DoctorCheck {
                name,
                ok: mode == expected,
                detail: format!("{:o}", mode),
            }
        }
        Err(error) => DoctorCheck {
            name,
            ok: false,
            detail: error.to_string(),
        },
    }
}

#[cfg(not(unix))]
fn permission_check(name: &'static str, _path: &Path, _expected: u32) -> DoctorCheck {
    DoctorCheck {
        name,
        ok: false,
        detail: "unsupported platform".to_string(),
    }
}
