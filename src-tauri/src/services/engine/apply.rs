//! Three-way apply of selected deltas (spec §6); fail closed on divergence.
//! Scratch stays outside the worktree so no merge junk is ever staged.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::github::models::PromotionOnConflict;

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
    /// Paths written or deleted via `on_conflict = take_subsystem` instead of
    /// failing. The engine reports these distinctly (`conflict_taken_from_subsystem`).
    pub taken_from_subsystem: Vec<String>,
}

/// Reason recorded when an add overwrites an existing target file whose
/// copy is not newer than the source commit (subsystem repos were split
/// from the main system, so shared files appear as adds in fresh history).
pub const ADD_OVER_EXISTING_REASON: &str = "replaced the target's existing copy";

#[derive(Debug, Clone, Default)]
pub struct ApplyOptions {
    /// Paths where an add may overwrite an existing divergent target file.
    /// Membership is decided by the promotion engine's recency guard;
    /// `apply` only consults it, never computes it.
    pub allowed_add_overwrite: HashSet<String>,
    /// `fail` (default) refuses every unmergeable path; `take_subsystem`
    /// writes the subsystem's content (or applies its deletion) and records
    /// the path in [`ApplyOutcomeSet::taken_from_subsystem`]. Never bypasses
    /// path-safety, classification, dependency, lockfile, or NOOP guards —
    /// those fail closed before this policy is ever consulted.
    pub on_conflict: PromotionOnConflict,
}

pub(crate) fn is_binary(bytes: &[u8]) -> bool {
    let len = bytes.len().min(8000);
    bytes[..len].contains(&0)
}

