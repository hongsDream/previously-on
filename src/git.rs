use crate::domain::{
    ChangeAttribution, ChangeStatus, CurrentValidationV1, FileChangeV1, Freshness, GitSnapshotV1,
    TemporalRevalidationV1, TemporalStatusV1, SCHEMA_VERSION_V1,
};
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

const FINGERPRINT_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryIdentity {
    /// Stable identity shared by every linked worktree of the repository.
    pub id: String,
    /// Concrete worktree root used for snapshots and diffs.
    pub root: PathBuf,
    pub common_dir: PathBuf,
    pub remote_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ContentState {
    Missing,
    Sha256(String),
    Unsupported,
}

impl ContentState {
    fn from_stored(value: Option<String>) -> Self {
        value.map_or(Self::Missing, Self::Sha256)
    }

    fn stored(self) -> Option<Option<String>> {
        match self {
            Self::Missing => Some(None),
            Self::Sha256(value) => Some(Some(value)),
            Self::Unsupported => None,
        }
    }
}

pub fn repository_identity(path: impl AsRef<Path>) -> Result<RepositoryIdentity> {
    let root_text = git(path.as_ref(), &["rev-parse", "--show-toplevel"])?;
    let root = PathBuf::from(root_text.trim())
        .canonicalize()
        .with_context(|| format!("canonicalize Git root {}", root_text.trim()))?;
    let common_dir_text = git(&root, &["rev-parse", "--git-common-dir"])?;
    let common_dir_raw = PathBuf::from(common_dir_text.trim());
    let common_dir = if common_dir_raw.is_absolute() {
        common_dir_raw
    } else {
        root.join(common_dir_raw)
    }
    .canonicalize()
    .with_context(|| format!("canonicalize Git common dir {}", common_dir_text.trim()))?;
    let remote_url = git_optional(&root, &["remote", "get-url", "origin"])
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    // The common Git directory is shared by linked worktrees. The actual worktree root
    // remains separate so snapshots never run against a sibling checkout.
    let id = common_dir.to_string_lossy().into_owned();
    Ok(RepositoryIdentity {
        id,
        root,
        common_dir,
        remote_url,
    })
}

pub fn capture_snapshot(path: impl AsRef<Path>) -> Result<GitSnapshotV1> {
    let identity = repository_identity(path)?;
    let branch = git_optional(
        &identity.root,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
    )
    .map(|value| value.trim().to_string())
    .filter(|value| !value.is_empty());
    let head = git_optional(&identity.root, &["rev-parse", "--verify", "HEAD"])
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let dirty_files = parse_status_paths(&git_bytes(
        &identity.root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?)
    .into_iter()
    .filter_map(|path| validated_repository_relative_path(&path))
    .collect::<Vec<_>>();
    let mut changes = if let Some(head) = head.as_deref() {
        diff_changes(&identity.root, Some(head), None)?
    } else {
        diff_changes(&identity.root, None, None)?
    };
    changes.retain(|change| {
        validated_repository_relative_path(&change.path).is_some()
            && change
                .previous_path
                .as_deref()
                .is_none_or(|path| validated_repository_relative_path(path).is_some())
    });
    let tracked = changes
        .iter()
        .map(|change| change.path.clone())
        .collect::<BTreeSet<_>>();
    for path in dirty_files.iter().filter(|path| !tracked.contains(*path)) {
        changes.push(FileChangeV1 {
            schema_version: SCHEMA_VERSION_V1,
            repository_id: identity.id.clone(),
            session_id: String::new(),
            task_id: None,
            path: path.clone(),
            previous_path: None,
            status: ChangeStatus::Added,
            additions: None,
            deletions: None,
            attribution: ChangeAttribution::ObservedChangedIn,
            before_head: head.clone(),
            after_head: head.clone(),
        });
    }
    changes.sort_by(|a, b| (&a.path, &a.previous_path).cmp(&(&b.path, &b.previous_path)));
    let content_fingerprints = capture_dirty_fingerprints(&identity.root, &dirty_files, &changes)?;
    Ok(GitSnapshotV1 {
        schema_version: SCHEMA_VERSION_V1,
        repository_id: identity.id,
        root: identity.root.to_string_lossy().into_owned(),
        remote_url: identity.remote_url,
        branch,
        head,
        captured_at: Utc::now(),
        dirty_files,
        working_tree_changes: changes,
        content_fingerprints,
    })
}

fn capture_dirty_fingerprints(
    root: &Path,
    dirty_files: &[String],
    changes: &[FileChangeV1],
) -> Result<BTreeMap<String, Option<String>>> {
    let paths = dirty_files
        .iter()
        .map(String::as_str)
        .chain(changes.iter().flat_map(|change| {
            [Some(change.path.as_str()), change.previous_path.as_deref()]
                .into_iter()
                .flatten()
        }))
        .filter_map(validated_repository_relative_path)
        .collect::<BTreeSet<_>>();
    let mut fingerprints = BTreeMap::new();
    for path in paths {
        if let Some(fingerprint) = fingerprint_repository_path(root, &path)?.stored() {
            fingerprints.insert(path, fingerprint);
        }
    }
    Ok(fingerprints)
}

fn fingerprint_repository_path(root: &Path, relative_path: &str) -> Result<ContentState> {
    let Some(relative_path) = validated_repository_relative_path(relative_path) else {
        return Ok(ContentState::Unsupported);
    };
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("canonicalize repository root {}", root.display()))?;
    let path = canonical_root.join(relative_path);
    let parent = path.parent().unwrap_or(&canonical_root);
    let canonical_parent = canonicalize_existing_ancestor(parent)?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Ok(ContentState::Unsupported);
    }
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ContentState::Missing);
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("inspect Git snapshot path {}", path.display()));
        }
    };
    let mut hasher = Sha256::new();
    if metadata.file_type().is_symlink() {
        hasher.update(b"symlink\0");
        hasher.update(
            std::fs::read_link(&path)
                .with_context(|| format!("read Git snapshot symlink {}", path.display()))?
                .as_os_str()
                .as_encoded_bytes(),
        );
    } else if metadata.is_file() {
        let canonical_path = path
            .canonicalize()
            .with_context(|| format!("canonicalize Git snapshot path {}", path.display()))?;
        if !canonical_path.starts_with(&canonical_root) {
            return Ok(ContentState::Unsupported);
        }
        hasher.update(b"file\0");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            hasher.update((metadata.permissions().mode() & 0o111).to_le_bytes());
        }
        // Stream with fixed memory so a repository containing a very large file cannot force an
        // allocation proportional to file size. Raw content is never retained or returned.
        let mut file = std::fs::File::open(&path)
            .with_context(|| format!("open Git snapshot path {}", path.display()))?;
        let mut buffer = [0_u8; FINGERPRINT_BUFFER_BYTES];
        loop {
            let read = file
                .read(&mut buffer)
                .with_context(|| format!("hash Git snapshot path {}", path.display()))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
    } else {
        return Ok(ContentState::Unsupported);
    }
    Ok(ContentState::Sha256(hex::encode(hasher.finalize())))
}

