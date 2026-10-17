//! Shared blob-set helper (spec integrity batch §1).
//!
//! For a path and a commit range, collect the source blob IDs:
//! oldest selected base → each selected commit's version for that path,
//! in ancestry order. Blob IDs are content-addressed, so equality is
//! byte-identical. `None` represents "absent" (the path does not exist at
//! that revision) and is a first-class member: a target that is absent while
//! the set contains `None` is behind-or-equal, not diverged.
//!
//! Both the gap refusal (§2) and the overwrite refusal (§3) consume this.
//! The batch entry point collects ALL paths of a commit in ONE `ls-tree -r`
//! per revision rather than per-path subprocesses.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::git;

/// Smuggling predicate shared by the engine (`mod.rs`) and the preflight
/// (`preflight.rs`) — one helper, two call sites, so preview and apply cannot
/// disagree (second addendum A).
///
/// A skipped SHA counts against a path only when the newest *selected* commit
/// touching that path is a proper **descendant** of the skipped SHA:
/// `is_ancestor(skipped, newest_toucher) && skipped != newest_toucher`.
/// Smuggling requires the skipped bytes to be inside a blob that actually
/// ships, which needs ancestry. Counter-example: chain `A→B→C→D`, select
/// `[B, D]`, `C` touches `P`, `D` touches only `Q` — writing `P` comes from
/// `B`, which predates `C`, so nothing of `C` ships to `P` and there is no
/// gap on `P`. True positives are unaffected, including merge-second-parent
/// shapes: when the newest toucher descends from the skip, the skip's bytes
/// can be in the shipped blob and the refusal still fires.
pub fn skipped_ships_through(repo_dir: &Path, skipped_sha: &str, newest_toucher_sha: &str) -> bool {
    if skipped_sha == newest_toucher_sha {
        return false;
    }
    git::is_ancestor(repo_dir, skipped_sha, newest_toucher_sha)
}

/// Blob ID for `rev:path`, or `None` when the path is absent at `rev`.
/// Uses `ls-tree` plumbing so paths with spaces are safe (no `rev:path`
/// shell splitting — argv is explicit).
pub fn blob_id(repo_dir: &Path, rev: &str, path: &str) -> Option<String> {
    let map = tree_blob_map(repo_dir, rev);
    map.get(path).cloned()
}

/// `path -> blob sha` for a revision, in ONE `ls-tree -r` call.
/// Absent paths simply have no entry.
pub fn tree_blob_map(repo_dir: &Path, rev: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(out) = git::run(repo_dir, &["ls-tree", "-r", rev]) else {
        return map;
    };
    if out.status != 0 {
        return map;
    }
    for line in out.stdout_str().lines() {
        let Some((meta, path)) = line.split_once('\t') else {
            continue;
        };
        if let Some(sha) = meta.split(' ').nth(2) {
            map.insert(path.to_string(), sha.to_string());
        }
    }
    map
}

/// Ordered selection: `(commit_sha, base_sha)` in ancestry order.
/// `base_sha` is `None` for a root commit.
pub type OrderedSelection<'a> = &'a [(String, Option<String>)];

/// Collect blob sets for ALL `paths` in ONE `ls-tree -r` per revision.
///
/// For each path, the set is:
/// `{ blob(oldest base) or None } ∪ { blob(each selected commit) }`
/// where absent is `None`. The caller passes the already ancestry-ordered
/// selection (see `build_plan`); this helper never re-sorts.
pub fn collect_blob_sets(
    repo_dir: &Path,
    ordered: OrderedSelection<'_>,
    paths: &[String],
) -> HashMap<String, HashSet<Option<String>>> {
    if ordered.is_empty() || paths.is_empty() {
        return HashMap::new();
    }
    // ONE ls-tree per distinct revision.
    let mut revs: Vec<&str> = Vec::new();
    if let Some(base) = ordered.first().and_then(|(_, b)| b.as_deref()) {
        revs.push(base);
    }
    for (sha, _) in ordered.iter() {
        if !revs.contains(&sha.as_str()) {
            revs.push(sha.as_str());
        }
    }
    let mut trees: HashMap<&str, HashMap<String, String>> = HashMap::new();
    for rev in revs {
        trees.insert(rev, tree_blob_map(repo_dir, rev));
    }
    let oldest_base = ordered.first().and_then(|(_, b)| b.as_deref());
    let mut out: HashMap<String, HashSet<Option<String>>> = HashMap::new();
    for path in paths {
        let mut set: HashSet<Option<String>> = HashSet::new();
        match oldest_base {
            Some(base) => {
                set.insert(trees.get(base).and_then(|m| m.get(path)).cloned());
            }
            None => {
                set.insert(None);
            }
        }
        for (sha, _) in ordered.iter() {
            let blob = trees.get(sha.as_str()).and_then(|m| m.get(path)).cloned();
            set.insert(blob);
        }
        out.insert(path.clone(), set);
    }
    out
}

