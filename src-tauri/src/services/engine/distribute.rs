//! Trunk -> subsystem distribution: the operator picks one trunk commit and one
//! target subsystem, reviews the files and scoped dependencies, and delivers the
//! change as a pull request on `distribute/<trunk-branch>-<short-sha>`.
//!
//! Key invariants:
//! - File bytes and modes are read from the exact selected commit (never trunk HEAD).
//! - Deletions in the selected commit delete the file in the subsystem if present
//!   (and can be unchecked by the operator).
//! - Dependencies are scoped to packages imported by the non-excluded distributed
//!   files plus any dependency delta introduced by the selected commit's own
//!   `package.json` (for packages the subsystem already uses or distributed files import).
//! - Lockfiles are never written; the PR notes the install step.
//! - Always pushes a unique branch and opens a PR; never pushes to the base branch.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::classify::{self, PathStatus};
use super::deps::{self, CodeFile};
use super::git;
use super::github_ops::{CreatePrRequest, GitHubOps, PullRequestInfo};
use super::provenance::{self, ProvenanceInput};
use super::semver::{self, ComparisonKind};
use super::Blocker;
use crate::error::{AppError, AppResult, ErrorCode};
use crate::github::gh_cli::GitHubAccount;
use crate::services::registry::{assert_full_sha, SubsystemEntry};

pub const DISTRIBUTION_PROGRESS_EVENT: &str = "distribution-progress";
pub const DISTRIBUTE_MODE: &str = "distribute";