fn canonicalize_existing_ancestor(path: &Path) -> Result<PathBuf> {
    let mut candidate = path;
    loop {
        match candidate.canonicalize() {
            Ok(canonical) => return Ok(canonical),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                candidate = candidate
                    .parent()
                    .context("repository path has no existing ancestor")?;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("canonicalize repository path {}", path.display()));
            }
        }
    }
}

pub fn correlate_changes(
    repository_path: impl AsRef<Path>,
    before: &GitSnapshotV1,
    after: &GitSnapshotV1,
    session_id: &str,
    task_id: Option<&str>,
    tool_evidence_paths: &[String],
) -> Result<Vec<FileChangeV1>> {
    if before.repository_id != after.repository_id {
        bail!("cannot correlate snapshots from different repositories");
    }
    let mut changes = if before.head != after.head {
        match (before.head.as_deref(), after.head.as_deref()) {
            (Some(from), Some(to)) => diff_changes(repository_path.as_ref(), Some(from), Some(to))?,
            _ => Vec::new(),
        }
    } else {
        Vec::new()
    };

    let before_map = before
        .working_tree_changes
        .iter()
        .map(|change| (change.path.clone(), change_signature(change)))
        .collect::<BTreeMap<_, _>>();
    for change in &after.working_tree_changes {
        if before_map.get(&change.path) != Some(&change_signature(change)) {
            changes.push(change.clone());
        }
    }

    // `git diff --numstat` can remain identical when an already-dirty or untracked file changes
    // again. Exact content fingerprints close that gap without persisting file content. Only keys
    // captured on both sides participate, so a clean snapshot's intentionally absent projection
    // is not confused with a deleted file.
    for path in before
        .content_fingerprints
        .keys()
        .filter(|path| after.content_fingerprints.contains_key(*path))
    {
        let before_fingerprint = before.content_fingerprints.get(path);
        let after_fingerprint = after.content_fingerprints.get(path);
        if before_fingerprint == after_fingerprint
            || changes
                .iter()
                .any(|change| &change.path == path || change.previous_path.as_ref() == Some(path))
        {
            continue;
        }
        let template = after
            .working_tree_changes
            .iter()
            .find(|change| &change.path == path)
            .or_else(|| {
                before
                    .working_tree_changes
                    .iter()
                    .find(|change| &change.path == path)
            });
        let status = match (
            before_fingerprint.and_then(Option::as_ref),
            after_fingerprint.and_then(Option::as_ref),
        ) {
            (None, Some(_)) => ChangeStatus::Added,
            (Some(_), None) => ChangeStatus::Deleted,
            _ => ChangeStatus::Modified,
        };
        changes.push(template.map_or_else(
            || FileChangeV1 {
                schema_version: SCHEMA_VERSION_V1,
                repository_id: String::new(),
                session_id: String::new(),
                task_id: None,
                path: path.clone(),
                previous_path: None,
                status,
                additions: None,
                deletions: None,
                attribution: ChangeAttribution::ObservedChangedIn,
                before_head: before.head.clone(),
                after_head: after.head.clone(),
            },
            |change| FileChangeV1 {
                status,
                additions: None,
                deletions: None,
                ..change.clone()
            },
        ));
    }

    let after_paths = after
        .working_tree_changes
        .iter()
        .map(|change| change.path.as_str())
        .collect::<BTreeSet<_>>();
    for change in &before.working_tree_changes {
        if !after_paths.contains(change.path.as_str()) && before.head == after.head {
            changes.push(FileChangeV1 {
                status: ChangeStatus::Modified,
                additions: None,
                deletions: None,
                ..change.clone()
            });
        }
    }

    let evidence = tool_evidence_paths
        .iter()
        .filter_map(|path| validated_repository_relative_path(path))
        .collect::<BTreeSet<_>>();
    let mut deduped = BTreeMap::new();
    for mut change in changes {
        let Some(path) = validated_repository_relative_path(&change.path) else {
            continue;
        };
        let previous = match change.previous_path.as_deref() {
            Some(previous) => {
                let Some(previous) = validated_repository_relative_path(previous) else {
                    continue;
                };
                Some(previous)
            }
            None => None,
        };
        change.repository_id = before.repository_id.clone();
        change.session_id = session_id.to_string();
        change.task_id = task_id.map(str::to_string);
        change.before_head = before.head.clone();
        change.after_head = after.head.clone();
        let directly_observed = evidence.contains(&path)
            || previous
                .as_ref()
                .map(|path| evidence.contains(path))
                .unwrap_or(false);
        change.attribution = if directly_observed {
            ChangeAttribution::ModifiedBy
        } else {
            ChangeAttribution::ObservedChangedIn
        };
        change.path = path;
        change.previous_path = previous;
        deduped.insert((change.path.clone(), change.previous_path.clone()), change);
    }
    Ok(deduped.into_values().collect())
}

