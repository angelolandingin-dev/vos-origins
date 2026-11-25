//! Snapshot-based bidirectional folder reconciliation engine (`RECONCILE-DESIGN.md`).
//! Overwrites selected directories byte-for-byte from source to destination while
//! preserving path-safety, secret, globalFiles, lockfile, and manifest guards.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::apply;
use super::classify::{self, PathStatus};
use super::deps::{self, CodeFile};
use super::distribute::{self, DistributionDepAction, DistributionDependency};
use super::git;
use super::github_ops::{CreatePrRequest, GitHubOps, PullRequestInfo};
use super::provenance::{self, ProvenanceInput};
use super::semver::{self, ComparisonKind};
use super::{is_branch_checked_out_elsewhere, is_global_match, stringify_manifest, Blocker};
use crate::error::{AppError, AppResult, ErrorCode};
use crate::fixtures;
use crate::github::gh_cli::GitHubAccount;
use crate::github::models::{DiffHunk, PromotionDelivery};
use crate::services::live_read;
use crate::services::reconcile_scope;
use crate::services::registry::{DependencyStrategy, PackageJsonPolicy, SubsystemEntry};

pub const RECONCILE_PROGRESS_EVENT: &str = "reconcile-progress";
pub const RECONCILE_MODE: &str = "reconcile";

