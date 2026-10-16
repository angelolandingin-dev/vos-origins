//! Three-way apply of selected deltas (spec §6); fail closed on divergence.
//! Scratch stays outside the worktree so no merge junk is ever staged.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::git;

#[derive(Debug, Clone)]
pub struct ApplyConflict {
    pub message: String,
    pub path: Option<String>,
    pub kind: String,
}

impl std::fmt::Display for ApplyConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ApplyConflict {}

impl ApplyConflict {
    fn new(message: impl Into<String>, path: Option<String>, kind: &str) -> Self {
        Self {
            message: message.into(),
            path,
            kind: kind.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyResult {
    pub path: String,
    pub outcome: ApplyOutcome,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    Written,
    Merged,
    Deleted,
    Unchanged,
}

#[derive(Debug, Clone)]
pub struct ModeOp {
    pub path: String,
    pub executable: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ApplyOutcomeSet {
    pub results: Vec<ApplyResult>,
    pub mode_ops: Vec<ModeOp>,
}

/// Reason recorded when an add overwrites an existing target file whose
/// copy is not newer than the source commit (subsystem repos were split
/// from the main system, so shared files appear as adds in fresh history).
pub const ADD_OVER_EXISTING_REASON: &str =
    "updated: add over existing (subsystem version is newer)";

#[derive(Debug, Clone, Default)]
pub struct ApplyOptions {
    /// Paths where an add may overwrite an existing divergent target file.
    /// Membership is decided by the promotion engine's recency guard;
    /// `apply` only consults it, never computes it.
    pub allowed_add_overwrite: HashSet<String>,
}

fn is_binary(bytes: &[u8]) -> bool {
    let len = bytes.len().min(8000);
    bytes[..len].contains(&0)
}

fn is_reparse_or_symlink(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return true;
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
                if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    return true;
                }
            }
            false
        }
        Err(_) => false,
    }
}

/// Symlinked parents/files would let a write escape the worktree, so every
/// existing component is checked; containment is the second line of defence.
pub fn assert_real_path_within(dir: &Path, rel_path: &str) -> Result<(), ApplyConflict> {
    if rel_path.starts_with('/') || rel_path.contains("..") {
        return Err(ApplyConflict::new(
            format!("Refusing to touch a path outside the worktree: {rel_path}"),
            Some(rel_path.to_string()),
            "path-escape",
        ));
    }
    let root = dir.to_path_buf();
    let mut current = root.clone();
    for segment in rel_path.split('/').filter(|s| !s.is_empty()) {
        current.push(segment);
        if is_reparse_or_symlink(&current) {
            return Err(ApplyConflict::new(
                format!("Refusing to traverse a symlink in the target worktree: {rel_path}"),
                Some(rel_path.to_string()),
                "symlink-traversal",
            ));
        }
    }
    Ok(())
}

fn write_file(dir: &Path, rel_path: &str, bytes: &[u8]) -> Result<(), ApplyConflict> {
    assert_real_path_within(dir, rel_path)?;
    let abs = dir.join(rel_path.replace('/', std::path::MAIN_SEPARATOR_STR));
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            ApplyConflict::new(
                format!("Could not create {}: {err}", parent.display()),
                Some(rel_path.to_string()),
                "io",
            )
        })?;
    }
    std::fs::write(&abs, bytes).map_err(|err| {
        ApplyConflict::new(
            format!("Could not write {}: {err}", abs.display()),
            Some(rel_path.to_string()),
            "io",
        )
    })
}

fn read_worktree_file(dir: &Path, rel_path: &str) -> Result<Option<Vec<u8>>, ApplyConflict> {
    assert_real_path_within(dir, rel_path)?;
    let abs = dir.join(rel_path.replace('/', std::path::MAIN_SEPARATOR_STR));
    match std::fs::metadata(&abs) {
        Ok(meta) if meta.is_dir() => Ok(None),
        Ok(_) => std::fs::read(&abs).map(Some).map_err(|err| {
            ApplyConflict::new(
                format!("Could not read {}: {err}", abs.display()),
                Some(rel_path.to_string()),
                "io",
            )
        }),
        Err(_) => Ok(None),
    }
}