const MAX_BLOB_BYTES: usize = 1024 * 1024;
const MAX_LISTED_PATHS: usize = 200;
const MAX_TRUNK_COMMITS: usize = 300;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrunkCommit {
    pub sha: String,
    pub short_sha: String,
    pub subject: String,
    pub author_name: String,
    pub committed_at: String,
    pub committed_unix: i64,
    pub is_merge: bool,
    pub files_changed: u32,
    pub paths: Vec<TrunkPath>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrunkPath {
    pub path: String,
    /// `added`, `modified` or `deleted`, matching the UI's change glyphs.
    pub change: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DistributionPathAction {
    Add,
    Update,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DistributionFile {
    pub path: String,
    pub action: DistributionPathAction,
    /// `added`, `modified` or `deleted` in the selected trunk commit.
    pub trunk_change: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DistributionSkip {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DistributionDepAction {
    Add,
    Update,
    Move,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DistributionDependency {
    pub name: String,
    pub action: DistributionDepAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_section: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_spec: Option<String>,
    pub section: String,
    pub spec: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DistributionPlan {
    pub subsystem_id: String,
    pub name: String,
    pub main_branch: String,
    pub commit_sha: String,
    pub commit_subject: String,
    pub source_branch: String,
    pub source_sha: String,
    pub branch: String,
    pub files: Vec<DistributionFile>,
    pub skipped: Vec<DistributionSkip>,
    pub dependencies: Vec<DistributionDependency>,
    pub blockers: Vec<Blocker>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DistributionOutcome {
    pub subsystem_id: String,
    pub ok: bool,
    pub main_branch: String,
    pub source_commit_sha: String,
    pub branch: String,
    pub commit_sha: Option<String>,
    pub pull_request: Option<PullRequestInfo>,
    pub applied: Vec<String>,
    pub deleted: Vec<String>,
    pub dependencies: Vec<DistributionDependency>,
    pub blockers: Vec<Blocker>,
    pub warnings: Vec<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub mode: String,
    pub blob: String,
}

#[derive(Debug, Clone)]
pub struct CommitDelta {
    pub path: String,
    pub new_mode: String,
    pub new_blob: String,
    pub status: char,
}

pub struct CommitSnapshot {
    pub dir: PathBuf,
    pub branch: String,
    pub sha: String,
    pub subject: String,
    pub deltas: Vec<CommitDelta>,
    pub manifest: Value,
    pub parent_manifest: Value,
}

impl CommitSnapshot {
    pub fn load(main_dir: &Path, main_branch: &str, commit_sha: &str) -> AppResult<Self> {
        let branch = main_branch.trim();
        if branch.is_empty() {
            return Err(
                AppError::invalid_request("Choose a trunk branch to distribute from.")
                    .with_remedy("Pick one of the main repository's branches."),
            );
        }
        let sha = commit_sha.trim();
        if assert_full_sha(sha, "commit_sha").is_err() {
            return Err(AppError::invalid_request(format!(
                "'{sha}' is not a full 40/64-character lowercase commit SHA."
            ))
            .with_remedy("Select a commit from the trunk commit list."));
        }

        let reference = git::resolve_branch_ref(main_dir, branch).ok_or_else(|| {
            AppError::new(
                ErrorCode::BranchNotPermitted,
                format!("Trunk branch '{branch}' does not exist in the main checkout."),
            )
            .with_remedy("Fetch the main repository or choose another branch.")
        })?;
        let branch_tip = git::rev_parse(main_dir, &reference).ok_or_else(|| {
            AppError::new(
                ErrorCode::RepositoryInaccessible,
                format!("Could not resolve trunk branch '{branch}'."),
            )
            .with_remedy("Check the main checkout under the checkouts cache, then retry.")
        })?;

        let is_ancestor = git::run(main_dir, &["merge-base", "--is-ancestor", sha, &branch_tip])
            .map(|out| out.status == 0)
            .unwrap_or(false);
        if !is_ancestor {
            return Err(AppError::invalid_request(format!(
                "Commit '{sha}' is not reachable from trunk branch '{branch}'."
            ))
            .with_remedy("Refresh the commit list and select a commit on this branch."));
        }

        let subject = git::run(main_dir, &["show", "-s", "--format=%s", sha])
            .ok()
            .filter(|out| out.status == 0)
            .map(|out| out.stdout_trimmed())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("sync changes from {branch}"));

        let deltas = read_commit_deltas(main_dir, sha)?;
        let manifest = read_manifest(main_dir, sha);
        let parent_ref = format!("{sha}^1");
        let parent_manifest = if git::rev_parse(main_dir, &parent_ref).is_some() {
            read_manifest(main_dir, &parent_ref)
        } else {
            serde_json::json!({})
        };

        Ok(Self {
            dir: main_dir.to_path_buf(),
            branch: branch.to_string(),
            sha: sha.to_string(),
            subject,
            deltas,
            manifest,
            parent_manifest,
        })
    }
}

/// First-parent commits on `main_branch`, newest first, including merge commits
/// diffed against their first parent (`-m --first-parent`).
pub fn trunk_commits(main_dir: &Path, main_branch: &str) -> AppResult<Vec<TrunkCommit>> {
    let branch = main_branch.trim();
    if branch.is_empty() {
        return Err(
            AppError::invalid_request("Choose a trunk branch to load commits from.")
                .with_remedy("Pick one of the main repository's branches."),
        );
    }
    let reference = git::resolve_branch_ref(main_dir, branch).ok_or_else(|| {
        AppError::new(
            ErrorCode::BranchNotPermitted,
            format!("Trunk branch '{branch}' does not exist in the main checkout."),
        )
        .with_remedy("Fetch the main repository or choose another branch.")
    })?;

    const REC: char = '\u{1e}';
    const FIELD: char = '\u{1f}';
    let fmt = format!("--format=%H{FIELD}%P{FIELD}%an{FIELD}%cI{FIELD}%ct{FIELD}%s{REC}");
    let max_count = format!("--max-count={MAX_TRUNK_COMMITS}");
    let args = [
        "log",
        max_count.as_str(),
        "--first-parent",
        "-m",
        &fmt,
        "--raw",
        "--no-abbrev",
        "--no-renames",
        &reference,
    ];
    let out = git::run(main_dir, &args).map_err(|err| {
        AppError::new(
            ErrorCode::RepositoryInaccessible,
            format!(
                "Could not read trunk branch '{branch}': {}",
                git::sanitize(&err.to_string())
            ),
        )
        .with_remedy("Confirm the main checkout is readable, then retry.")
    })?;
    if out.status != 0 {
        return Err(AppError::new(
            ErrorCode::RepositoryInaccessible,
            format!(
                "Could not read trunk branch '{branch}': {}",
                git::sanitize(&out.stderr)
            ),
        )
        .with_remedy("Confirm the main checkout is readable, then retry."));
    }

    let text = out.stdout_str();
    let mut commits: Vec<TrunkCommit> = Vec::new();
    let mut current: Option<TrunkCommit> = None;

    let flush = |current: &mut Option<TrunkCommit>, commits: &mut Vec<TrunkCommit>| {
        if let Some(mut commit) = current.take() {
            if commit.sha.len() == 40 || commit.sha.len() == 64 {
                commit.files_changed = commit.paths.len() as u32;
                commits.push(commit);
            }
        }
    };

    for raw_line in text.lines() {
        let line = raw_line.trim_end_matches([REC, '\r']);
        if line.contains(FIELD) {
            flush(&mut current, &mut commits);
            let mut parts = line.split(FIELD);
            let sha = parts.next().unwrap_or("").trim().to_string();
            if sha.is_empty() {
                continue;
            }
            let parents_raw = parts.next().unwrap_or("");
            let is_merge = parents_raw.split_whitespace().count() > 1;
            let author_name = parts.next().unwrap_or("").to_string();
            let committed_at = parts.next().unwrap_or("").to_string();
            let committed_unix = parts.next().unwrap_or("0").parse::<i64>().unwrap_or(0);
            let subject = parts.next().unwrap_or("").to_string();
            let short_sha: String = sha.chars().take(7).collect();
            current = Some(TrunkCommit {
                sha,
                short_sha,
                subject,
                author_name,
                committed_at,
                committed_unix,
                is_merge,
                files_changed: 0,
                paths: Vec::new(),
            });
        } else if let Some(commit) = current.as_mut() {
            if let Some(delta) = parse_raw_diff_row(line) {
                commit.paths.push(TrunkPath {
                    path: delta.path,
                    change: status_to_change(delta.status).to_string(),
                });
            }
        }
    }
    flush(&mut current, &mut commits);
    Ok(commits)
}

fn read_commit_deltas(repo_dir: &Path, sha: &str) -> AppResult<Vec<CommitDelta>> {
    let out = git::run(
        repo_dir,
        &[
            "diff-tree",
            "--root",
            "-m",
            "--first-parent",
            "--raw",
            "--no-abbrev",
            "--no-renames",
            "-r",
            "--format=",
            sha,
        ],
    )
    .map_err(|err| {
        AppError::new(
            ErrorCode::RepositoryInaccessible,
            format!(
                "Could not inspect commit '{sha}': {}",
                git::sanitize(&err.to_string())
            ),
        )
    })?;
    if out.status != 0 {
        return Err(AppError::new(
            ErrorCode::RepositoryInaccessible,
            format!(
                "Could not inspect commit '{sha}': {}",
                git::sanitize(&out.stderr)
            ),
        ));
    }
    let mut deltas = Vec::new();
    let mut seen = HashSet::new();
    for line in out.stdout_str().lines() {
        if let Some(delta) = parse_raw_diff_row(line) {
            if seen.insert(delta.path.clone()) {
                deltas.push(delta);
            }
        }
    }
    Ok(deltas)
}

fn parse_raw_diff_row(line: &str) -> Option<CommitDelta> {
    let row = line.trim();
    if !row.starts_with(':') {
        return None;
    }
    let (meta, raw_path) = row.split_once('\t')?;
    let mut cols = meta.split(' ');
    let (_old_mode, new_mode, _old_sha, new_sha, status_str) = (
        cols.next()?,
        cols.next()?,
        cols.next()?,
        cols.next()?,
        cols.next()?,
    );
    let status = status_str.chars().next()?;
    let path = unquote_git_path(raw_path);
    if path.is_empty() {
        return None;
    }
    Some(CommitDelta {
        path,
        new_mode: new_mode.to_string(),
        new_blob: new_sha.to_string(),
        status,
    })
}

fn unquote_git_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if !(trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2) {
        return trimmed.to_string();
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    let bytes = inner.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'\\' => {
                    out.push(b'\\');
                    i += 2;
                }
                b'"' => {
                    out.push(b'"');
                    i += 2;
                }
                b'n' => {
                    out.push(b'\n');
                    i += 2;
                }
                b't' => {
                    out.push(b'\t');
                    i += 2;
                }
                b'r' => {
                    out.push(b'\r');
                    i += 2;
                }
                c if (b'0'..=b'7').contains(&c) && i + 3 < bytes.len() => {
                    if let Ok(oct) = std::str::from_utf8(&bytes[i + 1..i + 4]) {
                        if let Ok(val) = u8::from_str_radix(oct, 8) {
                            out.push(val);
                            i += 4;
                            continue;
                        }
                    }
                    out.push(bytes[i]);
                    i += 1;
                }
                _ => {
                    out.push(bytes[i + 1]);
                    i += 2;
                }
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

fn status_to_change(status: char) -> &'static str {
    match status {
        'A' => "added",
        'D' => "deleted",
        _ => "modified",
    }
}

/// Unique branch name `distribute/<safe-trunk>-<short7>` so concurrent or
/// sequential distributions from different trunk commits do not collide.
pub fn distribution_branch(main_branch: &str, commit_sha: &str) -> String {
    let safe_branch: String = main_branch
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let safe_branch = safe_branch.trim_matches(['-', '.']);
    let safe_branch = if safe_branch.is_empty() {
        "trunk"
    } else {
        safe_branch
    };
    let short: String = commit_sha.chars().take(7).collect();
    format!("distribute/{safe_branch}-{short}")
}

pub fn plan(
    commit: &CommitSnapshot,
    child_dir: &Path,
    entry: &SubsystemEntry,
    github: &dyn GitHubOps,
    excluded_paths: &HashSet<String>,
) -> DistributionPlan {
    let branch = distribution_branch(&commit.branch, &commit.sha);
    let mut plan = DistributionPlan {
        subsystem_id: entry.id.clone(),
        name: entry.name.clone(),
        main_branch: commit.branch.clone(),
        commit_sha: commit.sha.clone(),
        commit_subject: commit.subject.clone(),
        source_branch: entry.source_branch.clone(),
        source_sha: String::new(),
        branch: branch.clone(),
        files: Vec::new(),
        skipped: Vec::new(),
        dependencies: Vec::new(),
        blockers: Vec::new(),
        warnings: Vec::new(),
    };

    let Some(child_ref) = git::resolve_branch_ref(child_dir, &entry.source_branch) else {
        plan.blockers.push(Blocker::new(
            "SOURCE_BRANCH_MISSING",
            format!(
                "Subsystem branch '{}' is not present in its checkout.",
                entry.source_branch
            ),
        ));
        return plan;
    };
    let Some(child_sha) = git::rev_parse(child_dir, &child_ref) else {
        plan.blockers.push(Blocker::new(
            "SOURCE_BRANCH_MISSING",
            format!(
                "Could not resolve branch '{}' for subsystem '{}'.",
                entry.source_branch, entry.id
            ),
        ));
        return plan;
    };
    plan.source_sha = child_sha.clone();

    if git::run(child_dir, &["check-ref-format", "--branch", &branch])
        .map(|out| out.status != 0)
        .unwrap_or(false)
    {
        plan.blockers.push(Blocker::new(
            "BRANCH_NAME_REJECTED",
            format!("Git refuses the distribution branch name '{branch}'."),
        ));
        return plan;
    }

    let child_index = tree_index(child_dir, &child_sha);
    if child_index.is_empty() {
        plan.blockers.push(Blocker::new(
            "SOURCE_TREE_EMPTY",
            format!(
                "Branch '{}' has no tracked files for subsystem '{}'.",
                entry.source_branch, entry.id
            ),
        ));
        return plan;
    }

    match github.list_branches(&entry.repository) {
        Ok(branches) => {
            if let Some(existing) = branches.iter().find(|b| b.name == branch) {
                plan.blockers.push(Blocker {
                    code: "DISTRIBUTION_BRANCH_EXISTS".to_string(),
                    message: format!("Branch '{branch}' already exists on {}.", entry.repository),
                    details: Some(format!(
                        "A distribution PR for this commit may already be open. Merge or delete '{branch}' first (at {}).",
                        existing.sha
                    )),
                });
            }
        }
        Err(err) => plan.warnings.push(format!(
            "Could not list branches on {} ({err}); the branch collision check was skipped.",
            entry.repository
        )),
    }

    if git::local_branch_exists(child_dir, &branch) {
        plan.blockers.push(Blocker {
            code: "DISTRIBUTION_BRANCH_EXISTS".to_string(),
            message: format!("A local branch '{branch}' already exists in the subsystem checkout."),
            details: Some(format!(
                "Delete it in {} before distributing this commit again.",
                child_dir.display()
            )),
        });
    }

    let mut lockfile_seen = false;
    let mut shared_configs: Vec<String> = Vec::new();
    let mut commit_touched_root_manifest = false;

    for delta in &commit.deltas {
        let path = &delta.path;
        if is_lockfile(path) {
            lockfile_seen = true;
            plan.skipped.push(DistributionSkip {
                path: path.clone(),
                reason: "lockfile: never written; run install in the subsystem and commit it"
                    .to_string(),
            });
            continue;
        }
        if path == "package.json" {
            commit_touched_root_manifest = true;
            plan.skipped.push(DistributionSkip {
                path: path.clone(),
                reason: "package.json: dependencies are reconciled automatically".to_string(),
            });
            continue;
        }
        if path.ends_with("/package.json") {
            plan.skipped.push(DistributionSkip {
                path: path.clone(),
                reason: "nested package.json: never overwritten automatically".to_string(),
            });
            continue;
        }

        let mode_opt = if delta.status == 'D' {
            None
        } else {
            Some(delta.new_mode.as_str())
        };
        let classification = classify::classify_path(path, &entry.integration_policy, mode_opt);
        if !matches!(
            classification.status,
            PathStatus::Apply | PathStatus::ExplicitReview
        ) {
            plan.skipped.push(DistributionSkip {
                path: path.clone(),
                reason: classification.reason,
            });
            continue;
        }

        if delta.status == 'D' {
            if child_index.contains_key(path) {
                plan.files.push(DistributionFile {
                    path: path.clone(),
                    action: DistributionPathAction::Delete,
                    trunk_change: "deleted".to_string(),
                });
            } else {
                plan.skipped.push(DistributionSkip {
                    path: path.clone(),
                    reason: "deleted in trunk commit and already absent in subsystem".to_string(),
                });
            }
            continue;
        }

        if let Some(child_entry) = child_index.get(path) {
            if child_entry.blob == delta.new_blob {
                plan.skipped.push(DistributionSkip {
                    path: path.clone(),
                    reason: "already matches subsystem".to_string(),
                });
                continue;
            }
            if classification.status == PathStatus::ExplicitReview && !excluded_paths.contains(path)
            {
                shared_configs.push(path.clone());
            }
            plan.files.push(DistributionFile {
                path: path.clone(),
                action: DistributionPathAction::Update,
                trunk_change: status_to_change(delta.status).to_string(),
            });
        } else {
            if classification.status == PathStatus::ExplicitReview && !excluded_paths.contains(path)
            {
                shared_configs.push(path.clone());
            }
            plan.files.push(DistributionFile {
                path: path.clone(),
                action: DistributionPathAction::Add,
                trunk_change: status_to_change(delta.status).to_string(),
            });
        }
    }

    plan.files.sort_by(|a, b| a.path.cmp(&b.path));
    plan.skipped.sort_by(|a, b| a.path.cmp(&b.path));

    if !shared_configs.is_empty() {
        shared_configs.sort();
        plan.warnings.push(format!(
            "Includes shared config file(s): {}. Uncheck them if the subsystem should keep its own copy.",
            shared_configs.join(", ")
        ));
    }

    let included_write_paths: Vec<String> = plan
        .files
        .iter()
        .filter(|file| {
            file.action != DistributionPathAction::Delete && !excluded_paths.contains(&file.path)
        })
        .map(|file| file.path.clone())
        .collect();
    let included_delete_count = plan
        .files
        .iter()
        .filter(|file| {
            file.action == DistributionPathAction::Delete && !excluded_paths.contains(&file.path)
        })
        .count();

    let child_manifest = read_manifest(child_dir, &child_sha);
    let has_child_manifest = child_manifest
        .as_object()
        .map(|o| !o.is_empty())
        .unwrap_or(false);

    if !has_child_manifest {
        if !included_write_paths.is_empty() || commit_touched_root_manifest {
            plan.blockers.push(Blocker::new(
                "SUBSYSTEM_MANIFEST_MISSING",
                format!(
                    "Subsystem '{}' has no readable package.json on '{}'.",
                    entry.id, entry.source_branch
                ),
            ));
            return plan;
        }
    } else {
        let distributed_code = read_code_files(commit, &included_write_paths);
        let mut candidate_packages = imported_names(&distributed_code);

        // If the selected commit itself changed root `package.json`, also consider
        // any added/updated packages that the subsystem already declares so trunk
        // dependency upgrades propagate without pulling in unrelated trunk packages.
        if commit_touched_root_manifest {
            for pkg in manifest_changed_packages(&commit.parent_manifest, &commit.manifest) {
                if declared(&child_manifest, &pkg).is_some() {
                    candidate_packages.insert(pkg);
                }
            }
        }

        plan.dependencies = dependency_plan(&commit.manifest, &child_manifest, &candidate_packages);

        for dep in &plan.dependencies {
            if dep.action == DistributionDepAction::Update {
                plan.warnings.push(format!(
                    "Overrides the subsystem's {} {} with trunk's {}; the subsystem's code may need changes.",
                    dep.name,
                    dep.from_spec.clone().unwrap_or_default(),
                    dep.spec
                ));
            }
        }

        if let Ok(updated) = reconciled_manifest(&child_manifest, &plan.dependencies) {
            plan.blockers.extend(prerequisite_blockers(
                &distributed_code,
                &updated,
                &plan.dependencies,
            ));
        }
    }

    if included_write_paths.is_empty() && included_delete_count == 0 && plan.dependencies.is_empty()
    {
        plan.warnings.push(
            "Nothing to distribute for this subsystem with the current file selection.".to_string(),
        );
    }
    if lockfile_seen || !plan.dependencies.is_empty() {
        plan.warnings.push(
            "Lockfiles are never written: run install in the subsystem and commit the lockfile after merging."
                .to_string(),
        );
    }

    plan
}

pub struct RunInput<'a> {
    pub commit: &'a CommitSnapshot,
    pub child_dir: &'a Path,
    pub entry: &'a SubsystemEntry,
    pub main_repo: &'a str,
    pub plan: &'a DistributionPlan,
    pub excluded_paths: &'a HashSet<String>,
    pub message: Option<&'a str>,
    pub attribute_to_account: bool,
    pub account: Option<GitHubAccount>,
    pub temp_root: &'a Path,
    pub remote_url: &'a str,
    pub github: &'a dyn GitHubOps,
}

pub fn run(input: RunInput<'_>) -> DistributionOutcome {
    let plan = input.plan;
    let mut outcome = DistributionOutcome {
        subsystem_id: plan.subsystem_id.clone(),
        ok: false,
        main_branch: plan.main_branch.clone(),
        source_commit_sha: plan.commit_sha.clone(),
        branch: plan.branch.clone(),
        commit_sha: None,
        pull_request: None,
        applied: Vec::new(),
        deleted: Vec::new(),
        dependencies: plan.dependencies.clone(),
        blockers: plan.blockers.clone(),
        warnings: plan.warnings.clone(),
        message: None,
    };
    if !plan.blockers.is_empty() {
        return outcome;
    }

    let selected: Vec<&DistributionFile> = plan
        .files
        .iter()
        .filter(|file| !input.excluded_paths.contains(&file.path))
        .collect();
    if selected.is_empty() && plan.dependencies.is_empty() {
        outcome.blockers.push(Blocker::new(
            "NO_CHANGES",
            "Nothing is selected to distribute.",
        ));
        return outcome;
    }

    let applied: Vec<String> = selected
        .iter()
        .filter(|f| f.action != DistributionPathAction::Delete)
        .map(|f| f.path.clone())
        .collect();
    let deleted: Vec<String> = selected
        .iter()
        .filter(|f| f.action == DistributionPathAction::Delete)
        .map(|f| f.path.clone())
        .collect();

    match apply_and_publish(&input, &selected, &applied, &deleted) {
        Ok((commit_sha, pull_request)) => {
            outcome.ok = true;
            outcome.commit_sha = Some(commit_sha);
            outcome.pull_request = pull_request;
            outcome.applied = applied;
            outcome.deleted = deleted;
        }
        Err(err) => {
            outcome.blockers.push(Blocker {
                code: format!("{:?}", err.code),
                message: err.message.clone(),
                details: err.remedy.clone().or(err.details.clone()),
            });
        }
    }
    outcome
}

type Published = (String, Option<PullRequestInfo>);

fn apply_and_publish(
    input: &RunInput<'_>,
    selected: &[&DistributionFile],
    applied: &[String],
    deleted: &[String],
) -> AppResult<Published> {
    let branch = input.plan.branch.clone();
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    );
    let worktree = input
        .temp_root
        .join(format!("distribute-{}-{nonce}", input.entry.id));
    std::fs::create_dir_all(input.temp_root).ok();
    let branch_existed = git::local_branch_exists(input.child_dir, &branch);

    let result = populate_worktree(input, selected, &worktree).and_then(|commit_sha| {
        publish(input, &worktree, &branch, applied, deleted).map(|pr| (commit_sha, pr))
    });
    git::worktree_remove(input.child_dir, &worktree);
    let _ = std::fs::remove_dir_all(&worktree);
    if !branch_existed {
        let _ = git::run(input.child_dir, &["branch", "-D", &branch]);
    }
    result
}

fn populate_worktree(
    input: &RunInput<'_>,
    selected: &[&DistributionFile],
    worktree: &Path,
) -> AppResult<String> {
    git::worktree_add(
        input.child_dir,
        worktree,
        &input.plan.branch,
        &input.plan.source_sha,
    )
    .map_err(|err| {
        AppError::new(
            ErrorCode::Internal,
            format!(
                "Could not create a distribution worktree: {}",
                git::sanitize(&err.to_string())
            ),
        )
        .with_remedy("Check that no other distribution is running for this subsystem, then retry.")
    })?;

    let delta_by_path: HashMap<&str, &CommitDelta> = input
        .commit
        .deltas
        .iter()
        .map(|d| (d.path.as_str(), d))
        .collect();

    let mut code_files: Vec<CodeFile> = Vec::new();
    let mut executable_paths: Vec<(&str, bool)> = Vec::new();

    for file in selected {
        let path = classify::inspect_path_safety(&file.path).map_err(|reason| {
            AppError::invalid_request(format!("'{}' is not a safe path: {reason}", file.path))
        })?;
        let target = worktree.join(&path);

        if file.action == DistributionPathAction::Delete {
            if target.exists() {
                std::fs::remove_file(&target).map_err(|err| {
                    AppError::new(
                        ErrorCode::Internal,
                        format!("Could not remove '{}': {err}", file.path),
                    )
                })?;
                prune_empty_parents(&target, worktree);
            }
            continue;
        }

        let mode = delta_by_path
            .get(file.path.as_str())
            .map(|d| d.new_mode.as_str())
            .unwrap_or("100644");
        if mode == "120000" || mode == "160000" {
            return Err(AppError::invalid_request(format!(
                "'{}' is a symlink or submodule entry; distribution only copies regular files.",
                file.path
            )));
        }

        let bytes =
            git::read_blob(&input.commit.dir, &input.commit.sha, &path).ok_or_else(|| {
                AppError::new(
                    ErrorCode::RepositoryInaccessible,
                    format!(
                        "Could not read '{}' from commit '{}'.",
                        file.path, input.commit.sha
                    ),
                )
                .with_remedy("Fetch the main repository again and retry.")
            })?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                AppError::new(
                    ErrorCode::Internal,
                    format!("Could not create '{}': {err}", parent.display()),
                )
            })?;
        }
        std::fs::write(&target, &bytes).map_err(|err| {
            AppError::new(
                ErrorCode::Internal,
                format!("Could not write '{}': {err}", file.path),
            )
        })?;
        executable_paths.push((file.path.as_str(), mode == "100755"));
        if deps::is_code_path(&path) && bytes.len() <= MAX_BLOB_BYTES {
            code_files.push(CodeFile {
                path,
                content: String::from_utf8_lossy(&bytes).to_string(),
            });
        }
    }

    git::add_all(worktree).map_err(|err| {
        AppError::new(
            ErrorCode::Internal,
            format!(
                "Could not stage the distributed files: {}",
                git::sanitize(&err.to_string())
            ),
        )
    })?;
    for (path, executable) in executable_paths {
        git::update_index_chmod(worktree, path, executable);
    }

    if !input.plan.dependencies.is_empty() {
        let manifest_path = worktree.join("package.json");
        let original_text = std::fs::read_to_string(&manifest_path).ok();
        let manifest: Value = original_text
            .as_deref()
            .and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        let updated = reconciled_manifest(&manifest, &input.plan.dependencies)?;
        let text = super::stringify_manifest(&updated, original_text.as_deref());
        std::fs::write(&manifest_path, text).map_err(|err| {
            AppError::new(
                ErrorCode::Internal,
                format!("Could not write package.json in the distribution worktree: {err}"),
            )
        })?;
        if let Some(blocker) =
            prerequisite_blockers(&code_files, &updated, &input.plan.dependencies)
                .into_iter()
                .next()
        {
            return Err(
                AppError::new(ErrorCode::DependencyConflict, blocker.message)
                    .with_details(blocker.code),
            );
        }
        git::add_all(worktree).map_err(|err| {
            AppError::new(
                ErrorCode::Internal,
                format!(
                    "Could not stage package.json: {}",
                    git::sanitize(&err.to_string())
                ),
            )
        })?;
    }

    if git::status_porcelain(worktree).trim().is_empty() {
        return Err(AppError::invalid_request(
            "The distribution produced no changes on the subsystem branch.",
        )
        .with_remedy("Main and the subsystem already agree; nothing to open a pull request for."));
    }

    let subject = input
        .message
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| input.commit.subject.clone());
    let message = provenance::build_distribution_message(&ProvenanceInput {
        subsystem_id: &input.entry.id,
        source_repo: input.main_repo,
        source_commits: std::slice::from_ref(&input.commit.sha),
        mode: DISTRIBUTE_MODE,
        target: &input.entry.source_branch,
        subject: &subject,
    });
    let author = provenance::commit_author(input.attribute_to_account, input.account.as_ref());
    let message_file = input.temp_root.join(format!(
        "{}.msg",
        worktree
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "distribute".to_string())
    ));
    std::fs::write(&message_file, message.as_bytes()).map_err(|err| {
        AppError::new(
            ErrorCode::Internal,
            format!("Could not stage the distribution commit message: {err}"),
        )
    })?;
    let commit_sha = git::commit_with_message(worktree, &message_file, &author.name, &author.email)
        .map_err(|err| {
            let _ = std::fs::remove_file(&message_file);
            AppError::new(
                ErrorCode::Internal,
                format!(
                    "Could not commit the distribution: {}",
                    git::sanitize(&err.to_string())
                ),
            )
            .with_remedy("Retry; if it fails again, inspect the subsystem checkout.")
        })?;
    let _ = std::fs::remove_file(&message_file);

    // `git add -A` skips ignored paths, so verify every selected path landed in the commit.
    let mut expected: HashSet<&str> = selected.iter().map(|f| f.path.as_str()).collect();
    if !input.plan.dependencies.is_empty() {
        expected.insert("package.json");
    }
    let committed = committed_paths(worktree, &commit_sha);
    let mut missing: Vec<&str> = expected
        .iter()
        .copied()
        .filter(|path| !committed.contains(*path))
        .collect();
    if !missing.is_empty() {
        missing.sort();
        return Err(AppError::new(
            ErrorCode::Internal,
            format!(
                "The distribution commit is missing {} selected path(s): {}.",
                missing.len(),
                missing.join(", ")
            ),
        )
        .with_remedy(
            "The subsystem's .gitignore excludes them; unignore the path or uncheck it before distributing.",
        ));
    }

    Ok(commit_sha)
}