const MAX_BLOB_BYTES: usize = 1024 * 1024;
const MAX_LISTED_PATHS: usize = 200;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileDirection {
    #[default]
    SubsystemToMain,
    MainToSubsystem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileNodeKind {
    Directory,
    File,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileDirectoryNode {
    pub kind: ReconcileNodeKind,
    pub path: String,
    pub name: String,
    pub parent: Option<String>,
    pub depth: usize,
    pub has_children: bool,
    pub added: usize,
    pub modified: usize,
    pub deleted: usize,
    pub unchanged: usize,
    pub total_source_files: usize,
    pub exists_in_dest: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileTreeSnapshot {
    pub direction: ReconcileDirection,
    pub subsystem_id: String,
    pub source_repo: String,
    pub source_branch: String,
    pub source_sha: String,
    pub dest_repo: String,
    pub dest_branch: String,
    pub dest_sha: String,
    pub directories: Vec<ReconcileDirectoryNode>,
    pub saved_folders: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileFileAction {
    Add,
    Overwrite,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileFile {
    pub path: String,
    pub folder: String,
    pub action: ReconcileFileAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileSkip {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconcilePlan {
    pub direction: ReconcileDirection,
    pub subsystem_id: String,
    pub subsystem_name: String,
    pub source_repo: String,
    pub source_branch: String,
    pub source_sha: String,
    pub dest_repo: String,
    pub dest_branch: String,
    pub dest_sha: String,
    pub branch: String,
    pub delivery: PromotionDelivery,
    pub folders: Vec<String>,
    pub files: Vec<ReconcileFile>,
    pub unchanged_count: usize,
    pub skipped: Vec<ReconcileSkip>,
    pub dependencies: Vec<DistributionDependency>,
    pub blockers: Vec<Blocker>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileFileDiff {
    pub path: String,
    pub action: ReconcileFileAction,
    pub language: String,
    pub hunks: Vec<DiffHunk>,
    pub binary: bool,
    pub truncated: bool,
    pub source_url: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileOutcome {
    pub ok: bool,
    pub direction: ReconcileDirection,
    pub subsystem_id: String,
    pub source_branch: String,
    pub source_sha: String,
    pub dest_branch: String,
    pub branch: String,
    pub delivery: PromotionDelivery,
    pub commit_sha: Option<String>,
    pub pull_request: Option<PullRequestInfo>,
    pub applied: Vec<String>,
    pub overwritten: Vec<String>,
    pub deleted: Vec<String>,
    pub dependencies: Vec<DistributionDependency>,
    pub blockers: Vec<Blocker>,
    pub warnings: Vec<String>,
    pub local_commands: Vec<String>,
    pub message: Option<String>,
}

pub struct InspectTreeInput<'a> {
    pub direction: ReconcileDirection,
    pub entry: &'a SubsystemEntry,
    pub main_repo: &'a str,
    pub main_branch: &'a str,
    pub source_dir: &'a Path,
    pub source_branch: &'a str,
    pub source_sha: &'a str,
    pub dest_dir: &'a Path,
    pub dest_branch: &'a str,
    pub dest_sha: &'a str,
    pub saved_folders: Vec<String>,
}

pub struct PlanInput<'a> {
    pub direction: ReconcileDirection,
    pub entry: &'a SubsystemEntry,
    pub main_repo: &'a str,
    pub main_branch: &'a str,
    pub source_dir: &'a Path,
    pub source_branch: &'a str,
    pub source_sha: &'a str,
    pub dest_dir: &'a Path,
    pub dest_branch: &'a str,
    pub dest_sha: &'a str,
    pub delivery: PromotionDelivery,
    pub folders: &'a [String],
    pub excluded_paths: &'a HashSet<String>,
    pub github: &'a dyn GitHubOps,
}

pub struct RunInput<'a> {
    pub plan: &'a ReconcilePlan,
    pub entry: &'a SubsystemEntry,
    pub source_dir: &'a Path,
    pub dest_dir: &'a Path,
    pub excluded_paths: &'a HashSet<String>,
    pub message: Option<&'a str>,
    pub attribute_to_account: bool,
    pub account: Option<GitHubAccount>,
    pub temp_root: &'a Path,
    pub remote_url: Option<&'a str>,
    pub github: &'a dyn GitHubOps,
}

fn safe_branch_segment(raw: &str, fallback: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches(['-', '.']);
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn reconcile_branch_name(
    direction: ReconcileDirection,
    subsystem_id: &str,
    main_branch: &str,
    source_sha: &str,
) -> String {
    let safe_main = safe_branch_segment(main_branch, "trunk");
    let short: String = source_sha.chars().take(7).collect();
    match direction {
        ReconcileDirection::SubsystemToMain => {
            let safe_sub = safe_branch_segment(subsystem_id, "subsystem");
            format!("reconcile/{safe_sub}-{safe_main}-{short}")
        }
        ReconcileDirection::MainToSubsystem => format!("reconcile/{safe_main}-{short}"),
    }
}

fn is_reconcilable_for_tree(
    direction: ReconcileDirection,
    entry: &SubsystemEntry,
    path: &str,
    mode: Option<&str>,
) -> bool {
    if direction == ReconcileDirection::SubsystemToMain {
        if let Some(globals) = entry.global_files.as_deref() {
            if is_global_match(path, globals) {
                return false;
            }
        }
    }
    let classification = classify::classify_path(path, &entry.integration_policy, mode);
    matches!(
        classification.status,
        PathStatus::Apply | PathStatus::ExplicitReview
    )
}

#[derive(Default)]
struct DirTallies {
    added: usize,
    modified: usize,
    deleted: usize,
    unchanged: usize,
    total_source_files: usize,
    exists_in_dest: bool,
}

fn collect_ancestor_dirs(path: &str) -> Vec<String> {
    let mut dirs = Vec::new();
    let mut current = path;
    while let Some((parent, _)) = current.rsplit_once('/') {
        if parent.is_empty() {
            break;
        }
        dirs.push(parent.to_string());
        current = parent;
    }
    dirs
}

fn emit_dfs(
    parent: Option<&str>,
    child_dirs: &BTreeMap<Option<String>, Vec<String>>,
    child_files: &BTreeMap<Option<String>, Vec<String>>,
    dir_nodes: &mut BTreeMap<String, ReconcileDirectoryNode>,
    file_nodes: &mut BTreeMap<String, ReconcileDirectoryNode>,
    out: &mut Vec<ReconcileDirectoryNode>,
) {
    let key = parent.map(str::to_string);
    if let Some(sub_dirs) = child_dirs.get(&key) {
        for dir_path in sub_dirs {
            if let Some(node) = dir_nodes.remove(dir_path) {
                out.push(node);
            }
            emit_dfs(
                Some(dir_path),
                child_dirs,
                child_files,
                dir_nodes,
                file_nodes,
                out,
            );
        }
    }
    if let Some(sub_files) = child_files.get(&key) {
        for file_path in sub_files {
            if let Some(node) = file_nodes.remove(file_path) {
                out.push(node);
            }
        }
    }
}

pub fn inspect_tree(input: InspectTreeInput<'_>) -> AppResult<ReconcileTreeSnapshot> {
    let source_tree = distribute::tree_index(input.source_dir, input.source_sha);
    let dest_tree = distribute::tree_index(input.dest_dir, input.dest_sha);

    let (source_repo, dest_repo) = match input.direction {
        ReconcileDirection::SubsystemToMain => {
            (input.entry.repository.clone(), input.main_repo.to_string())
        }
        ReconcileDirection::MainToSubsystem => {
            (input.main_repo.to_string(), input.entry.repository.clone())
        }
    };

    let mut dirs: BTreeMap<String, DirTallies> = BTreeMap::new();
    for path in source_tree.keys() {
        for dir in collect_ancestor_dirs(path) {
            dirs.entry(dir).or_default();
        }
    }

    let mut file_nodes: BTreeMap<String, ReconcileDirectoryNode> = BTreeMap::new();
    for (path, src_entry) in &source_tree {
        let classification = classify::classify_path(
            path,
            &input.entry.integration_policy,
            Some(src_entry.mode.as_str()),
        );
        if matches!(
            classification.status,
            PathStatus::Apply | PathStatus::ExplicitReview
        ) {
            let (parent, name) = match path.rsplit_once('/') {
                Some((p, n)) => (Some(p.to_string()), n.to_string()),
                None => (None, path.clone()),
            };
            let dst_entry = dest_tree.get(path);
            let (added, modified, unchanged, exists_in_dest) = match dst_entry {
                None => (1, 0, 0, false),
                Some(dst) if dst.blob == src_entry.blob && dst.mode == src_entry.mode => {
                    (0, 0, 1, true)
                }
                Some(_) => (0, 1, 0, true),
            };
            file_nodes.insert(
                path.clone(),
                ReconcileDirectoryNode {
                    kind: ReconcileNodeKind::File,
                    path: path.clone(),
                    name,
                    parent,
                    depth: path.matches('/').count(),
                    has_children: false,
                    added,
                    modified,
                    deleted: 0,
                    unchanged,
                    total_source_files: 1,
                    exists_in_dest,
                },
            );
        }

        if !is_reconcilable_for_tree(
            input.direction,
            input.entry,
            path,
            Some(src_entry.mode.as_str()),
        ) {
            continue;
        }
        let ancestors = collect_ancestor_dirs(path);
        match dest_tree.get(path) {
            None => {
                for dir in ancestors {
                    if let Some(t) = dirs.get_mut(&dir) {
                        t.added += 1;
                        t.total_source_files += 1;
                    }
                }
            }
            Some(dst_entry) => {
                let identical =
                    src_entry.blob == dst_entry.blob && src_entry.mode == dst_entry.mode;
                for dir in ancestors {
                    if let Some(t) = dirs.get_mut(&dir) {
                        t.exists_in_dest = true;
                        t.total_source_files += 1;
                        if identical {
                            t.unchanged += 1;
                        } else {
                            t.modified += 1;
                        }
                    }
                }
            }
        }
    }

    for (path, dst_entry) in &dest_tree {
        if source_tree.contains_key(path) {
            continue;
        }
        if !is_reconcilable_for_tree(
            input.direction,
            input.entry,
            path,
            Some(dst_entry.mode.as_str()),
        ) {
            continue;
        }
        for dir in collect_ancestor_dirs(path) {
            if let Some(t) = dirs.get_mut(&dir) {
                t.exists_in_dest = true;
                t.deleted += 1;
            }
        }
    }

    let mut child_dirs: BTreeMap<Option<String>, Vec<String>> = BTreeMap::new();
    for path in dirs.keys() {
        let parent = path.rsplit_once('/').map(|(p, _)| p.to_string());
        child_dirs.entry(parent).or_default().push(path.clone());
    }
    let mut child_files: BTreeMap<Option<String>, Vec<String>> = BTreeMap::new();
    for (path, node) in &file_nodes {
        child_files
            .entry(node.parent.clone())
            .or_default()
            .push(path.clone());
    }

    let mut dir_nodes: BTreeMap<String, ReconcileDirectoryNode> = dirs
        .into_iter()
        .map(|(path, tallies)| {
            let (parent, name) = match path.rsplit_once('/') {
                Some((p, n)) => (Some(p.to_string()), n.to_string()),
                None => (None, path.clone()),
            };
            let depth = path.matches('/').count();
            let key = Some(path.clone());
            let has_children = child_dirs.contains_key(&key) || child_files.contains_key(&key);
            (
                path.clone(),
                ReconcileDirectoryNode {
                    kind: ReconcileNodeKind::Directory,
                    path,
                    name,
                    parent,
                    depth,
                    has_children,
                    added: tallies.added,
                    modified: tallies.modified,
                    deleted: tallies.deleted,
                    unchanged: tallies.unchanged,
                    total_source_files: tallies.total_source_files,
                    exists_in_dest: tallies.exists_in_dest,
                },
            )
        })
        .collect();

    let mut directories = Vec::with_capacity(dir_nodes.len() + file_nodes.len());
    emit_dfs(
        None,
        &child_dirs,
        &child_files,
        &mut dir_nodes,
        &mut file_nodes,
        &mut directories,
    );

    Ok(ReconcileTreeSnapshot {
        direction: input.direction,
        subsystem_id: input.entry.id.clone(),
        source_repo,
        source_branch: input.source_branch.to_string(),
        source_sha: input.source_sha.to_string(),
        dest_repo,
        dest_branch: input.dest_branch.to_string(),
        dest_sha: input.dest_sha.to_string(),
        directories,
        saved_folders: input.saved_folders,
    })
}

fn matching_selection<'a>(path: &str, selected: &'a [String]) -> Option<(&'a str, bool)> {
    for entry in selected {
        if path == entry {
            return Some((entry.as_str(), true));
        }
        if path.starts_with(&format!("{entry}/")) {
            return Some((entry.as_str(), false));
        }
    }
    None
}

pub fn plan(input: PlanInput<'_>) -> ReconcilePlan {
    let delivery = match input.direction {
        ReconcileDirection::MainToSubsystem => PromotionDelivery::PushPr,
        ReconcileDirection::SubsystemToMain => input.delivery,
    };
    let (source_repo, dest_repo) = match input.direction {
        ReconcileDirection::SubsystemToMain => {
            (input.entry.repository.clone(), input.main_repo.to_string())
        }
        ReconcileDirection::MainToSubsystem => {
            (input.main_repo.to_string(), input.entry.repository.clone())
        }
    };
    let branch = reconcile_branch_name(
        input.direction,
        &input.entry.id,
        input.main_branch,
        input.source_sha,
    );

    let canonical_folders = match reconcile_scope::normalize_folders(input.folders) {
        Ok(folders) => folders,
        Err(err) => {
            return ReconcilePlan {
                direction: input.direction,
                subsystem_id: input.entry.id.clone(),
                subsystem_name: input.entry.name.clone(),
                source_repo,
                source_branch: input.source_branch.to_string(),
                source_sha: input.source_sha.to_string(),
                dest_repo,
                dest_branch: input.dest_branch.to_string(),
                dest_sha: input.dest_sha.to_string(),
                branch,
                delivery,
                folders: Vec::new(),
                files: Vec::new(),
                unchanged_count: 0,
                skipped: Vec::new(),
                dependencies: Vec::new(),
                blockers: vec![Blocker::new("INVALID_FOLDER_SCOPE", err.message)],
                warnings: Vec::new(),
            };
        }
    };

    let mut out = ReconcilePlan {
        direction: input.direction,
        subsystem_id: input.entry.id.clone(),
        subsystem_name: input.entry.name.clone(),
        source_repo,
        source_branch: input.source_branch.to_string(),
        source_sha: input.source_sha.to_string(),
        dest_repo: dest_repo.clone(),
        dest_branch: input.dest_branch.to_string(),
        dest_sha: input.dest_sha.to_string(),
        branch: branch.clone(),
        delivery,
        folders: canonical_folders.clone(),
        files: Vec::new(),
        unchanged_count: 0,
        skipped: Vec::new(),
        dependencies: Vec::new(),
        blockers: Vec::new(),
        warnings: Vec::new(),
    };

    if canonical_folders.is_empty() {
        out.blockers.push(Blocker::new(
            "NO_FOLDERS_SELECTED",
            "Select at least one folder to reconcile.",
        ));
        return out;
    }

    let source_tree = distribute::tree_index(input.source_dir, input.source_sha);
    let dest_tree = distribute::tree_index(input.dest_dir, input.dest_sha);

    let mut missing_folders = Vec::new();
    for entry_path in &canonical_folders {
        let prefix = format!("{entry_path}/");
        let exists = source_tree.contains_key(entry_path)
            || source_tree.keys().any(|path| path.starts_with(&prefix));
        if !exists {
            missing_folders.push(entry_path.clone());
        }
    }
    if !missing_folders.is_empty() {
        out.blockers.push(
            Blocker::new(
                "SOURCE_FOLDER_MISSING",
                format!(
                    "Selected path(s) do not exist in {} ({}): {}. Remove or update them before reconciling.",
                    input.source_branch,
                    out.source_repo,
                    missing_folders.join(", "),
                ),
            )
            .with_details(missing_folders.join("\n")),
        );
        return out;
    }

    let mut all_paths: Vec<String> = source_tree
        .keys()
        .chain(dest_tree.keys())
        .filter(|path| matching_selection(path, &canonical_folders).is_some())
        .cloned()
        .collect();
    all_paths.sort();
    all_paths.dedup();

    let mut lockfile_seen = false;
    let mut explicit_review_paths: Vec<String> = Vec::new();
    let mut explicit_global_paths: Vec<String> = Vec::new();

    for path in all_paths {
        let Some((folder, is_exact_file)) = matching_selection(&path, &canonical_folders) else {
            continue;
        };
        if input.direction == ReconcileDirection::SubsystemToMain {
            if let Some(globals) = input.entry.global_files.as_deref() {
                if is_global_match(&path, globals) {
                    if is_exact_file {
                        if !input.excluded_paths.contains(&path) {
                            explicit_global_paths.push(path.clone());
                        }
                    } else {
                        out.skipped.push(ReconcileSkip {
                            path,
                            reason: "protected by subsystem globalFiles policy".to_string(),
                        });
                        continue;
                    }
                }
            }
        }

        let src_entry = source_tree.get(&path);
        let dst_entry = dest_tree.get(&path);
        let mode_opt = src_entry
            .map(|e| e.mode.as_str())
            .or_else(|| dst_entry.map(|e| e.mode.as_str()));

        let classification =
            classify::classify_path(&path, &input.entry.integration_policy, mode_opt);
        if classification.status == PathStatus::LockfileDeferred {
            lockfile_seen = true;
        }

        match classification.status {
            PathStatus::Apply | PathStatus::ExplicitReview => {
                let action = match (src_entry, dst_entry) {
                    (Some(_), None) => ReconcileFileAction::Add,
                    (Some(s), Some(d)) => {
                        if s.blob == d.blob && s.mode == d.mode {
                            out.unchanged_count += 1;
                            continue;
                        }
                        ReconcileFileAction::Overwrite
                    }
                    (None, Some(_)) => ReconcileFileAction::Delete,
                    (None, None) => continue,
                };
                if classification.status == PathStatus::ExplicitReview
                    && !input.excluded_paths.contains(&path)
                {
                    explicit_review_paths.push(path.clone());
                }
                out.files.push(ReconcileFile {
                    path,
                    folder: folder.to_string(),
                    action,
                });
            }
            _ => {
                out.skipped.push(ReconcileSkip {
                    path,
                    reason: classification.reason,
                });
            }
        }
    }

    if !explicit_global_paths.is_empty() {
        out.warnings.push(format!(
            "Explicitly reconciling protected global file(s): {}. Uncheck if main should keep its own copy.",
            explicit_global_paths.join(", "),
        ));
    }

    if !explicit_review_paths.is_empty() {
        out.warnings.push(format!(
            "Includes shared config / nested manifest path(s): {}. Uncheck them if the destination should keep its own copy.",
            explicit_review_paths.join(", "),
        ));
    }

    let included_write_paths: Vec<String> = out
        .files
        .iter()
        .filter(|file| {
            file.action != ReconcileFileAction::Delete && !input.excluded_paths.contains(&file.path)
        })
        .map(|file| file.path.clone())
        .collect();
    let included_delete_count = out
        .files
        .iter()
        .filter(|file| {
            file.action == ReconcileFileAction::Delete && !input.excluded_paths.contains(&file.path)
        })
        .count();

    if input.entry.dependency_strategy == DependencyStrategy::Reconcile {
        let code_files = read_code_files(input.source_dir, input.source_sha, &included_write_paths);
        let candidates = distribute::imported_names(&code_files);
        if !candidates.is_empty() {
            let source_manifest = distribute::read_manifest(input.source_dir, input.source_sha);
            let dest_manifest = distribute::read_manifest(input.dest_dir, input.dest_sha);
            let has_dest_manifest = dest_manifest
                .as_object()
                .map(|obj| !obj.is_empty())
                .unwrap_or(false);

            if !has_dest_manifest {
                if !distribute::prerequisite_blockers(&code_files, &dest_manifest, &[]).is_empty() {
                    out.blockers.push(Blocker::new(
                        "DEST_MANIFEST_MISSING",
                        format!(
                            "Destination '{}' ({}) has no readable root package.json.",
                            input.dest_branch, dest_repo
                        ),
                    ));
                }
            } else {
                let planned_deps =
                    distribute::dependency_plan(&source_manifest, &dest_manifest, &candidates);

                match input.direction {
                    ReconcileDirection::SubsystemToMain => {
                        if input.entry.integration_policy.package_json
                            == PackageJsonPolicy::ExplicitReview
                        {
                            if !planned_deps.is_empty() {
                                let names: Vec<String> = planned_deps
                                    .iter()
                                    .map(|d| format!("{} ({})", d.name, d.spec))
                                    .collect();
                                out.warnings.push(format!(
                                    "package.json policy is explicit-review; root package.json will not be updated automatically for: {}.",
                                    names.join(", ")
                                ));
                            }
                            out.blockers.extend(distribute::prerequisite_blockers(
                                &code_files,
                                &dest_manifest,
                                &[],
                            ));
                        } else {
                            let mut applicable = Vec::new();
                            for dep in planned_deps {
                                let mut blocked = false;
                                if let Some(from_spec) = dep.from_spec.as_deref() {
                                    match semver::compare_ranges(from_spec, &dep.spec).kind {
                                        ComparisonKind::Incompatible => {
                                            out.blockers.push(Blocker::new(
                                                "INCOMPATIBLE_DEPENDENCY",
                                                format!(
                                                    "Dependency '{}' has incompatible ranges: destination '{}' vs subsystem '{}'.",
                                                    dep.name, from_spec, dep.spec
                                                ),
                                            ));
                                            blocked = true;
                                        }
                                        ComparisonKind::Unknown => {
                                            out.blockers.push(Blocker::new(
                                                "DEPENDENCY_RANGE_UNKNOWN",
                                                format!(
                                                    "Dependency '{}' uses non-semver spec ('{}' vs '{}') that cannot be reconciled safely automatically.",
                                                    dep.name, from_spec, dep.spec
                                                ),
                                            ));
                                            blocked = true;
                                        }
                                        ComparisonKind::Identical | ComparisonKind::Compatible => {}
                                    }
                                }
                                if !blocked {
                                    applicable.push(dep);
                                }
                            }
                            out.dependencies = applicable;
                            if let Ok(updated) =
                                distribute::reconciled_manifest(&dest_manifest, &out.dependencies)
                            {
                                out.blockers.extend(distribute::prerequisite_blockers(
                                    &code_files,
                                    &updated,
                                    &out.dependencies,
                                ));
                            }
                        }
                    }
                    ReconcileDirection::MainToSubsystem => {
                        for dep in &planned_deps {
                            if dep.action == DistributionDepAction::Update {
                                out.warnings.push(format!(
                                    "Overrides the subsystem's {} {} with trunk's {}; the subsystem's code may need changes.",
                                    dep.name,
                                    dep.from_spec.clone().unwrap_or_default(),
                                    dep.spec
                                ));
                            }
                        }
                        out.dependencies = planned_deps;
                        if let Ok(updated) =
                            distribute::reconciled_manifest(&dest_manifest, &out.dependencies)
                        {
                            out.blockers.extend(distribute::prerequisite_blockers(
                                &code_files,
                                &updated,
                                &out.dependencies,
                            ));
                        }
                    }
                }
            }
        }
    }

    if lockfile_seen || !out.dependencies.is_empty() {
        out.warnings.push(
            "Lockfiles are never written: run install in the destination repo and commit the lockfile separately."
                .to_string(),
        );
    }

    // Delivery preflight checks.
    if delivery == PromotionDelivery::PushPr {
        match input.github.list_branches(&dest_repo) {
            Ok(branches) => {
                if let Some(existing) = branches.iter().find(|b| b.name == branch) {
                    out.blockers.push(
                        Blocker::new(
                            "RECONCILE_BRANCH_EXISTS",
                            format!("Branch '{branch}' already exists on {dest_repo}."),
                        )
                        .with_details(format!(
                            "A reconciliation PR may already be open. Merge or delete '{branch}' first (at {}).",
                            existing.sha
                        )),
                    );
                }
            }
            Err(err) => out.warnings.push(format!(
                "Could not list branches on {dest_repo} ({err}); the remote branch collision check was skipped."
            )),
        }
        if git::local_branch_exists(input.dest_dir, &branch) {
            out.blockers.push(Blocker::new(
                "RECONCILE_BRANCH_EXISTS",
                format!(
                    "A local branch '{branch}' already exists in '{}'.",
                    input.dest_dir.display()
                ),
            ));
        }
    } else if input.direction == ReconcileDirection::SubsystemToMain {
        if delivery == PromotionDelivery::Local && git::local_branch_exists(input.dest_dir, &branch)
        {
            out.blockers.push(Blocker::new(
                "RECONCILE_BRANCH_EXISTS",
                format!(
                    "Local branch '{branch}' already exists in '{}'.",
                    input.dest_dir.display()
                ),
            ));
        }
        if matches!(
            delivery,
            PromotionDelivery::LocalDirect | PromotionDelivery::Worktree
        ) && git::local_tip(input.dest_dir, input.dest_branch).is_none()
        {
            out.blockers.push(Blocker::new(
                "TARGET_NOT_LOCAL",
                format!(
                    "Target branch '{}' has no local ref in '{}'; check it out or fetch it first, or choose PR delivery.",
                    input.dest_branch,
                    input.dest_dir.display()
                ),
            ));
        }
        if delivery == PromotionDelivery::Worktree {
            match git::current_branch(input.dest_dir) {
                Some(current) if current == input.dest_branch => {}
                other => {
                    let actual = other.unwrap_or_else(|| "a detached HEAD".to_string());
                    out.blockers.push(Blocker::new(
                        "TARGET_NOT_CHECKED_OUT",
                        format!(
                            "Target branch '{}' is not checked out in '{}' (currently on {actual}); worktree delivery writes into the working tree.",
                            input.dest_branch,
                            input.dest_dir.display()
                        ),
                    ));
                }
            }
        }
        if (delivery == PromotionDelivery::Worktree
            || (delivery == PromotionDelivery::LocalDirect
                && git::current_branch(input.dest_dir).is_some_and(|c| c == input.dest_branch)))
            && !git::status_porcelain(input.dest_dir).trim().is_empty()
        {
            out.blockers.push(Blocker::new(
                "DIRTY_WORKTREE",
                format!(
                    "Your checkout of '{}' in '{}' has uncommitted changes; commit or stash them first.",
                    input.dest_branch,
                    input.dest_dir.display()
                ),
            ));
        }
        if delivery == PromotionDelivery::LocalDirect
            && !git::current_branch(input.dest_dir).is_some_and(|c| c == input.dest_branch)
            && is_branch_checked_out_elsewhere(input.dest_dir, input.dest_branch)
        {
            out.blockers.push(Blocker::new(
                "TARGET_CHECKED_OUT_ELSEWHERE",
                format!(
                    "Target branch '{}' is checked out in a different linked worktree; finish or remove that worktree first.",
                    input.dest_branch
                ),
            ));
        }
    }

    if included_write_paths.is_empty() && included_delete_count == 0 && out.dependencies.is_empty()
    {
        out.warnings
            .push("Nothing to reconcile with the current folder and file selection.".to_string());
    }

    out
}

pub fn file_diff(
    source_dir: &Path,
    source_sha: &str,
    dest_dir: &Path,
    dest_sha: &str,
    raw_path: &str,
    action: ReconcileFileAction,
) -> AppResult<ReconcileFileDiff> {
    let path = classify::inspect_path_safety(raw_path).map_err(|reason| {
        AppError::invalid_request(format!("Invalid path '{raw_path}': {reason}"))
    })?;
    let old_bytes = if action == ReconcileFileAction::Add {
        Vec::new()
    } else {
        git::read_blob(dest_dir, dest_sha, &path).unwrap_or_default()
    };
    let new_bytes = if action == ReconcileFileAction::Delete {
        Vec::new()
    } else {
        git::read_blob(source_dir, source_sha, &path).unwrap_or_default()
    };

    let language = fixtures::language_for(&path).to_string();
    if apply::is_binary(&old_bytes) || apply::is_binary(&new_bytes) {
        return Ok(ReconcileFileDiff {
            path,
            action,
            language,
            hunks: Vec::new(),
            binary: true,
            truncated: false,
            source_url: None,
            note: Some("Binary file — inline diff unavailable.".to_string()),
        });
    }
    if old_bytes.len() > MAX_BLOB_BYTES || new_bytes.len() > MAX_BLOB_BYTES {
        return Ok(ReconcileFileDiff {
            path,
            action,
            language,
            hunks: Vec::new(),
            binary: false,
            truncated: true,
            source_url: None,
            note: Some("File exceeds 1 MiB preview limit.".to_string()),
        });
    }

    let tmp_dir = std::env::temp_dir().join(format!(
        "vos-reconcile-diff-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp_dir).map_err(|err| {
        AppError::new(
            ErrorCode::Internal,
            format!("Could not prepare temp diff directory: {err}"),
        )
    })?;
    let old_file = tmp_dir.join("old");
    let new_file = tmp_dir.join("new");
    let write_res =
        std::fs::write(&old_file, &old_bytes).and_then(|()| std::fs::write(&new_file, &new_bytes));
    if let Err(err) = write_res {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return Err(AppError::new(
            ErrorCode::Internal,
            format!("Could not write temp diff files: {err}"),
        ));
    }

    let old_str = old_file.to_string_lossy().to_string();
    let new_str = new_file.to_string_lossy().to_string();
    let diff_out = git::run_global(&[
        "diff",
        "--no-index",
        "--no-color",
        "--unified=3",
        "--",
        &old_str,
        &new_str,
    ]);
    let _ = std::fs::remove_dir_all(&tmp_dir);

    let out = diff_out.map_err(|err| {
        AppError::new(
            ErrorCode::Internal,
            format!(
                "Could not diff '{path}': {}",
                git::sanitize(&err.to_string())
            ),
        )
    })?;
    // `git diff --no-index` exits 0 when identical, 1 when different, >1 on error.
    if out.status > 1 {
        return Err(AppError::new(
            ErrorCode::Internal,
            format!(
                "git diff failed for '{path}': {}",
                git::sanitize(&out.stderr)
            ),
        ));
    }

    let text = out.stdout_str();
    let (mut hunks, _, _) = live_read::parse_unified_hunks(text.lines());
    let truncated = live_read::bound_hunks(&mut hunks);
    let note = if truncated {
        Some(live_read::truncation_note())
    } else if action == ReconcileFileAction::Delete {
        Some("File exists in destination only and will be deleted to match source.".to_string())
    } else {
        None
    };

    Ok(ReconcileFileDiff {
        path,
        action,
        language,
        hunks,
        binary: false,
        truncated,
        source_url: None,
        note,
    })
}

fn read_code_files(repo_dir: &Path, rev: &str, paths: &[String]) -> Vec<CodeFile> {
    let mut files = Vec::new();
    for path in paths {
        if !deps::is_code_path(path) {
            continue;
        }
        if let Some(bytes) = git::read_blob(repo_dir, rev, path) {
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

pub fn default_commit_subject(folders: &[String]) -> String {
    if folders.len() == 1 {
        format!("sync {}", folders[0])
    } else {
        format!("sync {} paths", folders.len())
    }
}

pub fn run(input: RunInput<'_>) -> ReconcileOutcome {
    let plan = input.plan;
    let mut outcome = ReconcileOutcome {
        ok: false,
        direction: plan.direction,
        subsystem_id: plan.subsystem_id.clone(),
        source_branch: plan.source_branch.clone(),
        source_sha: plan.source_sha.clone(),
        dest_branch: plan.dest_branch.clone(),
        branch: plan.branch.clone(),
        delivery: plan.delivery,
        commit_sha: None,
        pull_request: None,
        applied: Vec::new(),
        overwritten: Vec::new(),
        deleted: Vec::new(),
        dependencies: plan.dependencies.clone(),
        blockers: plan.blockers.clone(),
        warnings: plan.warnings.clone(),
        local_commands: Vec::new(),
        message: None,
    };
    if !plan.blockers.is_empty() {
        return outcome;
    }

    let selected: Vec<&ReconcileFile> = plan
        .files
        .iter()
        .filter(|file| !input.excluded_paths.contains(&file.path))
        .collect();
    if selected.is_empty() && plan.dependencies.is_empty() {
        outcome.blockers.push(Blocker::new(
            "NO_CHANGES",
            "Nothing is selected to reconcile.",
        ));
        return outcome;
    }

    let applied: Vec<String> = selected
        .iter()
        .filter(|f| f.action == ReconcileFileAction::Add)
        .map(|f| f.path.clone())
        .collect();
    let overwritten: Vec<String> = selected
        .iter()
        .filter(|f| f.action == ReconcileFileAction::Overwrite)
        .map(|f| f.path.clone())
        .collect();
    let deleted: Vec<String> = selected
        .iter()
        .filter(|f| f.action == ReconcileFileAction::Delete)
        .map(|f| f.path.clone())
        .collect();

    if plan.delivery == PromotionDelivery::Worktree {
        match run_worktree_stage(&input, &selected) {
            Ok((staged_count, commands)) => {
                outcome.ok = true;
                outcome.branch = plan.dest_branch.clone();
                outcome.applied = applied;
                outcome.overwritten = overwritten;
                outcome.deleted = deleted;
                outcome.local_commands = commands;
                outcome.message = Some(format!(
                    "Staged {} reconciled path(s) in '{}' on '{}'. Review with `git diff --staged`, then commit when ready.",
                    staged_count,
                    input.dest_dir.display(),
                    plan.dest_branch,
                ));
            }
            Err(blocker) => outcome.blockers.push(blocker),
        }
        return outcome;
    }

    match apply_and_deliver(&input, &selected, &applied, &overwritten, &deleted) {
        Ok((commit_sha, pull_request, commands, summary)) => {
            outcome.ok = true;
            outcome.commit_sha = Some(commit_sha);
            outcome.pull_request = pull_request;
            outcome.applied = applied;
            outcome.overwritten = overwritten;
            outcome.deleted = deleted;
            outcome.local_commands = commands;
            outcome.message = Some(summary);
        }
        Err(blocker) => outcome.blockers.push(blocker),
    }
    outcome
}

struct AppliedWork {
    written: Vec<String>,
    deleted: Vec<String>,
    executable_ops: Vec<(String, bool)>,
}

fn apply_selected_into_dir(
    input: &RunInput<'_>,
    selected: &[&ReconcileFile],
    target_dir: &Path,
) -> Result<AppliedWork, Blocker> {
    let source_tree = distribute::tree_index(input.source_dir, &input.plan.source_sha);
    let mut written_paths = Vec::new();
    let mut deleted_paths = Vec::new();
    let mut executable_ops = Vec::new();

    for file in selected {
        let rel = classify::inspect_path_safety(&file.path)
            .map_err(|reason| Blocker::new("UNSAFE_PATH", format!("'{}': {reason}", file.path)))?;
        apply::assert_real_path_within(target_dir, &rel)
            .map_err(|err| Blocker::new("UNSAFE_PATH", err.message))?;
        let abs = target_dir.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));

        if file.action == ReconcileFileAction::Delete {
            if abs.exists() {
                std::fs::remove_file(&abs).map_err(|err| {
                    Blocker::new("ENGINE_ERROR", format!("Could not delete '{rel}': {err}"))
                })?;
                distribute::prune_empty_parents(&abs, target_dir);
            }
            deleted_paths.push(rel);
            continue;
        }

        let mode = source_tree
            .get(&rel)
            .map(|e| e.mode.as_str())
            .unwrap_or("100644");
        if mode == "120000" || mode == "160000" {
            return Err(Blocker::new(
                "UNSAFE_PATH",
                format!("'{rel}' is a symlink or submodule entry."),
            ));
        }

        let bytes =
            git::read_blob(input.source_dir, &input.plan.source_sha, &rel).ok_or_else(|| {
                Blocker::new(
                    "REPOSITORY_INACCESSIBLE",
                    format!(
                        "Could not read '{rel}' from source commit '{}'.",
                        input.plan.source_sha
                    ),
                )
            })?;
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                Blocker::new(
                    "ENGINE_ERROR",
                    format!("Could not create '{}': {err}", parent.display()),
                )
            })?;
        }
        std::fs::write(&abs, &bytes).map_err(|err| {
            Blocker::new("ENGINE_ERROR", format!("Could not write '{rel}': {err}"))
        })?;
        executable_ops.push((rel.clone(), mode == "100755"));
        written_paths.push(rel);
    }

    if !input.plan.dependencies.is_empty() {
        let manifest_path = target_dir.join("package.json");
        let original_text = std::fs::read_to_string(&manifest_path).ok();
        let manifest: Value = original_text
            .as_deref()
            .and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        let updated = distribute::reconciled_manifest(&manifest, &input.plan.dependencies)
            .map_err(|err| Blocker::new("DEPENDENCY_CONFLICT", err.message))?;
        let text = stringify_manifest(&updated, original_text.as_deref());
        std::fs::write(&manifest_path, text).map_err(|err| {
            Blocker::new(
                "ENGINE_ERROR",
                format!("Could not write package.json: {err}"),
            )
        })?;
        written_paths.push("package.json".to_string());
    }

    Ok(AppliedWork {
        written: written_paths,
        deleted: deleted_paths,
        executable_ops,
    })
}

fn run_worktree_stage(
    input: &RunInput<'_>,
    selected: &[&ReconcileFile],
) -> Result<(usize, Vec<String>), Blocker> {
    let repo = input.dest_dir;
    let target_branch = &input.plan.dest_branch;
    let target_tip = &input.plan.dest_sha;

    if git::current_branch(repo).as_deref() != Some(target_branch.as_str()) {
        return Err(Blocker::new(
            "TARGET_NOT_CHECKED_OUT",
            format!(
                "Target branch '{target_branch}' is not checked out in '{}'.",
                repo.display()
            ),
        ));
    }
    if !git::status_porcelain(repo).trim().is_empty() {
        return Err(Blocker::new(
            "DIRTY_WORKTREE",
            format!(
                "Your checkout of '{target_branch}' in '{}' has uncommitted changes.",
                repo.display()
            ),
        ));
    }

    let AppliedWork {
        written,
        deleted,
        executable_ops,
    } = apply_selected_into_dir(input, selected, repo)?;
    let written_set: HashSet<String> = written.iter().cloned().collect();
    let deleted_set: HashSet<String> = deleted.iter().cloned().collect();

    if !written.is_empty() {
        let mut sorted: Vec<&str> = written.iter().map(String::as_str).collect();
        sorted.sort_unstable();
        let mut args: Vec<&str> = vec!["add", "--"];
        args.extend(sorted);
        if let Err(err) = git::run_ok(repo, &args) {
            rollback_paths(repo, target_tip, &written_set, &deleted_set);
            return Err(Blocker::new(
                "ENGINE_ERROR",
                format!(
                    "Could not stage reconciled files ({}); working tree rolled back.",
                    git::sanitize(&err.to_string())
                ),
            ));
        }
    }
    if !deleted.is_empty() {
        let mut sorted: Vec<&str> = deleted.iter().map(String::as_str).collect();
        sorted.sort_unstable();
        let mut args: Vec<&str> = vec!["rm", "--"];
        args.extend(sorted);
        if let Err(err) = git::run_ok(repo, &args) {
            rollback_paths(repo, target_tip, &written_set, &deleted_set);
            return Err(Blocker::new(
                "ENGINE_ERROR",
                format!(
                    "Could not stage reconciled deletions ({}); working tree rolled back.",
                    git::sanitize(&err.to_string())
                ),
            ));
        }
    }
    for (path, executable) in &executable_ops {
        git::update_index_chmod(repo, path, *executable);
    }

    let subject = input
        .message
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "reconcile({}): {}",
                input.entry.id,
                default_commit_subject(&input.plan.folders)
            )
        });
    let safe_subject = subject.replace('\\', "\\\\").replace('"', "\\\"");
    let commands = vec![
        format!("cd {}", repo.display()),
        "git diff --staged".to_string(),
        format!("git commit -m \"{safe_subject}\""),
    ];
    Ok((written.len() + deleted.len(), commands))
}

fn rollback_paths(
    repo: &Path,
    target_tip: &str,
    written: &HashSet<String>,
    deleted: &HashSet<String>,
) {
    let mut paths: Vec<&str> = written
        .iter()
        .chain(deleted.iter())
        .map(String::as_str)
        .collect();
    paths.sort_unstable();
    if !paths.is_empty() {
        let mut args: Vec<&str> = vec!["reset", "-q", "HEAD", "--"];
        args.extend(paths);
        let _ = git::run(repo, &args);
    }
    for rel in written.iter().chain(deleted.iter()) {
        let abs = repo.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        match git::read_blob(repo, target_tip, rel) {
            Some(bytes) => {
                if let Some(parent) = abs.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&abs, &bytes);
            }
            None => {
                let _ = std::fs::remove_file(&abs);
                distribute::prune_empty_parents(&abs, repo);
            }
        }
    }
}