pub fn assess_task_freshness(
    repository_path: impl AsRef<Path>,
    baseline: Option<&GitSnapshotV1>,
    files: &[FileChangeV1],
) -> Result<Freshness> {
    Ok(
        match revalidate_task(repository_path, baseline, files)?.status {
            TemporalStatusV1::Unchanged => Freshness::Fresh,
            TemporalStatusV1::Broken => Freshness::Broken,
            TemporalStatusV1::Changed | TemporalStatusV1::Diverged | TemporalStatusV1::Degraded => {
                Freshness::Stale
            }
        },
    )
}

pub fn revalidate_task(
    repository_path: impl AsRef<Path>,
    baseline: Option<&GitSnapshotV1>,
    files: &[FileChangeV1],
) -> Result<TemporalRevalidationV1> {
    let current = capture_snapshot(&repository_path)?;
    let mut checked_paths = files
        .iter()
        .flat_map(|change| [Some(change.path.as_str()), change.previous_path.as_deref()])
        .flatten()
        .filter_map(validated_repository_relative_path)
        .collect::<BTreeSet<_>>();
    let Some(baseline) = baseline else {
        return Ok(TemporalRevalidationV1 {
            schema_version: SCHEMA_VERSION_V1,
            status: TemporalStatusV1::Degraded,
            baseline_head: None,
            current_head: current.head,
            merge_base: None,
            related_changes: Vec::new(),
            checked_paths: checked_paths.into_iter().collect(),
            warnings: vec!["No deterministic Git baseline is available.".to_string()],
        });
    };
    if baseline.repository_id != current.repository_id {
        return Ok(TemporalRevalidationV1 {
            schema_version: SCHEMA_VERSION_V1,
            status: TemporalStatusV1::Degraded,
            baseline_head: baseline.head.clone(),
            current_head: current.head,
            merge_base: None,
            related_changes: Vec::new(),
            checked_paths: checked_paths.into_iter().collect(),
            warnings: vec![
                "The Git baseline belongs to a different logical repository.".to_string(),
            ],
        });
    }
    if checked_paths.is_empty() {
        return Ok(TemporalRevalidationV1 {
            schema_version: SCHEMA_VERSION_V1,
            status: TemporalStatusV1::Degraded,
            baseline_head: baseline.head.clone(),
            current_head: current.head,
            merge_base: None,
            related_changes: Vec::new(),
            checked_paths: Vec::new(),
            warnings: vec!["No task-related paths are available for revalidation.".to_string()],
        });
    }
    let (Some(baseline_head), Some(current_head)) =
        (baseline.head.as_deref(), current.head.as_deref())
    else {
        return Ok(TemporalRevalidationV1 {
            schema_version: SCHEMA_VERSION_V1,
            status: TemporalStatusV1::Degraded,
            baseline_head: baseline.head.clone(),
            current_head: current.head,
            merge_base: None,
            related_changes: Vec::new(),
            checked_paths: checked_paths.into_iter().collect(),
            warnings: vec!["Baseline or current HEAD is unavailable.".to_string()],
        });
    };
    if !commit_exists(repository_path.as_ref(), baseline_head)? {
        return Ok(TemporalRevalidationV1 {
            schema_version: SCHEMA_VERSION_V1,
            status: TemporalStatusV1::Degraded,
            baseline_head: baseline.head.clone(),
            current_head: current.head,
            merge_base: None,
            related_changes: Vec::new(),
            checked_paths: checked_paths.into_iter().collect(),
            warnings: vec!["The baseline commit is no longer resolvable.".to_string()],
        });
    }

    let ancestor = is_ancestor(repository_path.as_ref(), baseline_head, current_head)?;
    let merge_base = git_optional(
        repository_path.as_ref(),
        &["merge-base", baseline_head, current_head],
    )
    .map(|value| value.trim().to_string())
    .filter(|value| !value.is_empty());
    let mut changes = diff_changes(
        repository_path.as_ref(),
        Some(baseline_head),
        Some(current_head),
    )?;
    changes.extend(current.working_tree_changes.clone());
    changes.retain(|change| {
        checked_paths.contains(&normalize_path(&change.path))
            || change
                .previous_path
                .as_deref()
                .map(normalize_path)
                .is_some_and(|path| checked_paths.contains(&path))
    });

    // A checkpoint may intentionally be captured while its task changes are
    // still uncommitted. Compare those paths with the exact redacted snapshot
    // state instead of treating the same dirty diff as a later modification.
    // This also keeps the baseline valid if the identical content is committed
    // after the checkpoint.
    let root = PathBuf::from(&current.root);
    let baseline_content = baseline
        .content_fingerprints
        .iter()
        .filter_map(|(path, fingerprint)| {
            let path = validated_repository_relative_path(path)?;
            checked_paths
                .contains(&path)
                .then(|| (path, ContentState::from_stored(fingerprint.clone())))
        })
        .collect::<BTreeMap<_, _>>();
    let mut current_content = BTreeMap::new();
    for path in baseline_content.keys() {
        current_content.insert(path.clone(), fingerprint_repository_path(&root, path)?);
    }
    changes.retain(|change| {
        let paths = [Some(change.path.as_str()), change.previous_path.as_deref()]
            .into_iter()
            .flatten()
            .map(normalize_path)
            .collect::<Vec<_>>();
        !paths.iter().all(|path| {
            baseline_content
                .get(path)
                .zip(current_content.get(path))
                .is_some_and(|(baseline, current)| baseline == current)
        })
    });

    for (path, expected) in &baseline_content {
        let Some(actual) = current_content.get(path) else {
            continue;
        };
        if actual == expected
            || changes.iter().any(|change| {
                normalize_path(&change.path) == *path
                    || change
                        .previous_path
                        .as_deref()
                        .map(normalize_path)
                        .as_deref()
                        == Some(path.as_str())
            })
        {
            continue;
        }
        changes.push(FileChangeV1 {
            schema_version: SCHEMA_VERSION_V1,
            repository_id: current.repository_id.clone(),
            session_id: String::new(),
            task_id: None,
            path: path.clone(),
            previous_path: None,
            status: match (expected, actual) {
                (ContentState::Missing, ContentState::Sha256(_)) => ChangeStatus::Added,
                (ContentState::Sha256(_), ContentState::Missing) => ChangeStatus::Deleted,
                (ContentState::Unsupported, _) | (_, ContentState::Unsupported) => {
                    ChangeStatus::TypeChanged
                }
                _ => ChangeStatus::Modified,
            },
            additions: None,
            deletions: None,
            attribution: ChangeAttribution::ObservedChangedIn,
            before_head: baseline.head.clone(),
            after_head: current.head.clone(),
        });
    }

    let expected_absent = files
        .iter()
        .flat_map(|change| match change.status {
            ChangeStatus::Deleted => vec![normalize_path(&change.path)],
            ChangeStatus::Renamed => change
                .previous_path
                .as_deref()
                .map(normalize_path)
                .into_iter()
                .collect(),
            _ => Vec::new(),
        })
        .collect::<BTreeSet<_>>();
    let mut deduped = BTreeMap::new();
    for change in changes {
        if change.status == ChangeStatus::Renamed {
            if let Some(previous) = change.previous_path.as_deref() {
                checked_paths.remove(&normalize_path(previous));
            }
            checked_paths.insert(normalize_path(&change.path));
        }
        deduped.insert((change.path.clone(), change.previous_path.clone()), change);
    }
    let related_changes = deduped.into_values().collect::<Vec<_>>();
    let missing = checked_paths
        .iter()
        .filter(|path| !expected_absent.contains(*path))
        .any(|path| {
            matches!(
                fingerprint_repository_path(&root, path),
                Ok(ContentState::Missing)
            )
        });
    let fingerprint_broken = baseline_content.iter().any(|(path, expected)| {
        matches!(expected, ContentState::Sha256(_))
            && matches!(current_content.get(path), Some(ContentState::Missing))
    });
    let fingerprint_changed = baseline_content
        .iter()
        .any(|(path, expected)| current_content.get(path) != Some(expected));
    let status = if !ancestor {
        TemporalStatusV1::Diverged
    } else if missing
        || fingerprint_broken
        || related_changes
            .iter()
            .any(|change| change.status == ChangeStatus::Deleted)
    {
        TemporalStatusV1::Broken
    } else if related_changes.is_empty() && !fingerprint_changed {
        TemporalStatusV1::Unchanged
    } else {
        TemporalStatusV1::Changed
    };
    Ok(TemporalRevalidationV1 {
        schema_version: SCHEMA_VERSION_V1,
        status,
        baseline_head: baseline.head.clone(),
        current_head: current.head,
        merge_base,
        related_changes,
        checked_paths: checked_paths.into_iter().collect(),
        warnings: Vec::new(),
    })
}

