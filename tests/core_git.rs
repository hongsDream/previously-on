use previously_on::domain::{ChangeAttribution, ChangeStatus, Freshness};
use previously_on::git::{
    assess_task_freshness, capture_snapshot, correlate_changes, is_ancestor, repository_identity,
};
use std::process::Command;
use tempfile::TempDir;

fn git(path: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {} failed", args.join(" "));
}

#[test]
fn preserves_rename_and_binary_metadata_without_overclaiming_external_changes() {
    let temp = TempDir::new().unwrap();
    git(temp.path(), &["init", "-q"]);
    git(
        temp.path(),
        &["config", "user.email", "lineage@example.test"],
    );
    git(temp.path(), &["config", "user.name", "Context Lineage"]);
    std::fs::write(temp.path().join("old-name.txt"), "rename me\n").unwrap();
    std::fs::write(temp.path().join("asset.bin"), [0_u8, 1, 2, 3]).unwrap();
    git(temp.path(), &["add", "."]);
    git(temp.path(), &["commit", "-qm", "initial"]);

    let before = capture_snapshot(temp.path()).unwrap();
    git(temp.path(), &["mv", "old-name.txt", "new-name.txt"]);
    std::fs::write(temp.path().join("asset.bin"), [0_u8, 255, 2, 3]).unwrap();
    let after = capture_snapshot(temp.path()).unwrap();
    let changes = correlate_changes(
        temp.path(),
        &before,
        &after,
        "session-rename",
        Some("task-rename"),
        &["new-name.txt".into()],
    )
    .unwrap();

    let renamed = changes
        .iter()
        .find(|change| change.path == "new-name.txt")
        .unwrap();
    assert_eq!(renamed.status, ChangeStatus::Renamed);
    assert_eq!(renamed.previous_path.as_deref(), Some("old-name.txt"));
    assert_eq!(renamed.attribution, ChangeAttribution::ModifiedBy);

    let binary = changes
        .iter()
        .find(|change| change.path == "asset.bin")
        .unwrap();
    assert_eq!(binary.status, ChangeStatus::Modified);
    assert_eq!(binary.additions, None);
    assert_eq!(binary.deletions, None);
    assert_eq!(binary.attribution, ChangeAttribution::ObservedChangedIn);
}

#[test]
fn captures_repo_and_only_claims_causality_for_tool_observed_paths() {
    let temp = TempDir::new().unwrap();
    git(temp.path(), &["init", "-q"]);
    git(
        temp.path(),
        &["config", "user.email", "lineage@example.test"],
    );
    git(temp.path(), &["config", "user.name", "Context Lineage"]);
    std::fs::write(temp.path().join("tracked.txt"), "before\n").unwrap();
    std::fs::write(temp.path().join("external.txt"), "before\n").unwrap();
    git(temp.path(), &["add", "."]);
    git(temp.path(), &["commit", "-qm", "initial"]);

    let identity = repository_identity(temp.path()).unwrap();
    assert_eq!(identity.root, temp.path().canonicalize().unwrap());
    let before = capture_snapshot(temp.path()).unwrap();
    std::fs::write(temp.path().join("tracked.txt"), "after\n").unwrap();
    std::fs::write(temp.path().join("external.txt"), "outside\n").unwrap();
    let after = capture_snapshot(temp.path()).unwrap();
    let changes = correlate_changes(
        temp.path(),
        &before,
        &after,
        "session-1",
        Some("task-1"),
        &["tracked.txt".into()],
    )
    .unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(
        changes
            .iter()
            .find(|change| change.path == "tracked.txt")
            .unwrap()
            .attribution,
        ChangeAttribution::ModifiedBy
    );
    assert_eq!(
        changes
            .iter()
            .find(|change| change.path == "external.txt")
            .unwrap()
            .attribution,
        ChangeAttribution::ObservedChangedIn
    );

    git(temp.path(), &["add", "."]);
    git(temp.path(), &["commit", "-qm", "capture baseline"]);
    let baseline = capture_snapshot(temp.path()).unwrap();
    assert!(is_ancestor(
        temp.path(),
        before.head.as_deref().unwrap(),
        baseline.head.as_deref().unwrap()
    )
    .unwrap());
    assert!(!is_ancestor(
        temp.path(),
        baseline.head.as_deref().unwrap(),
        before.head.as_deref().unwrap()
    )
    .unwrap());
    assert_eq!(
        assess_task_freshness(temp.path(), Some(&baseline), &changes).unwrap(),
        Freshness::Fresh
    );
    std::fs::write(temp.path().join("tracked.txt"), "changed again\n").unwrap();
    assert_eq!(
        assess_task_freshness(temp.path(), Some(&baseline), &changes).unwrap(),
        Freshness::Stale
    );
    std::fs::remove_file(temp.path().join("tracked.txt")).unwrap();
    assert_eq!(
        assess_task_freshness(temp.path(), Some(&baseline), &changes).unwrap(),
        Freshness::Broken
    );
}