fn prune_empty_parents(file_path: &Path, stop_at: &Path) {
    let mut current = file_path.parent();
    while let Some(dir) = current {
        if dir == stop_at || !dir.starts_with(stop_at) {
            break;
        }
        if std::fs::remove_dir(dir).is_err() {
            break;
        }
        current = dir.parent();
    }
}

fn committed_paths(worktree: &Path, sha: &str) -> HashSet<String> {
    let out = git::run(
        worktree,
        &[
            "show",
            "--pretty=format:",
            "--name-only",
            "--no-renames",
            sha,
        ],
    )
    .ok();
    out.map(|out| {
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(unquote_git_path)
            .collect()
    })
    .unwrap_or_default()
}

fn publish(
    input: &RunInput<'_>,
    worktree: &Path,
    branch: &str,
    applied: &[String],
    deleted: &[String],
) -> AppResult<Option<PullRequestInfo>> {
    git::push(worktree, input.remote_url, branch).map_err(|err| {
        AppError::new(
            ErrorCode::Internal,
            format!(
                "Could not push branch '{branch}' to {}: {}",
                input.entry.repository,
                git::sanitize(&err.to_string())
            ),
        )
        .with_remedy("Check the token's push permission on the subsystem repository, then retry.")
    })?;

    let body = distribution_body(input, applied, deleted);
    let subject = input
        .message
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or(&input.commit.subject);
    let title = format!("distribute({}): {subject}", input.entry.id);
    let pull_request = input
        .github
        .create_pull_request(
            &input.entry.repository,
            &CreatePrRequest {
                head: branch.to_string(),
                base: input.entry.source_branch.clone(),
                title,
                body,
                draft: false,
            },
        )
        .map_err(|err| {
            AppError::new(
                ErrorCode::Internal,
                format!("Branch '{branch}' was pushed, but opening the pull request failed: {err}"),
            )
            .with_remedy("Open the pull request manually from the pushed branch.")
        })?;
    Ok(Some(pull_request))
}

