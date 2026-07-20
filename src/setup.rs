use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use toml_edit::{value, Array, DocumentMut, Item, Table};

pub const MANAGED_ID: &str = "previously-on-v1";
const MANIFEST_VERSION: u32 = 1;
const JOURNAL_VERSION: u32 = 1;
const HOOK_EVENTS: [&str; 6] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PreCompact",
    "Stop",
];

#[derive(Debug, Clone)]
pub struct SetupPaths {
    pub codex_home: PathBuf,
    pub data_dir: PathBuf,
    pub executable: PathBuf,
}

impl SetupPaths {
    pub fn hooks_path(&self) -> PathBuf {
        self.codex_home.join("hooks.json")
    }

    pub fn config_path(&self) -> PathBuf {
        self.codex_home.join("config.toml")
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.data_dir.join("setup-manifest.json")
    }

    fn journal_path(&self) -> PathBuf {
        self.data_dir.join("setup-recovery-journal.json")
    }

    fn backup_dir(&self) -> PathBuf {
        self.data_dir.join("setup-backups")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupManifestV1 {
    pub version: u32,
    pub managed_id: String,
    pub installed_at: String,
    pub repository: PathBuf,
    pub executable: PathBuf,
    pub hooks_path: PathBuf,
    pub config_path: PathBuf,
    pub hooks_backup: BackupRecord,
    pub config_backup: BackupRecord,
    pub installed_hooks_sha256: String,
    pub installed_config_sha256: String,
    #[serde(default)]
    pub hooks_feature_before: Option<bool>,
    #[serde(default)]
    pub hooks_feature_managed: bool,
    #[serde(default)]
    pub ai_refresh_enabled: bool,
    #[serde(default)]
    pub ai_refresh_profile_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupRecord {
    pub existed: bool,
    pub backup_path: Option<PathBuf>,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SetupJournalOperation {
    Install,
    Uninstall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetupJournalV1 {
    version: u32,
    managed_id: String,
    operation: SetupJournalOperation,
    targets: Vec<SetupJournalTargetV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetupJournalTargetV1 {
    path: PathBuf,
    original_existed: bool,
    original_bytes: Vec<u8>,
    original_sha256: Option<String>,
    desired_bytes: Option<Vec<u8>>,
    desired_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UninstallResult {
    pub removed: bool,
    pub degraded: bool,
    pub warnings: Vec<String>,
}

pub fn install_codex(paths: &SetupPaths, repository: &Path) -> Result<SetupManifestV1> {
    install_codex_with_options(paths, repository, false)
}

pub fn install_codex_with_options(
    paths: &SetupPaths,
    repository: &Path,
    enable_ai_refresh: bool,
) -> Result<SetupManifestV1> {
    let repository = repository
        .canonicalize()
        .with_context(|| format!("repository does not exist: {}", repository.display()))?;
    if !repository.join(".git").exists() {
        // Worktrees commonly use a `.git` file, so existence is sufficient.
        bail!(
            "repository is not a Git work tree: {}",
            repository.display()
        );
    }

    let data_dir_exists =
        validate_optional_trusted_private_directory(&paths.data_dir, "setup data directory")?;
    if data_dir_exists {
        let _ = recover_setup_journal(paths)?;
    }

    // Validate an existing ownership record before creating directories, backups, reserve files,
    // or replaying a recovery journal. A malformed/unsupported/foreign manifest must be a strict
    // no-op rather than being silently treated as a first install.
    let validated_manifest = read_optional_manifest(&paths.manifest_path())?;
    if let Some(manifest) = validated_manifest.as_ref() {
        validate_manifest_for_install(manifest, paths, &repository)?;
    }
    let initial_config = read_optional_string(&paths.config_path())?.unwrap_or_default();
    let effective_ai_refresh = enable_ai_refresh
        || validated_manifest
            .as_ref()
            .is_some_and(|manifest| manifest.ai_refresh_enabled);
    validate_ai_profile_collision(
        &initial_config,
        effective_ai_refresh,
        validated_manifest.as_ref(),
    )?;
    let repository_identity = crate::git::repository_identity(&repository).ok();

    ensure_trusted_data_dir(&paths.data_dir)?;
    fs::create_dir_all(&paths.codex_home)
        .with_context(|| format!("create Codex home directory {}", paths.codex_home.display()))?;
    ensure_trusted_private_directory(&paths.backup_dir(), "setup backup directory")?;
    crate::hook::ensure_reserve_file(&paths.data_dir.join("queue/events.jsonl"))?;
    let existing_manifest = read_optional_manifest(&paths.manifest_path())?;
    if let Some(manifest) = existing_manifest.as_ref() {
        validate_manifest_for_install(manifest, paths, &repository)?;
    }

    let hooks_backup = existing_manifest
        .as_ref()
        .map(|m| m.hooks_backup.clone())
        .unwrap_or(backup_file(
            &paths.hooks_path(),
            &paths.backup_dir(),
            "hooks.json",
        )?);
    let config_backup = existing_manifest
        .as_ref()
        .map(|m| m.config_backup.clone())
        .unwrap_or(backup_file(
            &paths.config_path(),
            &paths.backup_dir(),
            "config.toml",
        )?);

    let hooks = merge_hooks(
        &read_json_object(&paths.hooks_path())?,
        &paths.executable,
        &paths.data_dir,
    )?;
    let hooks_bytes = serde_json::to_vec_pretty(&hooks)?;
    let config_source = read_optional_string(&paths.config_path())?.unwrap_or_default();
    let (hooks_feature_before, hooks_feature_managed) = match &existing_manifest {
        Some(manifest) => (
            manifest.hooks_feature_before,
            manifest.hooks_feature_managed,
        ),
        None => {
            let before = read_hooks_feature(&config_source)?;
            (before, before != Some(true))
        }
    };
    let config = merge_config_with_ai_refresh(
        &config_source,
        &paths.executable,
        &paths.data_dir,
        &repository,
        effective_ai_refresh,
        existing_manifest.as_ref(),
    )?;
    let ai_refresh_profile_sha256 = if effective_ai_refresh {
        let current_before = profile_hash_from_source(&config_source)?;
        let previous_owned = existing_manifest
            .as_ref()
            .and_then(|manifest| manifest.ai_refresh_profile_sha256.clone());
        match (current_before, previous_owned) {
            (Some(current), Some(previous)) if current != previous => Some(previous),
            _ => profile_hash_from_source(&config)?,
        }
    } else {
        None
    };

    let manifest = SetupManifestV1 {
        version: MANIFEST_VERSION,
        managed_id: MANAGED_ID.to_string(),
        installed_at: Utc::now().to_rfc3339(),
        repository,
        executable: paths.executable.clone(),
        hooks_path: paths.hooks_path(),
        config_path: paths.config_path(),
        hooks_backup,
        config_backup,
        installed_hooks_sha256: digest(&hooks_bytes),
        installed_config_sha256: digest(config.as_bytes()),
        hooks_feature_before,
        hooks_feature_managed,
        ai_refresh_enabled: effective_ai_refresh,
        ai_refresh_profile_sha256,
    };
    let journal = SetupJournalV1 {
        version: JOURNAL_VERSION,
        managed_id: MANAGED_ID.to_string(),
        operation: SetupJournalOperation::Install,
        // The manifest is deliberately last: its presence means both integration files have
        // been durably replaced. Recovery safely reapplies every target after interruption.
        targets: vec![
            journal_target(&paths.hooks_path(), Some(hooks_bytes))?,
            journal_target(&paths.config_path(), Some(config.into_bytes()))?,
            journal_target(
                &paths.manifest_path(),
                Some(serde_json::to_vec_pretty(&manifest)?),
            )?,
        ],
    };
    persist_and_apply_journal(paths, &journal)?;
    if let Some(identity) = repository_identity {
        crate::store::reactivate_repository(&paths.data_dir, &identity.id)?;
    }
    Ok(manifest)
}

pub fn uninstall_codex(paths: &SetupPaths) -> Result<bool> {
    Ok(uninstall_codex_detailed(paths)?.removed)
}

pub fn uninstall_codex_detailed(paths: &SetupPaths) -> Result<UninstallResult> {
    ensure_trusted_data_dir(&paths.data_dir)?;
    let recovered_operation = recover_setup_journal(paths)?;
    let manifest = match read_manifest(&paths.manifest_path()) {
        Ok(manifest) => manifest,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(UninstallResult {
                removed: recovered_operation == Some(SetupJournalOperation::Uninstall),
                degraded: false,
                warnings: Vec::new(),
            })
        }
        Err(error) => return Err(error),
    };
    validate_manifest_for_uninstall(&manifest, paths)?;

    let mut warnings = Vec::new();
    let hooks_desired = uninstall_target_bytes(
        &paths.hooks_path(),
        &manifest.hooks_backup,
        &paths.backup_dir(),
        &manifest.installed_hooks_sha256,
        || {
            Ok(serde_json::to_vec_pretty(&remove_managed_hooks(
                &read_json_object(&paths.hooks_path())?,
            ))?)
        },
        &mut warnings,
        "hooks.json",
    )?;
    let config_desired = uninstall_target_bytes(
        &paths.config_path(),
        &manifest.config_backup,
        &paths.backup_dir(),
        &manifest.installed_config_sha256,
        || {
            let source = fs::read_to_string(paths.config_path())?;
            let mut config = remove_managed_config_with_profile(&source, &manifest)?;
            if manifest.hooks_feature_managed {
                config = restore_hooks_feature(&config, manifest.hooks_feature_before)?;
            }
            Ok(config.into_bytes())
        },
        &mut warnings,
        "config.toml",
    )?;
    let manifest_bytes = read_trusted_private_file(&paths.manifest_path(), "setup manifest")?;
    let archived_manifest = paths.data_dir.join(format!(
        "setup-manifest.uninstalled-{}-{}.json",
        Utc::now().timestamp(),
        uuid::Uuid::now_v7()
    ));
    let journal = SetupJournalV1 {
        version: JOURNAL_VERSION,
        managed_id: MANAGED_ID.to_string(),
        operation: SetupJournalOperation::Uninstall,
        targets: vec![
            journal_target(&paths.hooks_path(), hooks_desired)?,
            journal_target(&paths.config_path(), config_desired)?,
            journal_target(&archived_manifest, Some(manifest_bytes))?,
            journal_target(&paths.manifest_path(), None)?,
        ],
    };
    persist_and_apply_journal(paths, &journal)?;
    let degraded = !warnings.is_empty();
    if degraded {
        eprintln!(
            "PreviouslyOn uninstall completed in degraded mode: {}",
            warnings.join("; ")
        );
    }
    Ok(UninstallResult {
        removed: true,
        degraded,
        warnings,
    })
}

pub fn read_manifest(path: &Path) -> Result<SetupManifestV1> {
    let bytes = read_trusted_private_file(path, "setup manifest")
        .with_context(|| format!("read setup manifest {}", path.display()))?;
    let manifest: SetupManifestV1 = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse setup manifest {}", path.display()))?;
    ensure_manifest_matches(&manifest)?;
    Ok(manifest)
}

pub fn ai_refresh_profile_matches(paths: &SetupPaths, manifest: &SetupManifestV1) -> Result<bool> {
    if !manifest.ai_refresh_enabled {
        return Ok(false);
    }
    let source = read_optional_string(&paths.config_path())?.unwrap_or_default();
    Ok(profile_hash_from_source(&source)?.as_deref()
        == manifest.ai_refresh_profile_sha256.as_deref())
}

fn read_optional_manifest(path: &Path) -> Result<Option<SetupManifestV1>> {
    match read_optional_trusted_private_file(path, "setup manifest") {
        Ok(Some(bytes)) => {
            let manifest: SetupManifestV1 = serde_json::from_slice(&bytes)
                .with_context(|| format!("parse setup manifest {}", path.display()))?;
            ensure_manifest_matches(&manifest)?;
            Ok(Some(manifest))
        }
        Ok(None) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read setup manifest {}", path.display())),
    }
}

fn journal_target(path: &Path, desired_bytes: Option<Vec<u8>>) -> Result<SetupJournalTargetV1> {
    let original = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    Ok(SetupJournalTargetV1 {
        path: path.to_path_buf(),
        original_existed: original.is_some(),
        original_sha256: original.as_deref().map(digest),
        original_bytes: original.unwrap_or_default(),
        desired_sha256: desired_bytes.as_deref().map(digest),
        desired_bytes,
    })
}

fn persist_and_apply_journal(paths: &SetupPaths, journal: &SetupJournalV1) -> Result<()> {
    validate_setup_journal(paths, journal)?;
    let bytes = serde_json::to_vec_pretty(journal)?;
    write_private_atomic(&paths.journal_path(), &bytes)?;
    apply_setup_journal(paths, journal)?;
    remove_private_file_durable(&paths.journal_path())?;
    Ok(())
}

fn recover_setup_journal(paths: &SetupPaths) -> Result<Option<SetupJournalOperation>> {
    let bytes = match read_optional_trusted_private_file(
        &paths.journal_path(),
        "setup recovery journal",
    )? {
        Some(bytes) => bytes,
        None => return Ok(None),
    };
    let journal: SetupJournalV1 = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "parse setup recovery journal {}",
            paths.journal_path().display()
        )
    })?;
    if journal.version != JOURNAL_VERSION || journal.managed_id != MANAGED_ID {
        bail!("unsupported or foreign setup recovery journal");
    }
    validate_setup_journal(paths, &journal)?;
    apply_setup_journal(paths, &journal)?;
    remove_private_file_durable(&paths.journal_path())?;
    Ok(Some(journal.operation))
}

fn validate_setup_journal(paths: &SetupPaths, journal: &SetupJournalV1) -> Result<()> {
    if journal.version != JOURNAL_VERSION || journal.managed_id != MANAGED_ID {
        bail!("unsupported or foreign setup recovery journal");
    }

    for target in &journal.targets {
        match (target.original_existed, target.original_sha256.as_deref()) {
            (true, Some(hash)) if hash == digest(&target.original_bytes) => {}
            (false, None) if target.original_bytes.is_empty() => {}
            _ => bail!("setup recovery journal original target hash mismatch"),
        }
        match (&target.desired_bytes, target.desired_sha256.as_deref()) {
            (Some(bytes), Some(hash)) if hash == digest(bytes) => {}
            (None, None) => {}
            _ => bail!("setup recovery journal target hash mismatch"),
        }
    }

    let hooks_path = paths.hooks_path();
    let config_path = paths.config_path();
    let manifest_path = paths.manifest_path();
    let count = |path: &Path| {
        journal
            .targets
            .iter()
            .filter(|target| target.path == path)
            .count()
    };

    match journal.operation {
        SetupJournalOperation::Install => {
            if journal.targets.len() != 3
                || count(&hooks_path) != 1
                || count(&config_path) != 1
                || count(&manifest_path) != 1
                || journal.targets.iter().any(|target| {
                    !matches!(
                        target.path.as_path(),
                        path if path == hooks_path || path == config_path || path == manifest_path
                    ) || target.desired_bytes.is_none()
                })
            {
                bail!("setup recovery journal install targets do not match the exact allowlist");
            }
        }
        SetupJournalOperation::Uninstall => {
            let archives = journal
                .targets
                .iter()
                .filter(|target| is_uninstalled_manifest_archive(paths, &target.path))
                .collect::<Vec<_>>();
            if journal.targets.len() != 4
                || count(&hooks_path) != 1
                || count(&config_path) != 1
                || count(&manifest_path) != 1
                || archives.len() != 1
                || journal.targets.iter().any(|target| {
                    target.path != hooks_path
                        && target.path != config_path
                        && target.path != manifest_path
                        && !is_uninstalled_manifest_archive(paths, &target.path)
                })
                || journal
                    .targets
                    .iter()
                    .find(|target| target.path == manifest_path)
                    .is_some_and(|target| target.desired_bytes.is_some())
                || archives[0].desired_bytes.is_none()
                || archives[0].original_existed
            {
                bail!("setup recovery journal uninstall targets do not match the exact allowlist");
            }
        }
    }
    Ok(())
}

fn is_uninstalled_manifest_archive(paths: &SetupPaths, path: &Path) -> bool {
    if path.parent() != Some(paths.data_dir.as_path()) {
        return false;
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(middle) = name
        .strip_prefix("setup-manifest.uninstalled-")
        .and_then(|name| name.strip_suffix(".json"))
    else {
        return false;
    };
    let Some((timestamp, uuid)) = middle.split_once('-') else {
        return false;
    };
    !timestamp.is_empty()
        && timestamp.bytes().all(|byte| byte.is_ascii_digit())
        && timestamp.parse::<i64>().is_ok()
        && uuid::Uuid::parse_str(uuid).is_ok()
}

fn apply_setup_journal(paths: &SetupPaths, journal: &SetupJournalV1) -> Result<()> {
    validate_setup_journal(paths, journal)?;
    for target in &journal.targets {
        match &target.desired_bytes {
            Some(bytes) => {
                if target.desired_sha256.as_deref() != Some(digest(bytes).as_str()) {
                    bail!("setup recovery journal target hash mismatch");
                }
                write_private_atomic(&target.path, bytes)?;
            }
            None => remove_private_file_durable(&target.path)?,
        }
    }
    Ok(())
}

fn uninstall_target_bytes<F>(
    path: &Path,
    backup: &BackupRecord,
    backup_dir: &Path,
    installed_sha256: &str,
    remove_managed: F,
    warnings: &mut Vec<String>,
    label: &str,
) -> Result<Option<Vec<u8>>>
where
    F: FnOnce() -> Result<Vec<u8>>,
{
    let current = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if current.as_deref().map(digest).as_deref() == Some(installed_sha256) {
        return restore_backup_bytes(backup, backup_dir, label);
    }
    warnings.push(format!(
        "{label} changed after setup; removed only the managed block and preserved user changes"
    ));
    if current.is_none() {
        return Ok(None);
    }
    remove_managed().map(Some)
}

fn restore_backup_bytes(
    backup: &BackupRecord,
    backup_dir: &Path,
    label: &str,
) -> Result<Option<Vec<u8>>> {
    if !backup.existed {
        return Ok(None);
    }
    let path = backup
        .backup_path
        .as_deref()
        .context("setup backup path is missing")?;
    validate_backup_path(path, backup_dir, label, backup.sha256.as_deref())?;
    let bytes = read_trusted_private_file(path, "setup backup")
        .with_context(|| format!("read setup backup {}", path.display()))?;
    if backup.sha256.as_deref() != Some(digest(&bytes).as_str()) {
        bail!("setup backup hash mismatch: {}", path.display());
    }
    Ok(Some(bytes))
}

fn ensure_manifest_matches(manifest: &SetupManifestV1) -> Result<()> {
    if manifest.version != MANIFEST_VERSION || manifest.managed_id != MANAGED_ID {
        bail!("unsupported or foreign setup manifest");
    }
    Ok(())
}

fn validate_manifest_for_install(
    manifest: &SetupManifestV1,
    paths: &SetupPaths,
    repository: &Path,
) -> Result<()> {
    if manifest.repository != repository {
        bail!("setup manifest belongs to a different repository or Codex installation");
    }
    validate_manifest_for_uninstall(manifest, paths)
}

fn validate_manifest_for_uninstall(manifest: &SetupManifestV1, paths: &SetupPaths) -> Result<()> {
    if manifest.hooks_path != paths.hooks_path() || manifest.config_path != paths.config_path() {
        bail!("setup manifest belongs to a different repository or Codex installation");
    }
    validate_trusted_private_directory(&paths.backup_dir(), "setup backup directory")?;
    validate_backup_record(&manifest.hooks_backup, &paths.backup_dir(), "hooks.json")?;
    validate_backup_record(&manifest.config_backup, &paths.backup_dir(), "config.toml")?;
    if !is_sha256(&manifest.installed_hooks_sha256) || !is_sha256(&manifest.installed_config_sha256)
    {
        bail!("setup manifest contains an invalid installed file hash");
    }
    if manifest.ai_refresh_enabled
        && !manifest
            .ai_refresh_profile_sha256
            .as_deref()
            .is_some_and(is_sha256)
    {
        bail!("setup manifest contains an invalid AI refresh profile hash");
    }
    chrono::DateTime::parse_from_rfc3339(&manifest.installed_at)
        .context("setup manifest contains an invalid installation timestamp")?;
    Ok(())
}

fn validate_backup_record(record: &BackupRecord, backup_dir: &Path, label: &str) -> Result<()> {
    match (
        record.existed,
        record.backup_path.as_deref(),
        record.sha256.as_deref(),
    ) {
        (false, None, None) => Ok(()),
        (true, Some(path), Some(expected_hash)) if is_sha256(expected_hash) => {
            validate_backup_path(path, backup_dir, label, Some(expected_hash))?;
            let bytes = read_trusted_private_file(path, "setup backup")
                .with_context(|| format!("setup manifest {label} backup is unavailable"))?;
            if digest(&bytes) != expected_hash {
                bail!("setup manifest {label} backup hash mismatch");
            }
            Ok(())
        }
        _ => bail!("setup manifest contains an invalid {label} backup record"),
    }
}

fn validate_backup_path(
    path: &Path,
    backup_dir: &Path,
    label: &str,
    expected_hash: Option<&str>,
) -> Result<()> {
    let expected_hash = expected_hash.context("setup backup hash is missing")?;
    if !is_sha256(expected_hash) {
        bail!("setup manifest contains an invalid {label} backup hash");
    }
    let expected_name = format!("{label}.{}.bak", &expected_hash[..12]);
    if path != backup_dir.join(expected_name) {
        bail!("setup manifest contains an invalid {label} backup path");
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn ensure_trusted_data_dir(path: &Path) -> Result<()> {
    ensure_trusted_private_directory(path, "setup data directory")
}

fn ensure_trusted_private_directory(path: &Path, label: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => validate_trusted_private_directory(path, label),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create parent directory {}", parent.display()))?;
            }
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(path) {
                Ok(()) => set_private_directory(path)?,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            validate_trusted_private_directory(path, label)
        }
        Err(error) => Err(error.into()),
    }
}

fn validate_trusted_private_directory(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect {label} {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "{label} must be a regular directory, not a symlink: {}",
            path.display()
        );
    }
    validate_private_metadata(&metadata, true, label, path)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = OpenOptions::new();
        options
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
        let directory = options
            .open(path)
            .with_context(|| format!("open trusted {label} {}", path.display()))?;
        validate_private_metadata(&directory.metadata()?, true, label, path)?;
    }
    Ok(())
}

fn validate_optional_trusted_private_directory(path: &Path, label: &str) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            validate_trusted_private_directory(path, label)?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn read_optional_trusted_private_file(path: &Path, label: &str) -> Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "{label} must be a regular file, not a symlink: {}",
            path.display()
        );
    }
    validate_private_metadata(&metadata, false, label, path)?;

    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("open trusted {label} {}", path.display()))?;
    validate_private_metadata(&file.metadata()?, false, label, path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

fn read_trusted_private_file(path: &Path, label: &str) -> Result<Vec<u8>> {
    match read_optional_trusted_private_file(path, label)? {
        Some(bytes) => Ok(bytes),
        None => Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{label} does not exist: {}", path.display()),
        )
        .into()),
    }
}

