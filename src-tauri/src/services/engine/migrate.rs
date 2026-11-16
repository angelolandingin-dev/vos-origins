//! Reverse migration: main branch → one subsystem, delivered as a pull request
//! on `migrate-<main-branch>`. Ownership is the subsystem's own tree; main
//! supplies the bytes. Never writes a lockfile, never deletes a path.

use std::collections::{HashMap, HashSet};
use std::path::Path;

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
use crate::services::live_read;
use crate::services::registry::SubsystemEntry;

pub const MIGRATION_PROGRESS_EVENT: &str = "migration-progress";
pub const MIGRATE_MODE: &str = "migrate";

const MAX_BLOB_BYTES: usize = 1024 * 1024;
const MAX_LISTED_PATHS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MigratePathState {
    /// The subsystem's own history never contained main's current content.
    Changed,
    /// Both sides changed the path independently; main was touched last.
    Diverged,
    /// Absent from the subsystem tree; offered only when explicitly ticked.
    MainOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigratePath {
    pub path: String,
    pub state: MigratePathState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MigrateDepAction {
    Add,
    Update,
    Move,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateDependency {
    pub name: String,
    pub action: MigrateDepAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_section: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_spec: Option<String>,
    pub section: String,
    pub spec: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigratePlan {
    pub subsystem_id: String,
    pub name: String,
    pub source_branch: String,
    pub branch: String,
    pub main_sha: String,
    pub source_sha: String,
    pub paths: Vec<MigratePath>,
    pub main_only: Vec<String>,
    pub dependencies: Vec<MigrateDependency>,
    pub excluded: Vec<String>,
    pub blockers: Vec<Blocker>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigratePreview {
    pub main_branch: String,
    pub main_sha: String,
    pub subsystems: Vec<MigratePlan>,
}

#[derive(Debug, Clone, Default)]
pub struct Selection {
    pub excluded_paths: Vec<String>,
    pub include_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateOutcome {
    pub subsystem_id: String,
    pub ok: bool,
    pub branch: String,
    pub commit_sha: Option<String>,
    pub pull_request: Option<PullRequestInfo>,
    pub applied: Vec<String>,
    pub dependencies: Vec<MigrateDependency>,
    pub blockers: Vec<Blocker>,
    pub warnings: Vec<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateRunResult {
    pub main_branch: String,
    pub main_sha: String,
    pub outcomes: Vec<MigrateOutcome>,
}

#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub mode: String,
    pub blob: String,
}

#[derive(Debug, Clone, Default)]
pub struct PathHistory {
    pub blobs: HashSet<String>,
    pub last_epoch: i64,
}

pub struct MainSnapshot {
    pub dir: std::path::PathBuf,
    pub branch: String,
    pub sha: String,
    pub index: HashMap<String, TreeEntry>,
    pub history: HashMap<String, PathHistory>,
    pub manifest: Value,
}

impl MainSnapshot {
    pub fn load(main_dir: &Path, main_branch: &str) -> AppResult<Self> {
        let branch = main_branch.trim();
        if branch.is_empty() {
            return Err(
                AppError::invalid_request("Choose a main branch to migrate from.")
                    .with_remedy("Pick one of the main repository's branches."),
            );
        }
        let reference = git::resolve_branch_ref(main_dir, branch).ok_or_else(|| {
            AppError::new(
                ErrorCode::BranchNotPermitted,
                format!("Main branch '{branch}' does not exist in the main checkout."),
            )
            .with_remedy("Fetch the main repository or choose another branch.")
        })?;
        let sha = git::rev_parse(main_dir, &reference).ok_or_else(|| {
            AppError::new(
                ErrorCode::RepositoryInaccessible,
                format!("Could not resolve main branch '{branch}'."),
            )
            .with_remedy("Check the main checkout under the checkouts cache, then retry.")
        })?;
        let index = tree_index(main_dir, &sha);
        if index.is_empty() {
            return Err(AppError::new(
                ErrorCode::RepositoryInaccessible,
                format!("Main branch '{branch}' has no tracked files."),
            )
            .with_remedy("Confirm the main checkout is a readable clone of the main repository."));
        }
        Ok(Self {
            dir: main_dir.to_path_buf(),
            branch: branch.to_string(),
            sha: sha.clone(),
            index,
            history: history_of(main_dir, &sha),
            manifest: read_manifest(main_dir, &sha),
        })
    }
}

/// `migrate-<main branch>` with everything git refuses in a ref name flattened,
/// mirroring how integration branches are named on the forward path.
pub fn migration_branch(main_branch: &str) -> String {
    let safe: String = main_branch
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("migrate-{safe}")
}

pub fn plan(
    main: &MainSnapshot,
    child_dir: &Path,
    entry: &SubsystemEntry,
    github: &dyn GitHubOps,
) -> MigratePlan {
    let branch = migration_branch(&main.branch);
    let mut plan = MigratePlan {
        subsystem_id: entry.id.clone(),
        name: entry.name.clone(),
        source_branch: entry.source_branch.clone(),
        branch: branch.clone(),
        main_sha: main.sha.clone(),
        source_sha: String::new(),
        paths: Vec::new(),
        main_only: Vec::new(),
        dependencies: Vec::new(),
        excluded: Vec::new(),
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
            format!("Git refuses the migration branch name '{branch}'."),
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
                    code: "MIGRATION_BRANCH_EXISTS".to_string(),
                    message: format!("Branch '{branch}' already exists on {}.", entry.repository),
                    details: Some(format!(
                        "Merge or delete it first, or push the existing branch. It points at {}.",
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

    let child_history = history_of(child_dir, &child_sha);
    if child_history.is_empty() {
        plan.warnings.push(
            "The subsystem history could not be read; direction is decided by last-touch time only."
                .to_string(),
        );
    }
    // A leftover local branch would make `worktree_add` fail, and cleanup only
    // deletes branches this run created, so refuse before anything is written.
    if git::local_branch_exists(child_dir, &branch) {
        plan.blockers.push(Blocker {
            code: "MIGRATION_BRANCH_EXISTS".to_string(),
            message: format!("A local branch '{branch}' already exists in the subsystem checkout."),
            details: Some(format!(
                "Delete it in {} (or push it to {}) before migrating again.",
                child_dir.display(),
                entry.repository
            )),
        });
    }

    let mut lockfile_seen = false;
    for (path, child_entry) in &child_index {
        if is_deferred(path) {
            lockfile_seen |= is_lockfile(path);
            continue;
        }
        let Some(main_entry) = main.index.get(path) else {
            continue;
        };
        if main_entry.blob == child_entry.blob {
            continue;
        }
        let classification =
            classify::classify_path(path, &entry.integration_policy, Some(&main_entry.mode));
        if !matches!(
            classification.status,
            PathStatus::Apply | PathStatus::ExplicitReview
        ) {
            plan.excluded.push(path.clone());
            continue;
        }
        let state = direction(
            &main_entry.blob,
            &child_entry.blob,
            child_history.get(path),
            main.history.get(path),
        );
        if let Some(state) = state {
            plan.paths.push(MigratePath {
                path: path.clone(),
                state,
            });
        }
    }
    plan.paths.sort_by(|a, b| a.path.cmp(&b.path));
    plan.excluded.sort();

    let child_dirs: HashSet<&str> = child_index
        .keys()
        .filter_map(|path| path.rsplit_once('/').map(|(dir, _)| dir))
        .collect();
    for (path, main_entry) in &main.index {
        if child_index.contains_key(path) || is_deferred(path) {
            continue;
        }
        let parent = path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
        if !parent.is_empty() && !child_dirs.contains(parent) {
            continue;
        }
        let classification =
            classify::classify_path(path, &entry.integration_policy, Some(&main_entry.mode));
        if classification.status != PathStatus::Apply {
            continue;
        }
        plan.main_only.push(path.clone());
    }
    plan.main_only.sort();

    let child_manifest = read_manifest(child_dir, &child_sha);
    if child_manifest
        .as_object()
        .map(|o| o.is_empty())
        .unwrap_or(true)
    {
        plan.blockers.push(Blocker::new(
            "SUBSYSTEM_MANIFEST_MISSING",
            format!(
                "Subsystem '{}' has no readable package.json on '{}'.",
                entry.id, entry.source_branch
            ),
        ));
        return plan;
    }

    let mut imported = imported_packages(child_dir, &child_index);
    // Migrated main files bring their own imports; without them a package the
    // subsystem never imported would block the run instead of being added.
    imported.extend(imported_names(&read_code_files(main, &plan.paths)));
    plan.dependencies = dependency_plan(&main.manifest, &child_manifest, &imported);
    let overrides: Vec<String> = plan
        .dependencies
        .iter()
        .filter(|dep| dep.action == MigrateDepAction::Update)
        .map(|dep| {
            format!(
                "Overrides the subsystem's {} {} with main's {}; the subsystem's code may need changes.",
                dep.name,
                dep.from_spec.clone().unwrap_or_default(),
                dep.spec
            )
        })
        .collect();
    plan.warnings.extend(overrides);

    if let Ok(updated) = reconciled_manifest(&child_manifest, &plan.dependencies) {
        let files = read_code_files(main, &plan.paths);
        plan.blockers
            .extend(prerequisite_blockers(&files, &updated, &plan.dependencies));
    }

    if plan.paths.is_empty() && plan.dependencies.is_empty() && plan.main_only.is_empty() {
        plan.warnings
            .push("Main and this subsystem already agree; nothing to migrate.".to_string());
    }
    if lockfile_seen {
        plan.warnings.push(
            "Lockfiles are never written: run npm install in the subsystem and commit the lockfile."
                .to_string(),
        );
    }
    plan
}

pub struct RunInput<'a> {
    pub main: &'a MainSnapshot,
    pub child_dir: &'a Path,
    pub entry: &'a SubsystemEntry,
    pub main_repo: &'a str,
    pub plan: &'a MigratePlan,
    pub selection: &'a Selection,
    pub message: Option<&'a str>,
    pub temp_root: &'a Path,
    pub remote_url: &'a str,
    pub github: &'a dyn GitHubOps,
}

pub fn run(input: RunInput<'_>) -> MigrateOutcome {
    let plan = input.plan;
    let mut outcome = MigrateOutcome {
        subsystem_id: plan.subsystem_id.clone(),
        ok: false,
        branch: plan.branch.clone(),
        commit_sha: None,
        pull_request: None,
        applied: Vec::new(),
        dependencies: plan.dependencies.clone(),
        blockers: plan.blockers.clone(),
        warnings: plan.warnings.clone(),
        message: None,
    };
    if !plan.blockers.is_empty() {
        return outcome;
    }

    let excluded: HashSet<&str> = input
        .selection
        .excluded_paths
        .iter()
        .map(String::as_str)
        .collect();
    let mut selected: Vec<MigratePath> = plan
        .paths
        .iter()
        .filter(|p| !excluded.contains(p.path.as_str()))
        .cloned()
        .collect();
    let include: HashSet<&str> = input
        .selection
        .include_paths
        .iter()
        .map(String::as_str)
        .collect();
    for path in &plan.main_only {
        if include.contains(path.as_str()) {
            selected.push(MigratePath {
                path: path.clone(),
                state: MigratePathState::MainOnly,
            });
        }
    }
    if selected.is_empty() && plan.dependencies.is_empty() {
        outcome.blockers.push(Blocker::new(
            "NO_CHANGES",
            "Nothing is selected to migrate.",
        ));
        return outcome;
    }

    match apply_and_publish(&input, &selected) {
        Ok((commit_sha, pull_request)) => {
            outcome.ok = true;
            outcome.commit_sha = Some(commit_sha);
            outcome.pull_request = pull_request;
            outcome.applied = selected.iter().map(|p| p.path.clone()).collect();
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

fn apply_and_publish(input: &RunInput<'_>, selected: &[MigratePath]) -> AppResult<Published> {
    let branch = input.plan.branch.clone();
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    );
    let worktree = input
        .temp_root
        .join(format!("migrate-{}-{nonce}", input.entry.id));
    std::fs::create_dir_all(input.temp_root).ok();
    let applied: Vec<String> = selected.iter().map(|path| path.path.clone()).collect();
    let branch_existed = git::local_branch_exists(input.child_dir, &branch);

    let result = populate_worktree(input, selected, &worktree).and_then(|commit_sha| {
        publish(input, &worktree, &branch, &applied).map(|pr| (commit_sha, pr))
    });
    git::worktree_remove(input.child_dir, &worktree);
    // Only a branch this run created may be deleted; a pre-existing one holds
    // commits the operator may still need.
    if !branch_existed {
        let _ = git::run(input.child_dir, &["branch", "-D", &branch]);
    }
    result
}

fn populate_worktree(
    input: &RunInput<'_>,
    selected: &[MigratePath],
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
                "Could not create a migration worktree: {}",
                git::sanitize(&err.to_string())
            ),
        )
        .with_remedy("Check that no other migration is running for this subsystem, then retry.")
    })?;

    let mut code_files: Vec<CodeFile> = Vec::new();
    for item in selected {
        let path = classify::inspect_path_safety(&item.path).map_err(|reason| {
            AppError::invalid_request(format!("'{}' is not a safe path: {reason}", item.path))
        })?;
        let mode = input
            .main
            .index
            .get(&item.path)
            .map(|entry| entry.mode.clone())
            .unwrap_or_else(|| "100644".to_string());
        if mode == "120000" || mode == "160000" {
            return Err(AppError::invalid_request(format!(
                "'{}' is a symlink or submodule entry; migration only copies regular files.",
                item.path
            )));
        }
        let bytes = git::read_blob(&input.main.dir, &input.main.sha, &path).ok_or_else(|| {
            AppError::new(
                ErrorCode::RepositoryInaccessible,
                format!("Could not read '{}' from the main checkout.", item.path),
            )
            .with_remedy("Fetch the main branch again and retry.")
        })?;
        let target = worktree.join(&path);
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
                format!("Could not write '{}': {err}", item.path),
            )
        })?;
        if deps::is_code_path(&path) && bytes.len() <= MAX_BLOB_BYTES {
            code_files.push(CodeFile {
                path: path.clone(),
                content: String::from_utf8_lossy(&bytes).to_string(),
            });
        }
    }

    git::add_all(worktree).map_err(|err| {
        AppError::new(
            ErrorCode::Internal,
            format!(
                "Could not stage the migrated files: {}",
                git::sanitize(&err.to_string())
            ),
        )
    })?;
    for item in selected {
        let executable = input
            .main
            .index
            .get(&item.path)
            .map(|entry| entry.mode == "100755")
            .unwrap_or(false);
        git::update_index_chmod(worktree, &item.path, executable);
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
                format!("Could not write package.json in the migration worktree: {err}"),
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
            "The migration produced no changes on the subsystem branch.",
        )
        .with_remedy("Main and the subsystem already agree; nothing to open a pull request for."));
    }

    let subject = input
        .message
        .map(str::to_string)
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| format!("sync changes from {}", input.main.branch));
    let message = provenance::build_provenance_message(&ProvenanceInput {
        subsystem_id: &input.entry.id,
        source_repo: input.main_repo,
        source_commits: std::slice::from_ref(&input.main.sha),
        mode: MIGRATE_MODE,
        target: &input.entry.source_branch,
        subject: &subject,
    });
    let author = provenance::commit_author(false, None);
    // A linked worktree's `.git` is a file, so the message has to live beside
    // the worktree under the temp root; `commit_with_message` reads it via `-F`.
    let message_file = input.temp_root.join(format!(
        "{}.msg",
        worktree
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "migrate".to_string())
    ));
    std::fs::write(&message_file, message.as_bytes()).map_err(|err| {
        AppError::new(
            ErrorCode::Internal,
            format!("Could not stage the migration commit message: {err}"),
        )
    })?;
    let commit_sha = git::commit_with_message(worktree, &message_file, &author.name, &author.email)
        .map_err(|err| {
            AppError::new(
                ErrorCode::Internal,
                format!(
                    "Could not commit the migration: {}",
                    git::sanitize(&err.to_string())
                ),
            )
            .with_remedy("Retry; if it fails again, inspect the worktree under the temp folder.")
        })?;
    let _ = std::fs::remove_file(&message_file);

    // `git add -A` skips ignored paths, and the worktree is about to be
    // destroyed, so a selected path missing from the commit must fail loudly.
    let mut expected: HashSet<&str> = selected.iter().map(|item| item.path.as_str()).collect();
    if !input.plan.dependencies.is_empty() {
        expected.insert("package.json");
    }
    let committed = committed_paths(worktree, &commit_sha);
    let missing: Vec<&str> = expected
        .iter()
        .copied()
        .filter(|path| !committed.contains(*path))
        .collect();
    if !missing.is_empty() {
        let mut missing = missing;
        missing.sort();
        return Err(AppError::new(
            ErrorCode::Internal,
            format!(
                "The migration commit is missing {} selected path(s): {}.",
                missing.len(),
                missing.join(", ")
            ),
        )
        .with_remedy(
            "The subsystem's .gitignore excludes them; unignore the path or leave it out of the migration.",
        ));
    }
    Ok(commit_sha)
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
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