fn distribution_body(input: &RunInput<'_>, applied: &[String], deleted: &[String]) -> String {
    let mut body = String::new();
    body.push_str(&format!(
        "Distribution from `{}` (`{}`) into `{}` for subsystem `{}`.\n\n",
        input.main_repo, input.commit.branch, input.entry.source_branch, input.entry.id
    ));
    body.push_str(&format!("- Source commit: `{}`\n", input.commit.sha));
    body.push_str(&format!("- Subject: {}\n", input.commit.subject));
    if !input.plan.source_sha.is_empty() {
        body.push_str(&format!("- Subsystem base: `{}`\n", input.plan.source_sha));
    }
    body.push_str(&format!("- Files written: {}\n", applied.len()));
    if !deleted.is_empty() {
        body.push_str(&format!("- Files deleted: {}\n", deleted.len()));
    }
    if !input.plan.dependencies.is_empty() {
        body.push_str(&format!(
            "- Dependencies changed: {}\n",
            input.plan.dependencies.len()
        ));
    }
    body.push('\n');

    if !applied.is_empty() {
        body.push_str("Files written:\n");
        for path in applied.iter().take(MAX_LISTED_PATHS) {
            body.push_str(&format!("- `{path}`\n"));
        }
        if applied.len() > MAX_LISTED_PATHS {
            body.push_str(&format!(
                "- … and {} more\n",
                applied.len() - MAX_LISTED_PATHS
            ));
        }
        body.push('\n');
    }

    if !deleted.is_empty() {
        body.push_str("Files deleted:\n");
        for path in deleted.iter().take(MAX_LISTED_PATHS) {
            body.push_str(&format!("- `{path}`\n"));
        }
        if deleted.len() > MAX_LISTED_PATHS {
            body.push_str(&format!(
                "- … and {} more\n",
                deleted.len() - MAX_LISTED_PATHS
            ));
        }
        body.push('\n');
    }

    if !input.plan.dependencies.is_empty() {
        body.push_str("Dependencies:\n");
        for dep in &input.plan.dependencies {
            match dep.action {
                DistributionDepAction::Add => body.push_str(&format!(
                    "- add `{}` {} to {}\n",
                    dep.name, dep.spec, dep.section
                )),
                DistributionDepAction::Update => body.push_str(&format!(
                    "- update `{}` {} -> {} ({})\n",
                    dep.name,
                    dep.from_spec.clone().unwrap_or_default(),
                    dep.spec,
                    dep.section
                )),
                DistributionDepAction::Move => body.push_str(&format!(
                    "- move `{}` {} from {} to {}\n",
                    dep.name,
                    dep.spec,
                    dep.from_section.clone().unwrap_or_default(),
                    dep.section,
                )),
            }
        }
        body.push('\n');
    }

    if !input.plan.dependencies.is_empty() {
        body.push_str("The lockfile is not written by this distribution: run install in the subsystem and commit the lockfile before merging.\n");
    }
    body
}