fn delete_worktree_file(dir: &Path, rel_path: &str) -> Result<(), ApplyConflict> {
    assert_real_path_within(dir, rel_path)?;
    let abs = dir.join(rel_path.replace('/', std::path::MAIN_SEPARATOR_STR));
    if abs.exists() {
        std::fs::remove_file(&abs).map_err(|err| {
            ApplyConflict::new(
                format!("Could not delete {}: {err}", abs.display()),
                Some(rel_path.to_string()),
                "io",
            )
        })?;
    }
    Ok(())
}

enum MergeOutcome {
    Ok(Vec<u8>),
    Conflict(String),
}

fn three_way_merge(
    scratch_dir: &Path,
    rel_path: &str,
    ours: &[u8],
    base: &[u8],
    theirs: &[u8],
) -> MergeOutcome {
    if is_binary(ours) || is_binary(base) || is_binary(theirs) {
        return MergeOutcome::Conflict("binary content cannot be three-way merged".to_string());
    }
    let tag: String = rel_path
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let ours_file = scratch_dir.join(format!("{tag}.ours"));
    let base_file = scratch_dir.join(format!("{tag}.base"));
    let theirs_file = scratch_dir.join(format!("{tag}.theirs"));
    if std::fs::write(&ours_file, ours).is_err()
        || std::fs::write(&base_file, base).is_err()
        || std::fs::write(&theirs_file, theirs).is_err()
    {
        return MergeOutcome::Conflict("could not create merge scratch files".to_string());
    }
    let args = [
        "merge-file",
        "-p",
        &ours_file.to_string_lossy(),
        &base_file.to_string_lossy(),
        &theirs_file.to_string_lossy(),
    ];
    match git::run_global(&args) {
        Ok(out) if out.status == 0 => MergeOutcome::Ok(out.stdout),
        Ok(out) if out.status > 0 => {
            MergeOutcome::Conflict(format!("{} merge conflict(s)", out.status))
        }
        Ok(out) => MergeOutcome::Conflict(format!("git merge-file error (exit {})", out.status)),
        Err(err) => MergeOutcome::Conflict(format!("git merge-file failed to start: {err}")),
    }
}

pub fn apply_file_delta(
    objects_repo: &Path,
    worktree_dir: &Path,
    source_sha: &str,
    base_sha: Option<&str>,
    entry: &git::DeltaEntry,
    scratch_root: &Path,
    options: &ApplyOptions,
) -> Result<ApplyOutcomeSet, ApplyConflict> {
    let mut set = ApplyOutcomeSet::default();
    let scratch = unique_dir(scratch_root, "vos-merge");
    std::fs::create_dir_all(&scratch)
        .map_err(|err| ApplyConflict::new(format!("scratch dir: {err}"), None, "io"))?;

    let result = apply_entries(
        objects_repo,
        worktree_dir,
        source_sha,
        base_sha,
        entry,
        &scratch,
        &mut set,
        options,
    );
    let _ = std::fs::remove_dir_all(&scratch);
    result?;
    Ok(set)
}

