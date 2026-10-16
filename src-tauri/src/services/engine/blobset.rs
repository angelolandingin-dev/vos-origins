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
}