type Delivered = (String, Option<PullRequestInfo>, Vec<String>, String);

fn apply_and_deliver(
    input: &RunInput<'_>,
    selected: &[&ReconcileFile],
    applied: &[String],
    overwritten: &[String],
    deleted: &[String],
) -> Result<Delivered, Blocker> {
    let branch = input.plan.branch.clone();
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let worktree = input
        .temp_root
        .join(format!("reconcile-{}-{nonce}", input.entry.id));
    let _ = std::fs::create_dir_all(input.temp_root);
    let branch_existed = git::local_branch_exists(input.dest_dir, &branch);

    git::worktree_add(input.dest_dir, &worktree, &branch, &input.plan.dest_sha).map_err(|err| {
        Blocker::new(
            "ENGINE_ERROR",
            format!(
                "Could not create a reconciliation worktree: {}",
                git::sanitize(&err.to_string())
            ),
        )
    })?;

    let result = populate_and_commit(input, selected, &worktree).and_then(|commit_sha| {
        deliver_commit(
            input,
            &worktree,
            &branch,
            &commit_sha,
            applied,
            overwritten,
            deleted,
        )
    });

    git::worktree_remove(input.dest_dir, &worktree);
    let _ = std::fs::remove_dir_all(&worktree);
    if !branch_existed && input.plan.delivery != PromotionDelivery::Local {
        let _ = git::run(input.dest_dir, &["branch", "-D", &branch]);
    }
    if result.is_err() && !branch_existed && input.plan.delivery == PromotionDelivery::Local {
        let _ = git::run(input.dest_dir, &["branch", "-D", &branch]);
    }

    result
}