#[cfg(unix)]
fn validate_private_metadata(
    metadata: &fs::Metadata,
    directory: bool,
    label: &str,
    path: &Path,
) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    if metadata.uid() != unsafe { libc::geteuid() } {
        bail!(
            "{label} is not owned by the current user: {}",
            path.display()
        );
    }
    if metadata.mode() & 0o077 != 0 {
        let boundary = if directory { "0700" } else { "0600" };
        bail!(
            "{label} exceeds the private {boundary} boundary: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_metadata(
    _metadata: &fs::Metadata,
    _directory: bool,
    _label: &str,
    _path: &Path,
) -> Result<()> {
    Ok(())
}

pub fn managed_hook_command(executable: &Path, data_dir: &Path, event: &str) -> String {
    let executable = shell_quote(&executable.to_string_lossy());
    let data_dir = shell_quote(&data_dir.to_string_lossy());
    format!(
        "PREVIOUSLY_ON_MANAGED_ID={} PREVIOUSLY_ON_DATA_DIR={} {} hook {}",
        MANAGED_ID, data_dir, executable, event
    )
}

pub fn merge_hooks(
    original: &Map<String, Value>,
    executable: &Path,
    data_dir: &Path,
) -> Result<Value> {
    let mut root = original.clone();
    let hooks = root
        .entry("hooks".to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .context("hooks.json field `hooks` must be an object")?;

    for event in HOOK_EVENTS {
        let groups = hooks
            .entry(event.to_string())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .with_context(|| format!("hooks.json event `{event}` must be an array"))?;
        for group in groups.iter_mut() {
            remove_managed_from_group(group);
        }
        groups.retain(group_has_hooks);

        let hook = json!({
            "type": "command",
            "command": managed_hook_command(executable, data_dir, event),
            "timeout": if event == "Stop" { 30 } else { 10 }
        });
        let mut group = json!({ "hooks": [hook] });
        if event == "SessionStart" {
            group["matcher"] = json!("startup|resume|clear|compact");
        }
        groups.push(group);
    }
    Ok(Value::Object(root))
}

pub fn remove_managed_hooks(original: &Map<String, Value>) -> Value {
    let mut root = original.clone();
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let events: Vec<String> = hooks.keys().cloned().collect();
        for event in events {
            if let Some(groups) = hooks.get_mut(&event).and_then(Value::as_array_mut) {
                for group in groups.iter_mut() {
                    remove_managed_from_group(group);
                }
                groups.retain(group_has_hooks);
            }
        }
    }
    Value::Object(root)
}

fn remove_managed_from_group(group: &mut Value) {
    if let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) {
        hooks.retain(|hook| {
            !hook
                .get("command")
                .and_then(Value::as_str)
                .is_some_and(|command| command.contains(MANAGED_ID))
        });
    }
}