#[test]
fn linked_worktrees_share_a_logical_repository_but_keep_distinct_snapshot_roots() {
    let temp = TempDir::new().unwrap();
    let primary = temp.path().join("primary");
    let linked = temp.path().join("linked");
    std::fs::create_dir_all(&primary).unwrap();
    git(&primary, &["init", "-q"]);
    git(
        &primary,
        &["config", "user.email", "previously@example.test"],
    );
    git(&primary, &["config", "user.name", "PreviouslyOn"]);
    std::fs::write(primary.join("tracked.txt"), "primary\n").unwrap();
    git(&primary, &["add", "."]);
    git(&primary, &["commit", "-qm", "initial"]);
    git(
        &primary,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "linked-branch",
            linked.to_str().unwrap(),
        ],
    );

    let primary_identity = repository_identity(&primary).unwrap();
    let linked_identity = repository_identity(&linked).unwrap();
    assert_eq!(primary_identity.id, linked_identity.id);
    assert_eq!(primary_identity.common_dir, linked_identity.common_dir);
    assert_ne!(primary_identity.root, linked_identity.root);

    std::fs::write(linked.join("tracked.txt"), "linked\n").unwrap();
    let linked_snapshot = capture_snapshot(&linked).unwrap();
    assert_eq!(linked_snapshot.repository_id, primary_identity.id);
    assert_eq!(
        linked_snapshot.root,
        linked.canonicalize().unwrap().to_string_lossy()
    );
    assert!(linked_snapshot
        .dirty_files
        .iter()
        .any(|path| path == "tracked.txt"));
    assert!(primary.join("tracked.txt").read_link().is_err());
    assert_eq!(
        std::fs::read_to_string(primary.join("tracked.txt")).unwrap(),
        "primary\n"
    );
}

#[test]
fn detached_head_snapshots_remain_valid_and_conservative() {
    let temp = TempDir::new().unwrap();
    git(temp.path(), &["init", "-q"]);
    git(
        temp.path(),
        &["config", "user.email", "previously@example.test"],
    );
    git(temp.path(), &["config", "user.name", "PreviouslyOn"]);
    std::fs::write(temp.path().join("detached.txt"), "before\n").unwrap();
    git(temp.path(), &["add", "."]);
    git(temp.path(), &["commit", "-qm", "initial"]);
    git(temp.path(), &["checkout", "-q", "--detach", "HEAD"]);

    let before = capture_snapshot(temp.path()).unwrap();
    assert_eq!(before.branch, None);
    std::fs::write(temp.path().join("detached.txt"), "after\n").unwrap();
    let after = capture_snapshot(temp.path()).unwrap();
    let changes =
        correlate_changes(temp.path(), &before, &after, "detached-session", None, &[]).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].attribution, ChangeAttribution::ObservedChangedIn);
}

#[test]
fn rebased_history_never_implies_tool_causality() {
    let temp = TempDir::new().unwrap();
    git(temp.path(), &["init", "-q"]);
    git(
        temp.path(),
        &["config", "user.email", "previously@example.test"],
    );
    git(temp.path(), &["config", "user.name", "PreviouslyOn"]);
    std::fs::write(temp.path().join("base.txt"), "base\n").unwrap();
    git(temp.path(), &["add", "."]);
    git(temp.path(), &["commit", "-qm", "base"]);
    git(temp.path(), &["checkout", "-qb", "feature"]);
    std::fs::write(temp.path().join("feature.txt"), "feature\n").unwrap();
    git(temp.path(), &["add", "."]);
    git(temp.path(), &["commit", "-qm", "feature"]);
    let old_feature = capture_snapshot(temp.path()).unwrap();
    git(temp.path(), &["checkout", "-q", "master"]);
    std::fs::write(temp.path().join("main.txt"), "main\n").unwrap();
    git(temp.path(), &["add", "."]);
    git(temp.path(), &["commit", "-qm", "main"]);
    git(temp.path(), &["checkout", "-q", "feature"]);
    git(temp.path(), &["rebase", "-q", "master"]);
    let rebased = capture_snapshot(temp.path()).unwrap();

    assert!(!is_ancestor(
        temp.path(),
        old_feature.head.as_deref().unwrap(),
        rebased.head.as_deref().unwrap()
    )
    .unwrap());
    let changes = correlate_changes(
        temp.path(),
        &old_feature,
        &rebased,
        "rebase-session",
        None,
        &[],
    )
    .unwrap();
    assert!(changes
        .iter()
        .all(|change| change.attribution == ChangeAttribution::ObservedChangedIn));
}