fn populate_and_commit(
    input: &RunInput<'_>,
    selected: &[&ReconcileFile],
    worktree: &Path,
) -> Result<String, Blocker> {
    let AppliedWork {
        written,
        deleted: _deleted,
        executable_ops,
    } = apply_selected_into_dir(input, selected, worktree)?;

    git::add_all(worktree).map_err(|err| {
        Blocker::new(
            "ENGINE_ERROR",
            format!(
                "Could not stage reconciled files: {}",
                git::sanitize(&err.to_string())
            ),
        )
    })?;
    for (path, executable) in &executable_ops {
        git::update_index_chmod(worktree, path, *executable);
    }

    if git::status_porcelain(worktree).trim().is_empty() {
        return Err(Blocker::new(
            "NO_CHANGES",
            "Reconciliation produced no changes against the destination.",
        ));
    }

    let subject = input
        .message
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| default_commit_subject(&input.plan.folders));

    let msg = provenance::build_reconcile_message(&ProvenanceInput {
        subsystem_id: &input.entry.id,
        source_repo: &input.plan.source_repo,
        source_commits: std::slice::from_ref(&input.plan.source_sha),
        mode: RECONCILE_MODE,
        target: &input.plan.dest_branch,
        subject: &subject,
    });
    let author = provenance::commit_author(input.attribute_to_account, input.account.as_ref());
    let msg_file = input.temp_root.join(format!(
        "{}.msg",
        worktree
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "reconcile".to_string())
    ));
    std::fs::write(&msg_file, msg.as_bytes()).map_err(|err| {
        Blocker::new(
            "ENGINE_ERROR",
            format!("Could not write commit message file: {err}"),
        )
    })?;
    let commit_sha = git::commit_with_message(worktree, &msg_file, &author.name, &author.email)
        .map_err(|err| {
            let _ = std::fs::remove_file(&msg_file);
            Blocker::new(
                "ENGINE_ERROR",
                format!(
                    "Could not commit reconciliation: {}",
                    git::sanitize(&err.to_string())
                ),
            )
        })?;
    let _ = std::fs::remove_file(&msg_file);

    let expected: HashSet<&str> = selected
        .iter()
        .map(|f| f.path.as_str())
        .chain(if written.iter().any(|p| p == "package.json") {
            Some("package.json")
        } else {
            None
        })
        .collect();
    let committed = distribute::committed_paths(worktree, &commit_sha);
    let mut missing: Vec<&str> = expected
        .into_iter()
        .filter(|path| !committed.contains(*path))
        .collect();
    if !missing.is_empty() {
        missing.sort_unstable();
        return Err(Blocker::new(
            "STAGED_MISMATCH",
            format!(
                "Reconcile commit is missing {} selected path(s) (likely .gitignored): {}.",
                missing.len(),
                missing.join(", ")
            ),
        ));
    }

    Ok(commit_sha)
}