fn is_lockfile(path: &str) -> bool {
    matches!(
        path.rsplit('/')
            .next()
            .unwrap_or(path)
            .to_ascii_lowercase()
            .as_str(),
        "package-lock.json"
            | "npm-shrinkwrap.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "bun.lockb"
            | "bun.lock"
    )
}

pub fn tree_index(repo_dir: &Path, rev: &str) -> HashMap<String, TreeEntry> {
    let mut index = HashMap::new();
    let Ok(out) = git::run(repo_dir, &["ls-tree", "-r", "-z", "--full-tree", rev]) else {
        return index;
    };
    if out.status != 0 {
        return index;
    }
    for record in out.stdout.split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        let record = String::from_utf8_lossy(record);
        let Some((meta, path)) = record.split_once('\t') else {
            continue;
        };
        let mut cols = meta.split(' ');
        let (Some(mode), _kind, Some(blob)) = (cols.next(), cols.next(), cols.next()) else {
            continue;
        };
        index.insert(
            path.to_string(),
            TreeEntry {
                mode: mode.to_string(),
                blob: blob.to_string(),
            },
        );
    }
    index
}

fn read_manifest(repo_dir: &Path, rev: &str) -> Value {
    git::read_blob(repo_dir, rev, "package.json")
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

fn imported_names(files: &[CodeFile]) -> HashSet<String> {
    let mut names = HashSet::new();
    for file in files {
        for specifier in deps::extract_import_specifiers(&file.content) {
            if let Some(name) = deps::to_package_name(&specifier) {
                names.insert(name);
            }
        }
    }
    names
}

fn read_code_files(commit: &CommitSnapshot, paths: &[String]) -> Vec<CodeFile> {
    let mut files = Vec::new();
    for path in paths {
        if !deps::is_code_path(path) {
            continue;
        }
        if let Some(bytes) = git::read_blob(&commit.dir, &commit.sha, path) {
            if bytes.len() <= MAX_BLOB_BYTES {
                files.push(CodeFile {
                    path: path.clone(),
                    content: String::from_utf8_lossy(&bytes).to_string(),
                });
            }
        }
    }
    files
}

fn manifest_changed_packages(parent: &Value, current: &Value) -> HashSet<String> {
    let mut changed = HashSet::new();
    for section in deps::DEP_SECTIONS {
        let Some(curr_obj) = current.get(section).and_then(Value::as_object) else {
            continue;
        };
        for (name, curr_val) in curr_obj {
            let prev = declared(parent, name);
            let curr_spec = match curr_val {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            let is_same = prev
                .map(|(prev_sec, prev_spec)| prev_sec == section && prev_spec == curr_spec)
                .unwrap_or(false);
            if !is_same {
                changed.insert(name.clone());
            }
        }
    }
    changed
}

fn declared(manifest: &Value, name: &str) -> Option<(String, String)> {
    for section in deps::DEP_SECTIONS {
        if let Some(value) = manifest.get(section).and_then(|obj| obj.get(name)) {
            let spec = match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            return Some((section.to_string(), spec));
        }
    }
    None
}

fn dependency_plan(
    main: &Value,
    child: &Value,
    candidates: &HashSet<String>,
) -> Vec<DistributionDependency> {
    let mut names: Vec<&String> = candidates.iter().collect();
    names.sort();
    let mut plan = Vec::new();
    for name in names {
        let Some((main_section, main_spec)) = declared(main, name) else {
            continue;
        };
        match declared(child, name) {
            None => plan.push(DistributionDependency {
                name: name.clone(),
                action: DistributionDepAction::Add,
                from_section: None,
                from_spec: None,
                section: main_section,
                spec: main_spec,
            }),
            Some((child_section, child_spec)) => {
                if child_section == main_section {
                    if child_spec == main_spec {
                        continue;
                    }
                    let kind = semver::compare_ranges(&child_spec, &main_spec).kind;
                    if matches!(kind, ComparisonKind::Identical | ComparisonKind::Compatible) {
                        continue;
                    }
                    plan.push(DistributionDependency {
                        name: name.clone(),
                        action: DistributionDepAction::Update,
                        from_section: Some(child_section),
                        from_spec: Some(child_spec),
                        section: main_section,
                        spec: main_spec,
                    });
                } else {
                    plan.push(DistributionDependency {
                        name: name.clone(),
                        action: DistributionDepAction::Move,
                        from_section: Some(child_section),
                        from_spec: Some(child_spec),
                        section: main_section,
                        spec: main_spec,
                    });
                }
            }
        }
    }
    plan
}

fn reconciled_manifest(child: &Value, plan: &[DistributionDependency]) -> AppResult<Value> {
    let mut manifest = child.clone();
    let Some(root) = manifest.as_object_mut() else {
        return Err(AppError::invalid_request(
            "The subsystem package.json is not a JSON object.",
        ));
    };
    for dep in plan {
        if dep.action == DistributionDepAction::Move {
            for section in deps::DEP_SECTIONS {
                if let Some(obj) = root.get_mut(section).and_then(Value::as_object_mut) {
                    obj.remove(&dep.name);
                }
            }
        }
        let dest = root
            .entry(dep.section.clone())
            .or_insert_with(|| serde_json::json!({}));
        let Some(obj) = dest.as_object_mut() else {
            return Err(AppError::new(
                ErrorCode::DependencyConflict,
                format!(
                    "The '{}' section of the subsystem package.json is not an object.",
                    dep.section
                ),
            ));
        };
        obj.insert(dep.name.clone(), Value::String(dep.spec.clone()));
    }
    Ok(manifest)
}

fn prerequisite_blockers(
    files: &[CodeFile],
    manifest: &Value,
    plan: &[DistributionDependency],
) -> Vec<Blocker> {
    let added: Vec<String> = plan
        .iter()
        .filter(|dep| dep.action != DistributionDepAction::Update)
        .map(|dep| dep.name.clone())
        .collect();
    deps::check_import_prerequisites(files, manifest, &added)
        .into_iter()
        .map(|missing| {
            Blocker::new(
                "MISSING_DEPENDENCY_PREREQUISITE",
                format!(
                    "'{}' is imported by distributed code but declared in neither the subsystem nor the trunk commit's manifest.",
                    missing.name
                ),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::engine::github_ops::StubGitHubOps;
    use serde_json::json;

    fn temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vos-dist-{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn git_ok(dir: &Path, args: &[&str]) -> String {
        let out = git::run(dir, args).expect("git");
        assert_eq!(out.status, 0, "git {args:?} failed: {}", out.stderr);
        out.stdout_trimmed()
    }

    fn git_commit(dir: &Path, subject: &str) -> String {
        let author = ["-c", "user.name=t", "-c", "user.email=t@example.com"];
        let mut add = author.to_vec();
        add.extend(["add", "-A"]);
        git_ok(dir, &add);
        let mut commit = author.to_vec();
        commit.extend(["commit", "-q", "-m", subject]);
        git_ok(dir, &commit);
        git_ok(dir, &["rev-parse", "HEAD"])
    }

    fn write_file(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }

    fn test_entry() -> SubsystemEntry {
        serde_json::from_value(json!({
            "id": "hr",
            "name": "HR",
            "repository": "org/hr",
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
        }))
        .expect("valid subsystem entry")
    }

    #[test]
    fn trunk_commits_walk_first_parent_and_include_merge_commit_files() {
        let dir = temp_dir("first-parent-merge");
        git_ok(&dir, &["init", "-q", "-b", "main"]);
        write_file(&dir, "a.ts", "one\n");
        git_commit(&dir, "Trunk base");

        git_ok(&dir, &["checkout", "-q", "-b", "side"]);
        write_file(&dir, "b.ts", "two\n");
        git_commit(&dir, "Side work");

        git_ok(&dir, &["checkout", "-q", "main"]);
        write_file(&dir, "c.ts", "three\n");
        git_commit(&dir, "Trunk follow-up");

        git_ok(
            &dir,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "merge",
                "--no-ff",
                "-q",
                "side",
                "-m",
                "Merge side",
            ],
        );

        let commits = trunk_commits(&dir, "main").expect("commits");
        let subjects: Vec<&str> = commits.iter().map(|c| c.subject.as_str()).collect();
        assert!(subjects.contains(&"Merge side"), "{subjects:?}");
        assert!(subjects.contains(&"Trunk follow-up"), "{subjects:?}");
        assert!(
            !subjects.contains(&"Side work"),
            "first-parent walk must not include side branch commits: {subjects:?}"
        );

        let merge_commit = commits.iter().find(|c| c.subject == "Merge side").unwrap();
        assert!(merge_commit.is_merge);
        assert_eq!(merge_commit.paths.len(), 1);
        assert_eq!(merge_commit.paths[0].path, "b.ts");
        assert_eq!(merge_commit.paths[0].change, "added");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn distribution_copies_exact_selected_commit_not_trunk_head() {
        let root = temp_dir("exact-commit");
        let main_dir = root.join("main");
        let child_dir = root.join("child");
        let bare_dir = root.join("remote.git");
        let work_tmp = root.join("tmp");
        std::fs::create_dir_all(&main_dir).unwrap();
        std::fs::create_dir_all(&child_dir).unwrap();
        std::fs::create_dir_all(&bare_dir).unwrap();
        std::fs::create_dir_all(&work_tmp).unwrap();

        git_ok(&bare_dir, &["init", "--bare", "-q"]);

        git_ok(&main_dir, &["init", "-q", "-b", "main"]);
        write_file(
            &main_dir,
            "package.json",
            r#"{"dependencies":{"zod":"4.1.0"}}"#,
        );
        write_file(&main_dir, "src/shared.ts", "export const v = 1;\n");
        git_commit(&main_dir, "initial");

        write_file(
            &main_dir,
            "src/shared.ts",
            "import { z } from 'zod';\nexport const v = 2;\n",
        );
        let commit_v2 = git_commit(&main_dir, "bump shared to v2");

        write_file(
            &main_dir,
            "src/shared.ts",
            "import { z } from 'zod';\nexport const v = 3;\n",
        );
        git_commit(&main_dir, "bump shared to v3");

        git_ok(&child_dir, &["init", "-q", "-b", "master"]);
        write_file(
            &child_dir,
            "package.json",
            r#"{"dependencies":{"react":"^19.0.0"}}"#,
        );
        write_file(&child_dir, "src/shared.ts", "export const v = 1;\n");
        git_commit(&child_dir, "child init");

        let snapshot = CommitSnapshot::load(&main_dir, "main", &commit_v2).expect("snapshot");
        let entry = test_entry();
        let github = StubGitHubOps::new();
        let excluded = HashSet::new();

        let p = plan(&snapshot, &child_dir, &entry, &github, &excluded);
        assert!(p.blockers.is_empty(), "{:?}", p.blockers);
        assert_eq!(p.files.len(), 1);
        assert_eq!(p.files[0].path, "src/shared.ts");
        assert_eq!(p.files[0].action, DistributionPathAction::Update);
        assert_eq!(p.dependencies.len(), 1);
        assert_eq!(p.dependencies[0].name, "zod");
        assert_eq!(p.dependencies[0].action, DistributionDepAction::Add);

        let remote_url = bare_dir.to_string_lossy().replace('\\', "/");
        let outcome = run(RunInput {
            commit: &snapshot,
            child_dir: &child_dir,
            entry: &entry,
            main_repo: "org/main",
            plan: &p,
            excluded_paths: &excluded,
            message: None,
            attribute_to_account: false,
            account: None,
            temp_root: &work_tmp,
            remote_url: &remote_url,
            github: &github,
        });
        assert!(outcome.ok, "{:?}", outcome.blockers);
        assert_eq!(outcome.applied, vec!["src/shared.ts".to_string()]);
        assert!(outcome.deleted.is_empty());

        let pushed_branch = &p.branch;
        let content = git_ok(
            &bare_dir,
            &["show", &format!("{pushed_branch}:src/shared.ts")],
        );
        assert!(
            content.contains("v = 2"),
            "must copy v2 from selected commit, not v3 from trunk HEAD: {content}"
        );

        let commit_msg = git_ok(&bare_dir, &["show", "-s", "--format=%B", pushed_branch]);
        assert!(commit_msg.starts_with("distribute(hr): bump shared to v2"));
        assert!(commit_msg.contains(&format!("Source-Commit: {commit_v2}")));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn distribution_propagates_deletions_and_respects_exclusions() {
        let root = temp_dir("deletions-and-exclusions");
        let main_dir = root.join("main");
        let child_dir = root.join("child");
        let bare_dir = root.join("remote.git");
        let work_tmp = root.join("tmp");
        std::fs::create_dir_all(&main_dir).unwrap();
        std::fs::create_dir_all(&child_dir).unwrap();
        std::fs::create_dir_all(&bare_dir).unwrap();
        std::fs::create_dir_all(&work_tmp).unwrap();

        git_ok(&bare_dir, &["init", "--bare", "-q"]);

        git_ok(&main_dir, &["init", "-q", "-b", "main"]);
        write_file(
            &main_dir,
            "package.json",
            r#"{"dependencies":{"zod":"4.1.0","dayjs":"1.11.0"}}"#,
        );
        write_file(&main_dir, "src/old.ts", "export const old = true;\n");
        write_file(&main_dir, "src/only-main.ts", "export const m = true;\n");
        git_commit(&main_dir, "init");

        std::fs::remove_file(main_dir.join("src/old.ts")).unwrap();
        std::fs::remove_file(main_dir.join("src/only-main.ts")).unwrap();
        write_file(
            &main_dir,
            "src/new-with-dep.ts",
            "import dayjs from 'dayjs';\nexport const d = dayjs;\n",
        );
        let del_commit = git_commit(&main_dir, "remove old and add new");

        git_ok(&child_dir, &["init", "-q", "-b", "master"]);
        write_file(
            &child_dir,
            "package.json",
            r#"{"dependencies":{"react":"^19.0.0"}}"#,
        );
        write_file(&child_dir, "src/old.ts", "export const old = true;\n");
        git_commit(&child_dir, "child init");

        let snapshot = CommitSnapshot::load(&main_dir, "main", &del_commit).expect("snapshot");
        let entry = test_entry();
        let github = StubGitHubOps::new();

        // Exclude `src/new-with-dep.ts`: `dayjs` must NOT be added to dependencies!
        let mut excluded = HashSet::new();
        excluded.insert("src/new-with-dep.ts".to_string());

        let p = plan(&snapshot, &child_dir, &entry, &github, &excluded);
        assert!(p.blockers.is_empty(), "{:?}", p.blockers);
        assert!(
            p.dependencies.is_empty(),
            "excluded file's imports must not add dependencies: {:?}",
            p.dependencies
        );
        assert!(p
            .files
            .iter()
            .any(|f| f.path == "src/old.ts" && f.action == DistributionPathAction::Delete));
        assert!(p.skipped.iter().any(|s| s.path == "src/only-main.ts"));

        let remote_url = bare_dir.to_string_lossy().replace('\\', "/");
        let outcome = run(RunInput {
            commit: &snapshot,
            child_dir: &child_dir,
            entry: &entry,
            main_repo: "org/main",
            plan: &p,
            excluded_paths: &excluded,
            message: Some("drop deprecated helper"),
            attribute_to_account: false,
            account: None,
            temp_root: &work_tmp,
            remote_url: &remote_url,
            github: &github,
        });
        assert!(outcome.ok, "{:?}", outcome.blockers);
        assert!(outcome.applied.is_empty());
        assert_eq!(outcome.deleted, vec!["src/old.ts".to_string()]);

        let files_in_branch = git_ok(&bare_dir, &["ls-tree", "-r", "--name-only", &p.branch]);
        assert!(!files_in_branch.contains("src/old.ts"));
        assert!(!files_in_branch.contains("src/new-with-dep.ts"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn distribution_branch_includes_branch_and_short_sha() {
        let sha = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678";
        assert_eq!(
            distribution_branch("master", sha),
            "distribute/master-a1b2c3d"
        );
        assert_eq!(
            distribution_branch("release/1.2", sha),
            "distribute/release-1.2-a1b2c3d"
        );
    }

    #[test]
    fn dependency_plan_adds_updates_moves_and_skips_compatible() {
        let main = json!({
            "dependencies": { "react": "19.2.0", "zod": "4.1.0", "left-pad": "1.3.0" },
            "devDependencies": { "typescript": "^5.9.0" }
        });
        let child = json!({
            "dependencies": { "react": "^18.2.0", "lodash": "4.17.21" },
            "devDependencies": { "zod": "3.22.0" }
        });
        let candidates: HashSet<String> = ["react", "zod", "lodash"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let plan = dependency_plan(&main, &child, &candidates);
        let names: Vec<(&str, DistributionDepAction)> =
            plan.iter().map(|d| (d.name.as_str(), d.action)).collect();
        assert_eq!(
            names,
            vec![
                ("react", DistributionDepAction::Update),
                ("zod", DistributionDepAction::Move),
            ]
        );
    }

    #[test]
    fn dependency_plan_treats_compatible_ranges_as_unchanged() {
        let main = json!({ "dependencies": { "react": "19.2.0" } });
        let child = json!({ "dependencies": { "react": "^19.2.0" } });
        let candidates: HashSet<String> = ["react".to_string()].into_iter().collect();
        assert!(dependency_plan(&main, &child, &candidates).is_empty());
    }
}
