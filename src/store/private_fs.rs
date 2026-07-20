use anyhow::{bail, Context, Result};
use sha2::Digest;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub(crate) fn ensure_private_directory(path: &Path, label: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                bail!(
                    "{label} must be a real directory, not a symlink: {}",
                    path.display()
                );
            }
            validate_private_owner(&metadata, label, path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if metadata.mode() & 0o022 != 0 {
                    bail!("{label} is group/world writable: {}", path.display());
                }
                if metadata.mode() & 0o077 != 0 {
                    tighten_private_directory(path, label)?;
                }
            }
            validate_private_directory(path, label)
        }
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
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            validate_private_directory(path, label)
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn tighten_private_directory(path: &Path, label: &str) -> Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
    let directory = options
        .open(path)
        .with_context(|| format!("open trusted {label} {}", path.display()))?;
    let metadata = directory.metadata()?;
    if !metadata.is_dir() {
        bail!("{label} must be a real directory: {}", path.display());
    }
    validate_private_owner(&metadata, label, path)?;
    directory.set_permissions(fs::Permissions::from_mode(0o700))?;
    Ok(())
}

pub(crate) fn validate_private_directory(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect {label} {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "{label} must be a real directory, not a symlink: {}",
            path.display()
        );
    }
    validate_private_metadata(&metadata, true, label, path)
}

pub(crate) fn validate_private_regular_file(path: &Path, label: &str) -> Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "{label} must be a regular file, not a symlink: {}",
            path.display()
        );
    }
    validate_private_metadata(&metadata, false, label, path)?;
    Ok(true)
}

#[cfg(unix)]
pub(crate) fn validate_private_socket(path: &Path, label: &str) -> Result<bool> {
    use std::os::unix::fs::FileTypeExt;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_socket() {
        bail!(
            "{label} must be a Unix socket, not a symlink: {}",
            path.display()
        );
    }
    validate_private_metadata(&metadata, false, label, path)?;
    Ok(true)
}

pub(crate) fn open_private_file(
    path: &Path,
    label: &str,
    options: &mut OpenOptions,
) -> Result<fs::File> {
    validate_private_regular_file(path, label)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .with_context(|| format!("open {label} {}", path.display()))?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        bail!("{label} must be a regular file: {}", path.display());
    }
    validate_private_metadata(&metadata, false, label, path)?;
    Ok(file)
}

pub(crate) fn read_private_file(path: &Path, label: &str) -> Result<Option<Vec<u8>>> {
    if !validate_private_regular_file(path, label)? {
        return Ok(None);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    let mut file = open_private_file(path, label, &mut options)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

pub(super) fn ensure_private_regular_file(path: &Path, label: &str) -> Result<()> {
    if validate_private_regular_file(path, label)? {
        return Ok(());
    }
    let mut options = OpenOptions::new();
    options.create_new(true).read(true).write(true);
    open_private_file(path, label, &mut options)?;
    Ok(())
}

pub(super) fn secure_new_private_file(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect new {label} {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "new {label} must be a regular file, not a symlink: {}",
            path.display()
        );
    }
    validate_private_owner(&metadata, label, path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .with_context(|| format!("open new {label} {}", path.display()))?;
    validate_private_owner(&file.metadata()?, label, path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    validate_private_metadata(&file.metadata()?, false, label, path)
}

pub(super) fn database_companion_paths(database: &Path) -> Vec<(PathBuf, &'static str)> {
    let database = database.to_string_lossy();
    vec![
        (PathBuf::from(format!("{database}-wal")), "SQLite WAL"),
        (
            PathBuf::from(format!("{database}-shm")),
            "SQLite shared-memory file",
        ),
        (
            PathBuf::from(format!("{database}-journal")),
            "SQLite rollback journal",
        ),
        (
            PathBuf::from(format!("{database}.lock")),
            "database maintenance lock",
        ),
        (
            PathBuf::from(format!("{database}.purge-recovery.json")),
            "purge recovery journal",
        ),
    ]
}

pub(super) fn validate_database_companions(database: &Path) -> Result<()> {
    for (path, label) in database_companion_paths(database) {
        validate_private_regular_file(&path, label)?;
    }
    Ok(())
}

#[cfg(unix)]
fn validate_private_owner(metadata: &fs::Metadata, label: &str, path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    if metadata.uid() != unsafe { libc::geteuid() } {
        bail!(
            "{label} is not owned by the current user: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_owner(_metadata: &fs::Metadata, _label: &str, _path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn validate_private_metadata(
    metadata: &fs::Metadata,
    directory: bool,
    label: &str,
    path: &Path,
) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    validate_private_owner(metadata, label, path)?;
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

pub(super) fn repository_tombstone_path(data_dir: &Path, repository_id: &str) -> PathBuf {
    let identity_hash = hex::encode(sha2::Sha256::digest(repository_id.as_bytes()));
    data_dir
        .join("purge-tombstones")
        .join(format!("{identity_hash}.json"))
}

pub(super) fn acquire_database_lock(database_path: &Path) -> Result<fs::File> {
    let lock_path = PathBuf::from(format!("{}.lock", database_path.to_string_lossy()));
    if let Some(parent) = lock_path.parent() {
        ensure_private_directory(parent, "PreviouslyOn data directory")?;
    }
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    let file = open_private_file(&lock_path, "database maintenance lock", &mut options)?;
    file.lock()
        .with_context(|| format!("lock database maintenance file {}", lock_path.display()))?;
    Ok(file)
}

pub(super) fn write_private_atomic_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("atomic file path has no parent")?;
    ensure_private_directory(parent, "private data directory")?;
    validate_private_regular_file(path, "private data file")?;
    let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::now_v7()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    let mut file = open_private_file(&temporary, "temporary private data file", &mut options)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)?;
    validate_private_regular_file(path, "private data file")?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

pub(super) fn remove_file_and_sync_parent(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => {
            if let Some(parent) = path.parent() {
                fs::File::open(parent)?.sync_all()?;
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn remove_sidecar_if_present(database: &Path, suffix: &str) -> Result<()> {
    let sidecar = PathBuf::from(format!("{}-{suffix}", database.to_string_lossy()));
    validate_private_regular_file(&sidecar, "SQLite sidecar")?;
    match fs::remove_file(&sidecar) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("remove database sidecar {}", sidecar.display()))
        }
    }
}
