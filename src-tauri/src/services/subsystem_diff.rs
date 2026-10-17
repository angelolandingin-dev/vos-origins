//! Subsystem-versus-main diff: the surface that shows what a promotion left
//! behind.
//!
//! A subsystem's module can be promoted *partially* and nobody notices:
//! commit `A` added files 1-5, later commit `B` edited 4,5 and added 6,7,8,
//! and promoting `B` alone leaves main silently missing 1,2,3. This view
//! lists every path in the subsystem's tracked tree against the target branch
//! tip so the user can see the gap and select exactly the files that were
//! never promoted.
//!
//! Scope: the subsystem's tracked tree at its source tip is the universe —
//! subsystem checkouts carry the full `src/modules/<name>/...` prefix, so
//! paths map 1:1 into the main checkout with no translation table. Files
//! present in main but absent from the subsystem tree are main-owned: out of
//! scope, never listed.
//!
//! Buckets (blob identity only, never timestamps):
//! - `Missing`: in the subsystem tree, absent in main — never promoted.
//! - `Behind`: the blobs differ and main's blob is reachable from the source
//!   tip (a genuine ancestry claim via `rev-list --objects`, not mtime), so
//!   syncing is safe. Reachability is deliberately path-agnostic, like the
//!   promotion engine's own containment check: identical bytes produced by the
//!   subsystem anywhere in its history are subsystem content, never
//!   main-system-only content.
//! - `MainSideChanged`: the blobs differ and main's blob is reachable from no
//!   source tip — main was edited independently. No source commit is ever
//!   claimed for main's version.
//! - `Identical`: same blob on both sides (hidden by default in the UI).
//!
//! Performance: exactly four git spawns for any number of paths — one
//! `ls-tree` per side, one `rev-list --objects` over the source tip, and one
//! `log --raw` walk that yields every path's touching commits newest-first
//! (the same single walk the commit browser uses). Per-branch promotion
//! status for every touching commit reuses
//! [`crate::services::live_read::integration_statuses_live`], whose branch
//! walks are tip-keyed and memoized, so a page of N rows stays O(1) spawns
//! after the memo warms. Never one spawn per path, never one per commit.
//!
//! Two refinements ride on the same batch:
//! - The backwards-write warning ("reverts newer work", information only):
//!   one `rev-list --objects <write>..<tip>` per distinct newest-toucher
//!   among `Behind` rows (linear histories never warn — the target version is
//!   always an ancestor of the write — but still pay the range probe), plus
//!   one naming lookup per warned path (zero when nothing warns).
//! - The on-click file detail (latest commit + newest-first history with
//!   per-branch status) costs zero spawns: every row already carries its
//!   touchers from the single walk, capped at [`MAX_TOUCHES_PER_PATH`]. The
//!   walk has no rename detection, so a renamed path's history starts at its
//!   add commit — the UI states "history since the last rename" rather than
//!   implying a complete past.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::fixtures::now_rfc3339;
use crate::github::models::{
    CommitStatus, DataSource, FileChangeKind, SubsystemDiff, SubsystemDiffBucket,
    SubsystemDiffCommit, SubsystemDiffEntry,
};
use crate::services::engine::github_ops::GitHubOps;
use crate::services::engine::{blobset, git};
use crate::services::live_read;
use crate::services::registry::Registry;

/// Touches kept per path (newest-first). The full touch list also feeds the
/// commit-to-paths index uncapped, so exclusion prefill never loses a path.
pub const MAX_TOUCHES_PER_PATH: usize = 30;

/// Everything the diff needs, read in exactly four git spawns.
pub struct CollectedDiff {
    pub source_tree: HashMap<String, String>,
    pub target_tree: HashMap<String, String>,
    /// Every blob reachable from the source tip (one batched `rev-list`).
    pub history_blobs: HashSet<String>,
    /// `path ->` newest-first `(sha, status char)` touches.
    pub touches: HashMap<String, Vec<(String, char)>>,
    /// Paths whose touch list was capped at [`MAX_TOUCHES_PER_PATH`].
    pub touches_truncated: HashSet<String>,
    /// `sha ->` every path that commit touched (sorted, uncapped).
    pub commit_paths: HashMap<String, Vec<String>>,
    /// `sha -> (subject, author, committed_at)`.
    pub commit_meta: HashMap<String, (String, String, String)>,
    /// Every touching SHA, newest-first.
    pub commit_order: Vec<String>,
}