fn publish(
    input: &RunInput<'_>,
    worktree: &Path,
    branch: &str,
    applied: &[String],
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

    let body = migration_body(input, applied);
    let title = format!(
        "migrate({}): {} → {}",
        input.entry.id, input.main.branch, input.entry.source_branch
    );
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

fn migration_body(input: &RunInput<'_>, applied: &[String]) -> String {
    let mut body = String::new();
    body.push_str(&format!(
        "Reverse migration of `{}` into `{}` for subsystem `{}`.\n\n",
        input.main.branch, input.entry.source_branch, input.entry.id
    ));
    body.push_str(&format!("- Main commit: `{}`\n", input.main.sha));
    if !input.plan.source_sha.is_empty() {
        body.push_str(&format!("- Subsystem base: `{}`\n", input.plan.source_sha));
    }
    body.push_str(&format!("- Paths migrated: {}\n", applied.len()));
    if !input.plan.dependencies.is_empty() {
        body.push_str(&format!(
            "- Dependencies changed: {}\n",
            input.plan.dependencies.len()
        ));
    }
    body.push('\n');
    if !applied.is_empty() {
        body.push_str("Files:\n");
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
    if !input.plan.dependencies.is_empty() {
        body.push_str("Dependencies:\n");
        for dep in &input.plan.dependencies {
            match dep.action {
                MigrateDepAction::Add => body.push_str(&format!(
                    "- add `{}` {} to {}\n",
                    dep.name, dep.spec, dep.section
                )),
                MigrateDepAction::Update => body.push_str(&format!(
                    "- update `{}` {} → {} ({})\n",
                    dep.name,
                    dep.from_spec.clone().unwrap_or_default(),
                    dep.spec,
                    dep.section
                )),
                MigrateDepAction::Move => body.push_str(&format!(
                    "- move `{}` {} from {} to {} ({})\n",
                    dep.name,
                    dep.spec,
                    dep.from_section.clone().unwrap_or_default(),
                    dep.section,
                    dep.from_spec.clone().unwrap_or_default()
                )),
            }
        }
        body.push('\n');
    }
    body.push_str("The lockfile is not written by this migration: run `npm install` and commit the lockfile before merging.\n");
    body
}

fn direction(
    main_blob: &str,
    child_blob: &str,
    child_history: Option<&PathHistory>,
    main_history: Option<&PathHistory>,
) -> Option<MigratePathState> {
    if let Some(history) = child_history {
        if history.blobs.contains(main_blob) {
            // Main's current bytes are bytes the subsystem itself had before:
            // the subsystem moved on, so there is nothing to migrate backwards.
            return None;
        }
    }
    if let Some(history) = main_history {
        if history.blobs.contains(child_blob) {
            return Some(MigratePathState::Changed);
        }
    }
    let child_epoch = child_history.map(|h| h.last_epoch).unwrap_or(0);
    let main_epoch = main_history.map(|h| h.last_epoch).unwrap_or(0);
    if main_epoch > child_epoch {
        Some(MigratePathState::Diverged)
    } else {
        None
    }
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

fn is_deferred(path: &str) -> bool {
    is_lockfile(path) || path == "package.json" || path.ends_with("/package.json")
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

fn history_of(repo_dir: &Path, rev: &str) -> HashMap<String, PathHistory> {
    let mut history: HashMap<String, PathHistory> = HashMap::new();
    let Ok(commits) = live_read::parse_source_log(repo_dir, rev) else {
        return history;
    };
    for commit in &commits {
        for (path, blob, kind) in &commit.deltas {
            if *kind == 'D' || !is_blob_id(blob) || blob.bytes().all(|b| b == b'0') {
                continue;
            }
            let entry = history.entry(path.clone()).or_default();
            entry.blobs.insert(blob.clone());
            entry.last_epoch = entry.last_epoch.max(commit.committed_unix);
        }
    }
    history
}

fn is_blob_id(value: &str) -> bool {
    (value.len() == 40 || value.len() == 64) && value.chars().all(|c| c.is_ascii_hexdigit())
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

fn imported_packages(repo_dir: &Path, index: &HashMap<String, TreeEntry>) -> HashSet<String> {
    let mut paths: Vec<&String> = index.keys().filter(|p| deps::is_code_path(p)).collect();
    paths.sort();
    let shas: Vec<String> = paths
        .iter()
        .filter_map(|path| index.get(*path).map(|entry| entry.blob.clone()))
        .collect();
    let blobs = read_blobs_batch(repo_dir, &shas);
    let mut files = Vec::new();
    for path in paths {
        let Some(entry) = index.get(path) else {
            continue;
        };
        let Some(bytes) = blobs.get(&entry.blob) else {
            continue;
        };
        files.push(CodeFile {
            path: path.clone(),
            content: String::from_utf8_lossy(bytes).to_string(),
        });
    }
    imported_names(&files)
}

fn read_code_files(main: &MainSnapshot, paths: &[MigratePath]) -> Vec<CodeFile> {
    let mut files = Vec::new();
    for item in paths {
        if !deps::is_code_path(&item.path) {
            continue;
        }
        if let Some(bytes) = git::read_blob(&main.dir, &main.sha, &item.path) {
            if bytes.len() <= MAX_BLOB_BYTES {
                files.push(CodeFile {
                    path: item.path.clone(),
                    content: String::from_utf8_lossy(&bytes).to_string(),
                });
            }
        }
    }
    files
}

fn read_blobs_batch(repo_dir: &Path, shas: &[String]) -> HashMap<String, Vec<u8>> {
    let mut blobs = HashMap::new();
    if shas.is_empty() {
        return blobs;
    }
    let input = shas.join("\n") + "\n";
    let Ok(out) = git::run_with_stdin(repo_dir, &["cat-file", "--batch"], input.as_bytes()) else {
        return blobs;
    };
    if out.status != 0 {
        return blobs;
    }
    let bytes = out.stdout;
    let mut cursor = 0;
    while cursor < bytes.len() {
        let Some(newline) = bytes[cursor..].iter().position(|b| *b == b'\n') else {
            break;
        };
        let header = String::from_utf8_lossy(&bytes[cursor..cursor + newline]).to_string();
        cursor += newline + 1;
        let mut cols = header.split(' ');
        let (Some(sha), Some(kind)) = (cols.next(), cols.next()) else {
            break;
        };
        // "missing" rows have no payload, and a skipped blob must not stop the
        // scan: both leave the cursor exactly after the header.
        if kind != "blob" {
            continue;
        }
        let Some(size) = cols.next().and_then(|size| size.parse::<usize>().ok()) else {
            break;
        };
        let Some(end) = cursor.checked_add(size).filter(|end| *end <= bytes.len()) else {
            break;
        };
        if size <= MAX_BLOB_BYTES {
            blobs.insert(sha.to_string(), bytes[cursor..end].to_vec());
        }
        cursor = end + 1;
    }
    blobs
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
    imported: &HashSet<String>,
) -> Vec<MigrateDependency> {
    let mut names: Vec<&String> = imported.iter().collect();
    names.sort();
    let mut plan = Vec::new();
    for name in names {
        let Some((main_section, main_spec)) = declared(main, name) else {
            continue;
        };
        match declared(child, name) {
            None => plan.push(MigrateDependency {
                name: name.clone(),
                action: MigrateDepAction::Add,
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
                    // Overlapping ranges are not worth a rewrite: the subsystem's
                    // own range already admits main's version.
                    let kind = semver::compare_ranges(&child_spec, &main_spec).kind;
                    if matches!(kind, ComparisonKind::Identical | ComparisonKind::Compatible) {
                        continue;
                    }
                    plan.push(MigrateDependency {
                        name: name.clone(),
                        action: MigrateDepAction::Update,
                        from_section: Some(child_section),
                        from_spec: Some(child_spec),
                        section: main_section,
                        spec: main_spec,
                    });
                } else {
                    plan.push(MigrateDependency {
                        name: name.clone(),
                        action: MigrateDepAction::Move,
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

// Main's manifest is the source of truth on the reverse path, so ranges are
// written as-is instead of going through the forward path's fail-closed
// reconciliation; the plan reports every override as a warning.
fn reconciled_manifest(child: &Value, plan: &[MigrateDependency]) -> AppResult<Value> {
    let mut manifest = child.clone();
    let Some(root) = manifest.as_object_mut() else {
        return Err(AppError::invalid_request(
            "The subsystem package.json is not a JSON object.",
        ));
    };
    for dep in plan {
        if dep.action == MigrateDepAction::Move {
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
    plan: &[MigrateDependency],
) -> Vec<Blocker> {
    let added: Vec<String> = plan
        .iter()
        .filter(|dep| dep.action != MigrateDepAction::Update)
        .map(|dep| dep.name.clone())
        .collect();
    deps::check_import_prerequisites(files, manifest, &added)
        .into_iter()
        .map(|missing| {
            Blocker::new(
                "MISSING_DEPENDENCY_PREREQUISITE",
                format!(
                    "'{}' is imported by migrated code but declared in neither the subsystem nor the main manifest.",
                    missing.name
                ),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn blob_batch_survives_an_answer_larger_than_the_pipe_buffer() {
        let dir = std::env::temp_dir().join(format!(
            "vos-blob-batch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let total = 2000usize;
        let filler = "x".repeat(1024);
        for index in 0..total {
            std::fs::write(
                dir.join(format!("file-{index:03}.ts")),
                format!("{index}{filler}"),
            )
            .unwrap();
        }
        let run = |args: &[&str]| git::run(&dir, args).expect("git").stdout_trimmed();
        run(&["init", "-q", "-b", "main"]);
        run(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "add",
            "-A",
        ]);
        run(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-q",
            "-m",
            "fixture",
        ]);
        let shas: Vec<String> = run(&["ls-tree", "-r", "HEAD"])
            .lines()
            .filter_map(|line| line.split_whitespace().nth(2).map(str::to_string))
            .collect();
        assert_eq!(shas.len(), total);

        let (sender, receiver) = std::sync::mpsc::channel();
        let reader_dir = dir.clone();
        std::thread::spawn(move || {
            let blobs = read_blobs_batch(&reader_dir, &shas);
            let _ = sender.send(blobs.len());
        });
        let count = receiver
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("read_blobs_batch deadlocked on an answer batch larger than the pipe buffer");
        assert_eq!(count, total);
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn migration_branch_flattens_unsafe_characters() {
        assert_eq!(migration_branch("master"), "migrate-master");
        assert_eq!(migration_branch("release/1.2"), "migrate-release-1.2");
        assert_eq!(migration_branch("feat x"), "migrate-feat-x");
    }

    #[test]
    fn deferred_paths_are_never_copied() {
        assert!(is_deferred("package.json"));
        assert!(is_deferred("src/modules/hr/package.json"));
        assert!(is_deferred("package-lock.json"));
        assert!(!is_deferred("src/package.json.bak"));
    }

    #[test]
    fn direction_prefers_blob_history_over_clock() {
        let mut child = PathHistory::default();
        child.blobs.insert("main-blob".to_string());
        child.last_epoch = 100;
        assert_eq!(
            direction("main-blob", "child-blob", Some(&child), None),
            None,
            "main's bytes are bytes the subsystem already had: nothing to migrate"
        );

        let mut main_history = PathHistory::default();
        main_history.blobs.insert("child-blob".to_string());
        assert_eq!(
            direction("main-blob", "child-blob", None, Some(&main_history)),
            Some(MigratePathState::Changed)
        );

        let main_newer = PathHistory {
            last_epoch: 200,
            ..PathHistory::default()
        };
        let child_older = PathHistory {
            last_epoch: 100,
            ..PathHistory::default()
        };
        assert_eq!(
            direction(
                "main-blob",
                "child-blob",
                Some(&child_older),
                Some(&main_newer)
            ),
            Some(MigratePathState::Diverged)
        );
        assert_eq!(
            direction(
                "main-blob",
                "child-blob",
                Some(&main_newer),
                Some(&child_older)
            ),
            None
        );
    }

    #[test]
    fn dependency_plan_adds_updates_moves_and_keeps_non_candidates() {
        let main = json!({
            "dependencies": { "react": "19.2.0", "zod": "4.1.0", "left-pad": "1.3.0" },
            "devDependencies": { "typescript": "^5.9.0" }
        });
        let child = json!({
            "dependencies": { "react": "^18.2.0", "lodash": "4.17.21" },
            "devDependencies": { "zod": "3.22.0" }
        });
        let imported: HashSet<String> = ["react", "zod", "lodash"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let plan = dependency_plan(&main, &child, &imported);
        let names: Vec<(&str, MigrateDepAction)> =
            plan.iter().map(|d| (d.name.as_str(), d.action)).collect();
        assert_eq!(
            names,
            vec![
                ("react", MigrateDepAction::Update),
                ("zod", MigrateDepAction::Move),
            ],
            "imported but unchanged deps are skipped and unimported main-only deps are never added"
        );
        let zod = &plan[1];
        assert_eq!(zod.from_section.as_deref(), Some("devDependencies"));
        assert_eq!(zod.section, "dependencies");
        assert_eq!(zod.spec, "4.1.0");
    }

    #[test]
    fn dependency_plan_treats_equivalent_ranges_as_identical() {
        let main = json!({ "dependencies": { "react": "19.2.0" } });
        let child = json!({ "dependencies": { "react": "^19.2.0" } });
        let imported: HashSet<String> = ["react".to_string()].iter().cloned().collect();
        assert!(dependency_plan(&main, &child, &imported).is_empty());
    }

    #[test]
    fn reconciled_manifest_writes_planned_dependencies() {
        let child = json!({ "name": "hr", "dependencies": { "react": "^19.0.0" } });
        let plan = vec![MigrateDependency {
            name: "zod".to_string(),
            action: MigrateDepAction::Add,
            from_section: None,
            from_spec: None,
            section: "dependencies".to_string(),
            spec: "4.1.0".to_string(),
        }];
        let manifest = reconciled_manifest(&child, &plan).expect("reconciled");
        assert_eq!(manifest["dependencies"]["zod"], json!("4.1.0"));
        assert_eq!(manifest["dependencies"]["react"], json!("^19.0.0"));
    }
}