/// Single-path convenience over [`collect_blob_sets`].
pub fn collect_blob_set(
    repo_dir: &Path,
    ordered: OrderedSelection<'_>,
    path: &str,
) -> HashSet<Option<String>> {
    let paths = vec![path.to_string()];
    collect_blob_sets(repo_dir, ordered, &paths)
        .remove(path)
        .unwrap_or_default()
}

/// Blobs reachable from `tips`' ancestry, in ONE `git rev-list --objects`
/// spawn over all tips (never one spawn per blob — the crate already paid for
/// the per-path-spawn lesson).
///
/// Why reachability, not mere existence: after the fetch the engine's
/// checkout holds BOTH source and target objects, so `cat-file -e` on a
/// target-tip blob is trivially true and can never discriminate the
/// subsystem's own content from main-system-only content. Reachability from
/// the selected ancestry can: an earlier subsystem version (a previous
/// promotion outside the selected range) is an ancestor of the selection,
/// while main-system-only bytes committed on the target branch are not.
///
/// Fail-closed: any spawn failure or non-zero exit yields the empty set, so
/// every caller falls back to the range-only refusal standing. Empty tips
/// cost zero spawns.
pub fn blobs_in_selected_history(repo_dir: &Path, tips: &[String]) -> HashSet<String> {
    let mut found = HashSet::new();
    let mut uniq: Vec<&str> = Vec::with_capacity(tips.len());
    for tip in tips {
        if tip.is_empty() || uniq.contains(&tip.as_str()) {
            continue;
        }
        uniq.push(tip.as_str());
    }
    if uniq.is_empty() {
        return found;
    }
    let mut args: Vec<&str> = Vec::with_capacity(uniq.len() + 2);
    args.push("rev-list");
    args.push("--objects");
    args.extend(uniq);
    let Ok(out) = git::run(repo_dir, &args) else {
        return found;
    };
    if out.status != 0 {
        return found;
    }
    for line in out.stdout_str().lines() {
        if let Some(sha) = line.split_whitespace().next() {
            if !sha.is_empty() {
                found.insert(sha.to_string());
            }
        }
    }
    found
}

/// Blobs that are reachable from `tip` but NOT from `write`: the content a
/// write sourced at `write` would move backwards over. The prescribed test is
/// `git rev-list --objects <write>..<tip>` — reachability, never timestamps.
/// One spawn per distinct write (deduplicated; `write == tip` costs zero
/// spawns and yields the empty set), each covering ALL blobs for that range,
/// intersected in memory with the candidate target blobs by the caller — the
/// same batching discipline as [`overwrite_allowed`]. Fail-closed: any spawn
/// failure yields the empty set for that write (no warning, never a refusal).
pub fn blobs_after(
    repo_dir: &Path,
    writes: &[String],
    tip: &str,
) -> HashMap<String, HashSet<String>> {
    let mut uniq: Vec<&str> = Vec::with_capacity(writes.len());
    for write in writes {
        if write.is_empty() || write == tip || uniq.contains(&write.as_str()) {
            continue;
        }
        uniq.push(write.as_str());
    }
    let mut out: HashMap<String, HashSet<String>> = HashMap::new();
    for write in &uniq {
        let mut found = HashSet::new();
        let range = format!("{write}..{tip}");
        if let Ok(out_) = git::run(repo_dir, &["rev-list", "--objects", &range]) {
            if out_.status == 0 {
                for line in out_.stdout_str().lines() {
                    if let Some(sha) = line.split_whitespace().next() {
                        if !sha.is_empty() {
                            found.insert(sha.to_string());
                        }
                    }
                }
            }
        }
        out.insert((*write).to_string(), found);
    }
    out
}