#[allow(clippy::too_many_arguments)]
fn apply_entries(
    objects_repo: &Path,
    worktree_dir: &Path,
    source_sha: &str,
    base_sha: Option<&str>,
    entry: &git::DeltaEntry,
    scratch: &Path,
    set: &mut ApplyOutcomeSet,
    options: &ApplyOptions,
) -> Result<(), ApplyConflict> {
    let mut apply_one = |rel_path: &str| -> Result<(), ApplyConflict> {
        assert_real_path_within(worktree_dir, rel_path)?;
        let mode = git::path_mode(objects_repo, source_sha, rel_path);
        if let Some(m) = mode.as_deref() {
            if m == "120000" || m == "160000" {
                return Err(ApplyConflict::new(
                    format!("Refusing to materialize a symlink/submodule entry: {rel_path}"),
                    Some(rel_path.to_string()),
                    "unsafe-mode",
                ));
            }
        }
        let theirs = git::read_blob(objects_repo, source_sha, rel_path);
        let base = base_sha.and_then(|sha| git::read_blob(objects_repo, sha, rel_path));
        let ours = read_worktree_file(worktree_dir, rel_path)?;

        let record = |set: &mut ApplyOutcomeSet, outcome: ApplyOutcome, reason: Option<&str>| {
            set.results.push(ApplyResult {
                path: rel_path.to_string(),
                outcome,
                reason: reason.map(str::to_string),
            });
        };

        let theirs = match theirs {
            None => {
                if ours.is_none() {
                    record(set, ApplyOutcome::Unchanged, Some("already absent"));
                    return Ok(());
                }
                if base.is_some() && ours == base {
                    delete_worktree_file(worktree_dir, rel_path)?;
                    record(set, ApplyOutcome::Deleted, None);
                    return Ok(());
                }
                return Err(ApplyConflict::new(
                    format!("Target file diverged from the source baseline; refusing to delete {rel_path}"),
                    Some(rel_path.to_string()),
                    "delete-diverge",
                ));
            }
            Some(bytes) => bytes,
        };

        if ours.as_deref() == Some(theirs.as_slice()) {
            record(
                set,
                ApplyOutcome::Unchanged,
                Some("target already matches source"),
            );
            push_mode(set, rel_path, mode.as_deref());
            return Ok(());
        }

        let ours = match ours {
            None => {
                write_file(worktree_dir, rel_path, &theirs)?;
                record(
                    set,
                    ApplyOutcome::Written,
                    Some(if base.is_none() {
                        "added"
                    } else {
                        "restored-add"
                    }),
                );
                push_mode(set, rel_path, mode.as_deref());
                return Ok(());
            }
            Some(existing) => existing,
        };

        let base = match base {
            None => {
                if options.allowed_add_overwrite.contains(rel_path) {
                    write_file(worktree_dir, rel_path, &theirs)?;
                    record(set, ApplyOutcome::Written, Some(ADD_OVER_EXISTING_REASON));
                    push_mode(set, rel_path, mode.as_deref());
                    return Ok(());
                }
                return Err(ApplyConflict::new(
                    format!("Target already contains a newer copy of {rel_path} (main system's copy is newer); cannot apply an add over it"),
                    Some(rel_path.to_string()),
                    "add-over-diverge",
                ));
            }
            Some(bytes) => bytes,
        };

        // Covers binaries that `git merge-file` refuses.
        if ours == base {
            write_file(worktree_dir, rel_path, &theirs)?;
            record(
                set,
                ApplyOutcome::Written,
                Some("target matched the source baseline; source change applied"),
            );
            push_mode(set, rel_path, mode.as_deref());
            return Ok(());
        }

        match three_way_merge(scratch, rel_path, &ours, &base, &theirs) {
            MergeOutcome::Conflict(reason) => Err(ApplyConflict::new(
                format!("Three-way merge failed for {rel_path}: {reason}"),
                Some(rel_path.to_string()),
                "merge-conflict",
            )),
            MergeOutcome::Ok(content) => {
                if content == ours {
                    record(
                        set,
                        ApplyOutcome::Unchanged,
                        Some("merge produced no net change"),
                    );
                } else {
                    write_file(worktree_dir, rel_path, &content)?;
                    record(set, ApplyOutcome::Merged, None);
                }
                push_mode(set, rel_path, mode.as_deref());
                Ok(())
            }
        }
    };

    if entry.status == 'D' {
        apply_one(entry.path.as_deref().unwrap_or_default())?;
    } else if entry.status == 'R' {
        // Rename: the old path goes away, the new path carries the content.
        apply_one(entry.old_path.as_deref().unwrap_or_default())?;
        apply_one(entry.new_path.as_deref().unwrap_or_default())?;
    } else if entry.status == 'C' {
        // Copy: the source stays. Only the new path is materialized; the old
        // path must never be merged or rewritten as if it moved.
        apply_one(entry.new_path.as_deref().unwrap_or_default())?;
    } else {
        apply_one(entry.path.as_deref().unwrap_or_default())?;
    }
    Ok(())
}

fn push_mode(set: &mut ApplyOutcomeSet, rel_path: &str, mode: Option<&str>) {
    if let Some(m) = mode {
        if m == "100755" || m == "100644" {
            set.mode_ops.push(ModeOp {
                path: rel_path.to_string(),
                executable: m == "100755",
            });
        }
    }
}

pub fn apply_mode_ops(worktree_dir: &Path, mode_ops: &[ModeOp]) {
    for op in mode_ops {
        git::update_index_chmod(worktree_dir, &op.path, op.executable);
    }
}

fn unique_dir(root: &Path, prefix: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    root.join(format!("{prefix}-{pid}-{nanos}-{n}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_traversal_and_absolute_paths() {
        let dir = std::env::temp_dir();
        assert!(assert_real_path_within(&dir, "../evil.txt").is_err());
        assert!(assert_real_path_within(&dir, "/etc/passwd").is_err());
        assert!(assert_real_path_within(&dir, "src/app.ts").is_ok());
    }
}