fn group_has_hooks(group: &Value) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|hooks| !hooks.is_empty())
}

pub fn merge_config(
    source: &str,
    executable: &Path,
    data_dir: &Path,
    repository: &Path,
) -> Result<String> {
    merge_config_with_ai_refresh(source, executable, data_dir, repository, false, None)
}

fn merge_config_with_ai_refresh(
    source: &str,
    executable: &Path,
    data_dir: &Path,
    repository: &Path,
    enable_ai_refresh: bool,
    existing_manifest: Option<&SetupManifestV1>,
) -> Result<String> {
    let mut document = parse_toml(source)?;
    let features = ensure_table(&mut document, "features")?;
    features["hooks"] = value(true);
    let servers = ensure_table(&mut document, "mcp_servers")?;
    if let Some(existing) = servers.get("previously_on") {
        let is_managed = existing
            .as_table()
            .and_then(|table| table.get("env"))
            .and_then(Item::as_table)
            .and_then(|env| env.get("PREVIOUSLY_ON_MANAGED_ID"))
            .and_then(Item::as_value)
            .and_then(toml_edit::Value::as_str)
            == Some(MANAGED_ID);
        if !is_managed {
            bail!(
                "Codex MCP name `previously_on` already exists and is not managed by {}",
                MANAGED_ID
            );
        }
    }
    servers.remove("previously_on");
    let mut server = Table::new();
    server["command"] = value(executable.to_string_lossy().to_string());
    let mut args = Array::new();
    args.push("mcp");
    server["args"] = value(args);
    server["startup_timeout_sec"] = value(15);
    server["enabled"] = value(true);
    let mut tools = Table::new();
    let mut continue_task = Table::new();
    // The boundary hook can propose this write, but Codex must always stop for fresh user consent
    // before PreviouslyOn creates or starts another local task.
    continue_task["approval_mode"] = value("prompt");
    tools["continue_task"] = Item::Table(continue_task);
    server["tools"] = Item::Table(tools);
    let mut env = Table::new();
    env["PREVIOUSLY_ON_MANAGED_ID"] = value(MANAGED_ID);
    env["PREVIOUSLY_ON_DATA_DIR"] = value(data_dir.to_string_lossy().to_string());
    server["env"] = Item::Table(env);
    servers["previously_on"] = Item::Table(server);
    if enable_ai_refresh {
        let current_hash = profile_hash(&document);
        let owned_unchanged = existing_manifest
            .and_then(|manifest| manifest.ai_refresh_profile_sha256.as_deref())
            .is_some_and(|expected| current_hash.as_deref() == Some(expected));
        if current_hash.is_none() || owned_unchanged {
            install_ai_refresh_profile(&mut document)?;
        }
        // A profile modified after setup is user-owned from this point forward. Keep it exactly as
        // found; capability verification will fail closed if it no longer meets the requirements.
    }
    let _ = repository; // The single-repository allowlist is recorded in the setup manifest.
    Ok(document.to_string())
}