/// CRLF→LF for text comparison and merge input so a `core.autocrlf=true`
/// checkout (CRLF bytes in the worktree, LF bytes in blobs) never fakes a
/// divergence. Only ever called when none of the inputs is binary; binaries
/// keep exact byte semantics and are never normalised.
pub(crate) fn strip_cr(bytes: &[u8]) -> Vec<u8> {
    if !bytes.contains(&b'\r') {
        return bytes.to_vec();
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' && i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
            out.push(b'\n');
            i += 2;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
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
    // Whole-segment ".." only. A substring check also matches legitimate names
    // like Next.js catch-all routes (`[...path]`), which made the apply refuse a
    // path the classifier had already accepted. `classify::inspect_path_safety`
    // performs the same segment-wise check, so the two now agree.
    let normalised = rel_path.replace('\\', "/");
    let bytes = normalised.as_bytes();
    let drive_letter = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if normalised.starts_with('/')
        || drive_letter
        || normalised.split('/').any(|segment| segment == "..")
    {
        return Err(ApplyConflict::new(
            format!("Refusing to touch a path outside the worktree: {rel_path}"),
            Some(rel_path.to_string()),
            "path-escape",
        ));
    }
    let root = dir.to_path_buf();
    let mut current = root.clone();
    for segment in normalised.split('/').filter(|s| !s.is_empty()) {
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

pub(crate) enum MergeOutcome {
    Ok(Vec<u8>),
    Conflict(#[allow(dead_code)] String),
}

/// Three-way text merge shared by the real apply and the preflight dry run
/// (both call this helper, so they can never disagree).
///
/// Runs scratch `git merge-file -p --diff3`. When it reports conflicts, each
/// `--diff3` block is post-processed: a block whose `ours` and `theirs` sides
/// are equivalent ignoring whitespace (per-line trim + collapsing interior
/// whitespace runs) is resolved automatically by keeping the **target's
/// (`ours`) block bytes**, so the main system's formatting is preserved and
/// nothing is re-indented. Blocks that differ by real content stay conflicting
/// and are returned as [`MergeOutcome::Conflict`] (governed by the caller's
/// `on_conflict` policy).
pub(crate) fn three_way_merge(
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
        "--diff3",
        &ours_file.to_string_lossy(),
        &base_file.to_string_lossy(),
        &theirs_file.to_string_lossy(),
    ];
    match git::run_global(&args) {
        Ok(out) if out.status == 0 => MergeOutcome::Ok(out.stdout),
        Ok(out) if out.status > 0 => match resolve_whitespace_only_blocks(&out.stdout) {
            Some(resolved) => MergeOutcome::Ok(resolved),
            None => MergeOutcome::Conflict(format!("{} merge conflict(s)", out.status)),
        },
        Ok(out) => MergeOutcome::Conflict(format!("git merge-file error (exit {})", out.status)),
        Err(err) => MergeOutcome::Conflict(format!("git merge-file failed to start: {err}")),
    }
}

/// Post-process `git merge-file --diff3` conflict output: `Some(merged)` when
/// every conflict block is whitespace-only (resolved with the `ours` block
/// bytes), `None` when any block differs by real content or the markers are
/// malformed (fail safe: still a conflict).
///
/// Operates on raw bytes split at `\n`; reassembly is an exact round-trip, so
/// resolved blocks keep the target's bytes verbatim (indentation, trailing
/// newline state, everything outside the stripped whitespace comparison).
fn resolve_whitespace_only_blocks(output: &[u8]) -> Option<Vec<u8>> {
    let lines: Vec<&[u8]> = output.split(|b| *b == b'\n').collect();
    // Fast path: no conflict markers at all. (merge-file only exits nonzero
    // on conflicts, so this is defensive; treating marker-free output as a
    // clean merge would risk silently accepting garbage.)
    if !lines.iter().any(|l| l.starts_with(b"<<<<<<<")) {
        return None;
    }
    let mut merged: Vec<&[u8]> = Vec::with_capacity(lines.len());
    let mut i = 0;
    let mut saw_block = false;
    while i < lines.len() {
        if !lines[i].starts_with(b"<<<<<<<") {
            merged.push(lines[i]);
            i += 1;
            continue;
        }
        saw_block = true;
        i += 1;
        let mut ours_block: Vec<&[u8]> = Vec::new();
        while i < lines.len() && !lines[i].starts_with(b"|||||||") {
            ours_block.push(lines[i]);
            i += 1;
        }
        if i >= lines.len() {
            return None;
        }
        i += 1; // consume `||||||| base`
        while i < lines.len() && lines[i] != b"=======" {
            i += 1; // the base side never decides whitespace equivalence
        }
        if i >= lines.len() {
            return None;
        }
        i += 1; // consume `=======`
        let mut theirs_block: Vec<&[u8]> = Vec::new();
        while i < lines.len() && !lines[i].starts_with(b">>>>>>>") {
            theirs_block.push(lines[i]);
            i += 1;
        }
        if i >= lines.len() {
            return None;
        }
        i += 1; // consume `>>>>>>> theirs`
        if !blocks_equal_ignoring_whitespace(&ours_block, &theirs_block) {
            return None;
        }
        merged.extend(ours_block);
    }
    if !saw_block {
        return None;
    }
    // `split`/`join` at `\n` round-trips the input exactly, so every kept
    // byte (ours blocks, clean regions, trailing newline state) is verbatim.
    Some(merged.join(&b'\n'))
}

/// Block equivalence ignoring whitespace: each line is trimmed and interior
/// runs of ASCII whitespace collapse to a single space before comparing.
/// Re-indentation (leading runs) and trailing whitespace vanish; `hello world`
/// vs `helloworld` still differ (a run is not nothing).
fn blocks_equal_ignoring_whitespace(ours: &[&[u8]], theirs: &[&[u8]]) -> bool {
    if ours.len() != theirs.len() {
        return false;
    }
    ours.iter()
        .zip(theirs.iter())
        .all(|(a, b)| normalise_ws_line(a) == normalise_ws_line(b))
}

fn normalise_ws_line(line: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(line.len());
    let mut pending_space = false;
    let mut started = false;
    for &b in line {
        if b.is_ascii_whitespace() {
            if started {
                pending_space = true;
            }
        } else {
            if pending_space {
                out.push(b' ');
                pending_space = false;
            }
            started = true;
            out.push(b);
        }
    }
    out
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
    _scratch: &Path,
    set: &mut ApplyOutcomeSet,
    _options: &ApplyOptions,
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
        let theirs_opt = git::read_blob(objects_repo, source_sha, rel_path);
        let base_opt = base_sha.and_then(|sha| git::read_blob(objects_repo, sha, rel_path));
        let ours_opt = read_worktree_file(worktree_dir, rel_path)?;

        // Copy semantics: the subsystem owns this path, so the target's copy is
        // written outright - never merged. The main system is never edited
        // directly (every change arrives through a promotion), so reconciling a
        // delta produced only false conflicts: CRLF vs LF, whole-file
        // re-indentation, import unions, and add-vs-modify ambiguity.
        let record = |set: &mut ApplyOutcomeSet, outcome: ApplyOutcome, reason: Option<&str>| {
            set.results.push(ApplyResult {
                path: rel_path.to_string(),
                outcome,
                reason: reason.map(str::to_string),
            });
        };
        // Line endings stay irrelevant: a `core.autocrlf=true` checkout holds
        // CRLF worktree bytes while blobs hold LF, so equality is decided on
        // CRLF-stripped text and binaries keep exact byte semantics. Writes
        // always use the original blob bytes, so checkouts stay byte-identical
        // to blobs.
        let text_path = !theirs_opt.as_ref().is_some_and(|b| is_binary(b))
            && !base_opt.as_ref().is_some_and(|b| is_binary(b))
            && !ours_opt.as_ref().is_some_and(|b| is_binary(b));
        let normalised = |bytes: &[u8]| -> Vec<u8> {
            if text_path {
                strip_cr(bytes)
            } else {
                bytes.to_vec()
            }
        };
        let theirs = match theirs_opt {
            Some(bytes) => bytes,
            None => {
                // The subsystem deleted this path: so does the target.
                if ours_opt.is_none() {
                    record(set, ApplyOutcome::Unchanged, Some("already absent"));
                } else {
                    delete_worktree_file(worktree_dir, rel_path)?;
                    record(set, ApplyOutcome::Deleted, None);
                }
                return Ok(());
            }
        };
        let theirs_norm = normalised(&theirs);
        if ours_opt.as_ref().map(|b| normalised(b)).as_deref() == Some(theirs_norm.as_slice()) {
            record(
                set,
                ApplyOutcome::Unchanged,
                Some("target already matches source"),
            );
            push_mode(set, rel_path, mode.as_deref());
            return Ok(());
        }
        // A mode-only delta: the commit left the content alone, so the subsystem
        // has nothing new for this path and the target's bytes stay.
        if base_opt.as_ref().map(|b| normalised(b)).as_deref() == Some(theirs_norm.as_slice()) {
            record(
                set,
                ApplyOutcome::Unchanged,
                Some("source left the content unchanged; target version kept"),
            );
            push_mode(set, rel_path, mode.as_deref());
            return Ok(());
        }
        let existed = ours_opt.is_some();
        write_file(worktree_dir, rel_path, &theirs)?;
        record(
            set,
            ApplyOutcome::Written,
            Some(if existed {
                ADD_OVER_EXISTING_REASON
            } else if base_opt.is_none() {
                "added"
            } else {
                "restored-add"
            }),
        );
        push_mode(set, rel_path, mode.as_deref());
        Ok(())
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
        assert!(assert_real_path_within(&dir, "a/../../b.txt").is_err());
        assert!(assert_real_path_within(&dir, "/etc/passwd").is_err());
        assert!(assert_real_path_within(&dir, "C:/windows/system32").is_err());
        assert!(assert_real_path_within(&dir, "src/app.ts").is_ok());
        // Next.js catch-all routes carry ".." as a substring, not a segment.
        assert!(
            assert_real_path_within(&dir, "src/app/api/fm/consignment/[...path]/route.ts").is_ok()
        );
        assert!(assert_real_path_within(&dir, "src/app/[...slug]/page.tsx").is_ok());
    }

    #[test]
    fn whitespace_line_normalisation_collapses_runs_and_trims() {
        assert_eq!(normalise_ws_line(b"        <div>"), b"<div>");
        assert_eq!(normalise_ws_line(b"  <div>"), b"<div>");
        assert_eq!(normalise_ws_line(b"a  =\t b;  "), b"a = b;");
        assert_eq!(normalise_ws_line(b"   "), b"");
        assert_eq!(normalise_ws_line(b""), b"");
        // A run is not nothing: `hello world` and `helloworld` still differ.
        assert_ne!(
            normalise_ws_line(b"hello world"),
            normalise_ws_line(b"helloworld")
        );
    }

    #[test]
    fn whitespace_only_diff3_block_resolves_to_ours_bytes() {
        let output = b"head\n<<<<<<< ours\n  <div>\n    <p>x</p>\n||||||| base\n<div>\n<p>x</p>\n=======\n        <div>\n                <p>x</p>\n>>>>>>> theirs\ntail\n";
        let resolved = resolve_whitespace_only_blocks(output).expect("whitespace-only resolves");
        assert_eq!(
            resolved,
            b"head\n  <div>\n    <p>x</p>\ntail\n".to_vec(),
            "the target's (ours) block bytes are kept verbatim"
        );
    }

    #[test]
    fn real_content_difference_stays_conflicting() {
        let output =
            b"<<<<<<< ours\nimport B;\n||||||| base\nbody\n=======\nimport C;\n>>>>>>> theirs\n";
        assert!(
            resolve_whitespace_only_blocks(output).is_none(),
            "real content differences must stay conflicting"
        );
    }

    #[test]
    fn mixed_blocks_stay_conflicting_when_any_block_is_real() {
        let output = b"<<<<<<< ours\n  <div>\n||||||| base\n<div>\n=======\n        <div>\n>>>>>>> theirs\n<<<<<<< ours\nimport B;\n||||||| base\nbody\n=======\nimport C;\n>>>>>>> theirs\n";
        assert!(
            resolve_whitespace_only_blocks(output).is_none(),
            "one real block poisons the whole file"
        );
    }

    #[test]
    fn malformed_markers_fail_safe_to_conflict() {
        assert!(resolve_whitespace_only_blocks(b"no markers here\n").is_none());
        assert!(resolve_whitespace_only_blocks(b"<<<<<<< ours\nunterminated\n").is_none());
    }
}