/// Batched collection: two `ls-tree` (one per side), one `rev-list
/// --objects` over the source tip, one `log --raw` walk for touches.
/// Both `source_tip` and `target_tip` are resolved SHAs (or ref names the
/// caller already resolved); resolving them costs the caller's rev-parses,
/// never per-path work.
pub fn collect_diff(
    source_dir: &Path,
    source_tip: &str,
    target_dir: &Path,
    target_tip: &str,
) -> CollectedDiff {
    let source_tree = blobset::tree_blob_map(source_dir, source_tip);
    let target_tree = blobset::tree_blob_map(target_dir, target_tip);
    let history_blobs = blobset::blobs_in_selected_history(
        source_dir,
        std::slice::from_ref(&source_tip.to_string()),
    );

    let mut touches: HashMap<String, Vec<(String, char)>> = HashMap::new();
    let mut commit_paths: HashMap<String, Vec<String>> = HashMap::new();
    let mut commit_meta: HashMap<String, (String, String, String)> = HashMap::new();
    let mut commit_order: Vec<String> = Vec::new();
    // A failed walk degrades to empty touches (fail-closed downstream: with
    // no touches a path lists no commits, and an empty history set refuses
    // every `Behind` claim as `MainSideChanged` — never a false sync-safe).
    if let Ok(commits) = live_read::parse_source_log(source_dir, source_tip) {
        for commit in &commits {
            commit_order.push(commit.sha.clone());
            commit_meta.insert(
                commit.sha.clone(),
                (
                    commit.subject.clone(),
                    commit.author_name.clone(),
                    commit.committed_at.clone(),
                ),
            );
            for (raw_path, _, status) in &commit.deltas {
                // `--raw` without rename detection never emits multi-path
                // rows, but renames carry `old\tnew`: both sides count as
                // touched, exactly like the engine's delta expansion.
                let paths: Vec<&str> = if *status == 'R' || *status == 'C' {
                    raw_path.split('\t').collect()
                } else {
                    vec![raw_path.as_str()]
                };
                for path in paths {
                    if path.is_empty() {
                        continue;
                    }
                    touches
                        .entry(path.to_string())
                        .or_default()
                        .push((commit.sha.clone(), *status));
                    commit_paths
                        .entry(commit.sha.clone())
                        .or_default()
                        .push(path.to_string());
                }
            }
        }
    }
    let mut touches_truncated = HashSet::new();
    for (path, list) in touches.iter_mut() {
        if list.len() > MAX_TOUCHES_PER_PATH {
            list.truncate(MAX_TOUCHES_PER_PATH);
            touches_truncated.insert(path.clone());
        }
    }
    for paths in commit_paths.values_mut() {
        paths.sort();
        paths.dedup();
    }
    CollectedDiff {
        source_tree,
        target_tree,
        history_blobs,
        touches,
        touches_truncated,
        commit_paths,
        commit_meta,
        commit_order,
    }
}

/// Bucket for one path from blob identity alone. `None` means out of scope:
/// the path is absent from the subsystem tree (main-owned), so it is never
/// listed — in particular never as `Missing`.
pub fn classify_blobs(
    subsystem_blob: Option<&str>,
    main_blob: Option<&str>,
    history_blobs: &HashSet<String>,
) -> Option<SubsystemDiffBucket> {
    match (subsystem_blob, main_blob) {
        (None, _) => None,
        (Some(_), None) => Some(SubsystemDiffBucket::Missing),
        (Some(sub), Some(main)) if sub == main => Some(SubsystemDiffBucket::Identical),
        (Some(_), Some(main)) if history_blobs.contains(main) => Some(SubsystemDiffBucket::Behind),
        (Some(_), Some(_)) => Some(SubsystemDiffBucket::MainSideChanged),
    }
}

/// The change kind shown per path is what the subsystem last did to it: the
/// newest touching commit's status char. Paths with no touch inside the
/// source-log window fall back to tree presence.
pub fn change_kind_of(newest_status: Option<char>, main_present: bool) -> FileChangeKind {
    match newest_status {
        Some('A') => FileChangeKind::Added,
        Some('D') => FileChangeKind::Deleted,
        Some('R') | Some('C') => FileChangeKind::Renamed,
        Some(_) => FileChangeKind::Modified,
        None if !main_present => FileChangeKind::Added,
        None => FileChangeKind::Modified,
    }
}