pub fn current_validation(value: &TemporalRevalidationV1) -> CurrentValidationV1 {
    CurrentValidationV1 {
        schema_version: SCHEMA_VERSION_V1,
        status: value.status,
        current_head: value.current_head.clone(),
        verified_paths: value.checked_paths.clone(),
        warnings: value.warnings.clone(),
    }
}

fn commit_exists(repository_path: &Path, commit: &str) -> Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository_path)
        .args(["cat-file", "-e", &format!("{commit}^{{commit}}")])
        .output()
        .context("run git cat-file -e")?;
    Ok(output.status.success())
}

pub fn is_ancestor(
    repository_path: impl AsRef<Path>,
    ancestor: &str,
    descendant: &str,
) -> Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository_path.as_ref())
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .output()
        .context("run git merge-base --is-ancestor")?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(anyhow!(
            "git merge-base --is-ancestor failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

pub fn diff_changes(
    repository_path: &Path,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Vec<FileChangeV1>> {
    let mut name_args = vec!["diff", "--name-status", "-z", "--find-renames"];
    let mut numstat_args = vec!["diff", "--numstat", "--find-renames"];
    match (from, to) {
        (Some(from), Some(to)) => {
            name_args.extend([from, to, "--"]);
            numstat_args.extend([from, to, "--"]);
        }
        (Some(from), None) => {
            name_args.extend([from, "--"]);
            numstat_args.extend([from, "--"]);
        }
        (None, Some(to)) => {
            name_args.extend([to, "--"]);
            numstat_args.extend([to, "--"]);
        }
        (None, None) => {
            name_args.push("--");
            numstat_args.push("--");
        }
    }
    let name_status = git_bytes(repository_path, &name_args)?;
    let numstat = git(repository_path, &numstat_args)?;
    let stats = parse_numstat(&numstat);
    let mut changes = parse_name_status(&name_status);
    for change in &mut changes {
        if let Some((additions, deletions)) = stats.get(&change.path) {
            change.additions = *additions;
            change.deletions = *deletions;
        }
    }
    changes.retain_mut(|change| {
        let Some(path) = validated_repository_relative_path(&change.path) else {
            return false;
        };
        let previous_path = match change.previous_path.as_deref() {
            Some(previous) => {
                let Some(previous) = validated_repository_relative_path(previous) else {
                    return false;
                };
                Some(previous)
            }
            None => None,
        };
        change.path = path;
        change.previous_path = previous_path;
        true
    });
    changes.sort_by(|a, b| (&a.path, &a.previous_path).cmp(&(&b.path, &b.previous_path)));
    Ok(changes)
}

fn parse_name_status(output: &[u8]) -> Vec<FileChangeV1> {
    let fields = output
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| String::from_utf8_lossy(field).into_owned())
        .collect::<Vec<_>>();
    let mut index = 0;
    let mut changes = Vec::new();
    while index < fields.len() {
        let code = &fields[index];
        index += 1;
        let Some(first_path) = fields.get(index).cloned() else {
            break;
        };
        index += 1;
        let status_code = code.chars().next().unwrap_or('?');
        let (status, previous_path, path) = match status_code {
            'R' | 'C' => {
                let Some(second_path) = fields.get(index).cloned() else {
                    break;
                };
                index += 1;
                (
                    if status_code == 'R' {
                        ChangeStatus::Renamed
                    } else {
                        ChangeStatus::Copied
                    },
                    Some(normalize_path(&first_path)),
                    normalize_path(&second_path),
                )
            }
            'A' => (ChangeStatus::Added, None, normalize_path(&first_path)),
            'M' => (ChangeStatus::Modified, None, normalize_path(&first_path)),
            'D' => (ChangeStatus::Deleted, None, normalize_path(&first_path)),
            'T' => (ChangeStatus::TypeChanged, None, normalize_path(&first_path)),
            'U' => (ChangeStatus::Unmerged, None, normalize_path(&first_path)),
            _ => (ChangeStatus::Unknown, None, normalize_path(&first_path)),
        };
        changes.push(FileChangeV1 {
            schema_version: SCHEMA_VERSION_V1,
            repository_id: String::new(),
            session_id: String::new(),
            task_id: None,
            path,
            previous_path,
            status,
            additions: None,
            deletions: None,
            attribution: ChangeAttribution::ObservedChangedIn,
            before_head: None,
            after_head: None,
        });
    }
    changes
}

fn parse_numstat(output: &str) -> BTreeMap<String, (Option<u64>, Option<u64>)> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '\t');
            let additions = fields.next()?;
            let deletions = fields.next()?;
            let path = fields.next()?;
            let path = if path.contains(" => ") {
                path.rsplit(" => ")
                    .next()
                    .unwrap_or(path)
                    .replace(['{', '}'], "")
            } else {
                path.to_string()
            };
            Some((
                normalize_path(&path),
                (additions.parse().ok(), deletions.parse().ok()),
            ))
        })
        .collect()
}