fn validate_ai_profile_collision(
    source: &str,
    enable_ai_refresh: bool,
    manifest: Option<&SetupManifestV1>,
) -> Result<()> {
    if !enable_ai_refresh {
        return Ok(());
    }
    let document = parse_toml(source)?;
    let Some(current_hash) = profile_hash(&document) else {
        return Ok(());
    };
    let manifest_owned = manifest
        .filter(|manifest| manifest.ai_refresh_enabled)
        .and_then(|manifest| manifest.ai_refresh_profile_sha256.as_deref());
    if manifest_owned.is_none() {
        bail!(
            "Codex permission profile `previously-input-only` already exists and is not managed by {}",
            MANAGED_ID
        );
    }
    // An existing owned profile may have been edited by the user. Reinstall preserves it instead
    // of overwriting, so both the unchanged and changed cases are valid here.
    let _ = current_hash;
    Ok(())
}

fn install_ai_refresh_profile(document: &mut DocumentMut) -> Result<()> {
    let permissions = ensure_table(document, "permissions")?;
    permissions.remove("previously-input-only");
    let mut profile = Table::new();
    let mut filesystem = Table::new();
    filesystem[":root"] = value("deny");
    filesystem[":minimal"] = value("read");
    filesystem[":tmpdir"] = value("deny");
    filesystem[":slash_tmp"] = value("deny");
    profile["filesystem"] = Item::Table(filesystem);
    let mut network = Table::new();
    network["enabled"] = value(false);
    profile["network"] = Item::Table(network);
    permissions["previously-input-only"] = Item::Table(profile);
    Ok(())
}