/// The resolved promotion for a path selection: the union of **unpromoted**
/// commits touching the selected paths (newest-first), plus the exclusions
/// prefill — every other path those same commits touch that is *not*
/// selected, so the user never hand-maintains that list.
pub struct SelectionResolution {
    pub commits: Vec<String>,
    pub excluded_paths: Vec<String>,
    /// Selected paths classified main-side changed. They contribute their
    /// real subsystem touchers like any other path, but main's version came
    /// from nowhere in subsystem history — the UI must say so plainly rather
    /// than present the commits as clean sources.
    pub main_side_selected: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn resolve_selection(
    selected: &HashSet<String>,
    touches: &HashMap<String, Vec<String>>,
    promoted: &HashSet<String>,
    commit_paths: &HashMap<String, Vec<String>>,
    commit_order: &[String],
    main_side: &HashSet<String>,
) -> SelectionResolution {
    let mut wanted: HashSet<String> = HashSet::new();
    for path in selected {
        if let Some(shas) = touches.get(path) {
            for sha in shas {
                if !promoted.contains(sha) {
                    wanted.insert(sha.clone());
                }
            }
        }
    }
    let mut commits: Vec<String> = commit_order
        .iter()
        .filter(|sha| wanted.contains(*sha))
        .cloned()
        .collect();
    // `commit_order` covers every toucher, but stay total if a caller passes
    // a SHA from elsewhere: append the stragglers sorted.
    let mut stragglers: Vec<String> = wanted
        .iter()
        .filter(|sha| !commits.contains(sha))
        .cloned()
        .collect();
    stragglers.sort();
    commits.extend(stragglers);

    let mut excluded: HashSet<String> = HashSet::new();
    for sha in &commits {
        if let Some(paths) = commit_paths.get(sha) {
            for path in paths {
                if !selected.contains(path) {
                    excluded.insert(path.clone());
                }
            }
        }
    }
    let mut excluded_paths: Vec<String> = excluded.into_iter().collect();
    excluded_paths.sort();
    let mut main_side_selected: Vec<String> = selected
        .iter()
        .filter(|path| main_side.contains(*path))
        .cloned()
        .collect();
    main_side_selected.sort();
    SelectionResolution {
        commits,
        excluded_paths,
        main_side_selected,
    }
}

/// Full diff for one subsystem against one target branch: batched collection
/// plus one batched per-branch status call for every touching commit, reusing
/// the existing status machinery (reachable trailers + open-PR overlay —
/// never a second opinion).
pub fn subsystem_diff_live(
    target_dir: &Path,
    source_dir: &Path,
    registry: &Registry,
    github: &dyn GitHubOps,
    subsystem_id: &str,
    target_branch: &str,
) -> AppResult<SubsystemDiff> {
    let entry = live_read::find_live_subsystem(registry, subsystem_id)?;
    let branch = live_read::require_allowed_target(registry, target_branch)?;
    let target_ref = git::resolve_branch_ref(target_dir, &branch).ok_or_else(|| {
        AppError::new(
            ErrorCode::BranchNotPermitted,
            format!("Target branch '{branch}' does not exist in the target checkout."),
        )
        .with_remedy("Choose a target branch that exists and is allowed by promotion.allowedTargetBranches / allowedTargetBranchPatterns.")
    })?;
    let target_tip = git::rev_parse(target_dir, &target_ref).ok_or_else(|| {
        AppError::new(
            ErrorCode::BranchNotPermitted,
            format!("Target branch '{branch}' does not exist in the target checkout."),
        )
        .with_remedy("Choose a target branch that exists and is allowed by promotion.allowedTargetBranches / allowedTargetBranchPatterns.")
    })?;
    let source_tip = git::rev_parse(source_dir, &entry.source_branch).ok_or_else(|| {
        AppError::new(
            ErrorCode::RepositoryInaccessible,
            format!(
                "Could not read source branch '{}' for subsystem '{}.",
                entry.source_branch, entry.id
            ),
        )
        .with_remedy("Confirm the subsystem sourceBranch exists and is readable.")
    })?;

    let collected = collect_diff(source_dir, &source_tip, target_dir, &target_tip);
    if collected.source_tree.is_empty() {
        return Err(AppError::new(
            ErrorCode::RepositoryInaccessible,
            format!(
                "Source branch '{}' has no tracked files for subsystem '{}'.",
                entry.source_branch, entry.id
            ),
        )
        .with_remedy("Confirm the subsystem sourceBranch points at the module tree."));
    }
    if collected.history_blobs.is_empty() {
        // Fail closed: an unreadable object list must never mass-label paths
        // `MainSideChanged` (every `Behind` claim would silently flip).
        return Err(AppError::new(
            ErrorCode::RepositoryInaccessible,
            format!(
                "Could not read the source history behind '{}' for subsystem '{}'.",
                entry.source_branch, entry.id
            ),
        )
        .with_remedy("Confirm the source checkout is a readable git clone, then retry."));
    }

    // One batched status call for every distinct touching commit on this
    // branch — the same reachable-trailer + open-PR machinery the commit
    // browser and promotion guards use.
    let mut shas: Vec<String> = collected.commit_meta.keys().cloned().collect();
    shas.sort();
    let statuses = live_read::integration_statuses_live(
        target_dir,
        registry,
        github,
        &branch,
        Some(&entry.id),
        &shas,
    )?;
    let mut status_by_sha: HashMap<String, CommitStatus> = HashMap::new();
    for status in statuses {
        status_by_sha.insert(status.source_sha.clone(), status.status);
    }

    let mut paths: Vec<&String> = collected.source_tree.keys().collect();
    paths.sort();
    // First pass: buckets only, gathering the distinct write commits (newest
    // toucher per differing path) for the backwards-write check below.
    struct Row {
        path: String,
        bucket: SubsystemDiffBucket,
        change: FileChangeKind,
        main_blob: Option<String>,
        write: Option<String>,
    }
    let mut rows: Vec<Row> = Vec::with_capacity(paths.len());
    for path in &paths {
        let sub_blob = collected.source_tree.get(*path);
        let main_blob = collected.target_tree.get(*path).map(String::as_str);
        let Some(bucket) = classify_blobs(
            sub_blob.map(String::as_str),
            main_blob,
            &collected.history_blobs,
        ) else {
            continue;
        };
        let path_touches = collected.touches.get(*path);
        let newest = path_touches.and_then(|list| list.first());
        rows.push(Row {
            path: (*path).clone(),
            bucket,
            change: change_kind_of(newest.map(|(_, c)| *c), main_blob.is_some()),
            main_blob: main_blob.map(str::to_string),
            // The write producing the current content is the newest toucher.
            // Only `Behind` rows can warn: missing paths have no target blob,
            // identical paths already match, and main-side blobs sit outside
            // subsystem history (so outside every newer-set) by definition.
            write: match (&main_blob, sub_blob.map(String::as_str), newest) {
                (Some(main), Some(sub), Some((sha, _)))
                    if *main != sub && bucket == SubsystemDiffBucket::Behind =>
                {
                    Some(sha.clone())
                }
                _ => None,
            },
        });
    }
    // "Reverts newer work" (warning only, never a refusal): one
    // `rev-list --objects <write>..<tip>` per distinct write, intersected in
    // memory with the target blobs. A path warns when main holds content from
    // a LATER subsystem commit than the write producing the subsystem
    // version. Main-side blobs are outside history by definition, so they can
    // never warn — only genuinely newer subsystem content does.
    let mut writes: Vec<String> = rows.iter().filter_map(|row| row.write.clone()).collect();
    writes.sort();
    writes.dedup();
    let newer_by_write = blobset::blobs_after(source_dir, &writes, &source_tip);
    let mut warned: HashMap<String, crate::github::models::RevertsNewerWork> = HashMap::new();
    for row in &rows {
        let (Some(write), Some(target_blob)) = (row.write.as_deref(), row.main_blob.as_deref())
        else {
            continue;
        };
        let newer = newer_by_write
            .get(write)
            .is_some_and(|set| set.contains(target_blob));
        if !newer {
            continue;
        }
        // One naming lookup per warned path (zero when nothing warns): the
        // warning names the newer commit so the operator can verify it.
        // Unresolvable falls back to a nameless warning rather than a guess.
        let (sha, subject) =
            blobset::newest_commit_for_blob(source_dir, target_blob, &source_tip, &row.path)
                .unwrap_or_default();
        warned.insert(
            row.path.clone(),
            crate::github::models::RevertsNewerWork { sha, subject },
        );
    }

    let mut entries: Vec<SubsystemDiffEntry> = Vec::with_capacity(rows.len());
    for row in rows {
        let path_touches = collected.touches.get(&row.path);
        let mut commits: Vec<SubsystemDiffCommit> = Vec::new();
        if let Some(list) = path_touches {
            for (sha, _) in list {
                let (subject, author, committed_at) =
                    collected.commit_meta.get(sha).cloned().unwrap_or_default();
                let status = status_by_sha.get(sha).cloned().unwrap_or_else(|| {
                    CommitStatus::unknown(
                        &branch,
                        "This SHA could not be verified against the target branch history.",
                    )
                });
                commits.push(SubsystemDiffCommit {
                    sha: sha.clone(),
                    short_sha: sha.chars().take(7).collect(),
                    subject,
                    author_name: author,
                    committed_at,
                    status,
                    touched_paths: collected.commit_paths.get(sha).cloned().unwrap_or_default(),
                });
            }
        }
        let sub_blob = collected.source_tree.get(&row.path);
        entries.push(SubsystemDiffEntry {
            path: row.path.clone(),
            bucket: row.bucket,
            change: row.change,
            subsystem_blob: sub_blob.cloned(),
            main_blob: row.main_blob,
            commits_truncated: collected.touches_truncated.contains(&row.path),
            commits,
            reverts_newer: warned.remove(&row.path),
        });
    }
    Ok(SubsystemDiff {
        subsystem_id: entry.id.clone(),
        source_branch: entry.source_branch.clone(),
        source_tip,
        target_branch: branch,
        target_tip,
        entries,
        fetched_at: now_rfc3339(),
        data_source: DataSource::Live,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn tmp(prefix: &str) -> std::path::PathBuf {
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("vos-diff-{prefix}-{n}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        git_dated(dir, args, None)
    }

    fn git_dated(dir: &Path, args: &[&str], date: Option<&str>) -> String {
        let mut command = Command::new("git");
        command.arg("-C").arg(dir).args(args);
        if let Some(stamp) = date {
            command.env("GIT_AUTHOR_DATE", stamp);
            command.env("GIT_COMMITTER_DATE", stamp);
        }
        let out = command.output().expect("spawn git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn init(dir: &Path) {
        std::fs::create_dir_all(dir).expect("mkdir repo");
        git(dir, &["init", "-q", "-b", "master"]);
        git(dir, &["config", "user.email", "t@example.com"]);
        git(dir, &["config", "user.name", "T"]);
        git(dir, &["config", "commit.gpgsign", "false"]);
    }

    fn write(dir: &Path, rel: &str, content: &str) {
        let abs = dir.join(rel);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).expect("mkdir parent");
        }
        std::fs::write(&abs, content).expect("write");
    }

    fn commit_all(dir: &Path, message: &str) -> String {
        commit_all_at(dir, message, None)
    }

    fn commit_all_at(dir: &Path, message: &str, date: Option<&str>) -> String {
        git(dir, &["add", "-A"]);
        git_dated(
            dir,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.com",
                "commit",
                "--no-verify",
                "-q",
                "-m",
                message,
            ],
            date,
        );
        git(dir, &["rev-parse", "HEAD"])
    }

    fn bucket_of(collected: &CollectedDiff, path: &str) -> Option<SubsystemDiffBucket> {
        classify_blobs(
            collected.source_tree.get(path).map(String::as_str),
            collected.target_tree.get(path).map(String::as_str),
            &collected.history_blobs,
        )
    }

    fn test_registry() -> Registry {
        let text = r#"{
  "mainRepository": "ORG/main-system",
  "promotion": {
    "allowedTargetBranches": ["master"],
    "allowedTargetBranchPatterns": [],
    "requirePullRequest": true
  },
  "subsystems": [{
      "id": "mod",
      "name": "Module",
      "repository": "ORG/mod",
      "sourceBranch": "master",
      "enabled": true,
      "pathStrategy": "preserve-relative-paths",
      "dependencyStrategy": "reconcile",
      "integrationPolicy": {
        "applicationFiles": "selected-commit-delta",
        "packageJson": "merge-dependency-delta",
        "lockfiles": "regenerate-in-target",
        "sharedConfiguration": "review-explicitly",
        "githubWorkflows": "main-owned",
        "secretFiles": "never-import"
      }
    }]
}"#;
        Registry::from_json(text, "test").expect("registry")
    }

    /// The headline scenario: `A` added f0..f5, `B` edited f0/f4/f5 and added
    /// f6..f8, and main promoted `B`'s files only (plus f0's older version and
    /// a main-only file).
    fn partial_promotion_repos() -> (std::path::PathBuf, std::path::PathBuf, String, String) {
        let src = tmp("src");
        init(&src);
        for i in 0..6 {
            write(&src, &format!("f{i}.txt"), "v1\n");
        }
        let a = commit_all(&src, "A adds f0 to f5");
        write(&src, "f0.txt", "v2\n");
        write(&src, "f4.txt", "v2\n");
        write(&src, "f5.txt", "v2\n");
        write(&src, "f6.txt", "v1\n");
        write(&src, "f7.txt", "v1\n");
        write(&src, "f8.txt", "v1\n");
        let b = commit_all(&src, "B edits f0 f4 f5 and adds f6 f7 f8");

        let main = tmp("main");
        init(&main);
        // Main holds f0's OLDER version plus B's files: a partial promotion.
        write(&main, "f0.txt", "v1\n");
        write(&main, "f4.txt", "v2\n");
        write(&main, "f5.txt", "v2\n");
        write(&main, "f6.txt", "v1\n");
        write(&main, "f7.txt", "v1\n");
        write(&main, "f8.txt", "v1\n");
        write(&main, "extra.txt", "main-owned\n");
        let _ = commit_all(&main, "main partial: B files plus f0 v1");
        (src, main, a, b)
    }

    #[test]
    fn partial_promotion_leaves_missing_visible() {
        let (src, main, a, b) = partial_promotion_repos();
        let src_tip = git(&src, &["rev-parse", "HEAD"]);
        let main_tip = git(&main, &["rev-parse", "HEAD"]);
        let collected = collect_diff(&src, &src_tip, &main, &main_tip);

        // The headline case: never promoted, silently missing.
        for missing in ["f1.txt", "f2.txt", "f3.txt"] {
            assert_eq!(
                bucket_of(&collected, missing),
                Some(SubsystemDiffBucket::Missing)
            );
            // Missing rows list the commits that would promote them (A).
            assert_eq!(
                collected.touches.get(missing).map(|t| t.len()),
                Some(1),
                "{missing} is touched only by A"
            );
            assert_eq!(collected.touches[missing][0].0, a);
        }
        // Main's f0 is an ancestor version (v1 behind tip v2): safe to sync.
        assert_eq!(
            bucket_of(&collected, "f0.txt"),
            Some(SubsystemDiffBucket::Behind)
        );
        // Promoted files match exactly.
        for same in ["f4.txt", "f5.txt", "f6.txt", "f7.txt", "f8.txt"] {
            assert_eq!(
                bucket_of(&collected, same),
                Some(SubsystemDiffBucket::Identical)
            );
        }
        // Touches are newest-first: the detail's latest commit needs no query.
        assert_eq!(
            collected.touches["f4.txt"]
                .iter()
                .map(|(s, _)| s.clone())
                .collect::<Vec<_>>(),
            vec![b.clone(), a.clone()]
        );
        assert_eq!(
            collected.touches["f0.txt"]
                .iter()
                .map(|(s, _)| s.clone())
                .collect::<Vec<_>>(),
            vec![b.clone(), a.clone()]
        );
        // Main-only files are out of scope: never listed, never Missing.
        assert!(!collected.source_tree.contains_key("extra.txt"));
        assert_eq!(bucket_of(&collected, "extra.txt"), None);
    }

    #[test]
    fn newer_dated_target_with_older_content_is_still_behind() {
        // Blob identity decides, dates never do: the target commit is NEWER
        // by committer date but holds OLDER content — still Behind.
        let src = tmp("srcdate");
        init(&src);
        write(&src, "f.txt", "v1\n");
        let _a = commit_all_at(&src, "A base", Some("2020-01-01T00:00:00Z"));
        write(&src, "f.txt", "v2\n");
        let _b = commit_all_at(&src, "B edit", Some("2020-02-01T00:00:00Z"));

        let main = tmp("maindate");
        init(&main);
        write(&main, "f.txt", "v1\n");
        // Deliberately newer date, older content.
        let _ = commit_all_at(
            &main,
            "main newer date older content",
            Some("2030-01-01T00:00:00Z"),
        );

        let src_tip = git(&src, &["rev-parse", "HEAD"]);
        let main_tip = git(&main, &["rev-parse", "HEAD"]);
        let collected = collect_diff(&src, &src_tip, &main, &main_tip);
        assert_eq!(
            bucket_of(&collected, "f.txt"),
            Some(SubsystemDiffBucket::Behind),
            "a newer-dated target holding older content is behind, not main-side changed"
        );
    }

    #[test]
    fn main_side_changed_never_claims_a_source_commit() {
        let (src, main, _a, _b) = partial_promotion_repos();
        // Main independently edits f4: its blob exists nowhere in subsystem history.
        write(&main, "f4.txt", "main-side edit\n");
        let _ = commit_all(&main, "main independent edit");
        let src_tip = git(&src, &["rev-parse", "HEAD"]);
        let main_tip = git(&main, &["rev-parse", "HEAD"]);
        let collected = collect_diff(&src, &src_tip, &main, &main_tip);
        assert_eq!(
            bucket_of(&collected, "f4.txt"),
            Some(SubsystemDiffBucket::MainSideChanged)
        );

        // Resolution contributes f4's REAL subsystem touchers (for the
        // overwrite the operator can choose) but flags the path, so the UI
        // says plainly that no subsystem commit produced main's copy.
        let mut touches: HashMap<String, Vec<String>> = HashMap::new();
        for (path, list) in &collected.touches {
            touches.insert(path.clone(), list.iter().map(|(s, _)| s.clone()).collect());
        }
        let selected: HashSet<String> = ["f4.txt".to_string()].into_iter().collect();
        let main_side: HashSet<String> = ["f4.txt".to_string()].into_iter().collect();
        let resolution = resolve_selection(
            &selected,
            &touches,
            &HashSet::new(),
            &collected.commit_paths,
            &collected.commit_order,
            &main_side,
        );
        assert_eq!(resolution.main_side_selected, vec!["f4.txt".to_string()]);
        assert!(
            !resolution.commits.is_empty(),
            "real touchers are listed, never synthesised"
        );
    }

    #[test]
    fn selecting_missing_yields_union_plus_prefilled_exclusions() {
        let (src, main, a, _b) = partial_promotion_repos();
        let src_tip = git(&src, &["rev-parse", "HEAD"]);
        let main_tip = git(&main, &["rev-parse", "HEAD"]);
        let collected = collect_diff(&src, &src_tip, &main, &main_tip);
        let mut touches: HashMap<String, Vec<String>> = HashMap::new();
        for (path, list) in &collected.touches {
            touches.insert(path.clone(), list.iter().map(|(s, _)| s.clone()).collect());
        }
        // "I just select all the ones that werent promoted": every Missing file.
        let selected: HashSet<String> = ["f1.txt", "f2.txt", "f3.txt"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let resolution = resolve_selection(
            &selected,
            &touches,
            &HashSet::new(),
            &collected.commit_paths,
            &collected.commit_order,
            &HashSet::new(),
        );
        // The union is exactly {A}: B touched none of the selected paths.
        assert_eq!(resolution.commits, vec![a.clone()]);
        // ...and A also touched f0/f4/f5, which the user did not select:
        // pre-excluded so the promotion writes only what was chosen.
        assert_eq!(
            resolution.excluded_paths,
            vec![
                "f0.txt".to_string(),
                "f4.txt".to_string(),
                "f5.txt".to_string()
            ]
        );
        assert!(resolution.main_side_selected.is_empty());
    }

    #[test]
    fn promoted_commits_stay_out_of_the_union() {
        let (src, main, a, _b) = partial_promotion_repos();
        let src_tip = git(&src, &["rev-parse", "HEAD"]);
        let main_tip = git(&main, &["rev-parse", "HEAD"]);
        let collected = collect_diff(&src, &src_tip, &main, &main_tip);
        let mut touches: HashMap<String, Vec<String>> = HashMap::new();
        for (path, list) in &collected.touches {
            touches.insert(path.clone(), list.iter().map(|(s, _)| s.clone()).collect());
        }
        // A is already promoted on this branch: selecting its files resolves
        // to nothing (nothing left to promote), not to A again.
        let promoted: HashSet<String> = [a.clone()].into_iter().collect();
        let selected: HashSet<String> = ["f1.txt".to_string()].into_iter().collect();
        let resolution = resolve_selection(
            &selected,
            &touches,
            &promoted,
            &collected.commit_paths,
            &collected.commit_order,
            &HashSet::new(),
        );
        assert!(resolution.commits.is_empty());
        assert!(resolution.excluded_paths.is_empty());
    }

    #[test]
    fn touches_are_newest_first_and_capped() {
        let src = tmp("cap");
        init(&src);
        write(&src, "f.txt", "v0\n");
        let _ = commit_all(&src, "base");
        let mut shas: Vec<String> = Vec::new();
        for i in 1..=35 {
            write(&src, "f.txt", &format!("v{i}\n"));
            shas.push(commit_all(&src, &format!("edit {i}")));
        }
        let main = tmp("capmain");
        init(&main);
        write(&main, "other.txt", "x\n");
        let _ = commit_all(&main, "main base");

        let src_tip = git(&src, &["rev-parse", "HEAD"]);
        let main_tip = git(&main, &["rev-parse", "HEAD"]);
        let collected = collect_diff(&src, &src_tip, &main, &main_tip);
        let list = &collected.touches["f.txt"];
        // Bounded: the 30 most recent touches, newest first — the click
        // detail renders these with zero extra spawns.
        assert_eq!(list.len(), MAX_TOUCHES_PER_PATH);
        assert!(collected.touches_truncated.contains("f.txt"));
        let got: Vec<String> = list.iter().map(|(s, _)| s.clone()).collect();
        let mut expected = shas.clone();
        expected.reverse();
        expected.truncate(MAX_TOUCHES_PER_PATH);
        assert_eq!(got, expected);
    }

    #[test]
    fn rename_starts_a_new_history_window() {
        // The walk has no rename detection: a rename reads as delete + add,
        // so the new path's history starts at the rename commit. The UI
        // states "history since the last rename" rather than implying more.
        let src = tmp("rename");
        init(&src);
        write(&src, "old.txt", "content\n");
        let a = commit_all(&src, "A adds old");
        git(&src, &["mv", "old.txt", "new.txt"]);
        let b = commit_all(&src, "B renames to new");
        let main = tmp("renamemain");
        init(&main);
        write(&main, "other.txt", "x\n");
        let _ = commit_all(&main, "main base");

        let src_tip = git(&src, &["rev-parse", "HEAD"]);
        let main_tip = git(&main, &["rev-parse", "HEAD"]);
        let collected = collect_diff(&src, &src_tip, &main, &main_tip);
        assert_eq!(
            collected.touches.get("new.txt").map(|t| t.len()),
            Some(1),
            "the renamed path's history starts at the rename"
        );
        assert_eq!(collected.touches["new.txt"][0].0, b);
        assert_eq!(
            bucket_of(&collected, "new.txt"),
            Some(SubsystemDiffBucket::Missing)
        );
        let _ = a;
    }

    #[test]
    fn collect_costs_four_spawns_for_many_paths() {
        let src = tmp("spawnsrc");
        init(&src);
        for i in 0..12 {
            write(&src, &format!("f{i}.txt"), "v1\n");
        }
        let _ = commit_all(&src, "base");
        for i in 0..12 {
            write(&src, &format!("f{i}.txt"), "v2\n");
        }
        let _ = commit_all(&src, "edit all");
        let main = tmp("spawnmain");
        init(&main);
        write(&main, "other.txt", "x\n");
        let _ = commit_all(&main, "main base");

        let src_tip = git(&src, &["rev-parse", "HEAD"]);
        let main_tip = git(&main, &["rev-parse", "HEAD"]);
        let before_src = git::git_spawns_for_tests(&src);
        let before_main = git::git_spawns_for_tests(&main);
        let collected = collect_diff(&src, &src_tip, &main, &main_tip);
        let after_src = git::git_spawns_for_tests(&src);
        let after_main = git::git_spawns_for_tests(&main);
        assert_eq!(
            after_src - before_src,
            3,
            "source side: one ls-tree + one rev-list + one log, never one per path"
        );
        assert_eq!(after_main - before_main, 1, "target side: one ls-tree");
        assert_eq!(collected.source_tree.len(), 12);
    }

    #[test]
    fn merged_sibling_content_warns_with_the_newer_commit_named() {
        // A(f=v1); B child (f=v2, newer date); C child of A (f=v3, older
        // date); M merges B+C keeping v2; D (other file) is the tip. Main
        // holds v3: reachable from the tip but NOT from the write (B), so
        // writing the tip version moves f backwards over C's content.
        let src = tmp("mergesrc");
        init(&src);
        write(&src, "f.txt", "v1\n");
        let _a = commit_all_at(&src, "A base", Some("2020-01-01T00:00:00Z"));
        git(&src, &["branch", "side"]);
        write(&src, "f.txt", "v2\n");
        let b = commit_all_at(&src, "B version", Some("2020-03-01T00:00:00Z"));
        git(&src, &["checkout", "-q", "side"]);
        write(&src, "f.txt", "v3\n");
        let c = commit_all_at(&src, "C version", Some("2020-02-01T00:00:00Z"));
        git(&src, &["checkout", "-q", "master"]);
        // Keep B's v2 across the merge: the merge itself must not touch f.
        git(&src, &["merge", "-q", "-X", "ours", "--no-edit", "side"]);
        write(&src, "other.txt", "other\n");
        let _d = commit_all(&src, "D other file");

        let main = tmp("mergemain");
        init(&main);
        write(&main, "f.txt", "v3\n");
        let _ = commit_all(&main, "main holds v3");

        let registry = test_registry();
        let github = crate::services::engine::github_ops::StubGitHubOps::new();
        let diff =
            subsystem_diff_live(&main, &src, &registry, &github, "mod", "master").expect("diff");
        let entry = diff
            .entries
            .iter()
            .find(|e| e.path == "f.txt")
            .expect("f.txt listed");
        assert_eq!(entry.bucket, SubsystemDiffBucket::Behind);
        // The newest toucher is B (newer committer date): the log order the
        // detail renders is [B, C, A], newest first.
        let order: Vec<String> = entry
            .commits
            .iter()
            .map(|commit| commit.sha.clone())
            .collect();
        assert_eq!(order.first().cloned(), Some(b.clone()));
        assert!(order.contains(&c));
        let merge_sha = git(&src, &["rev-parse", "master~1"]);
        assert!(
            !order.contains(&merge_sha),
            "the merge keeps B's version, so it touches nothing: {order:?}"
        );
        for commit in &entry.commits {
            assert_eq!(
                commit.status.kind,
                crate::github::models::CommitStatusKind::Eligible,
                "statuses reuse the per-branch machinery (no trailers: eligible)"
            );
        }
        let warn = entry
            .reverts_newer
            .as_ref()
            .expect("sibling content newer than the write warns");
        assert_eq!(warn.sha, c, "the warning names the newer commit");
        assert_eq!(warn.subject, "C version");
    }

    #[test]
    fn assembly_lists_entries_with_statuses_newest_first() {
        let (src, main, a, b) = partial_promotion_repos();
        let registry = test_registry();
        let github = crate::services::engine::github_ops::StubGitHubOps::new();
        let diff =
            subsystem_diff_live(&main, &src, &registry, &github, "mod", "master").expect("diff");
        assert_eq!(diff.subsystem_id, "mod");
        assert_eq!(diff.target_branch, "master");
        assert!(!diff.source_tip.is_empty() && !diff.target_tip.is_empty());
        let by_path: HashMap<&str, &SubsystemDiffEntry> =
            diff.entries.iter().map(|e| (e.path.as_str(), e)).collect();
        // The universe is the subsystem tree: main-only extra.txt is absent.
        assert!(!by_path.contains_key("extra.txt"));
        let f1 = by_path["f1.txt"];
        assert_eq!(f1.bucket, SubsystemDiffBucket::Missing);
        assert_eq!(f1.change, crate::github::models::FileChangeKind::Added);
        assert_eq!(f1.commits.len(), 1);
        assert_eq!(f1.commits[0].sha, a);
        assert_eq!(
            f1.commits[0].status.kind,
            crate::github::models::CommitStatusKind::Eligible
        );
        // The exclusion prefill rides on touched_paths: A touched f0..f5.
        assert_eq!(
            f1.commits[0].touched_paths,
            vec![
                "f0.txt".to_string(),
                "f1.txt".to_string(),
                "f2.txt".to_string(),
                "f3.txt".to_string(),
                "f4.txt".to_string(),
                "f5.txt".to_string(),
            ]
        );
        let f4 = by_path["f4.txt"];
        assert_eq!(f4.bucket, SubsystemDiffBucket::Identical);
        assert_eq!(f4.change, crate::github::models::FileChangeKind::Modified);
        assert_eq!(
            f4.commits
                .iter()
                .map(|commit| commit.sha.clone())
                .collect::<Vec<_>>(),
            vec![b.clone(), a.clone()],
            "history is newest-first for the click detail"
        );
        assert!(f4.reverts_newer.is_none());
        let f0 = by_path["f0.txt"];
        assert_eq!(f0.bucket, SubsystemDiffBucket::Behind);
        // An ancestor target version never warns (proven again at the entry
        // level: v1 is reachable from the write B).
        assert!(f0.reverts_newer.is_none());
        // Entries arrive sorted by path for a stable render.
        let mut sorted: Vec<&str> = by_path.keys().copied().collect();
        sorted.sort();
        let in_order: Vec<&str> = diff.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(in_order, sorted);
    }

    #[test]
    fn diff_wire_keys_match_typescript_contract() {
        use crate::github::models::{
            CommitStatus, CommitStatusKind, DataSource, FileChangeKind, RevertsNewerWork,
            SubsystemDiff, SubsystemDiffBucket, SubsystemDiffCommit, SubsystemDiffEntry,
        };

        fn sorted_keys(value: &serde_json::Value) -> Vec<String> {
            let mut keys: Vec<String> = value
                .as_object()
                .expect("value must be an object")
                .keys()
                .cloned()
                .collect();
            keys.sort();
            keys
        }

        // A dropped `rename_all` blanked this UI once with both gates green:
        // `cargo test` cannot see TS and `tsc` cannot see serde, so the exact
        // camelCase key sets are pinned here against `src/api/types.ts`.
        let diff = SubsystemDiff {
            subsystem_id: "mod".to_string(),
            source_branch: "master".to_string(),
            source_tip: "a".repeat(40),
            target_branch: "staging".to_string(),
            target_tip: "b".repeat(40),
            entries: vec![SubsystemDiffEntry {
                path: "src/modules/mod/a.ts".to_string(),
                bucket: SubsystemDiffBucket::MainSideChanged,
                change: FileChangeKind::Modified,
                subsystem_blob: Some("c".repeat(40)),
                main_blob: Some("d".repeat(40)),
                commits: vec![SubsystemDiffCommit {
                    sha: "e".repeat(40),
                    short_sha: "e".repeat(7),
                    subject: "subject".to_string(),
                    author_name: "T".to_string(),
                    committed_at: "2026-01-01T00:00:00Z".to_string(),
                    status: CommitStatus::of(
                        CommitStatusKind::Eligible,
                        "staging",
                        Some("detail".to_string()),
                        None,
                    ),
                    touched_paths: vec!["src/modules/mod/a.ts".to_string()],
                }],
                commits_truncated: false,
                reverts_newer: Some(RevertsNewerWork {
                    sha: "f".repeat(40),
                    subject: "newer work".to_string(),
                }),
            }],
            fetched_at: "2026-01-01T00:00:00Z".to_string(),
            data_source: DataSource::Live,
        };
        let value = serde_json::to_value(&diff).expect("diff serializes");
        assert_eq!(
            sorted_keys(&value),
            vec![
                "dataSource",
                "entries",
                "fetchedAt",
                "sourceBranch",
                "sourceTip",
                "subsystemId",
                "targetBranch",
                "targetTip",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>(),
        );
        let entry = &value["entries"][0];
        assert_eq!(
            sorted_keys(entry),
            vec![
                "bucket",
                "change",
                "commits",
                "commitsTruncated",
                "mainBlob",
                "path",
                "revertsNewer",
                "subsystemBlob",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>(),
        );
        assert_eq!(
            entry["bucket"].as_str(),
            Some("main_side_changed"),
            "bucket serializes snake_case"
        );
        assert_eq!(
            sorted_keys(&entry["commits"][0]),
            vec![
                "authorName",
                "committedAt",
                "sha",
                "shortSha",
                "status",
                "subject",
                "touchedPaths",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>(),
        );
        assert_eq!(
            sorted_keys(&entry["commits"][0]["status"]),
            vec!["detail", "evidence", "kind", "label", "targetBranch",]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            sorted_keys(&entry["revertsNewer"]),
            vec!["sha", "subject",]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>(),
        );
        // Every bucket variant pins its wire string.
        for (bucket, wire) in [
            (SubsystemDiffBucket::Missing, "missing"),
            (SubsystemDiffBucket::Behind, "behind"),
            (SubsystemDiffBucket::MainSideChanged, "main_side_changed"),
            (SubsystemDiffBucket::Identical, "identical"),
        ] {
            let serialized = serde_json::to_value(bucket).expect("bucket serializes");
            assert_eq!(serialized.as_str(), Some(wire));
        }
    }
}