fn parse_status_paths(output: &[u8]) -> Vec<String> {
    let fields = output
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| String::from_utf8_lossy(field).into_owned())
        .collect::<Vec<_>>();
    let mut paths = BTreeSet::new();
    let mut index = 0;
    while index < fields.len() {
        let field = &fields[index];
        if field.len() < 4 {
            index += 1;
            continue;
        }
        let status = &field[..2];
        paths.insert(normalize_path(&field[3..]));
        index += 1;
        if status.contains('R') || status.contains('C') {
            index += 1;
        }
    }
    paths.into_iter().collect()
}

fn git(repository_path: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository_path)
        .args(args)
        .output()
        .with_context(|| format!("run git {}", args.join(" ")))?;
    if !output.status.success() {
        return Err(anyhow!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).context("git output was not UTF-8")
}

fn git_bytes(repository_path: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository_path)
        .args(args)
        .output()
        .with_context(|| format!("run git {}", args.join(" ")))?;
    if !output.status.success() {
        return Err(anyhow!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

fn git_optional(repository_path: &Path, args: &[&str]) -> Option<String> {
    git(repository_path, args).ok()
}

fn normalize_path(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
}

pub(crate) fn validated_repository_relative_path(path: &str) -> Option<String> {
    if path.contains('\0') {
        return None;
    }
    let normalized = path.replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.ends_with('/')
        || normalized.as_bytes().get(1) == Some(&b':')
    {
        return None;
    }
    let components = normalized.split('/').collect::<Vec<_>>();
    if components
        .iter()
        .any(|component| component.is_empty() || *component == "." || *component == "..")
        || crate::redaction::is_sensitive_path(&normalized)
    {
        return None;
    }
    Some(components.join("/"))
}

fn change_signature(
    change: &FileChangeV1,
) -> (ChangeStatus, Option<u64>, Option<u64>, Option<String>) {
    (
        change.status,
        change.additions,
        change.deletions,
        change.previous_path.clone(),
    )
}