fn deliver_commit(
    input: &RunInput<'_>,
    worktree: &Path,
    branch: &str,
    commit_sha: &str,
    applied: &[String],
    overwritten: &[String],
    deleted: &[String],
) -> Result<Delivered, Blocker> {
    let plan = input.plan;
    match plan.delivery {
        PromotionDelivery::LocalDirect => {
            let repo = input.dest_dir;
            let dest_branch = &plan.dest_branch;
            let current = git::local_tip(repo, dest_branch).ok_or_else(|| {
                Blocker::new(
                    "TARGET_MOVED",
                    format!("Target branch '{dest_branch}' lost its local ref."),
                )
            })?;
            if current != plan.dest_sha {
                return Err(Blocker::new(
                    "TARGET_MOVED",
                    format!(
                        "Target branch '{dest_branch}' moved while reconciling (expected {}, now {}).",
                        &plan.dest_sha[..7.min(plan.dest_sha.len())],
                        &current[..7.min(current.len())],
                    ),
                ));
            }
            if git::current_branch(repo).as_deref() == Some(dest_branch.as_str()) {
                if !git::status_porcelain(repo).trim().is_empty() {
                    return Err(Blocker::new(
                        "DIRTY_WORKTREE",
                        format!("Checkout of '{dest_branch}' has uncommitted changes."),
                    ));
                }
                git::merge_ff_only(repo, commit_sha).map_err(|err| {
                    Blocker::new(
                        "TARGET_MOVED",
                        format!(
                            "Could not fast-forward '{dest_branch}': {}",
                            git::sanitize(&err.to_string())
                        ),
                    )
                })?;
            } else {
                let refname = format!("refs/heads/{dest_branch}");
                git::update_ref(repo, &refname, commit_sha, &plan.dest_sha).map_err(|err| {
                    Blocker::new(
                        "TARGET_MOVED",
                        format!(
                            "Could not update '{dest_branch}': {}",
                            git::sanitize(&err.to_string())
                        ),
                    )
                })?;
            }
            let commands = vec![
                format!("cd {}", repo.display()),
                format!("git push origin {dest_branch}"),
            ];
            let summary = format!(
                "Committed reconciliation directly onto '{dest_branch}' in '{}' ({}). Push when ready.",
                repo.display(),
                &commit_sha[..7.min(commit_sha.len())]
            );
            Ok((commit_sha.to_string(), None, commands, summary))
        }
        PromotionDelivery::Local => {
            let repo = input.dest_dir;
            let commands = vec![
                format!("cd {}", repo.display()),
                format!("git checkout {branch}"),
                format!("git push -u origin {branch}"),
            ];
            let summary = format!(
                "Created local reconciliation branch '{branch}' in '{}' ({}).",
                repo.display(),
                &commit_sha[..7.min(commit_sha.len())]
            );
            Ok((commit_sha.to_string(), None, commands, summary))
        }
        PromotionDelivery::PushPr => {
            let remote_url = input
                .remote_url
                .filter(|u| !u.trim().is_empty())
                .ok_or_else(|| {
                    Blocker::new(
                        "PUSH_FAILED",
                        "No authenticated push URL available; cannot push branch.",
                    )
                })?;
            git::push(worktree, remote_url, branch).map_err(|err| {
                Blocker::new(
                    "PUSH_FAILED",
                    format!(
                        "Could not push branch '{branch}' to {}: {}",
                        plan.dest_repo,
                        git::sanitize(&err.to_string())
                    ),
                )
            })?;

            let subject = input
                .message
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| default_commit_subject(&plan.folders));
            let title = format!("reconcile({}): {subject}", input.entry.id);
            let body = build_reconcile_pr_body(input, applied, overwritten, deleted);
            let pr = input
                .github
                .create_pull_request(
                    &plan.dest_repo,
                    &CreatePrRequest {
                        head: branch.to_string(),
                        base: plan.dest_branch.clone(),
                        title,
                        body,
                        draft: false,
                    },
                )
                .map_err(|err| {
                    Blocker::new(
                        "PR_CREATION_FAILED",
                        format!("Branch '{branch}' was pushed, but opening the pull request failed: {err}"),
                    )
                })?;
            let pr_note = pr
                .url
                .as_deref()
                .map(|u| format!(" ({u})"))
                .unwrap_or_default();
            let summary = format!(
                "Pushed '{branch}' and opened pull request targeting '{}' on {}{pr_note}.",
                plan.dest_branch, plan.dest_repo
            );
            Ok((commit_sha.to_string(), Some(pr), Vec::new(), summary))
        }
        PromotionDelivery::Worktree => unreachable!("worktree handled earlier"),
    }
}

