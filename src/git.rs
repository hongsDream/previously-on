use crate::domain::{
    ChangeAttribution, ChangeStatus, FileChangeV1, Freshness, GitSnapshotV1, SCHEMA_VERSION_V1,
};
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryIdentity {
    /// Stable identity shared by every linked worktree of the repository.
    pub id: String,
    /// Concrete worktree root used for snapshots and diffs.
    pub root: PathBuf,
    pub common_dir: PathBuf,
    pub remote_url: Option<String>,
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
    )?);
    let mut changes = if let Some(head) = head.as_deref() {
        diff_changes(&identity.root, Some(head), None)?
    } else {
        diff_changes(&identity.root, None, None)?
    };
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
    })
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
        .map(|path| normalize_path(path))
        .collect::<BTreeSet<_>>();
    let mut deduped = BTreeMap::new();
    for mut change in changes {
        change.repository_id = before.repository_id.clone();
        change.session_id = session_id.to_string();
        change.task_id = task_id.map(str::to_string);
        change.before_head = before.head.clone();
        change.after_head = after.head.clone();
        let path = normalize_path(&change.path);
        let previous = change.previous_path.as_deref().map(normalize_path);
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
    let Some(baseline) = baseline else {
        return Ok(Freshness::Stale);
    };
    let current = capture_snapshot(&repository_path)?;
    let root = PathBuf::from(&current.root);
    let tracked_paths = files
        .iter()
        .filter(|change| change.status != ChangeStatus::Deleted)
        .map(|change| normalize_path(&change.path))
        .filter(|path| !path.is_empty() && path != "[REDACTED]")
        .collect::<BTreeSet<_>>();
    if tracked_paths.iter().any(|path| !root.join(path).exists()) {
        return Ok(Freshness::Broken);
    }
    let baseline_changes = baseline
        .working_tree_changes
        .iter()
        .filter(|change| tracked_paths.contains(&normalize_path(&change.path)))
        .map(|change| (normalize_path(&change.path), change_signature(change)))
        .collect::<BTreeMap<_, _>>();
    let current_changes = current
        .working_tree_changes
        .iter()
        .filter(|change| tracked_paths.contains(&normalize_path(&change.path)))
        .map(|change| (normalize_path(&change.path), change_signature(change)))
        .collect::<BTreeMap<_, _>>();
    if baseline.head != current.head || baseline_changes != current_changes {
        return Ok(Freshness::Stale);
    }
    Ok(Freshness::Fresh)
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