fn profile_hash(document: &DocumentMut) -> Option<String> {
    document
        .get("permissions")
        .and_then(Item::as_table)
        .and_then(|permissions| permissions.get("previously-input-only"))
        .map(|profile| digest(format!("{profile:?}").as_bytes()))
}

fn profile_hash_from_source(source: &str) -> Result<Option<String>> {
    Ok(profile_hash(&parse_toml(source)?))
}

fn read_hooks_feature(source: &str) -> Result<Option<bool>> {
    let document = parse_toml(source)?;
    Ok(document
        .get("features")
        .and_then(Item::as_table)
        .and_then(|table| table.get("hooks"))
        .and_then(Item::as_value)
        .and_then(toml_edit::Value::as_bool))
}

fn restore_hooks_feature(source: &str, previous: Option<bool>) -> Result<String> {
    let mut document = parse_toml(source)?;
    let current = document
        .get("features")
        .and_then(Item::as_table)
        .and_then(|table| table.get("hooks"))
        .and_then(Item::as_value)
        .and_then(toml_edit::Value::as_bool);
    // A user change after setup wins. Restore only while the value still equals the value that
    // PreviouslyOn installed.
    if current == Some(true) {
        match previous {
            Some(value_before) => {
                let features = ensure_table(&mut document, "features")?;
                features["hooks"] = value(value_before);
            }
            None => {
                let remove_features = if let Some(features) =
                    document.get_mut("features").and_then(Item::as_table_mut)
                {
                    features.remove("hooks");
                    features.is_empty()
                } else {
                    false
                };
                if remove_features {
                    document.remove("features");
                }
            }
        }
    }
    Ok(document.to_string())
}