/// Newest commit at or under `tip` whose version of `path` is `blob`, as
/// `(sha, subject)` — the commit a "reverts newer work" warning names, so the
/// operator can verify it. A bare `--find-object` walk lists removers as well
/// as introducers (newest-first would name the commit that *removed* the
/// version), so each candidate is checked with `<sha>:<path>` until one
/// actually holds the blob. One `log` spawn plus one `rev-parse` per candidate
/// (a handful at most, capped below); callers only call it for paths that
/// already warned (usually zero). `None` when unresolvable: the caller warns
/// namelessly rather than guessing.
pub fn newest_commit_for_blob(
    repo_dir: &Path,
    blob: &str,
    tip: &str,
    path: &str,
) -> Option<(String, String)> {
    if blob.is_empty() || tip.is_empty() || path.is_empty() {
        return None;
    }
    let flag = format!("--find-object={blob}");
    let Ok(out) = git::run(
        repo_dir,
        &["log", "--format=%H%x1f%s", &flag, tip, "--", path],
    ) else {
        return None;
    };
    if out.status != 0 {
        return None;
    }
    // A flapping blob could list many candidates; beyond a cap, give up and
    // let the caller warn namelessly rather than spawning per candidate.
    let text = out.stdout_str();
    let candidates: Vec<&str> = text.lines().take(33).collect();
    if candidates.len() > 32 {
        return None;
    }
    for line in candidates {
        let (sha, subject) = line.split_once('\u{1f}').unwrap_or(("", ""));
        if sha.len() != 40 && sha.len() != 64 {
            continue;
        }
        // `<sha>:<path>` resolves to the blob itself (no `^{commit}` peel:
        // [`git::rev_parse`] would resolve the commit instead).
        let spec = format!("{sha}:{path}");
        let held = git::run(repo_dir, &["rev-parse", "--verify", "--quiet", &spec])
            .ok()
            .filter(|out| out.status == 0)
            .map(|out| out.stdout_trimmed());
        if held.as_deref() == Some(blob) {
            return Some((sha.to_string(), subject.to_string()));
        }
    }
    None
}
///
/// Overwrite-allowance for MANY paths sharing ONE history spawn.
///
/// `true` when the target copy is safe to overwrite — either it is
/// behind-or-equal by range membership (the target blob is in the path's blob
/// set, with `None` meaning absent on both sides), OR the target blob is
/// reachable from the SELECTED ancestry (`history_tips`, usually the selected
/// commit SHAs), meaning the subsystem itself produced those bytes (typically
/// an earlier promotion outside the selected range) and overwriting loses
/// nothing that only the main system had.
///
/// This predicate is deliberately PATH-AGNOSTIC: identical content appearing
/// elsewhere in the source still means the bytes are not unique to the main
/// system, and the overwritten content remains in git history regardless. A
/// blob reachable from NO selected tip is main-system-only content and stays
/// refused — that is the case this refusal exists to protect.
///
/// Both the engine (`mod.rs`: global guard + overwrite refusal) and the
/// preflight (`preflight.rs`: same two guards) call THIS function, so preview
/// and apply cannot drift — the same sharing that kept the last defect in
/// this area fixed.
///
/// Spawn cost: exactly one `rev-list --objects` when at least one path needs
/// the history probe, zero otherwise (range-allowed and absent-target paths
/// never spawn).
pub fn overwrite_allowed(
    repo_dir: &Path,
    blob_sets: &HashMap<String, HashSet<Option<String>>>,
    target_tree: &HashMap<String, String>,
    paths: &[String],
    history_tips: &[String],
) -> HashMap<String, bool> {
    let mut out: HashMap<String, bool> = HashMap::with_capacity(paths.len());
    // Target blob -> paths waiting on the history probe.
    let mut pending: HashMap<String, Vec<String>> = HashMap::new();
    for path in paths {
        let target_blob: Option<String> = target_tree.get(path).cloned();
        let range_ok = match blob_sets.get(path) {
            Some(set) => set.contains(&target_blob),
            None => target_blob.is_none(),
        };
        if range_ok {
            out.insert(path.clone(), true);
            continue;
        }
        match target_blob {
            Some(blob) => {
                pending.entry(blob).or_default().push(path.clone());
            }
            None => {
                out.insert(path.clone(), false);
            }
        }
    }
    if pending.is_empty() {
        return out;
    }
    let history = blobs_in_selected_history(repo_dir, history_tips);
    for (blob, owners) in &pending {
        let ok = history.contains(blob);
        for path in owners {
            out.insert(path.clone(), ok);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn tmp(prefix: &str) -> std::path::PathBuf {
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("vos-blobset-{prefix}-{n}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("spawn git");
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
        if let Some(p) = abs.parent() {
            std::fs::create_dir_all(p).expect("mkdir parent");
        }
        std::fs::write(&abs, content).expect("write");
    }

    fn commit(dir: &Path, msg: &str) -> String {
        git(dir, &["add", "-A"]);
        git(
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
                msg,
            ],
        );
        git(dir, &["rev-parse", "HEAD"])
    }

    #[test]
    fn blob_set_collects_oldest_base_through_each_commit() {
        let repo = tmp("order");
        init(&repo);
        write(&repo, "a.txt", "v1\n");
        let _a = commit(&repo, "A base");
        write(&repo, "a.txt", "v2\n");
        let b = commit(&repo, "B edit");
        write(&repo, "a.txt", "v3\n");
        let c = commit(&repo, "C edit");
        // Selection [B, C]: oldest base is A's tree (v1), then B (v2), C (v3).
        let parent_b = git(&repo, &["rev-parse", &format!("{b}^")]);
        let ordered = vec![
            (b.clone(), Some(parent_b.clone())),
            (c.clone(), Some(b.clone())),
        ];
        let set = collect_blob_set(&repo, &ordered, "a.txt");
        // Three distinct blobs.
        assert_eq!(set.len(), 3, "{set:?}");
        // Each revision's blob is a member.
        let blob_b = blob_id(&repo, &b, "a.txt");
        let blob_c = blob_id(&repo, &c, "a.txt");
        let blob_base = blob_id(&repo, &parent_b, "a.txt");
        assert!(set.contains(&blob_b));
        assert!(set.contains(&blob_c));
        assert!(set.contains(&blob_base));
    }

    #[test]
    fn blob_set_tracks_absence_as_none() {
        let repo = tmp("absent");
        init(&repo);
        write(&repo, "keep.txt", "keep\n");
        let _a = commit(&repo, "A base");
        write(&repo, "new.txt", "fresh\n");
        let b = commit(&repo, "B add");
        let parent_b = git(&repo, &["rev-parse", &format!("{b}^")]);
        let ordered = vec![(b.clone(), Some(parent_b.clone()))];
        let set = collect_blob_set(&repo, &ordered, "new.txt");
        // Oldest base absent (None) + B present.
        assert!(set.contains(&None), "{set:?}");
        assert!(set.contains(&blob_id(&repo, &b, "new.txt")));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn blob_set_allows_undone_effect_detection() {
        let repo = tmp("undone");
        init(&repo);
        write(&repo, "a.txt", "v1\n");
        let _a = commit(&repo, "A base");
        write(&repo, "a.txt", "v2\n");
        let b = commit(&repo, "B edit");
        write(&repo, "a.txt", "v1\n");
        let c = commit(&repo, "C undo");
        let parent_b = git(&repo, &["rev-parse", &format!("{b}^")]);
        let ordered = vec![
            (b.clone(), Some(parent_b.clone())),
            (c.clone(), Some(b.clone())),
        ];
        // Newest toucher (C) equals oldest base (v1): effect fully undone.
        let newest = blob_id(&repo, &c, "a.txt");
        let oldest = blob_id(&repo, &parent_b, "a.txt");
        assert_eq!(newest, oldest);
        let set = collect_blob_set(&repo, &ordered, "a.txt");
        assert!(set.contains(&newest));
    }

    #[test]
    fn batch_collect_uses_one_tree_per_rev() {
        let repo = tmp("batch");
        init(&repo);
        write(&repo, "a.txt", "a\n");
        write(&repo, "b.txt", "b\n");
        let _a = commit(&repo, "A base");
        write(&repo, "a.txt", "a2\n");
        let b = commit(&repo, "B edit a");
        let parent_b = git(&repo, &["rev-parse", &format!("{b}^")]);
        let ordered = vec![(b.clone(), Some(parent_b))];
        let paths = vec!["a.txt".to_string(), "b.txt".to_string()];
        let sets = collect_blob_sets(&repo, &ordered, &paths);
        assert_eq!(sets.len(), 2);
        // b.txt unchanged across range: single blob + base (same) → 1 member.
        assert_eq!(sets["b.txt"].len(), 1, "{:?}", sets["b.txt"]);
        // a.txt changed: 2 members.
        assert_eq!(sets["a.txt"].len(), 2, "{:?}", sets["a.txt"]);
    }

    #[test]
    fn skipped_ships_only_through_descendants() {
        let repo = tmp("ancestry");
        init(&repo);
        write(&repo, "p.txt", "a\n");
        let _a = commit(&repo, "A base");
        write(&repo, "p.txt", "b\n");
        let b = commit(&repo, "B");
        write(&repo, "p.txt", "c\n");
        let c = commit(&repo, "C");
        write(&repo, "p.txt", "d\n");
        let d = commit(&repo, "D");
        // Newest toucher D descends from C → smuggles.
        assert!(skipped_ships_through(&repo, &c, &d));
        // Newest toucher B predates C → cannot ship C's bytes.
        assert!(!skipped_ships_through(&repo, &c, &b));
        // Self is never smuggling.
        assert!(!skipped_ships_through(&repo, &c, &c));
    }

    #[test]
    fn earlier_promoted_blob_is_subsystem_content_path_agnostic() {
        // The regression: the target holds C1's version of a path while the
        // selection is [C5]. Range membership alone refuses; containment in
        // the source object DB must allow — even when the same bytes only
        // appear under a DIFFERENT path (path-agnostic by design).
        let repo = tmp("contain");
        init(&repo);
        write(&repo, "shared.txt", "v0\n");
        write(&repo, "elsewhere.txt", "unrelated-bytes\n");
        let _a = commit(&repo, "A base");
        write(&repo, "shared.txt", "v1\n");
        let c1 = commit(&repo, "C1 selective");
        write(&repo, "shared.txt", "v4\n");
        let c4 = commit(&repo, "C4 selective");
        write(&repo, "shared.txt", "v5\n");
        let c5 = commit(&repo, "C5 edit");
        let parent_c5 = git(&repo, &["rev-parse", &format!("{c5}^")]);
        assert_eq!(parent_c5, c4, "C4 must be the parent of C5");
        let ordered = vec![(c5.clone(), Some(parent_c5))];
        let paths = vec!["shared.txt".to_string()];
        let sets = collect_blob_sets(&repo, &ordered, &paths);
        // Target tree holding v1 (the subsystem's own earlier content,
        // outside the [C5] range whose set is {v4, v5}).
        let v1_blob = blob_id(&repo, &c1, "shared.txt").expect("v1 blob");
        // Sanity: v1 is NOT in the [C5] range set…
        assert!(
            !sets["shared.txt"].contains(&Some(v1_blob.clone())),
            "v1 must be outside the selected range: {:?}",
            sets["shared.txt"]
        );
        // …but containment allows it: the bytes are subsystem content.
        let mut target_at_v1: HashMap<String, String> = HashMap::new();
        target_at_v1.insert("shared.txt".to_string(), v1_blob);
        let allowed = overwrite_allowed(
            &repo,
            &sets,
            &target_at_v1,
            &paths,
            std::slice::from_ref(&c5),
        );
        assert_eq!(allowed.get("shared.txt"), Some(&true));
        // Path-agnostic: bytes that exist in the source ONLY under a different
        // path are still subsystem content, never main-system-only content.
        let x_blob = blob_id(&repo, "HEAD", "elsewhere.txt").expect("x blob");
        assert!(
            !sets["shared.txt"].contains(&Some(x_blob.clone())),
            "x must be outside the range set too: {:?}",
            sets["shared.txt"]
        );
        let mut target_at_x: HashMap<String, String> = HashMap::new();
        target_at_x.insert("shared.txt".to_string(), x_blob);
        let allowed = overwrite_allowed(
            &repo,
            &sets,
            &target_at_x,
            &paths,
            std::slice::from_ref(&c5),
        );
        assert_eq!(allowed.get("shared.txt"), Some(&true));
    }

    #[test]
    fn main_only_blob_absent_from_source_stays_refused() {
        let repo = tmp("mainonly");
        init(&repo);
        write(&repo, "app.txt", "v1\n");
        let _a = commit(&repo, "A base");
        write(&repo, "app.txt", "v2\n");
        let b = commit(&repo, "B edit");
        let parent_b = git(&repo, &["rev-parse", &format!("{b}^")]);
        let ordered = vec![(b.clone(), Some(parent_b))];
        let paths = vec!["app.txt".to_string()];
        let sets = collect_blob_sets(&repo, &ordered, &paths);
        // A blob the source never produced (main-system-only content).
        let mut target_tree: HashMap<String, String> = HashMap::new();
        target_tree.insert(
            "app.txt".to_string(),
            "0123456789abcdef0123456789abcdef01234567".to_string(),
        );
        let allowed =
            overwrite_allowed(&repo, &sets, &target_tree, &paths, std::slice::from_ref(&b));
        assert_eq!(allowed.get("app.txt"), Some(&false));
    }

    #[test]
    fn containment_check_costs_one_batched_spawn_regardless_of_path_count() {
        let repo = tmp("spawnbound");
        init(&repo);
        // 12 paths, each at v1.
        let mut paths: Vec<String> = Vec::new();
        for i in 0..12 {
            let name = format!("f{i}.txt");
            write(&repo, &name, "v1\n");
            paths.push(name);
        }
        let _a = commit(&repo, "A base");
        for i in 0..12 {
            write(&repo, &format!("f{i}.txt"), "v2\n");
        }
        let b = commit(&repo, "B edit all");
        for i in 0..12 {
            write(&repo, &format!("f{i}.txt"), "v3\n");
        }
        let c = commit(&repo, "C edit all");
        // Select [B] only: the range set is {v1, v2} while the target holds
        // v3 — subsystem content outside the range, so every path needs the
        // history probe (scoped to a tip whose ancestry contains v3).
        let parent_b = git(&repo, &["rev-parse", &format!("{b}^")]);
        let ordered = vec![(b.clone(), Some(parent_b))];
        let sets = collect_blob_sets(&repo, &ordered, &paths);
        for p in &paths {
            let v3 = blob_id(&repo, "HEAD", p).expect("v3 blob");
            assert!(
                !sets[p].contains(&Some(v3)),
                "v3 must be outside the [B] range set"
            );
        }
        let target_at_v3 = tree_blob_map(&repo, "HEAD");
        let before = super::git::git_spawns_for_tests(&repo);
        let allowed = overwrite_allowed(
            &repo,
            &sets,
            &target_at_v3,
            &paths,
            std::slice::from_ref(&c),
        );
        let after = super::git::git_spawns_for_tests(&repo);
        assert_eq!(
            after - before,
            1,
            "12 history probes must cost one batched spawn, not one per path"
        );
        assert!(
            allowed.values().all(|v| *v),
            "every v3 blob is subsystem content: {allowed:?}"
        );
        // No candidates at all costs zero spawns.
        let tip_tree = tree_blob_map(&repo, &b);
        let before = super::git::git_spawns_for_tests(&repo);
        let allowed = overwrite_allowed(&repo, &sets, &tip_tree, &paths, std::slice::from_ref(&b));
        let after = super::git::git_spawns_for_tests(&repo);
        assert_eq!(after - before, 0, "range-allowed paths must not spawn");
        assert!(allowed.values().all(|v| *v));
    }

    #[test]
    fn blobs_after_contains_only_post_write_content() {
        let repo = tmp("after");
        init(&repo);
        write(&repo, "f.txt", "v1\n");
        let a = commit(&repo, "A base");
        write(&repo, "f.txt", "v2\n");
        let b = commit(&repo, "B edit");
        write(&repo, "f.txt", "v3\n");
        let c = commit(&repo, "C edit");
        let v1 = blob_id(&repo, &a, "f.txt").expect("v1 blob");
        let v2 = blob_id(&repo, &b, "f.txt").expect("v2 blob");
        let v3 = blob_id(&repo, &c, "f.txt").expect("v3 blob");

        // The prescribed test: `rev-list --objects <write>..<tip>` —
        // reachability, never timestamps.
        let after_a = blobs_after(&repo, std::slice::from_ref(&a), &c);
        let set_a = &after_a[&a];
        assert!(
            !set_a.contains(&v1),
            "the write's own version is not after it"
        );
        assert!(set_a.contains(&v2) && set_a.contains(&v3));
        let after_b = blobs_after(&repo, std::slice::from_ref(&b), &c);
        let set_b = &after_b[&b];
        assert!(!set_b.contains(&v1) && !set_b.contains(&v2));
        assert!(set_b.contains(&v3));
        // `write == tip` costs zero spawns and yields the empty set.
        let before = super::git::git_spawns_for_tests(&repo);
        let after_c = blobs_after(&repo, std::slice::from_ref(&c), &c);
        let after = super::git::git_spawns_for_tests(&repo);
        assert_eq!(after - before, 0, "write == tip must not spawn");
        assert!(!after_c.contains_key(&c));
    }

    #[test]
    fn blobs_after_batches_one_spawn_per_write_regardless_of_path_count() {
        let repo = tmp("afterbatch");
        init(&repo);
        let mut paths: Vec<String> = Vec::new();
        for i in 0..12 {
            let name = format!("f{i}.txt");
            write(&repo, &name, "v1\n");
            paths.push(name);
        }
        let a = commit(&repo, "A base");
        for i in 0..12 {
            write(&repo, &format!("f{i}.txt"), "v2\n");
        }
        let b = commit(&repo, "B edit all");
        for i in 0..12 {
            write(&repo, &format!("f{i}.txt"), "v3\n");
        }
        let c = commit(&repo, "C edit all");
        // Every path shares the same two writes: 12 paths, 2 distinct
        // writes, 2 spawns — never one per path.
        let mut writes: Vec<String> = Vec::new();
        for _ in &paths {
            writes.push(a.clone());
            writes.push(b.clone());
        }
        let before = super::git::git_spawns_for_tests(&repo);
        let sets = blobs_after(&repo, &writes, &c);
        let after = super::git::git_spawns_for_tests(&repo);
        assert_eq!(
            after - before,
            2,
            "duplicate writes deduplicate to one spawn each, paths never spawn"
        );
        assert_eq!(sets.len(), 2);
        let v3 = blob_id(&repo, &c, "f0.txt").expect("v3 blob");
        assert!(sets[&b].contains(&v3));
    }

    #[test]
    fn newest_commit_for_blob_names_the_introducer() {
        let repo = tmp("introducer");
        init(&repo);
        write(&repo, "f.txt", "v1\n");
        let _a = commit(&repo, "A base");
        write(&repo, "f.txt", "v2\n");
        let b = commit(&repo, "B version with a name");
        write(&repo, "f.txt", "v3\n");
        let c = commit(&repo, "C edit");
        let v2 = blob_id(&repo, &b, "f.txt").expect("v2 blob");
        let (sha, subject) =
            newest_commit_for_blob(&repo, &v2, &c, "f.txt").expect("v2 resolves to its introducer");
        assert_eq!(sha, b);
        assert_eq!(subject, "B version with a name");
        assert!(
            newest_commit_for_blob(
                &repo,
                "0123456789abcdef0123456789abcdef01234567",
                &c,
                "f.txt"
            )
            .is_none(),
            "an unknown blob never gains a fabricated commit"
        );
    }
}