/// PR body omits `Source-Commit:` trailers intentionally (`RECONCILE-DESIGN.md` §5):
/// merged commits still carry full trailers in git history, while an open
/// single-module reconcile PR will not falsely block `Promote Commits`.
fn build_reconcile_pr_body(
    input: &RunInput<'_>,
    applied: &[String],
    overwritten: &[String],
    deleted: &[String],
) -> String {
    let plan = input.plan;
    let direction_label = match plan.direction {
        ReconcileDirection::SubsystemToMain => "Subsystem → Main",
        ReconcileDirection::MainToSubsystem => "Main → Subsystem",
    };
    let mut body = String::new();
    body.push_str(&format!(
        "Folder reconciliation ({direction_label}) for `{}`.\n\n",
        input.entry.id
    ));
    body.push_str(&format!(
        "- Source: `{}` (`{}` @ `{}`)\n",
        plan.source_repo, plan.source_branch, plan.source_sha
    ));
    body.push_str(&format!(
        "- Destination: `{}` (`{}`)\n",
        plan.dest_repo, plan.dest_branch
    ));
    body.push_str(&format!(
        "- Folders ({}): {}\n",
        plan.folders.len(),
        plan.folders
            .iter()
            .map(|f| format!("`{f}`"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    body.push_str(&format!(
        "- Summary: {} added, {} overwritten, {} deleted\n\n",
        applied.len(),
        overwritten.len(),
        deleted.len()
    ));

    append_path_section(&mut body, "Added", applied);
    append_path_section(&mut body, "Overwritten", overwritten);
    append_path_section(&mut body, "Deleted", deleted);

    if !plan.dependencies.is_empty() {
        body.push_str("Dependencies:\n");
        for dep in &plan.dependencies {
            match dep.action {
                DistributionDepAction::Add => body.push_str(&format!(
                    "- add `{}` {} ({})\n",
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
                    dep.section
                )),
            }
        }
        body.push_str(
            "\nLockfiles are not written; run install in the destination repo separately.\n",
        );
    }
    body
}

fn append_path_section(body: &mut String, heading: &str, paths: &[String]) {
    if paths.is_empty() {
        return;
    }
    body.push_str(&format!("{heading}:\n"));
    for path in paths.iter().take(MAX_LISTED_PATHS) {
        body.push_str(&format!("- `{path}`\n"));
    }
    if paths.len() > MAX_LISTED_PATHS {
        body.push_str(&format!(
            "- … and {} more\n",
            paths.len() - MAX_LISTED_PATHS
        ));
    }
    body.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::engine::github_ops::StubGitHubOps;
    use crate::services::engine::history;
    use serde_json::json;
    use std::path::PathBuf;

    fn temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vos-reconcile-{prefix}-{}-{}",
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
            "id": "hrm",
            "name": "HRM",
            "repository": "org/hrm",
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
    fn inspect_tree_aggregates_counts_and_skips_non_reconcilable() {
        let root = temp_dir("tree");
        let sub_dir = root.join("sub");
        let main_dir = root.join("main");
        std::fs::create_dir_all(&sub_dir).unwrap();
        std::fs::create_dir_all(&main_dir).unwrap();

        git_ok(&sub_dir, &["init", "-q", "-b", "master"]);
        write_file(&sub_dir, "src/modules/hrm/new.ts", "export const a = 1;\n");
        write_file(&sub_dir, "src/modules/hrm/mod.ts", "export const b = 2;\n");
        write_file(&sub_dir, "src/modules/hrm/same.ts", "export const c = 3;\n");
        write_file(&sub_dir, "src/modules/hrm/.env.local", "SECRET=1\n");
        write_file(
            &sub_dir,
            "src/modules/hrm/protected.ts",
            "export const p = 1;\n",
        );
        let sub_sha = git_commit(&sub_dir, "sub");

        git_ok(&main_dir, &["init", "-q", "-b", "master"]);
        write_file(
            &main_dir,
            "src/modules/hrm/mod.ts",
            "export const b = 99;\n",
        );
        write_file(
            &main_dir,
            "src/modules/hrm/same.ts",
            "export const c = 3;\n",
        );
        write_file(&main_dir, "src/modules/hrm/stale/old.ts", "stale\n");
        write_file(
            &main_dir,
            "src/modules/hrm/protected.ts",
            "export const p = 2;\n",
        );
        let main_sha = git_commit(&main_dir, "main");

        let mut entry = test_entry();
        entry.global_files = Some(vec!["src/modules/hrm/protected.ts".to_string()]);

        let snap = inspect_tree(InspectTreeInput {
            direction: ReconcileDirection::SubsystemToMain,
            entry: &entry,
            main_repo: "org/main",
            main_branch: "master",
            source_dir: &sub_dir,
            source_branch: "master",
            source_sha: &sub_sha,
            dest_dir: &main_dir,
            dest_branch: "master",
            dest_sha: &main_sha,
            saved_folders: vec!["src/modules/hrm".to_string()],
        })
        .expect("inspect");

        let hrm = snap
            .directories
            .iter()
            .find(|d| d.path == "src/modules/hrm")
            .expect("hrm dir");
        assert_eq!(hrm.added, 1);
        assert_eq!(hrm.modified, 1);
        assert_eq!(hrm.deleted, 1);
        assert_eq!(hrm.unchanged, 1);
        assert!(hrm.exists_in_dest);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn subsystem_to_main_local_direct_mirrors_folder_leaves_outside_untouched_and_parses_history() {
        let root = temp_dir("mirror-local-direct");
        let sub_dir = root.join("sub");
        let main_dir = root.join("main");
        let work_tmp = root.join("tmp");
        std::fs::create_dir_all(&sub_dir).unwrap();
        std::fs::create_dir_all(&main_dir).unwrap();
        std::fs::create_dir_all(&work_tmp).unwrap();

        git_ok(&sub_dir, &["init", "-q", "-b", "master"]);
        write_file(
            &sub_dir,
            "package.json",
            r#"{"dependencies":{"zod":"4.1.0"}}"#,
        );
        write_file(
            &sub_dir,
            "src/modules/hrm/index.ts",
            "import { z } from 'zod';\nexport const hrm = z.string();\n",
        );
        write_file(
            &sub_dir,
            "src/modules/hrm/new.ts",
            "export const added = true;\n",
        );
        write_file(&sub_dir, "src/modules/hrm/global.ts", "sub copy\n");
        write_file(&sub_dir, "src/modules/hrm/.env", "KEY=secret\n");
        let sub_sha = git_commit(&sub_dir, "sub tip");

        git_ok(&main_dir, &["init", "-q", "-b", "master"]);
        write_file(
            &main_dir,
            "package.json",
            r#"{"dependencies":{"react":"^19.0.0"}}"#,
        );
        write_file(
            &main_dir,
            "src/modules/hrm/index.ts",
            "export const hrm = 'old';\n",
        );
        write_file(&main_dir, "src/modules/hrm/legacy/drop.ts", "drop me\n");
        write_file(&main_dir, "src/modules/hrm/global.ts", "main protected\n");
        write_file(
            &main_dir,
            "src/modules/other/keep.ts",
            "must stay untouched\n",
        );
        let main_sha = git_commit(&main_dir, "main base");

        let mut entry = test_entry();
        entry.global_files = Some(vec!["src/modules/hrm/global.ts".to_string()]);
        let github = StubGitHubOps::new();
        let excluded = HashSet::new();
        let folders = vec!["src/modules/hrm".to_string()];

        let p = plan(PlanInput {
            direction: ReconcileDirection::SubsystemToMain,
            entry: &entry,
            main_repo: "org/main",
            main_branch: "master",
            source_dir: &sub_dir,
            source_branch: "master",
            source_sha: &sub_sha,
            dest_dir: &main_dir,
            dest_branch: "master",
            dest_sha: &main_sha,
            delivery: PromotionDelivery::LocalDirect,
            folders: &folders,
            excluded_paths: &excluded,
            github: &github,
        });
        assert!(p.blockers.is_empty(), "{:?}", p.blockers);
        assert_eq!(p.dependencies.len(), 1);
        assert_eq!(p.dependencies[0].name, "zod");
        assert!(p
            .skipped
            .iter()
            .any(|s| s.path == "src/modules/hrm/global.ts"));
        assert!(p.skipped.iter().any(|s| s.path == "src/modules/hrm/.env"));

        let outcome = run(RunInput {
            plan: &p,
            entry: &entry,
            source_dir: &sub_dir,
            dest_dir: &main_dir,
            excluded_paths: &excluded,
            message: Some("sync hrm module"),
            attribute_to_account: false,
            account: None,
            temp_root: &work_tmp,
            remote_url: None,
            github: &github,
        });
        assert!(outcome.ok, "{:?}", outcome.blockers);
        assert_eq!(outcome.applied, vec!["src/modules/hrm/new.ts".to_string()]);
        assert_eq!(
            outcome.overwritten,
            vec!["src/modules/hrm/index.ts".to_string()]
        );
        assert_eq!(
            outcome.deleted,
            vec!["src/modules/hrm/legacy/drop.ts".to_string()]
        );

        assert!(main_dir.join("src/modules/hrm/new.ts").exists());
        assert!(!main_dir.join("src/modules/hrm/legacy/drop.ts").exists());
        assert!(!main_dir.join("src/modules/hrm/legacy").exists());
        assert_eq!(
            std::fs::read_to_string(main_dir.join("src/modules/hrm/global.ts")).unwrap(),
            "main protected\n"
        );
        assert_eq!(
            std::fs::read_to_string(main_dir.join("src/modules/other/keep.ts")).unwrap(),
            "must stay untouched\n"
        );
        assert!(!main_dir.join("src/modules/hrm/.env").exists());

        let events = history::collect_events(&main_dir, "master");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].mode.as_deref(), Some("reconcile"));
        assert_eq!(events[0].subsystem.as_deref(), Some("hrm"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_source_folder_blocks_and_never_wipes_destination() {
        let root = temp_dir("missing-source");
        let sub_dir = root.join("sub");
        let main_dir = root.join("main");
        std::fs::create_dir_all(&sub_dir).unwrap();
        std::fs::create_dir_all(&main_dir).unwrap();

        git_ok(&sub_dir, &["init", "-q", "-b", "master"]);
        write_file(&sub_dir, "src/other.ts", "ok\n");
        let sub_sha = git_commit(&sub_dir, "sub");

        git_ok(&main_dir, &["init", "-q", "-b", "master"]);
        write_file(&main_dir, "src/modules/hrm/keep.ts", "keep\n");
        let main_sha = git_commit(&main_dir, "main");

        let entry = test_entry();
        let github = StubGitHubOps::new();
        let p = plan(PlanInput {
            direction: ReconcileDirection::SubsystemToMain,
            entry: &entry,
            main_repo: "org/main",
            main_branch: "master",
            source_dir: &sub_dir,
            source_branch: "master",
            source_sha: &sub_sha,
            dest_dir: &main_dir,
            dest_branch: "master",
            dest_sha: &main_sha,
            delivery: PromotionDelivery::LocalDirect,
            folders: &["src/modules/hrm".to_string()],
            excluded_paths: &HashSet::new(),
            github: &github,
        });
        assert!(p.blockers.iter().any(|b| b.code == "SOURCE_FOLDER_MISSING"));
        assert!(p.files.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dependency_rules_block_incompatible_on_sub_to_main_and_warn_on_main_to_sub() {
        let root = temp_dir("deps-rules");
        let sub_dir = root.join("sub");
        let main_dir = root.join("main");
        std::fs::create_dir_all(&sub_dir).unwrap();
        std::fs::create_dir_all(&main_dir).unwrap();

        git_ok(&sub_dir, &["init", "-q", "-b", "master"]);
        write_file(
            &sub_dir,
            "package.json",
            r#"{"dependencies":{"zod":"^4.0.0"}}"#,
        );
        write_file(
            &sub_dir,
            "src/modules/hrm/a.ts",
            "import { z } from 'zod';\nexport const v = z;\n",
        );
        let sub_sha = git_commit(&sub_dir, "sub");

        git_ok(&main_dir, &["init", "-q", "-b", "master"]);
        write_file(
            &main_dir,
            "package.json",
            r#"{"dependencies":{"zod":"^3.22.0"}}"#,
        );
        write_file(
            &main_dir,
            "src/modules/hrm/a.ts",
            "import { z } from 'zod';\nexport const v = z.string();\n",
        );
        let main_sha = git_commit(&main_dir, "main");

        let entry = test_entry();
        let github = StubGitHubOps::new();
        let folders = vec!["src/modules/hrm".to_string()];

        let sub_to_main = plan(PlanInput {
            direction: ReconcileDirection::SubsystemToMain,
            entry: &entry,
            main_repo: "org/main",
            main_branch: "master",
            source_dir: &sub_dir,
            source_branch: "master",
            source_sha: &sub_sha,
            dest_dir: &main_dir,
            dest_branch: "master",
            dest_sha: &main_sha,
            delivery: PromotionDelivery::Local,
            folders: &folders,
            excluded_paths: &HashSet::new(),
            github: &github,
        });
        assert!(sub_to_main
            .blockers
            .iter()
            .any(|b| b.code == "INCOMPATIBLE_DEPENDENCY"));

        let main_to_sub = plan(PlanInput {
            direction: ReconcileDirection::MainToSubsystem,
            entry: &entry,
            main_repo: "org/main",
            main_branch: "master",
            source_dir: &main_dir,
            source_branch: "master",
            source_sha: &main_sha,
            dest_dir: &sub_dir,
            dest_branch: "master",
            dest_sha: &sub_sha,
            delivery: PromotionDelivery::PushPr,
            folders: &folders,
            excluded_paths: &HashSet::new(),
            github: &github,
        });
        assert!(
            main_to_sub.blockers.is_empty(),
            "{:?}",
            main_to_sub.blockers
        );
        assert!(main_to_sub
            .warnings
            .iter()
            .any(|w| w.contains("Overrides the subsystem's zod")));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn push_pr_omits_source_commit_trailer_in_pr_body_and_worktree_stages_without_commit() {
        let root = temp_dir("push-pr-and-worktree");
        let sub_dir = root.join("sub");
        let main_dir = root.join("main");
        let bare_dir = root.join("remote.git");
        let work_tmp = root.join("tmp");
        std::fs::create_dir_all(&sub_dir).unwrap();
        std::fs::create_dir_all(&main_dir).unwrap();
        std::fs::create_dir_all(&bare_dir).unwrap();
        std::fs::create_dir_all(&work_tmp).unwrap();

        git_ok(&bare_dir, &["init", "--bare", "-q"]);
        git_ok(&sub_dir, &["init", "-q", "-b", "master"]);
        write_file(&sub_dir, "src/modules/hrm/x.ts", "export const x = 2;\n");
        let sub_sha = git_commit(&sub_dir, "sub");

        git_ok(&main_dir, &["init", "-q", "-b", "master"]);
        write_file(&main_dir, "src/modules/hrm/x.ts", "export const x = 1;\n");
        let main_sha = git_commit(&main_dir, "main");

        let entry = test_entry();
        let github = StubGitHubOps::new();
        let folders = vec!["src/modules/hrm".to_string()];

        let pr_plan = plan(PlanInput {
            direction: ReconcileDirection::SubsystemToMain,
            entry: &entry,
            main_repo: "org/main",
            main_branch: "master",
            source_dir: &sub_dir,
            source_branch: "master",
            source_sha: &sub_sha,
            dest_dir: &main_dir,
            dest_branch: "master",
            dest_sha: &main_sha,
            delivery: PromotionDelivery::PushPr,
            folders: &folders,
            excluded_paths: &HashSet::new(),
            github: &github,
        });
        let remote_url = bare_dir.to_string_lossy().replace('\\', "/");
        let pr_out = run(RunInput {
            plan: &pr_plan,
            entry: &entry,
            source_dir: &sub_dir,
            dest_dir: &main_dir,
            excluded_paths: &HashSet::new(),
            message: Some("sync x"),
            attribute_to_account: false,
            account: None,
            temp_root: &work_tmp,
            remote_url: Some(&remote_url),
            github: &github,
        });
        assert!(pr_out.ok, "{:?}", pr_out.blockers);
        let created_prs = github.create_pr_calls();
        assert_eq!(created_prs.len(), 1);
        assert!(!created_prs[0].payload.contains("Source-Commit:"));

        let wt_plan = plan(PlanInput {
            direction: ReconcileDirection::SubsystemToMain,
            entry: &entry,
            main_repo: "org/main",
            main_branch: "master",
            source_dir: &sub_dir,
            source_branch: "master",
            source_sha: &sub_sha,
            dest_dir: &main_dir,
            dest_branch: "master",
            dest_sha: &main_sha,
            delivery: PromotionDelivery::Worktree,
            folders: &folders,
            excluded_paths: &HashSet::new(),
            github: &github,
        });
        let wt_out = run(RunInput {
            plan: &wt_plan,
            entry: &entry,
            source_dir: &sub_dir,
            dest_dir: &main_dir,
            excluded_paths: &HashSet::new(),
            message: None,
            attribute_to_account: false,
            account: None,
            temp_root: &work_tmp,
            remote_url: None,
            github: &github,
        });
        assert!(wt_out.ok, "{:?}", wt_out.blockers);
        assert!(wt_out.commit_sha.is_none());
        assert_eq!(git_ok(&main_dir, &["rev-parse", "HEAD"]), main_sha);
        assert!(!git::status_porcelain(&main_dir).trim().is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn individual_file_selection_reconciles_single_global_file_and_keeps_dfs_order() {
        let root = temp_dir("individual-global-file");
        let sub_dir = root.join("sub");
        let main_dir = root.join("main");
        let work_tmp = root.join("tmp");
        std::fs::create_dir_all(&sub_dir).unwrap();
        std::fs::create_dir_all(&main_dir).unwrap();
        std::fs::create_dir_all(&work_tmp).unwrap();

        git_ok(&sub_dir, &["init", "-q", "-b", "master"]);
        write_file(&sub_dir, "src/app/layout.tsx", "export const v = 'sub';\n");
        write_file(&sub_dir, "src/app/other.tsx", "export const o = 'sub';\n");
        write_file(
            &sub_dir,
            "src/app/sub/page.tsx",
            "export const p = 'sub';\n",
        );
        let sub_sha = git_commit(&sub_dir, "sub");

        git_ok(&main_dir, &["init", "-q", "-b", "master"]);
        write_file(
            &main_dir,
            "src/app/layout.tsx",
            "export const v = 'main';\n",
        );
        write_file(&main_dir, "src/app/other.tsx", "export const o = 'main';\n");
        let main_sha = git_commit(&main_dir, "main");

        let mut entry = test_entry();
        entry.global_files = Some(vec!["src/app/layout.tsx".to_string()]);
        let github = StubGitHubOps::new();

        let snap = inspect_tree(InspectTreeInput {
            direction: ReconcileDirection::SubsystemToMain,
            entry: &entry,
            main_repo: "org/main",
            main_branch: "master",
            source_dir: &sub_dir,
            source_branch: "master",
            source_sha: &sub_sha,
            dest_dir: &main_dir,
            dest_branch: "master",
            dest_sha: &main_sha,
            saved_folders: Vec::new(),
        })
        .expect("inspect");

        let app_dir = snap
            .directories
            .iter()
            .find(|n| n.path == "src/app" && n.kind == ReconcileNodeKind::Directory)
            .expect("src/app dir");
        assert_eq!(app_dir.modified, 1);
        assert_eq!(app_dir.added, 1);

        let sub_idx = snap
            .directories
            .iter()
            .position(|n| n.path == "src/app/sub" && n.kind == ReconcileNodeKind::Directory)
            .unwrap();
        let layout_idx = snap
            .directories
            .iter()
            .position(|n| n.path == "src/app/layout.tsx" && n.kind == ReconcileNodeKind::File)
            .unwrap();
        assert!(
            sub_idx < layout_idx,
            "directories must appear before sibling files in DFS order"
        );

        let p = plan(PlanInput {
            direction: ReconcileDirection::SubsystemToMain,
            entry: &entry,
            main_repo: "org/main",
            main_branch: "master",
            source_dir: &sub_dir,
            source_branch: "master",
            source_sha: &sub_sha,
            dest_dir: &main_dir,
            dest_branch: "master",
            dest_sha: &main_sha,
            delivery: PromotionDelivery::LocalDirect,
            folders: &["src/app/layout.tsx".to_string()],
            excluded_paths: &HashSet::new(),
            github: &github,
        });
        assert!(p.blockers.is_empty(), "{:?}", p.blockers);
        assert_eq!(p.files.len(), 1);
        assert_eq!(p.files[0].path, "src/app/layout.tsx");
        assert_eq!(p.files[0].action, ReconcileFileAction::Overwrite);
        assert!(p
            .warnings
            .iter()
            .any(|w| w.contains("Explicitly reconciling protected global file")));

        let out = run(RunInput {
            plan: &p,
            entry: &entry,
            source_dir: &sub_dir,
            dest_dir: &main_dir,
            excluded_paths: &HashSet::new(),
            message: Some("sync layout.tsx"),
            attribute_to_account: false,
            account: None,
            temp_root: &work_tmp,
            remote_url: None,
            github: &github,
        });
        assert!(out.ok, "{:?}", out.blockers);
        assert_eq!(
            std::fs::read_to_string(main_dir.join("src/app/layout.tsx"))
                .unwrap()
                .replace("\r\n", "\n"),
            "export const v = 'sub';\n"
        );
        assert_eq!(
            std::fs::read_to_string(main_dir.join("src/app/other.tsx"))
                .unwrap()
                .replace("\r\n", "\n"),
            "export const o = 'main';\n"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