pub fn remove_managed_config(source: &str) -> Result<String> {
    let mut document = parse_toml(source)?;
    if let Some(servers) = document.get_mut("mcp_servers").and_then(Item::as_table_mut) {
        let managed = servers
            .get("previously_on")
            .and_then(Item::as_table)
            .and_then(|table| table.get("env"))
            .and_then(Item::as_table)
            .and_then(|env| env.get("PREVIOUSLY_ON_MANAGED_ID"))
            .and_then(Item::as_value)
            .and_then(toml_edit::Value::as_str)
            == Some(MANAGED_ID);
        if managed {
            servers.remove("previously_on");
        }
    }
    Ok(document.to_string())
}

fn remove_managed_config_with_profile(source: &str, manifest: &SetupManifestV1) -> Result<String> {
    let source = remove_managed_config(source)?;
    let mut document = parse_toml(&source)?;
    if manifest.ai_refresh_enabled {
        let expected = manifest.ai_refresh_profile_sha256.as_deref();
        if profile_hash(&document).as_deref() == expected {
            let remove_permissions = if let Some(permissions) =
                document.get_mut("permissions").and_then(Item::as_table_mut)
            {
                permissions.remove("previously-input-only");
                permissions.is_empty()
            } else {
                false
            };
            if remove_permissions {
                document.remove("permissions");
            }
        }
    }
    Ok(document.to_string())
}

fn parse_toml(source: &str) -> Result<DocumentMut> {
    if source.trim().is_empty() {
        Ok(DocumentMut::new())
    } else {
        source
            .parse::<DocumentMut>()
            .context("parse Codex config.toml")
    }
}

fn ensure_table<'a>(document: &'a mut DocumentMut, key: &str) -> Result<&'a mut Table> {
    if document.get(key).is_none() {
        document[key] = Item::Table(Table::new());
    }
    document[key]
        .as_table_mut()
        .with_context(|| format!("Codex config key `{key}` must be a table"))
}

fn backup_file(path: &Path, backup_dir: &Path, name: &str) -> Result<BackupRecord> {
    if !path.exists() {
        return Ok(BackupRecord {
            existed: false,
            backup_path: None,
            sha256: None,
        });
    }
    let bytes = fs::read(path)?;
    let hash = digest(&bytes);
    let backup_path = backup_dir.join(format!("{}.{}.bak", name, &hash[..12]));
    match read_optional_trusted_private_file(&backup_path, "setup backup")? {
        Some(existing) if existing == bytes => {}
        Some(_) => bail!(
            "existing setup backup hash mismatch: {}",
            backup_path.display()
        ),
        None => write_private_atomic(&backup_path, &bytes)?,
    }
    Ok(BackupRecord {
        existed: true,
        backup_path: Some(backup_path),
        sha256: Some(hash),
    })
}

fn read_json_object(path: &Path) -> Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let value: Value = serde_json::from_slice(&fs::read(path)?)
        .with_context(|| format!("parse {}", path.display()))?;
    value
        .as_object()
        .cloned()
        .with_context(|| format!("{} must contain a JSON object", path.display()))
}

fn read_optional_string(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(source) => Ok(Some(source)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        set_private_directory(parent)?;
    }
    let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    set_private_file(&temporary)?;
    fs::rename(&temporary, path)?;
    set_private_file(path)?;
    if let Some(parent) = path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

fn remove_private_file_durable(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    if let Some(parent) = path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_file(_path: &Path) -> Result<()> {
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
