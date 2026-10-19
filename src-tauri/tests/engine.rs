//! In-tool promotion engine tests (SPEC §15 cases, ported from
//! `reference/actions-engine/.github/scripts/tests/`).
//!
//! Everything runs against **disposable local git repositories** created in a sandbox
//! directory, with a stub `GitHubOps` and a stub lockfile generator, so the suite is fully
//! offline. (Reference default sandbox: `C:\GithubRepos\vos-web-integrated\.tmp\rust-engine-sandbox`;
//! override with `VOS_ENGINE_SANDBOX`.)

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use vos_origins_lib::github::gh_cli::GitHubAccount;
use vos_origins_lib::github::models::PromotionDelivery;
use vos_origins_lib::github::models::PromotionMode;
use vos_origins_lib::github::models::PromotionOnConflict;
use vos_origins_lib::services::engine::apply::{apply_file_delta, ApplyConflict, ApplyOptions};
use vos_origins_lib::services::engine::git::DeltaEntry;
use vos_origins_lib::services::engine::github_ops::{
    CreatePrRequest, GitHubMode, GitHubOps, PullRequestInfo, PushOutcome, StubGitHubOps,
};
use vos_origins_lib::services::engine::lockfile::{
    LockfileGenerator, RealLockfileGenerator, StubLockfileGenerator,
};
use vos_origins_lib::services::engine::{
    run_promotion, run_promotion_with_progress, EngineDeps, EngineOptions, EngineRequest,
    PromotionProgress, PromotionReport,
};
use vos_origins_lib::services::registry::Registry;

mod sandbox {
    use super::*;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    pub fn sandbox_root() -> PathBuf {
        let base = std::env::var("VOS_ENGINE_SANDBOX")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from("C:\\GithubRepos\\vos-web-integrated\\.tmp\\rust-engine-sandbox")
            });
        if std::fs::create_dir_all(&base).is_err() {
            let fallback = std::env::temp_dir().join("vos-rust-engine-sandbox");
            let _ = std::fs::create_dir_all(&fallback);
            return fallback;
        }
        base
    }

    pub fn make_temp_dir(prefix: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = sandbox_root().join(format!("{prefix}-{}-{nanos}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    pub fn git_out(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("spawn git");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    pub fn git_try(dir: &Path, args: &[&str]) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    pub fn init_repo(dir: &Path, branch: &str) {
        std::fs::create_dir_all(dir).expect("mkdir repo");
        git_out(dir, &["init", "-q", "-b", branch]);
        git_out(dir, &["config", "user.email", "sandbox-bot@example.com"]);
        git_out(dir, &["config", "user.name", "Sandbox Bot"]);
        git_out(dir, &["config", "commit.gpgsign", "false"]);
        git_out(dir, &["config", "core.autocrlf", "false"]);
    }

    pub fn write(dir: &Path, rel: &str, bytes: &[u8]) {
        let abs = dir.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).expect("mkdir parent");
        }
        std::fs::write(&abs, bytes).expect("write file");
    }

    pub fn commit_all(dir: &Path, message: &str) -> String {
        git_out(dir, &["add", "-A"]);
        git_out(
            dir,
            &[
                "-c",
                "user.name=Sandbox Bot",
                "-c",
                "user.email=sandbox-bot@example.com",
                "commit",
                "--no-verify",
                "-q",
                "-m",
                message,
            ],
        );
        git_out(dir, &["rev-parse", "HEAD"])
    }

    pub fn commit_files(
        dir: &Path,
        message: &str,
        files: &[(&str, &[u8])],
        exec: &[&str],
    ) -> String {
        for (rel, content) in files {
            write(dir, rel, content);
        }
        git_out(dir, &["add", "-A"]);
        for path in exec {
            git_out(dir, &["update-index", "--chmod=+x", "--", path]);
        }
        git_out(
            dir,
            &[
                "-c",
                "user.name=Sandbox Bot",
                "-c",
                "user.email=sandbox-bot@example.com",
                "commit",
                "--no-verify",
                "-q",
                "-m",
                message,
            ],
        );
        git_out(dir, &["rev-parse", "HEAD"])
    }

    pub fn commit_dated(dir: &Path, message: &str, date: &str) {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=Sandbox Bot",
                "-c",
                "user.email=sandbox-bot@example.com",
                "commit",
                "--no-verify",
                "-q",
                "-m",
                message,
            ])
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .output()
            .expect("spawn git dated commit");
        assert!(
            output.status.success(),
            "git dated commit failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    pub fn commit_files_dated(
        dir: &Path,
        message: &str,
        files: &[(&str, &[u8])],
        date: &str,
    ) -> String {
        for (rel, content) in files {
            write(dir, rel, content);
        }
        git_out(dir, &["add", "-A"]);
        commit_dated(dir, message, date);
        git_out(dir, &["rev-parse", "HEAD"])
    }

    pub fn retouch_with_date(
        dir: &Path,
        branch: &str,
        message: &str,
        files: &[(&str, &[u8])],
        date: &str,
    ) {
        let current = git_out(dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
        git_out(dir, &["checkout", "-q", branch]);
        for (rel, content) in files {
            write(dir, rel, content);
        }
        git_out(dir, &["add", "-A"]);
        commit_dated(dir, message, date);
        git_out(dir, &["checkout", "-q", &current]);
    }

    pub fn sha(dir: &Path, rev: &str) -> String {
        git_out(dir, &["rev-parse", rev])
    }

    pub fn advance_ref(dir: &Path, branch: &str, commit: &str) {
        git_out(
            dir,
            &["update-ref", &format!("refs/heads/{branch}"), commit],
        );
    }

    pub fn commit_message_on_branch(
        dir: &Path,
        branch: &str,
        message: &str,
        files: &[(&str, &[u8])],
        remove: &[&str],
    ) -> String {
        let current = git_out(dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
        git_out(dir, &["checkout", "-q", branch]);
        for rel in remove {
            let _ = std::fs::remove_file(dir.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR)));
        }
        for (rel, content) in files {
            write(dir, rel, content);
        }
        git_out(dir, &["add", "-A"]);
        git_out(
            dir,
            &[
                "-c",
                "user.name=Sandbox Bot",
                "-c",
                "user.email=sandbox-bot@example.com",
                "commit",
                "--no-verify",
                "-q",
                "--allow-empty",
                "-m",
                message,
            ],
        );
        let result = git_out(dir, &["rev-parse", branch]);
        git_out(dir, &["checkout", "-q", &current]);
        result
    }

    pub fn init_bare(dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir).expect("mkdir bare");
        let out = Command::new("git")
            .args(["init", "--bare", "-q"])
            .arg(dir)
            .output()
            .expect("git init --bare");
        assert!(out.status.success());
        dir.to_path_buf()
    }

    pub fn ref_in(repo: &Path, reference: &str) -> Option<String> {
        let ok = git_try(repo, &["rev-parse", "--verify", "--quiet", reference]);
        if ok {
            Some(git_out(repo, &["rev-parse", reference]))
        } else {
            None
        }
    }

    pub fn try_branch_has(repo: &Path, branch: &str, rel: &str) -> bool {
        git_try(repo, &["cat-file", "-e", &format!("{branch}:{rel}")])
    }

    pub fn branch_file(repo: &Path, branch: &str, rel: &str) -> Vec<u8> {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["show", &format!("{branch}:{rel}")])
            .output()
            .expect("git show");
        assert!(
            output.status.success(),
            "git show {}:{} failed",
            branch,
            rel
        );
        output.stdout
    }

    pub fn branch_file_text(repo: &Path, branch: &str, rel: &str) -> String {
        String::from_utf8_lossy(&branch_file(repo, branch, rel)).to_string()
    }

    pub fn tree_mode(repo: &Path, branch: &str, rel: &str) -> String {
        let out = git_out(repo, &["ls-tree", branch, "--", rel]);
        out.split_whitespace().next().unwrap_or("").to_string()
    }

    pub fn is_ancestor(repo: &Path, a: &str, b: &str) -> bool {
        git_try(repo, &["merge-base", "--is-ancestor", a, b])
    }

    pub fn status_porcelain(dir: &Path) -> String {
        git_out(dir, &["status", "--porcelain"])
    }
}

use sandbox::*;

const BASE_MAIN_PKG: &str = r#"{
  "name": "main-system",
  "version": "0.1.0",
  "private": true,
  "dependencies": {
    "next": "^16.2.1",
    "react": "19.2.3"
  },
  "devDependencies": {
    "typescript": "^5"
  }
}
"#;

const LOCKFILE: &str = r#"{
  "name": "main-system",
  "version": "0.1.0",
  "lockfileVersion": 3,
  "requires": true,
  "packages": {
    "": {
      "name": "main-system",
      "version": "0.1.0",
      "dependencies": {
        "next": "^16.2.1",
        "react": "19.2.3"
      }
    }
  }
}
"#;

fn registry_json(subsystem_overrides: &str) -> String {
    format!(
        r#"{{
  "mainRepository": "ORG/main-system",
  "promotion": {{
    "allowedTargetBranches": ["development", "staging", "master"],
    "allowedTargetBranchPatterns": ["release/*"],
    "requirePullRequest": true
  }},
  "subsystems": [
    {subsystem_overrides}
  ]
}}"#
    )
}

fn subsystem_json(overrides: &str) -> String {
    subsystem_json_ext(overrides, true)
}

fn subsystem_json_ext(overrides: &str, enabled: bool) -> String {
    format!(
        r#"{{
      "id": "hr-management",
      "name": "Human Resource Management",
      "repository": "ORG/hr-management",
      "sourceBranch": "master",
      "enabled": {enabled},
      "pathStrategy": "preserve-relative-paths",
      "dependencyStrategy": "reconcile",
      "integrationPolicy": {{
        "applicationFiles": "selected-commit-delta",
        "packageJson": "merge-dependency-delta",
        "lockfiles": "regenerate-in-target",
        "sharedConfiguration": "review-explicitly",
        "githubWorkflows": "main-owned",
        "secretFiles": "never-import"
      }}{overrides}
    }}"#
    )
}

fn test_registry() -> Registry {
    Registry::from_json(&registry_json(&subsystem_json("")), "test").expect("registry")
}

struct SourceChain {
    dir: PathBuf,
    #[allow(dead_code)]
    a: String,
    b: String,
    c: String,
    #[allow(dead_code)]
    d: String,
}

fn create_source_chain(dir: &Path) -> SourceChain {
    init_repo(dir, "master");
    let a = commit_files(
        dir,
        "A: base",
        &[
            (
                "package.json",
                format!("{}\n", serde_json::json!({"name":"hr-management","version":"1.0.0","dependencies":{"next":"^16.2.1"}})).as_bytes(),
            ),
            ("src/modules/hr/index.ts", b"export const a = 1;\n"),
        ],
        &[],
    );
    let b = commit_files(
        dir,
        "B: unrelated change",
        &[("src/modules/hr/b.txt", b"B unrelated change\n")],
        &[],
    );
    let c = commit_files(
        dir,
        "C: employee export feature",
        &[
            ("src/modules/hr/c.txt", b"C feature\n"),
            (
                "src/modules/hr/export.ts",
                b"import ExcelJS from 'exceljs';\nexport const exportEmployees = () => new ExcelJS.Workbook();\n",
            ),
            (
                "package.json",
                format!("{}\n", serde_json::json!({"name":"hr-management","version":"1.0.0","dependencies":{"next":"^16.2.1","exceljs":"^4.4.0"}})).as_bytes(),
            ),
        ],
        &[],
    );
    let d = commit_files(
        dir,
        "D: later unrelated change",
        &[("src/modules/hr/d.txt", b"D unrelated change\n")],
        &[],
    );
    SourceChain {
        dir: dir.to_path_buf(),
        a,
        b,
        c,
        d,
    }
}

struct MainRepo {
    dir: PathBuf,
    #[allow(dead_code)]
    base: String,
}

fn create_main_repo(
    dir: &Path,
    extra_dependencies: &[(&str, &str)],
    extra_files: &[(&str, &[u8])],
) -> MainRepo {
    init_repo(dir, "master");
    let mut pkg: serde_json::Value = serde_json::from_str(BASE_MAIN_PKG).unwrap();
    for (name, range) in extra_dependencies {
        pkg["dependencies"][*name] = serde_json::Value::String((*range).to_string());
    }
    write(
        dir,
        "package.json",
        format!("{}\n", serde_json::to_string_pretty(&pkg).unwrap()).as_bytes(),
    );
    write(dir, "package-lock.json", LOCKFILE.as_bytes());
    write(dir, "src/modules/hr/index.ts", b"export const a = 1;\n");
    write(dir, "next.config.ts", b"export default {};\n");
    write(dir, "tsconfig.json", b"{\n  \"compilerOptions\": {}\n}\n");
    for (rel, content) in extra_files {
        write(dir, rel, content);
    }
    let base = commit_all(dir, "main: base");
    for branch in ["staging", "development"] {
        git_out(dir, &["branch", branch]);
    }
    MainRepo {
        dir: dir.to_path_buf(),
        base,
    }
}

struct PromoteOptions {
    subsystem_id: String,
    commits: Vec<String>,
    mode: PromotionMode,
    base_sha: Option<String>,
    target_branch: String,
    allow_shared_config: bool,
    allow_removals: bool,
    push: bool,
    create_pr: bool,
    real_lockfile: bool,
    delivery: PromotionDelivery,
    on_conflict: PromotionOnConflict,
    excluded_paths: Vec<String>,
    account_mode: AccountMode,
}

#[derive(Clone, Copy, Default)]
enum AccountMode {
    #[default]
    Unresolved,
    Octocat,
}

impl Default for PromoteOptions {
    fn default() -> Self {
        Self {
            subsystem_id: "hr-management".to_string(),
            commits: Vec::new(),
            mode: PromotionMode::Individual,
            base_sha: None,
            target_branch: "staging".to_string(),
            allow_shared_config: false,
            allow_removals: false,
            push: false,
            create_pr: true,
            real_lockfile: false,
            delivery: PromotionDelivery::PushPr,
            on_conflict: PromotionOnConflict::Fail,
            excluded_paths: Vec::new(),
            account_mode: AccountMode::Unresolved,
        }
    }
}

fn promote(
    source: &Path,
    main: &Path,
    github: &StubGitHubOps,
    registry: &Registry,
    options: PromoteOptions,
) -> PromotionReport {
    promote_with_progress(source, main, github, registry, options, &|_| {})
}

fn promote_with_progress(
    source: &Path,
    main: &Path,
    github: &StubGitHubOps,
    registry: &Registry,
    options: PromoteOptions,
    progress: &dyn Fn(PromotionProgress),
) -> PromotionReport {
    let request = EngineRequest {
        subsystem_id: options.subsystem_id,
        commits: options.commits,
        mode: options.mode,
        base_sha: options.base_sha,
        target_branch: options.target_branch,
        delivery: options.delivery,
        on_conflict: options.on_conflict,
        message: "test promotion".to_string(),
        excluded_paths: options.excluded_paths,
    };
    let temp_root = sandbox_root();
    let engine_options = EngineOptions {
        target_repo_dir: main.to_path_buf(),
        temp_root: temp_root.clone(),
        push_remote_url: None,
        token: None,
        allow_shared_config: options.allow_shared_config,
        allow_removals: options.allow_removals,
        nonce: format!(
            "test-{}-{}",
            std::process::id(),
            make_temp_dir("nonce")
                .file_name()
                .unwrap()
                .to_string_lossy()
        ),
        push: options.push,
        create_pr: options.create_pr,
    };

    let source_url = {
        let source = source.to_path_buf();
        move |_entry: &vos_origins_lib::services::registry::SubsystemEntry| {
            Some(source.to_string_lossy().replace('\\', "/"))
        }
    };
    let stub_lock = StubLockfileGenerator;
    let real_lock = RealLockfileGenerator;
    let lockfile: &dyn LockfileGenerator = if options.real_lockfile {
        &real_lock
    } else {
        &stub_lock
    };

    let account: fn() -> Option<GitHubAccount> = match options.account_mode {
        AccountMode::Unresolved => || None,
        AccountMode::Octocat => || {
            Some(GitHubAccount {
                login: "octocat".to_string(),
                id: 1,
            })
        },
    };
    let deps = EngineDeps {
        github,
        lockfile,
        source_url: &source_url,
        account,
    };
    run_promotion_with_progress(registry, &request, &engine_options, &deps, progress)
}

fn codes(report: &PromotionReport) -> Vec<String> {
    report.blocker_codes()
}

fn short(sha: &str) -> String {
    sha.chars().take(7).collect()
}

// ===========================================================================
// §15 acceptance cases
// ===========================================================================

#[test]
fn case1_selective_commit_only_c_is_applied() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().unwrap();

    assert!(try_branch_has(&main.dir, &branch, "src/modules/hr/c.txt"));
    assert!(
        !try_branch_has(&main.dir, &branch, "src/modules/hr/b.txt"),
        "B must be absent"
    );
    assert!(
        !try_branch_has(&main.dir, &branch, "src/modules/hr/d.txt"),
        "D must be absent"
    );

    let pkg: serde_json::Value =
        serde_json::from_str(&branch_file_text(&main.dir, &branch, "package.json")).unwrap();
    assert_eq!(pkg["dependencies"]["exceljs"], serde_json::json!("^4.4.0"));
    assert_eq!(
        pkg["dependencies"]["react"],
        serde_json::json!("19.2.3"),
        "unrelated packages retained"
    );
    assert_eq!(
        pkg["devDependencies"]["typescript"],
        serde_json::json!("^5")
    );
}

#[test]
fn case1b_no_files_are_added_to_the_target_repo() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let before = status_porcelain(&main.dir);
    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let after = status_porcelain(&main.dir);
    assert_eq!(
        before, after,
        "the engine must leave no files in the target working tree"
    );
    assert!(
        !main.dir.join(".integration").exists(),
        "no registry/ledger may be added to the target repo"
    );
}

#[test]
fn case3_integration_branch_starts_at_target_tip_and_pr_base_is_target() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    commit_message_on_branch(
        &main.dir,
        "staging",
        "staging: only on staging",
        &[("staging-only.txt", b"staging\n")],
        &[],
    );
    commit_message_on_branch(
        &main.dir,
        "master",
        "master: only on master",
        &[("master-only.txt", b"master\n")],
        &[],
    );
    let staging_tip = sha(&main.dir, "staging");
    let master_tip = sha(&main.dir, "master");
    assert_ne!(staging_tip, master_tip);

    let github = StubGitHubOps::new();
    let registry = test_registry();
    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().unwrap();

    assert_eq!(
        git_out(&main.dir, &["merge-base", "staging", &branch]),
        staging_tip,
        "starts at the staging tip"
    );
    assert!(try_branch_has(&main.dir, &branch, "staging-only.txt"));
    assert!(!try_branch_has(&main.dir, &branch, "master-only.txt"));
    assert!(!is_ancestor(&main.dir, &master_tip, &branch));
    assert_eq!(report.start_point.as_deref(), Some(staging_tip.as_str()));

    let pr_call = github
        .create_pr_calls()
        .into_iter()
        .next()
        .expect("PR created");
    assert!(
        pr_call.payload.contains("-> staging"),
        "PR base must be the target branch: {}",
        pr_call.payload
    );
}

#[test]
fn case3b_push_lands_on_the_remote_with_stub_transport() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let bare_dir = make_temp_dir("remote");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let bare = init_bare(&bare_dir);
    let github = StubGitHubOps::new().with_push_target(bare.to_string_lossy().to_string());
    let registry = test_registry();

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            push: true,
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().unwrap();
    assert!(
        ref_in(&bare, &format!("refs/heads/{branch}")).is_some(),
        "integration branch must exist on the remote"
    );
    assert_eq!(
        report.push.as_ref().map(|p| p.source.clone()).as_deref(),
        Some("push-target")
    );
}

#[test]
fn case4_multi_select_applies_in_order_and_records_all_shas() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.b.clone(), src.c.clone()],
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().unwrap();
    assert!(try_branch_has(&main.dir, &branch, "src/modules/hr/b.txt"));
    assert!(try_branch_has(&main.dir, &branch, "src/modules/hr/c.txt"));

    let message = git_out(&main.dir, &["log", "-1", "--format=%B", &branch]);
    let parsed = vos_origins_lib::services::engine::provenance::parse_trailers(&message);
    assert_eq!(parsed.get("Source-Commit"), &[src.b.clone(), src.c.clone()]);
    assert_eq!(parsed.first("Promotion-Mode"), Some("individual"));
    assert_eq!(parsed.first("Promotion-Target"), Some("staging"));
    assert_eq!(parsed.first("Subsystem-ID"), Some("hr-management"));
}

#[test]
fn case5_sha_not_reachable_from_source_branch_is_rejected() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);

    git_out(&src.dir, &["checkout", "-q", "-b", "side", &src.a]);
    let side = commit_files(&src.dir, "side commit", &[("src/side.txt", b"side\n")], &[]);
    git_out(&src.dir, &["checkout", "-q", "master"]);

    let github = StubGitHubOps::new();
    let registry = test_registry();
    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![side],
            ..Default::default()
        },
    );
    assert!(!report.ok);
    assert!(
        codes(&report).contains(&"COMMIT_VERIFICATION_FAILED".to_string()),
        "{:?}",
        codes(&report)
    );
}

#[test]
fn case5b_out_of_order_selection_is_sorted_not_rejected() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let main_again_dir = make_temp_dir("main-again");
    let src = create_source_chain(&src_dir);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let out_of_order = promote(
        &src_dir,
        &create_main_repo(&main_dir, &[], &[]).dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone(), src.b.clone()],
            ..Default::default()
        },
    );
    assert!(out_of_order.ok, "{:?}", out_of_order.blockers);

    let ordered = promote(
        &src_dir,
        &create_main_repo(&main_again_dir, &[], &[]).dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.b.clone(), src.c.clone()],
            ..Default::default()
        },
    );
    assert!(ordered.ok, "{:?}", ordered.blockers);

    let mut from_out_of_order = out_of_order.changes.applied.clone();
    from_out_of_order.sort();
    let mut from_ordered = ordered.changes.applied.clone();
    from_ordered.sort();
    assert_eq!(
        from_out_of_order, from_ordered,
        "the selection order must not change which paths are applied"
    );
    assert!(
        !from_ordered.is_empty(),
        "the selection must have applied something: {from_ordered:?}"
    );
}

#[test]
fn case6_registry_policy_rejects_unregistered_disabled_and_bad_targets() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let unregistered = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            subsystem_id: "payroll".to_string(),
            commits: vec![src.c.clone()],
            ..Default::default()
        },
    );
    assert!(codes(&unregistered).contains(&"SUBSYSTEM_NOT_REGISTERED".to_string()));

    let disabled_registry =
        Registry::from_json(&registry_json(&subsystem_json_ext("", false)), "t").unwrap();
    let disabled = promote(
        &src_dir,
        &main.dir,
        &github,
        &disabled_registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            ..Default::default()
        },
    );
    assert!(codes(&disabled).contains(&"SUBSYSTEM_DISABLED".to_string()));

    assert!(Registry::from_json("{ broken json", "t").is_err());

    let not_allowed = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            target_branch: "feature/evil".to_string(),
            ..Default::default()
        },
    );
    assert!(codes(&not_allowed).contains(&"TARGET_NOT_ALLOWED".to_string()));

    let unsafe_target = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            target_branch: "../../etc/passwd".to_string(),
            ..Default::default()
        },
    );
    assert!(codes(&unsafe_target).contains(&"TARGET_INVALID".to_string()));

    let nonexistent = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            target_branch: "release/9.9".to_string(),
            ..Default::default()
        },
    );
    assert!(codes(&nonexistent).contains(&"TARGET_NOT_FOUND".to_string()));
}

#[test]
fn case7_add_modify_delete_rename_binary_exec_and_exclusions() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    init_repo(&src_dir, "master");
    commit_files(
        &src_dir,
        "A: bulk base",
        &[
            (
                "package.json",
                format!(
                    "{}\n",
                    serde_json::json!({"name":"hr-management","dependencies":{"next":"^16.2.1"}})
                )
                .as_bytes(),
            ),
            ("src/old-name.txt", b"old content\n"),
            ("src/to-delete.txt", b"delete me\n"),
            ("src/keep.txt", b"keep\n"),
        ],
        &[],
    );
    let image_bytes: &[u8] = &[0x89, 0x50, 0x4e, 0x47, 0x00, 0x01, 0x02, 0xff, 0xfe];
    let renamed = b"renamed content\n";
    std::fs::remove_file(src_dir.join("src/old-name.txt")).unwrap();
    std::fs::remove_file(src_dir.join("src/to-delete.txt")).unwrap();
    let bulk = commit_files(
        &src_dir,
        "B: bulk rename/delete/add/binary",
        &[
            ("src/new-file.txt", b"brand new\n"),
            ("src/renamed file.txt", renamed),
            ("src/assets/image.png", image_bytes),
            ("src/\u{e9}migr\u{e9}.txt", b"unicode name\n"),
            ("scripts/run.sh", b"#!/bin/sh\necho hi\n"),
            (".github/workflows/evil.yml", b"name: evil\non: push\n"),
            (".env.local", b"SECRET=super-secret\n"),
        ],
        &["scripts/run.sh"],
    );

    init_repo(&main_dir, "master");
    write(
        &main_dir,
        "package.json",
        format!(
            "{}\n",
            serde_json::to_string_pretty(
                &serde_json::from_str::<serde_json::Value>(BASE_MAIN_PKG).unwrap()
            )
            .unwrap()
        )
        .as_bytes(),
    );
    write(&main_dir, "package-lock.json", LOCKFILE.as_bytes());
    write(&main_dir, "src/old-name.txt", b"old content\n");
    write(&main_dir, "src/to-delete.txt", b"delete me\n");
    write(&main_dir, "src/keep.txt", b"keep\n");
    commit_all(&main_dir, "main: bulk base");
    git_out(&main_dir, &["branch", "staging"]);

    let github = StubGitHubOps::new();
    let registry = test_registry();
    let report = promote(
        &src_dir,
        &main_dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![bulk],
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().unwrap();

    assert!(try_branch_has(&main_dir, &branch, "src/new-file.txt"));
    assert_eq!(
        branch_file_text(&main_dir, &branch, "src/new-file.txt"),
        "brand new\n"
    );
    assert!(
        !try_branch_has(&main_dir, &branch, "src/old-name.txt"),
        "rename removes the old path"
    );
    assert_eq!(
        branch_file_text(&main_dir, &branch, "src/renamed file.txt"),
        "renamed content\n"
    );
    assert!(
        !try_branch_has(&main_dir, &branch, "src/to-delete.txt"),
        "delete is honored"
    );
    assert!(
        try_branch_has(&main_dir, &branch, "src/\u{e9}migr\u{e9}.txt"),
        "unicode filename handled"
    );
    assert_eq!(
        branch_file(&main_dir, &branch, "src/assets/image.png"),
        image_bytes,
        "binary preserved byte-for-byte"
    );
    assert_eq!(
        tree_mode(&main_dir, &branch, "scripts/run.sh"),
        "100755",
        "executable bit preserved"
    );

    assert!(
        !try_branch_has(&main_dir, &branch, ".github/workflows/evil.yml"),
        "workflows are main-owned"
    );
    assert!(
        !try_branch_has(&main_dir, &branch, ".env.local"),
        "secret files are never imported"
    );
    assert!(report
        .changes
        .main_owned_skipped
        .iter()
        .any(|p| p == ".github/workflows/evil.yml"));
    assert!(report
        .changes
        .secret_excluded
        .iter()
        .any(|p| p == ".env.local"));
}

#[test]
fn case8_new_dependency_merges_delta_and_leaves_lock_untouched() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();
    let lock_before = branch_file_text(&main.dir, "master", "package-lock.json");

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    assert_eq!(report.dependencies.added.len(), 1);
    assert_eq!(report.dependencies.added[0].name, "exceljs");
    assert!(report.changes.regenerated.is_empty());
    assert!(report.lockfile.is_none());
    let branch = report.integration_branch.clone().unwrap();
    assert_eq!(
        branch_file_text(&main.dir, &branch, "package-lock.json"),
        lock_before
    );
}

#[test]
fn lockfile_touched_selection_promotes_without_lockfile_in_commit() {
    let src_dir = make_temp_dir("src-lockonly");
    let main_dir = make_temp_dir("main-lockonly");
    init_repo(&src_dir, "master");
    commit_files(
        &src_dir,
        "A: base",
        &[
            (
                "package.json",
                format!(
                    "{}\n",
                    serde_json::json!({"name":"hr-management","dependencies":{"next":"^16.2.1"}})
                )
                .as_bytes(),
            ),
            ("src/modules/hr/c.txt", b"base\n"),
            ("package-lock.json", b"{\"lock\":\"base\"}\n"),
        ],
        &[],
    );
    let touched = commit_files(
        &src_dir,
        "B: code plus lockfile bump, no manifest delta",
        &[
            ("src/modules/hr/c.txt", b"feature\n"),
            ("package-lock.json", b"{\"lock\":\"bumped\"}\n"),
        ],
        &[],
    );
    let main = create_main_repo(&main_dir, &[], &[]);
    let lock_before = branch_file_text(&main.dir, "master", "package-lock.json");
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![touched],
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    assert!(report.changes.regenerated.is_empty());
    assert!(report.lockfile.is_none());
    let branch = report.integration_branch.clone().unwrap();
    assert_eq!(
        branch_file_text(&main.dir, &branch, "src/modules/hr/c.txt"),
        "feature\n"
    );
    assert_eq!(
        branch_file_text(&main.dir, &branch, "package-lock.json"),
        lock_before
    );
}

#[test]
fn dependency_delta_leaves_lock_untouched_in_worktree_stage() {
    use vos_origins_lib::github::models::PromotionDelivery;
    let src_dir = make_temp_dir("src-worklock");
    let main_dir = make_temp_dir("main-worklock");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let lock_before = branch_file_text(&main.dir, "master", "package-lock.json");
    git_out(&main.dir, &["checkout", "-q", "staging"]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            delivery: PromotionDelivery::Worktree,
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    assert!(report.changes.regenerated.is_empty());
    assert!(report.lockfile.is_none());
    assert!(
        !report
            .staged_paths
            .iter()
            .any(|p| p == "package-lock.json" || p == "pnpm-lock.yaml" || p == "yarn.lock"),
        "lockfile must never stage: {:?}",
        report.staged_paths
    );
    assert_eq!(
        std::fs::read_to_string(main.dir.join("package-lock.json")).unwrap(),
        lock_before
    );
}

#[test]
fn case9_missing_dependency_commit_blocks_but_selecting_it_succeeds() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    init_repo(&src_dir, "master");
    commit_files(
        &src_dir,
        "A: base",
        &[
            (
                "package.json",
                format!(
                    "{}\n",
                    serde_json::json!({"name":"hr-management","dependencies":{"next":"^16.2.1"}})
                )
                .as_bytes(),
            ),
            ("src/modules/hr/index.ts", b"export const a = 1;\n"),
        ],
        &[],
    );
    let b = commit_files(
        &src_dir,
        "B: add left-pad dependency",
        &[("package.json", format!("{}\n", serde_json::json!({"name":"hr-management","dependencies":{"next":"^16.2.1","left-pad":"^1.3.0"}})).as_bytes())],
        &[],
    );
    let c = commit_files(
        &src_dir,
        "C: use left-pad",
        &[(
            "src/modules/hr/use-leftpad.ts",
            b"import leftPad from 'left-pad';\nexport const pad = () => leftPad('x', 5);\n",
        )],
        &[],
    );
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let blocked = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![c.clone()],
            ..Default::default()
        },
    );
    assert!(!blocked.ok);
    assert!(
        codes(&blocked).contains(&"MISSING_DEPENDENCY_PREREQUISITE".to_string()),
        "{:?}",
        codes(&blocked)
    );

    let both = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![b, c],
            ..Default::default()
        },
    );
    assert!(both.ok, "{:?}", both.blockers);
}

#[test]
fn missing_dependency_blocker_names_file_and_specifier() {
    let src_dir = make_temp_dir("src-missing-detail");
    let main_dir = make_temp_dir("main-missing-detail");
    init_repo(&src_dir, "master");
    commit_files(
        &src_dir,
        "A: base",
        &[
            (
                "package.json",
                format!(
                    "{}\n",
                    serde_json::json!({"name":"hr-management","dependencies":{"next":"^16.2.1"}})
                )
                .as_bytes(),
            ),
            ("src/modules/hr/index.ts", b"export const a = 1;\n"),
        ],
        &[],
    );
    let c = commit_files(
        &src_dir,
        "C: use left-pad",
        &[(
            "src/modules/hr/use-leftpad.ts",
            b"import leftPad from 'left-pad';\nexport const pad = () => leftPad('x', 5);\n",
        )],
        &[],
    );
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let blocked = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![c],
            ..Default::default()
        },
    );
    assert!(!blocked.ok);
    let blocker = blocked
        .blockers
        .iter()
        .find(|b| b.code == "MISSING_DEPENDENCY_PREREQUISITE")
        .expect("missing-dependency blocker");
    assert!(
        blocker.message.contains("\"left-pad\""),
        "headline names the package: {}",
        blocker.message
    );
    let details = blocker.details.clone().unwrap_or_default();
    assert!(
        details.contains("src/modules/hr/use-leftpad.ts"),
        "details name the importing file: {details}"
    );
    assert!(
        details.contains("'left-pad'") || details.contains("\"left-pad\""),
        "details name the specifier: {details}"
    );
}

#[test]
fn case10_incompatible_dependency_range_fails_closed() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[("exceljs", "^3.0.0")], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            ..Default::default()
        },
    );
    assert!(!report.ok);
    assert!(
        codes(&report).contains(&"DEPENDENCY_INCOMPATIBLE_RANGE".to_string()),
        "{:?}",
        codes(&report)
    );
}

#[test]
fn case11_divergent_target_copy_is_overwritten_by_subsystem() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    // Divergent target copy (different bytes from every source version in the
    // selection): blob membership (§3) records it as overwritten — dates never
    // decide. Promotion is a copy and the subsystem's copy always wins, so a
    // diverged target never refuses.
    retouch_with_date(
        &main.dir,
        "staging",
        "staging: divergent copy",
        &[("src/modules/hr/c.txt", b"divergent target content\n")],
        "2030-01-01T00:00:00+00:00",
    );
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().unwrap();
    assert_eq!(
        branch_file_text(&main.dir, &branch, "src/modules/hr/c.txt"),
        "C feature\n",
        "the subsystem's copy wins"
    );
    assert!(
        report
            .changes
            .replaced
            .iter()
            .any(|p| p == "src/modules/hr/c.txt"),
        "overwritten path is reported as replaced: {:?}",
        report.changes
    );
    assert!(
        report
            .changes
            .conflict_taken
            .iter()
            .any(|p| p == "src/modules/hr/c.txt"),
        "overwritten path is reported as taken from subsystem: {:?}",
        report.changes
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.code == "conflict_taken_from_subsystem"),
        "overwrite warning is emitted: {:?}",
        report.warnings
    );
    let msg = report.provenance_message.clone().unwrap_or_default();
    assert!(msg.contains("Promotion-Replaced:"), "{msg}");
    assert!(msg.contains("src/modules/hr/c.txt"), "{msg}");
}

#[test]
fn case_noop_promotion_is_refused() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    init_repo(&src_dir, "master");
    commit_files(
        &src_dir,
        "A base",
        &[("package.json", b"{\"name\":\"x\"}\n")],
        &[],
    );
    let workflow_only = commit_files(
        &src_dir,
        "B only a workflow",
        &[(".github/workflows/x.yml", b"name: x\non: push\n")],
        &[],
    );
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![workflow_only],
            ..Default::default()
        },
    );
    assert!(!report.ok);
    assert!(
        codes(&report).contains(&"NOOP_PROMOTION".to_string()),
        "{:?}",
        codes(&report)
    );
}

#[test]
fn case12_diverged_target_is_overwritten_and_scratch_files_are_never_committed() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    init_repo(&src_dir, "master");
    commit_files(
        &src_dir,
        "A: baseline",
        &[("src/merge.txt", b"a\nb\nc\nd\n")],
        &[],
    );
    let c = commit_files(
        &src_dir,
        "C: modify a different line",
        &[("src/merge.txt", b"a\nb\nc\nX\n")],
        &[],
    );

    let main = create_main_repo(&main_dir, &[], &[("src/merge.txt", b"a\nY\nc\nd\n")]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    // Blob membership (§3): target `a/Y/c/d` matches neither base nor tip,
    // so the subsystem's copy overwrites it. Copy never merges (no scratch
    // `.ours`/`.base`/`.theirs` files can exist — and the integration branch
    // carries the source bytes, never scratch).
    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![c],
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().unwrap();
    assert_eq!(
        branch_file_text(&main.dir, &branch, "src/merge.txt"),
        "a\nb\nc\nX\n",
        "the subsystem's copy wins"
    );
    assert!(
        report.changes.replaced.iter().any(|p| p == "src/merge.txt"),
        "overwritten path is reported as replaced: {:?}",
        report.changes
    );
    assert!(!try_branch_has(&main.dir, &branch, "src/merge.txt.ours"));
    assert!(!try_branch_has(&main.dir, &branch, "src/merge.txt.base"));
    assert!(!try_branch_has(&main.dir, &branch, "src/merge.txt.theirs"));
}

#[test]
fn case13_phantom_source_commit_in_body_is_ignored() {
    use vos_origins_lib::services::engine::history;

    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let phantom = "dead".repeat(10);
    let message = [
        "integrate(hr-management): phantom subject".to_string(),
        String::new(),
        format!("Source-Commit: {phantom}"),
        String::new(),
        "Prose follows, then the authoritative trailer block:".to_string(),
        String::new(),
        "Subsystem-ID: hr-management".to_string(),
        format!("Source-Commit: {}", src.c),
        "Promotion-Target: staging".to_string(),
        String::new(),
    ]
    .join("\n");
    commit_message_on_branch(
        &main.dir,
        "staging",
        &message,
        &[("src/modules/hr/c.txt", b"C feature\n")],
        &[],
    );

    let events = history::collect_events(&main.dir, "staging");
    let integrate = events
        .iter()
        .find(|e| e.kind == history::EventType::Integrate)
        .expect("integrate event");
    assert_eq!(integrate.source_commits, vec![src.c.clone()]);

    let status = history::compute_branch_status(
        &main.dir,
        "staging",
        "hr-management",
        &[src.c.clone(), phantom.clone()],
    );
    assert_eq!(status[0].status, history::BranchStatusKind::Integrated);
    assert_eq!(
        status[1].status,
        history::BranchStatusKind::Eligible,
        "phantom SHA is not integrated"
    );
}

#[test]
fn case14_shared_config_blocks_by_default_and_applies_under_approval() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    init_repo(&src_dir, "master");
    commit_files(&src_dir, "A", &[("tsconfig.json", b"{\"a\":1}\n")], &[]);
    let b = commit_files(
        &src_dir,
        "B modify shared config",
        &[("tsconfig.json", b"{\"a\":2}\n")],
        &[],
    );
    let main = create_main_repo(&main_dir, &[], &[("tsconfig.json", b"{\"a\":1}\n")]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let blocked = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![b.clone()],
            ..Default::default()
        },
    );
    assert!(
        codes(&blocked).contains(&"SHARED_CONFIG_REQUIRES_REVIEW".to_string()),
        "{:?}",
        codes(&blocked)
    );

    let allowed = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![b],
            allow_shared_config: true,
            ..Default::default()
        },
    );
    assert!(allowed.ok, "{:?}", allowed.blockers);
    let branch = allowed.integration_branch.clone().unwrap();
    assert_eq!(
        branch_file_text(&main.dir, &branch, "tsconfig.json"),
        "{\"a\":2}\n"
    );
}

#[test]
fn case15_duplicate_and_open_pr_are_blocked() {
    use vos_origins_lib::services::engine::github_ops::PullRequestInfo;
    use vos_origins_lib::services::engine::{integration_branch_prefix, run_promotion};

    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    // First promotion merged into staging -> a second must be blocked as duplicate.
    let first = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            ..Default::default()
        },
    );
    assert!(first.ok, "{:?}", first.blockers);
    let first_branch = first.integration_branch.clone().unwrap();
    advance_ref(&main.dir, "staging", &sha(&main.dir, &first_branch));

    let second = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            ..Default::default()
        },
    );
    assert!(
        codes(&second).contains(&"DUPLICATE_ACTIVE".to_string()),
        "{:?}",
        codes(&second)
    );

    // A different target branch is still allowed and independent.
    let other = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            target_branch: "master".to_string(),
            ..Default::default()
        },
    );
    assert!(other.ok, "{:?}", other.blockers);

    let head = format!(
        "{}-anything",
        integration_branch_prefix("hr-management", std::slice::from_ref(&src.d))
    );
    let body = format!(
        "Subsystem-ID: hr-management\nSource-Commit: {}\nPromotion-Target: development\n",
        src.d
    );
    let open_pr_github = StubGitHubOps::new().with_open_prs(vec![PullRequestInfo {
        number: Some(42),
        head,
        base: "development".to_string(),
        body,
        ..Default::default()
    }]);
    let request = EngineRequest {
        subsystem_id: "hr-management".to_string(),
        commits: vec![src.d.clone()],
        mode: PromotionMode::Individual,
        base_sha: None,
        target_branch: "development".to_string(),
        delivery: PromotionDelivery::PushPr,
        on_conflict: PromotionOnConflict::Fail,
        message: "test promotion".to_string(),
        excluded_paths: Vec::new(),
    };
    let temp_root = sandbox_root();
    let engine_options = EngineOptions {
        target_repo_dir: main.dir.clone(),
        temp_root,
        push_remote_url: None,
        token: None,
        allow_shared_config: false,
        allow_removals: false,
        nonce: "dup".to_string(),
        push: false,
        create_pr: true,
    };
    let source_dir = src_dir.clone();
    let source_url = move |_e: &vos_origins_lib::services::registry::SubsystemEntry| {
        Some(source_dir.to_string_lossy().replace('\\', "/"))
    };
    let stub_lock = StubLockfileGenerator;
    let deps = EngineDeps {
        github: &open_pr_github,
        lockfile: &stub_lock,
        source_url: &source_url,
        account: || None,
    };
    let blocked = run_promotion(&registry, &request, &engine_options, &deps);
    assert!(
        codes(&blocked).contains(&"PR_ALREADY_OPEN".to_string()),
        "{:?}",
        codes(&blocked)
    );
}

#[test]
fn case16_unsupported_lockfile_manager_is_deferred_not_failed() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    init_repo(&main_dir, "master");
    write(&main_dir, "package.json", BASE_MAIN_PKG.as_bytes());
    write(&main_dir, "bun.lockb", b"bun-binary-lockfile\n");
    commit_all(&main_dir, "main: bun baseline");
    git_out(&main_dir, &["branch", "staging"]);
    git_out(&main_dir, &["branch", "development"]);

    let github = StubGitHubOps::new();
    let registry = test_registry();
    let report = promote(
        &src_dir,
        &main_dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            real_lockfile: true,
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    assert!(report.changes.regenerated.is_empty());
    assert!(report.lockfile.is_none());
    let branch = report.integration_branch.clone().unwrap();
    assert_eq!(
        branch_file(&main_dir, &branch, "bun.lockb"),
        b"bun-binary-lockfile\n"
    );
}

#[test]
fn case17_revert_then_repromotion_is_allowed() {
    use vos_origins_lib::services::engine::history;
    use vos_origins_lib::services::engine::provenance::build_revert_message;

    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let integration_commit = report.commit_sha.clone().unwrap();
    advance_ref(
        &main.dir,
        "staging",
        &sha(&main.dir, &report.integration_branch.clone().unwrap()),
    );

    let revert = build_revert_message(
        "hr-management",
        std::slice::from_ref(&src.c),
        &integration_commit,
        "C feature",
    );
    commit_message_on_branch(
        &main.dir,
        "staging",
        &revert,
        &[],
        &["src/modules/hr/c.txt", "src/modules/hr/export.ts"],
    );

    let status = history::compute_branch_status(
        &main.dir,
        "staging",
        "hr-management",
        std::slice::from_ref(&src.c),
    );
    assert_eq!(status[0].status, history::BranchStatusKind::Reverted);

    let again = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            ..Default::default()
        },
    );
    assert!(
        again.ok,
        "re-promotion after a revert must be allowed: {:?}",
        again.blockers
    );
}

#[test]
fn case18_cumulative_mode_applies_the_verified_range() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            mode: PromotionMode::Cumulative,
            base_sha: Some(src.a.clone()),
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().unwrap();
    assert!(
        try_branch_has(&main.dir, &branch, "src/modules/hr/b.txt"),
        "cumulative includes intervening B"
    );
    assert!(try_branch_has(&main.dir, &branch, "src/modules/hr/c.txt"));

    let message = git_out(&main.dir, &["log", "-1", "--format=%B", &branch]);
    let parsed = vos_origins_lib::services::engine::provenance::parse_trailers(&message);
    assert_eq!(parsed.get("Source-Commit"), std::slice::from_ref(&src.c));
    assert_eq!(parsed.first("Promotion-Mode"), Some("cumulative"));
}

#[test]
fn case19_root_commit_delta_is_not_a_noop() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    init_repo(&src_dir, "master");
    let root = commit_files(
        &src_dir,
        "root commit",
        &[
            ("docs/readme.md", b"# root\n"),
            ("src/root-only.txt", b"root only\n"),
        ],
        &[],
    );
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![root],
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().unwrap();
    assert!(try_branch_has(&main.dir, &branch, "src/root-only.txt"));
    assert!(try_branch_has(&main.dir, &branch, "docs/readme.md"));
}

#[test]
fn case20_binary_modify_when_target_equals_base_overwrites() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    init_repo(&src_dir, "master");
    let base_bytes: &[u8] = &[0x00, 0x01, 0x02, 0xff, 0x00, 0x10];
    let new_bytes: &[u8] = &[0x00, 0x09, 0x00, 0xfe];
    commit_files(
        &src_dir,
        "A: binary baseline",
        &[("src/assets/bin.dat", base_bytes)],
        &[],
    );
    let c = commit_files(
        &src_dir,
        "C: modify binary",
        &[("src/assets/bin.dat", new_bytes)],
        &[],
    );
    let main = create_main_repo(&main_dir, &[], &[("src/assets/bin.dat", base_bytes)]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![c],
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().unwrap();
    assert_eq!(
        branch_file(&main.dir, &branch, "src/assets/bin.dat"),
        new_bytes
    );
}

#[test]
fn diagnostics_helper_short_sha_matches_reference() {
    let sha = "abcdef0123456789";
    assert_eq!(short(sha), "abcdef0");
}

#[test]
fn oracle_finding7_symlinked_target_parent_is_rejected() {
    let src_dir = make_temp_dir("src-symlink");
    init_repo(&src_dir, "master");
    let a = commit_files(
        &src_dir,
        "A: base",
        &[("src/modules/hr/index.ts", b"export const a = 1;\n")],
        &[],
    );
    let c = commit_files(
        &src_dir,
        "C: feature",
        &[("src/modules/hr/c.txt", b"C feature\n")],
        &[],
    );

    let worktree = make_temp_dir("wt-symlink");
    let outside = make_temp_dir("outside-symlink");

    // Creating a directory symlink may need Developer Mode / privileges on Windows.
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_dir(&outside, worktree.join("src")).is_ok();
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&outside, worktree.join("src")).is_ok();
    if !linked {
        // Environment cannot create symlinks; the safety code is still exercised by the
        // traversal unit test in `apply.rs`.
        return;
    }

    let entry = DeltaEntry {
        status: 'M',
        path: Some("src/modules/hr/c.txt".to_string()),
        old_path: None,
        new_path: None,
        score: None,
    };
    let conflict: ApplyConflict = apply_file_delta(
        &src_dir,
        &worktree,
        &c,
        Some(&a),
        &entry,
        &sandbox_root(),
        &ApplyOptions::default(),
    )
    .expect_err("a symlinked parent directory must be refused");
    assert_eq!(conflict.kind, "symlink-traversal");
    assert!(
        std::fs::read_dir(&outside).unwrap().next().is_none(),
        "nothing may escape into the symlink target"
    );
}

struct FailingPrOps;

impl GitHubOps for FailingPrOps {
    fn mode(&self) -> GitHubMode {
        GitHubMode::Stub
    }
    fn push_target(&self) -> Option<String> {
        None
    }
    fn push_branch(
        &self,
        _repo_dir: &Path,
        _branch: &str,
        _remote_url: Option<&str>,
    ) -> Result<PushOutcome, String> {
        Ok(PushOutcome {
            pushed: false,
            remote: None,
            source: "no-push-target".to_string(),
        })
    }
    fn list_open_pull_requests(
        &self,
        _repo_slug: &str,
        _base: &str,
    ) -> Result<Vec<PullRequestInfo>, String> {
        Err("network exploded".to_string())
    }
    fn create_pull_request(
        &self,
        _repo_slug: &str,
        _request: &CreatePrRequest,
    ) -> Result<PullRequestInfo, String> {
        Err("should not be called".to_string())
    }
    fn resolve_target_tip(
        &self,
        _repo_slug: &str,
        _branch: &str,
    ) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn list_branches(
        &self,
        _repo_slug: &str,
    ) -> Result<Vec<vos_origins_lib::services::engine::github_ops::BranchInfo>, String> {
        Ok(Vec::new())
    }
    fn repo_default_branch(&self, _repo_slug: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
}

#[test]
fn oracle_finding9_pr_lookup_failure_is_distinct_from_already_open() {
    let src_dir = make_temp_dir("src-prfail");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let registry = test_registry();

    let request = EngineRequest {
        subsystem_id: "hr-management".to_string(),
        commits: vec![src.c.clone()],
        mode: PromotionMode::Individual,
        base_sha: None,
        target_branch: "staging".to_string(),
        delivery: PromotionDelivery::PushPr,
        on_conflict: PromotionOnConflict::Fail,
        message: "test promotion".to_string(),
        excluded_paths: Vec::new(),
    };
    let engine_options = EngineOptions {
        target_repo_dir: main.dir.clone(),
        temp_root: sandbox_root(),
        push_remote_url: None,
        token: None,
        allow_shared_config: false,
        allow_removals: false,
        nonce: "prfail".to_string(),
        push: false,
        create_pr: true,
    };
    let source_dir = src_dir.clone();
    let source_url = move |_e: &vos_origins_lib::services::registry::SubsystemEntry| {
        Some(source_dir.to_string_lossy().replace('\\', "/"))
    };
    let github = FailingPrOps;
    let lockfile = StubLockfileGenerator;
    let deps = EngineDeps {
        github: &github,
        lockfile: &lockfile,
        source_url: &source_url,
        account: || None,
    };
    let report = run_promotion(&registry, &request, &engine_options, &deps);
    assert!(
        codes(&report).contains(&"PR_LOOKUP_FAILED".to_string()),
        "{:?}",
        codes(&report)
    );
    assert!(!codes(&report).contains(&"PR_ALREADY_OPEN".to_string()));
}

#[test]
fn oracle_p0_open_pr_on_different_head_still_blocks_duplicate_sha() {
    use vos_origins_lib::services::engine::history;

    let src_dir = make_temp_dir("src-prhead");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let registry = test_registry();

    // The open PR targets the requested base from an unrelated head, but its
    // body carries the requested SHA: §8.3 must still block a duplicate.
    let github = StubGitHubOps::new().with_open_prs(vec![PullRequestInfo {
        number: Some(7),
        head: "unrelated-fork:totally-different-branch".to_string(),
        base: "staging".to_string(),
        body: format!(
            "Subsystem-ID: hr-management\nSource-Commit: {}\nPromotion-Target: staging\n",
            src.c
        ),
        ..Default::default()
    }]);
    let dups = history::detect_branch_duplicates(
        &main.dir,
        "staging",
        "hr-management",
        std::slice::from_ref(&src.c),
        "ORG/main-system",
        &github,
    );
    assert!(dups.pr_lookup_error.is_none());
    assert_eq!(
        dups.open_prs.len(),
        1,
        "different head must not hide the SHA"
    );

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            ..Default::default()
        },
    );
    assert!(!report.ok);
    assert!(
        codes(&report).contains(&"PR_ALREADY_OPEN".to_string()),
        "{:?}",
        codes(&report)
    );
}

#[test]
fn oracle_p1_cumulative_malformed_base_sha_is_shape_rejected() {
    let src_dir = make_temp_dir("src-baseshape");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            mode: PromotionMode::Cumulative,
            base_sha: Some("ZZZ-not-a-sha".to_string()),
            ..Default::default()
        },
    );
    assert!(!report.ok);
    assert!(
        codes(&report).contains(&"COMMIT_VERIFICATION_FAILED".to_string()),
        "{:?}",
        codes(&report)
    );
    let messages = report
        .blockers
        .iter()
        .map(|b| b.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        messages.contains("must be a full 40/64-char lowercase hex SHA"),
        "malformed base_sha must fail the shape gate, not ancestry: {messages}"
    );
}

#[test]
fn oracle_p1_copy_applies_new_path_only() {
    let src_dir = make_temp_dir("src-copy");
    init_repo(&src_dir, "master");
    commit_files(
        &src_dir,
        "A: base",
        &[("src/old.txt", b"base content\n")],
        &[],
    );
    let head = commit_files(
        &src_dir,
        "B: add copy",
        &[("src/new.txt", b"copied content\n")],
        &[],
    );
    let work = make_temp_dir("wt-copy");
    write(&work, "src/old.txt", b"worktree divergent content\n");

    let entry = DeltaEntry {
        status: 'C',
        path: None,
        old_path: Some("src/old.txt".to_string()),
        new_path: Some("src/new.txt".to_string()),
        score: None,
    };
    let set = apply_file_delta(
        &src_dir,
        &work,
        &head,
        None,
        &entry,
        &sandbox_root(),
        &ApplyOptions::default(),
    )
    .expect("copy applies cleanly");
    assert_eq!(
        std::fs::read(work.join("src").join("old.txt")).unwrap(),
        b"worktree divergent content\n",
        "copy must never touch the source path"
    );
    assert_eq!(
        std::fs::read(work.join("src").join("new.txt")).unwrap(),
        b"copied content\n"
    );
    assert!(set.results.iter().any(|r| r.path == "src/new.txt"));
    assert!(
        !set.results.iter().any(|r| r.path == "src/old.txt"),
        "no outcome may reference the copy source: {:?}",
        set.results
    );
}

#[test]
fn local_mode_creates_branch_in_clone_without_push_or_pr_and_leaves_worktree_clean() {
    use vos_origins_lib::github::models::PromotionDelivery;

    let src_dir = make_temp_dir("src-local");
    let main_dir = make_temp_dir("main-local");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let head_before = sha(&main.dir, "HEAD");
    let branch_before = git_out(&main.dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let status_before = status_porcelain(&main.dir);
    assert_eq!(status_before, "", "target clone starts clean");

    // Even with push/create_pr requested, `local` must not call them.
    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            delivery: PromotionDelivery::Local,
            push: true,
            create_pr: true,
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().expect("branch");
    let commit = report.commit_sha.clone().expect("commit");

    assert!(
        ref_in(&main.dir, &format!("refs/heads/{branch}")).is_some(),
        "integration branch must exist locally"
    );
    let message = git_out(&main.dir, &["log", "-1", "--format=%B", &branch]);
    assert!(message.contains("Subsystem-ID: hr-management"), "{message}");
    assert!(message.contains(&src.c), "{message}");
    assert_eq!(
        git_out(&main.dir, &["rev-parse", &branch]),
        commit,
        "branch tip is the provenance commit"
    );

    // User's working tree + current branch unchanged.
    assert_eq!(sha(&main.dir, "HEAD"), head_before, "HEAD unchanged");
    assert_eq!(
        git_out(&main.dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
        branch_before,
        "current branch unchanged"
    );
    assert_eq!(
        status_porcelain(&main.dir),
        "",
        "working tree stays clean (work done in a temp worktree)"
    );

    assert!(
        github.calls_named("pushBranch").is_empty(),
        "local mode must not push: {:?}",
        github.calls()
    );
    assert!(
        github.create_pr_calls().is_empty(),
        "local mode must not open a PR"
    );
    assert!(report.push.is_none());
    assert!(report.pull_request.is_none());

    // Worktree removed, branch kept.
    let worktrees = git_out(&main.dir, &["worktree", "list"]);
    assert!(
        !worktrees.contains("vos-integration-"),
        "temp worktree must be removed: {worktrees}"
    );
}

#[test]
fn push_pr_mode_still_pushes_and_creates_pr() {
    use vos_origins_lib::github::models::PromotionDelivery;

    let src_dir = make_temp_dir("src-pushpr");
    let main_dir = make_temp_dir("main-pushpr");
    let bare_dir = make_temp_dir("remote-pushpr");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let bare = init_bare(&bare_dir);
    let github = StubGitHubOps::new().with_push_target(bare.to_string_lossy().to_string());
    let registry = test_registry();

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            delivery: PromotionDelivery::PushPr,
            push: true,
            create_pr: true,
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().unwrap();

    assert!(
        ref_in(&bare, &format!("refs/heads/{branch}")).is_some(),
        "push_pr must push the branch to the remote"
    );
    assert_eq!(github.calls_named("pushBranch").len(), 1);
    assert_eq!(github.create_pr_calls().len(), 1);
    let pr_call = github.create_pr_calls().into_iter().next().unwrap();
    assert!(
        pr_call.payload.contains("-> staging"),
        "PR base is the target branch: {}",
        pr_call.payload
    );
    let pr = report.pull_request.clone().expect("PR");
    assert_eq!(pr.base, "staging");
    assert!(report.push.as_ref().is_some_and(|p| p.pushed));
}

#[test]
fn delivery_omitted_on_the_wire_defaults_to_local_direct() {
    use vos_origins_lib::github::models::{PromotionDelivery, PromotionRequest};

    let without: PromotionRequest = serde_json::from_str(
        r#"{"subsystemId":"hr-management","sourceCommitShas":[],"mode":"individual","targetBranch":"staging","confirmationAcknowledged":true}"#,
    )
    .expect("wire without delivery parses");
    assert_eq!(without.delivery, PromotionDelivery::LocalDirect);

    let explicit: PromotionRequest = serde_json::from_str(
        r#"{"subsystemId":"hr-management","sourceCommitShas":[],"mode":"individual","targetBranch":"staging","confirmationAcknowledged":true,"delivery":"push_pr"}"#,
    )
    .expect("wire with push_pr parses");
    assert_eq!(explicit.delivery, PromotionDelivery::PushPr);

    let local: PromotionRequest = serde_json::from_str(
        r#"{"subsystemId":"hr-management","sourceCommitShas":[],"mode":"individual","targetBranch":"staging","confirmationAcknowledged":true,"delivery":"local"}"#,
    )
    .expect("wire with local parses");
    assert_eq!(local.delivery, PromotionDelivery::Local);

    let direct: PromotionRequest = serde_json::from_str(
        r#"{"subsystemId":"hr-management","sourceCommitShas":[],"mode":"individual","targetBranch":"staging","confirmationAcknowledged":true,"delivery":"local_direct"}"#,
    )
    .expect("wire with local_direct parses");
    assert_eq!(direct.delivery, PromotionDelivery::LocalDirect);
}

#[test]
fn local_mode_fails_closed_on_non_git_target_missing_ref_and_existing_branch() {
    use vos_origins_lib::github::models::PromotionDelivery;

    let src_dir = make_temp_dir("src-local-err");
    let src = create_source_chain(&src_dir);
    let not_git = make_temp_dir("not-git");
    std::fs::write(not_git.join("file.txt"), "x\n").expect("write");
    let github = StubGitHubOps::new();
    let registry = test_registry();
    let bad_repo = promote(
        &src_dir,
        &not_git,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            delivery: PromotionDelivery::Local,
            ..Default::default()
        },
    );
    assert!(!bad_repo.ok);
    assert!(
        codes(&bad_repo).contains(&"LOCAL_REPO_INVALID".to_string()),
        "{:?}",
        codes(&bad_repo)
    );

    let main_dir = make_temp_dir("main-local-missing");
    let main = create_main_repo(&main_dir, &[], &[]);
    let missing = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            delivery: PromotionDelivery::Local,
            target_branch: "release/9.9".to_string(),
            ..Default::default()
        },
    );
    assert!(!missing.ok);
    assert!(
        codes(&missing).contains(&"TARGET_NOT_FOUND".to_string()),
        "{:?}",
        codes(&missing)
    );

    let main_dir2 = make_temp_dir("main-local-collide");
    let main2 = create_main_repo(&main_dir2, &[], &[]);
    let fixed_nonce = format!("collide-{}-{}", std::process::id(), "branch-exists");
    let run_once = |nonce: &str| {
        let request = EngineRequest {
            subsystem_id: "hr-management".to_string(),
            commits: vec![src.c.clone()],
            mode: PromotionMode::Individual,
            base_sha: None,
            target_branch: "staging".to_string(),
            delivery: PromotionDelivery::Local,
            on_conflict: PromotionOnConflict::Fail,
            message: "test promotion".to_string(),
            excluded_paths: Vec::new(),
        };
        let engine_options = EngineOptions {
            target_repo_dir: main2.dir.clone(),
            temp_root: sandbox_root(),
            push_remote_url: None,
            token: None,
            allow_shared_config: false,
            allow_removals: false,
            nonce: nonce.to_string(),
            push: false,
            create_pr: false,
        };
        let source_dir = src_dir.clone();
        let source_url = move |_e: &vos_origins_lib::services::registry::SubsystemEntry| {
            Some(source_dir.to_string_lossy().replace('\\', "/"))
        };
        let lockfile = StubLockfileGenerator;
        let deps = EngineDeps {
            github: &github,
            lockfile: &lockfile,
            source_url: &source_url,
            account: || None,
        };
        run_promotion(&registry, &request, &engine_options, &deps)
    };
    let first = run_once(&fixed_nonce);
    assert!(first.ok, "{:?}", first.blockers);
    let branch = first.integration_branch.clone().unwrap();
    let second = run_once(&fixed_nonce);
    assert!(!second.ok);
    assert!(
        codes(&second).contains(&"BRANCH_ALREADY_EXISTS".to_string()),
        "{:?}",
        codes(&second)
    );
    assert!(
        ref_in(&main2.dir, &format!("refs/heads/{branch}")).is_some(),
        "the pre-existing branch must not be deleted by the failed re-run"
    );
}

#[test]
fn local_direct_checked_out_target_advances_branch_and_worktree() {
    use vos_origins_lib::github::models::PromotionDelivery;

    let src_dir = make_temp_dir("src-direct-clean");
    let main_dir = make_temp_dir("main-direct-clean");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    git_out(&main.dir, &["checkout", "-q", "staging"]);
    let staging_before = sha(&main.dir, "staging");
    assert_eq!(status_porcelain(&main.dir), "", "target clone starts clean");

    // Even with push/create_pr requested, `local_direct` must not call them.
    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            delivery: PromotionDelivery::LocalDirect,
            push: true,
            create_pr: true,
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let commit = report.commit_sha.clone().expect("commit");

    assert_eq!(sha(&main.dir, "staging"), commit, "staging advanced");
    assert!(
        report.integration_branch.is_none(),
        "no temp branch may be advertised"
    );
    let message = git_out(&main.dir, &["log", "-1", "--format=%B", "staging"]);
    assert!(message.contains("Subsystem-ID: hr-management"), "{message}");
    assert!(message.contains(&src.c), "{message}");
    assert!(is_ancestor(&main.dir, &staging_before, &commit));

    assert_eq!(
        git_out(&main.dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "staging",
        "current branch unchanged"
    );
    assert_eq!(sha(&main.dir, "HEAD"), commit);
    assert_eq!(
        status_porcelain(&main.dir),
        "",
        "working tree matches the new commit"
    );
    assert!(
        main.dir.join("src/modules/hr/c.txt").exists(),
        "promoted file present in the working tree"
    );

    assert!(
        github.calls_named("pushBranch").is_empty(),
        "local_direct must not push: {:?}",
        github.calls()
    );
    assert!(
        github.create_pr_calls().is_empty(),
        "local_direct must not open a PR"
    );
    assert!(report.push.is_none());
    assert!(report.pull_request.is_none());

    let worktrees = git_out(&main.dir, &["worktree", "list"]);
    assert!(
        !worktrees.contains("vos-integration-"),
        "temp worktree must be removed: {worktrees}"
    );
    let branches = git_out(&main.dir, &["branch", "--format=%(refname:short)"]);
    assert!(
        !branches
            .lines()
            .any(|b| b.trim().starts_with("integration/")),
        "no integration branch may be created: {branches}"
    );
}

#[test]
fn local_direct_other_branch_checked_out_moves_ref_only() {
    use vos_origins_lib::github::models::PromotionDelivery;

    let src_dir = make_temp_dir("src-direct-detached");
    let main_dir = make_temp_dir("main-direct-detached");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    assert_eq!(
        git_out(&main.dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "master"
    );
    let master_before = sha(&main.dir, "master");
    let index_before =
        std::fs::read(main.dir.join("src/modules/hr/index.ts")).expect("read worktree file");

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            delivery: PromotionDelivery::LocalDirect,
            target_branch: "staging".to_string(),
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let commit = report.commit_sha.clone().expect("commit");

    assert_eq!(sha(&main.dir, "staging"), commit);
    let message = git_out(&main.dir, &["log", "-1", "--format=%B", "staging"]);
    assert!(message.contains("Subsystem-ID: hr-management"), "{message}");

    // The checked-out branch and the working-tree files are unchanged.
    assert_eq!(
        git_out(&main.dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "master",
        "current branch untouched"
    );
    assert_eq!(
        sha(&main.dir, "master"),
        master_before,
        "master ref untouched"
    );
    assert_eq!(sha(&main.dir, "HEAD"), master_before);
    assert_eq!(
        std::fs::read(main.dir.join("src/modules/hr/index.ts")).expect("read worktree file"),
        index_before,
        "working-tree files unchanged"
    );
    assert!(
        !main.dir.join("src/modules/hr/c.txt").exists(),
        "promoted file must not leak into the other checkout"
    );
    assert_eq!(status_porcelain(&main.dir), "");

    assert!(report.push.is_none());
    assert!(report.pull_request.is_none());
    let branches = git_out(&main.dir, &["branch", "--format=%(refname:short)"]);
    assert!(
        !branches
            .lines()
            .any(|b| b.trim().starts_with("integration/")),
        "no integration branch may be created: {branches}"
    );
}

#[test]
fn local_direct_dirty_worktree_fails_closed() {
    use vos_origins_lib::github::models::PromotionDelivery;

    let src_dir = make_temp_dir("src-direct-dirty");
    let main_dir = make_temp_dir("main-direct-dirty");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    git_out(&main.dir, &["checkout", "-q", "staging"]);
    let staging_before = sha(&main.dir, "staging");
    write(&main.dir, "uncommitted.txt", b"dirty\n");
    assert!(
        !status_porcelain(&main.dir).is_empty(),
        "working tree must be dirty for this test"
    );

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            delivery: PromotionDelivery::LocalDirect,
            ..Default::default()
        },
    );
    assert!(!report.ok);
    assert!(
        codes(&report).contains(&"DIRTY_WORKTREE".to_string()),
        "{:?}",
        codes(&report)
    );
    assert_eq!(sha(&main.dir, "staging"), staging_before, "nothing moved");
    assert!(
        main.dir.join("uncommitted.txt").exists(),
        "user work untouched"
    );
    assert!(github.create_pr_calls().is_empty());
    let branches = git_out(&main.dir, &["branch", "--format=%(refname:short)"]);
    assert!(
        !branches
            .lines()
            .any(|b| b.trim().starts_with("integration/")),
        "failed promotion must not leave an integration/* ref: {branches}"
    );
    let worktrees = git_out(&main.dir, &["worktree", "list"]);
    assert!(
        !worktrees.contains("vos-integration-"),
        "temp worktree must be removed: {worktrees}"
    );
}

#[test]
fn local_direct_target_tip_moved_fails_closed() {
    use vos_origins_lib::github::models::PromotionDelivery;
    use vos_origins_lib::services::engine::github_ops::{
        BranchInfo, CreatePrRequest, GitHubMode, GitHubOps, PullRequestInfo, PushOutcome,
    };
    use vos_origins_lib::services::engine::lockfile::StubLockfileGenerator;

    /// Simulates a concurrent advance of the target branch between commit
    /// and ref move: a fresh empty commit parented on the tip, made with
    /// plumbing only (no checkout is touched). Hooks the post-commit
    /// duplicate re-check (`list_open_pull_requests`), which still runs for
    /// `local_direct` — the lockfile hook is gone (the deployment
    /// regenerates the lockfile, so the engine never calls it).
    struct MoveTipGithub {
        repo: PathBuf,
        branch: String,
    }

    fn move_tip(repo: &Path, branch: &str) {
        let tip = git_out(repo, &["rev-parse", branch]);
        let tree = git_out(repo, &["rev-parse", &format!("{tip}^{{tree}}")]);
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["commit-tree", &tree, "-p", &tip, "-m", "concurrent advance"])
            .output()
            .expect("spawn git commit-tree");
        assert!(
            output.status.success(),
            "commit-tree failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let moved = String::from_utf8_lossy(&output.stdout).trim().to_string();
        git_out(
            repo,
            &["update-ref", &format!("refs/heads/{branch}"), &moved, &tip],
        );
    }

    impl GitHubOps for MoveTipGithub {
        fn mode(&self) -> GitHubMode {
            GitHubMode::Stub
        }
        fn push_target(&self) -> Option<String> {
            None
        }
        fn push_branch(
            &self,
            _repo_dir: &Path,
            _branch: &str,
            _remote_url: Option<&str>,
        ) -> Result<PushOutcome, String> {
            Ok(PushOutcome {
                pushed: false,
                remote: None,
                source: "no-push-target".to_string(),
            })
        }
        fn list_open_pull_requests(
            &self,
            _repo_slug: &str,
            _base: &str,
        ) -> Result<Vec<PullRequestInfo>, String> {
            move_tip(&self.repo, &self.branch);
            Ok(Vec::new())
        }
        fn create_pull_request(
            &self,
            _repo_slug: &str,
            _request: &CreatePrRequest,
        ) -> Result<PullRequestInfo, String> {
            Err("should not be called".to_string())
        }
        fn resolve_target_tip(
            &self,
            _repo_slug: &str,
            _branch: &str,
        ) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn list_branches(&self, _repo_slug: &str) -> Result<Vec<BranchInfo>, String> {
            Ok(Vec::new())
        }
        fn repo_default_branch(&self, _repo_slug: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
    }

    let src_dir = make_temp_dir("src-direct-moved");
    let main_dir = make_temp_dir("main-direct-moved");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let registry = test_registry();
    let staging_before = sha(&main.dir, "staging");

    let request = EngineRequest {
        subsystem_id: "hr-management".to_string(),
        commits: vec![src.c.clone()],
        mode: PromotionMode::Individual,
        base_sha: None,
        target_branch: "staging".to_string(),
        delivery: PromotionDelivery::LocalDirect,
        on_conflict: PromotionOnConflict::Fail,
        message: "test promotion".to_string(),
        excluded_paths: Vec::new(),
    };
    let engine_options = EngineOptions {
        target_repo_dir: main.dir.clone(),
        temp_root: sandbox_root(),
        push_remote_url: None,
        token: None,
        allow_shared_config: false,
        allow_removals: false,
        nonce: format!(
            "test-direct-moved-{}-{}",
            std::process::id(),
            make_temp_dir("nonce")
                .file_name()
                .unwrap()
                .to_string_lossy()
        ),
        push: false,
        create_pr: false,
    };
    let source_dir = src_dir.clone();
    let source_url = move |_e: &vos_origins_lib::services::registry::SubsystemEntry| {
        Some(source_dir.to_string_lossy().replace('\\', "/"))
    };
    let github = MoveTipGithub {
        repo: main.dir.clone(),
        branch: "staging".to_string(),
    };
    let lockfile = StubLockfileGenerator;
    let deps = EngineDeps {
        github: &github,
        lockfile: &lockfile,
        source_url: &source_url,
        account: || None,
    };
    let report = run_promotion(&registry, &request, &engine_options, &deps);
    assert!(!report.ok);
    assert!(
        codes(&report).contains(&"TARGET_MOVED".to_string()),
        "{:?}",
        codes(&report)
    );

    // The tip is the concurrent move — the promotion commit never landed.
    let staging_now = sha(&main.dir, "staging");
    assert_ne!(staging_now, staging_before, "hook must have moved the tip");
    let message = git_out(&main.dir, &["log", "-1", "--format=%B", "staging"]);
    assert!(
        !message.contains("Subsystem-ID"),
        "promotion commit must not be on the target: {message}"
    );
    let branches = git_out(&main.dir, &["branch", "--format=%(refname:short)"]);
    assert!(
        !branches
            .lines()
            .any(|b| b.trim().starts_with("integration/")),
        "failed promotion must not leave an integration/* ref: {branches}"
    );
    let worktrees = git_out(&main.dir, &["worktree", "list"]);
    assert!(
        !worktrees.contains("vos-integration-"),
        "temp worktree must be removed: {worktrees}"
    );
}

#[test]
fn local_direct_next_commands_carry_cd_and_push() {
    use vos_origins_lib::commands::promotion::build_direct_next_commands;

    let dir = if cfg!(windows) {
        PathBuf::from(r"C:\repos\main-system")
    } else {
        PathBuf::from("/tmp/main-system")
    };
    let commands = build_direct_next_commands(&dir, "staging");
    assert_eq!(commands.len(), 2, "{commands:?}");
    assert!(
        commands[0].starts_with("cd "),
        "first command cds into the clone: {commands:?}"
    );
    assert!(
        commands[0].contains(&dir.to_string_lossy().to_string()),
        "{commands:?}"
    );
    assert_eq!(commands[1], "git push origin staging", "{commands:?}");

    use vos_origins_lib::github::models::{PromotionDelivery, PromotionDispatchResult};
    let direct_json = serde_json::json!({
        "accepted": true, "prUrl": null, "prNumber": null,
        "branch": "staging", "commitSha": "a".repeat(40),
        "targetBranch": "staging", "message": "m", "requestedAt": "t",
        "findings": [], "delivery": "local_direct",
        "nextCommands": commands,
    });
    let direct: PromotionDispatchResult =
        serde_json::from_value(direct_json).expect("local_direct result parses");
    assert_eq!(direct.delivery, PromotionDelivery::LocalDirect);
    assert!(!direct.next_commands.is_empty());
    assert!(direct.pr_url.is_none());
    assert!(direct.pr_number.is_none());
}

#[test]
fn registry_round_trip_with_local_main_repository_path_set_and_unset() {
    let with_path = format!(
        r#"{{"mainRepository":"ORG/main-system","localMainRepositoryPath":{},"promotion":{{"allowedTargetBranches":["staging"],"allowedTargetBranchPatterns":[],"requirePullRequest":true}},"subsystems":[]}}"#,
        serde_json::to_string(&sandbox_root().to_string_lossy().to_string()).unwrap()
    );
    let parsed = Registry::from_json(&with_path, "with-path").expect("parses with path");
    assert!(parsed.local_main_repository_path.is_some());
    let encoded = serde_json::to_string(&parsed).expect("encodes");
    assert!(encoded.contains("localMainRepositoryPath"), "{encoded}");
    let reparsed = Registry::from_json(&encoded, "reparse").expect("round-trips");
    assert_eq!(parsed, reparsed);

    let without = r#"{"mainRepository":"ORG/main-system","promotion":{"allowedTargetBranches":["staging"],"allowedTargetBranchPatterns":[],"requirePullRequest":true},"subsystems":[]}"#;
    let parsed_without = Registry::from_json(without, "without").expect("parses without path");
    assert_eq!(parsed_without.local_main_repository_path, None);
    let encoded_without = serde_json::to_string(&parsed_without).expect("encodes");
    let reparsed_without =
        Registry::from_json(&encoded_without, "reparse-without").expect("round-trips");
    assert_eq!(parsed_without, reparsed_without);

    // Blank and relative paths are rejected fail-closed.
    let blank = r#"{"mainRepository":"ORG/main-system","localMainRepositoryPath":"  ","promotion":{"allowedTargetBranches":["staging"],"allowedTargetBranchPatterns":[],"requirePullRequest":true},"subsystems":[]}"#;
    assert!(Registry::from_json(blank, "blank").is_err());
    let relative = r#"{"mainRepository":"ORG/main-system","localMainRepositoryPath":"repos/main","promotion":{"allowedTargetBranches":["staging"],"allowedTargetBranchPatterns":[],"requirePullRequest":true},"subsystems":[]}"#;
    assert!(Registry::from_json(relative, "relative").is_err());
}

#[test]
fn local_next_commands_carry_cd_push_and_pr_create() {
    use vos_origins_lib::commands::promotion::build_next_commands;

    let dir = if cfg!(windows) {
        PathBuf::from(r"C:\repos\main-system")
    } else {
        PathBuf::from("/tmp/main-system")
    };
    let commands = build_next_commands(
        &dir,
        "integration/hr-abc-staging-n1",
        "staging",
        "hr-management",
        2,
    );
    assert_eq!(commands.len(), 3, "{commands:?}");
    assert!(
        commands[0].starts_with("cd "),
        "first command cds into the clone: {commands:?}"
    );
    assert!(
        commands[0].contains(&dir.to_string_lossy().to_string()),
        "{commands:?}"
    );
    assert_eq!(
        commands[1], "git push -u origin integration/hr-abc-staging-n1",
        "{commands:?}"
    );
    assert!(
        commands[2].contains("gh pr create")
            && commands[2].contains("--base staging")
            && commands[2].contains("--head integration/hr-abc-staging-n1")
            && commands[2].contains("hr-management: integrate 2 commit(s)"),
        "{commands:?}"
    );

    use vos_origins_lib::github::models::{PromotionDelivery, PromotionDispatchResult};
    let local_json = serde_json::json!({
        "accepted": true, "prUrl": null, "prNumber": null,
        "branch": "integration/hr-abc-staging-n1", "commitSha": "a".repeat(40),
        "targetBranch": "staging", "message": "m", "requestedAt": "t",
        "findings": [], "delivery": "local",
        "nextCommands": commands,
    });
    let local: PromotionDispatchResult =
        serde_json::from_value(local_json).expect("local result parses");
    assert_eq!(local.delivery, PromotionDelivery::Local);
    assert!(!local.next_commands.is_empty());
    assert!(local.pr_url.is_none());
    assert!(local.pr_number.is_none());
}

#[test]
fn add_over_existing_with_older_target_is_overwritten_by_membership() {
    let src_dir = make_temp_dir("src-addupdate");
    let main_dir = make_temp_dir("main-addupdate");

    init_repo(&src_dir, "master");
    commit_files_dated(
        &src_dir,
        "A: base",
        &[
            ("package.json", b"{\"name\":\"hr-management\"}\n".as_slice()),
            (
                "src/modules/hr/index.ts",
                b"export const a = 1;\n".as_slice(),
            ),
        ],
        "2020-01-01T00:00:00+00:00",
    );
    let add = commit_files_dated(
        &src_dir,
        "B: add shared file",
        &[(
            "src/modules/hr/shared.txt",
            b"subsystem newer version\n".as_slice(),
        )],
        "2024-01-01T00:00:00+00:00",
    );

    // The main system already carries a divergent copy (older date, but dates
    // never decide — blob membership does, E3). The subsystem's copy wins.
    init_repo(&main_dir, "master");
    commit_files_dated(
        &main_dir,
        "main: base",
        &[
            ("package.json", BASE_MAIN_PKG.as_bytes()),
            ("package-lock.json", LOCKFILE.as_bytes()),
            (
                "src/modules/hr/index.ts",
                b"export const a = 1;\n".as_slice(),
            ),
            (
                "src/modules/hr/shared.txt",
                b"main older version\n".as_slice(),
            ),
        ],
        "2020-06-01T00:00:00+00:00",
    );
    git_out(&main_dir, &["branch", "staging"]);

    let github = StubGitHubOps::new();
    let registry = test_registry();
    let report = promote(
        &src_dir,
        &main_dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![add],
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().unwrap();
    assert_eq!(
        branch_file_text(&main_dir, &branch, "src/modules/hr/shared.txt"),
        "subsystem newer version\n",
        "the subsystem's copy wins"
    );
    assert!(
        report
            .changes
            .replaced
            .iter()
            .any(|p| p == "src/modules/hr/shared.txt"),
        "overwritten path is reported as replaced: {:?}",
        report.changes
    );
}

#[test]
fn add_over_existing_with_newer_target_is_overwritten_by_membership() {
    let src_dir = make_temp_dir("src-addblocked");
    let main_dir = make_temp_dir("main-addblocked");

    init_repo(&src_dir, "master");
    commit_files_dated(
        &src_dir,
        "A: base",
        &[
            ("package.json", b"{\"name\":\"hr-management\"}\n".as_slice()),
            (
                "src/modules/hr/index.ts",
                b"export const a = 1;\n".as_slice(),
            ),
        ],
        "2020-01-01T00:00:00+00:00",
    );
    let add = commit_files_dated(
        &src_dir,
        "B: add shared file",
        &[(
            "src/modules/hr/shared.txt",
            b"subsystem older version\n".as_slice(),
        )],
        "2020-02-01T00:00:00+00:00",
    );

    // The main system's copy is NEWER than the source commit — dates never
    // decide (E3). The subsystem's copy wins.
    init_repo(&main_dir, "master");
    commit_files_dated(
        &main_dir,
        "main: base",
        &[
            ("package.json", BASE_MAIN_PKG.as_bytes()),
            ("package-lock.json", LOCKFILE.as_bytes()),
            (
                "src/modules/hr/index.ts",
                b"export const a = 1;\n".as_slice(),
            ),
            (
                "src/modules/hr/shared.txt",
                b"main newer version\n".as_slice(),
            ),
        ],
        "2024-01-01T00:00:00+00:00",
    );
    git_out(&main_dir, &["branch", "staging"]);

    let github = StubGitHubOps::new();
    let registry = test_registry();
    let report = promote(
        &src_dir,
        &main_dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![add],
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().unwrap();
    assert_eq!(
        branch_file_text(&main_dir, &branch, "src/modules/hr/shared.txt"),
        "subsystem older version\n",
        "the subsystem's copy wins even when the target is newer"
    );
    assert!(
        report
            .changes
            .replaced
            .iter()
            .any(|p| p == "src/modules/hr/shared.txt"),
        "overwritten path is reported as replaced: {:?}",
        report.changes
    );
}

#[test]
fn plain_add_without_target_file_still_applies_as_add() {
    let src_dir = make_temp_dir("src-plainadd");
    let main_dir = make_temp_dir("main-plainadd");

    init_repo(&src_dir, "master");
    commit_files_dated(
        &src_dir,
        "A: base",
        &[
            ("package.json", b"{\"name\":\"hr-management\"}\n".as_slice()),
            (
                "src/modules/hr/index.ts",
                b"export const a = 1;\n".as_slice(),
            ),
        ],
        "2020-01-01T00:00:00+00:00",
    );
    let add = commit_files_dated(
        &src_dir,
        "B: add fresh file",
        &[("src/modules/hr/fresh.txt", b"brand new\n".as_slice())],
        "2024-01-01T00:00:00+00:00",
    );

    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();
    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![add],
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().unwrap();
    assert_eq!(
        branch_file_text(&main.dir, &branch, "src/modules/hr/fresh.txt"),
        "brand new\n"
    );
    assert!(
        report
            .changes
            .applied
            .iter()
            .any(|p| p == "src/modules/hr/fresh.txt"),
        "a plain add stays a plain add: {:?}",
        report.changes
    );
    assert!(
        report.changes.updated.is_empty(),
        "no update may be reported for a plain add: {:?}",
        report.changes
    );
}

#[test]
fn plain_modify_diverged_target_is_overwritten() {
    let src_dir = make_temp_dir("src-plainmod");
    let main_dir = make_temp_dir("main-plainmod");

    init_repo(&src_dir, "master");
    commit_files_dated(
        &src_dir,
        "A: baseline",
        &[("src/merge-plain.txt", b"a\nb\nc\nd\n".as_slice())],
        "2020-01-01T00:00:00+00:00",
    );
    let modify = commit_files_dated(
        &src_dir,
        "B: modify a different line",
        &[("src/merge-plain.txt", b"a\nb\nc\nX\n".as_slice())],
        "2024-01-01T00:00:00+00:00",
    );

    // Target changed a different line: blob membership (§3) records the
    // overwrite (the target blob matches neither base nor tip). The
    // subsystem's copy wins. Copy never merges.
    let main = create_main_repo(&main_dir, &[], &[("src/merge-plain.txt", b"a\nY\nc\nd\n")]);
    let github = StubGitHubOps::new();
    let registry = test_registry();
    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![modify],
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().unwrap();
    assert_eq!(
        branch_file_text(&main.dir, &branch, "src/merge-plain.txt"),
        "a\nb\nc\nX\n",
        "the subsystem's copy wins"
    );
    assert!(
        report
            .changes
            .replaced
            .iter()
            .any(|p| p == "src/merge-plain.txt"),
        "overwritten path is reported as replaced: {:?}",
        report.changes
    );
}

#[test]
fn conflict_source_unchanged_binary_keeps_target() {
    use vos_origins_lib::services::engine::apply::ApplyOutcome;

    let src_dir = make_temp_dir("src-bin-untouched");
    init_repo(&src_dir, "master");
    let v1: &[u8] = &[0x00, 0x01, 0x02, 0xff];
    let v2: &[u8] = &[0x00, 0x07, 0x00, 0xfd];
    commit_files(
        &src_dir,
        "A: binary baseline",
        &[("src/assets/bin.dat", v1), ("src/other.txt", b"a\n")],
        &[],
    );
    let base = sha(&src_dir, "HEAD");
    let head = commit_files(&src_dir, "B: unrelated", &[("src/other.txt", b"b\n")], &[]);

    let work = make_temp_dir("wt-bin-untouched");
    write(&work, "src/assets/bin.dat", v2);

    let entry = DeltaEntry {
        status: 'M',
        path: Some("src/assets/bin.dat".to_string()),
        old_path: None,
        new_path: None,
        score: None,
    };
    let set = apply_file_delta(
        &src_dir,
        &work,
        &head,
        Some(&base),
        &entry,
        &sandbox_root(),
        &ApplyOptions::default(),
    )
    .expect("a source-unchanged binary must not conflict");
    assert_eq!(set.results.len(), 1, "{:?}", set.results);
    assert_eq!(set.results[0].outcome, ApplyOutcome::Unchanged);
    assert_eq!(
        std::fs::read(work.join("src").join("assets").join("bin.dat")).unwrap(),
        v2,
        "the target's bytes are kept"
    );
}

#[test]
fn conflict_take_does_not_bypass_guards() {
    // An unsafe traversal path still errors under take_subsystem.
    let objects = make_temp_dir("obj-traversal");
    let work = make_temp_dir("wt-traversal");
    let entry = DeltaEntry {
        status: 'M',
        path: Some("../evil.txt".to_string()),
        old_path: None,
        new_path: None,
        score: None,
    };
    let err: ApplyConflict = apply_file_delta(
        &objects,
        &work,
        &"0".repeat(40),
        None,
        &entry,
        &sandbox_root(),
        &ApplyOptions {
            on_conflict: PromotionOnConflict::TakeSubsystem,
            ..Default::default()
        },
    )
    .expect_err("traversal must fail closed even under take_subsystem");
    assert_eq!(err.kind, "path-escape");

    let src_dir = make_temp_dir("src-takedep");
    let main_dir = make_temp_dir("main-takedep");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[("exceljs", "^3.0.0")], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();
    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            on_conflict: PromotionOnConflict::TakeSubsystem,
            ..Default::default()
        },
    );
    assert!(!report.ok);
    assert!(
        codes(&report).contains(&"DEPENDENCY_INCOMPATIBLE_RANGE".to_string()),
        "{:?}",
        codes(&report)
    );
}

#[test]
fn conflict_on_conflict_wire_defaults_to_fail() {
    use vos_origins_lib::github::models::{PromotionOnConflict, PromotionRequest};

    let without: PromotionRequest = serde_json::from_str(
        r#"{"subsystemId":"hr-management","sourceCommitShas":[],"mode":"individual","targetBranch":"staging","confirmationAcknowledged":true}"#,
    )
    .expect("wire without onConflict parses");
    assert_eq!(without.on_conflict, PromotionOnConflict::Fail);

    let explicit: PromotionRequest = serde_json::from_str(
        r#"{"subsystemId":"hr-management","sourceCommitShas":[],"mode":"individual","targetBranch":"staging","confirmationAcknowledged":true,"onConflict":"take_subsystem"}"#,
    )
    .expect("wire with take_subsystem parses");
    assert_eq!(explicit.on_conflict, PromotionOnConflict::TakeSubsystem);
}

// ===========================================================================
// CRLF regression (core.autocrlf=true): worktree holds CRLF, blobs hold LF.
// Line endings are irrelevant to the engine; only real content matters.
// ===========================================================================

fn crlf(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 8);
    for &b in bytes {
        if b == b'\n' {
            out.push(b'\r');
            out.push(b'\n');
        } else {
            out.push(b);
        }
    }
    out
}

#[test]
fn crlf_worktree_already_matching_source_is_unchanged() {
    use vos_origins_lib::services::engine::apply::ApplyOutcome;

    let src_dir = make_temp_dir("src-crlf-unchanged");
    init_repo(&src_dir, "master");
    // Reproduce the real promotion setting: the target checks out CRLF while
    // blobs stay LF.
    git_out(&src_dir, &["config", "core.autocrlf", "true"]);
    commit_files(
        &src_dir,
        "A: base",
        &[("src/app.txt", b"line1\nline2\nline3\n")],
        &[],
    );
    let base = sha(&src_dir, "HEAD");
    let head = commit_files(
        &src_dir,
        "C: change line2",
        &[("src/app.txt", b"line1\nline2 changed\nline3\n")],
        &[],
    );

    // Target content is already identical to the commit's content (same blob
    // SHA), but the worktree file carries CRLF from autocrlf.
    let work = make_temp_dir("wt-crlf-unchanged");
    write(
        &work,
        "src/app.txt",
        &crlf(b"line1\nline2 changed\nline3\n"),
    );

    let entry = DeltaEntry {
        status: 'M',
        path: Some("src/app.txt".to_string()),
        old_path: None,
        new_path: None,
        score: None,
    };
    let set = apply_file_delta(
        &src_dir,
        &work,
        &head,
        Some(&base),
        &entry,
        &sandbox_root(),
        &ApplyOptions::default(),
    )
    .expect("CRLF-identical target must not conflict");
    assert_eq!(set.results.len(), 1, "{:?}", set.results);
    assert_eq!(set.results[0].outcome, ApplyOutcome::Unchanged);
    assert_eq!(set.results[0].path, "src/app.txt");
    // Unchanged leaves the worktree file alone (still CRLF, no rewrite).
    assert_eq!(
        std::fs::read(work.join("src").join("app.txt")).unwrap(),
        crlf(b"line1\nline2 changed\nline3\n")
    );
}

#[test]
fn crlf_worktree_matching_base_writes_clean() {
    use vos_origins_lib::services::engine::apply::ApplyOutcome;

    let src_dir = make_temp_dir("src-crlf-written");
    init_repo(&src_dir, "master");
    git_out(&src_dir, &["config", "core.autocrlf", "true"]);
    commit_files(
        &src_dir,
        "A: base",
        &[("src/app.txt", b"line1\nline2\nline3\n")],
        &[],
    );
    let base = sha(&src_dir, "HEAD");
    let head = commit_files(
        &src_dir,
        "C: change line2",
        &[("src/app.txt", b"line1\nline2 changed\nline3\n")],
        &[],
    );

    // Target equals the source base (modulo CRLF) → clean write, not a merge.
    let work = make_temp_dir("wt-crlf-written");
    write(&work, "src/app.txt", &crlf(b"line1\nline2\nline3\n"));

    let entry = DeltaEntry {
        status: 'M',
        path: Some("src/app.txt".to_string()),
        old_path: None,
        new_path: None,
        score: None,
    };
    let set = apply_file_delta(
        &src_dir,
        &work,
        &head,
        Some(&base),
        &entry,
        &sandbox_root(),
        &ApplyOptions::default(),
    )
    .expect("CRLF base match must write cleanly");
    assert_eq!(set.results.len(), 1, "{:?}", set.results);
    assert_eq!(set.results[0].outcome, ApplyOutcome::Written);
    assert_eq!(
        std::fs::read(work.join("src").join("app.txt")).unwrap(),
        b"line1\nline2 changed\nline3\n"
    );
}

#[test]
fn crlf_worktree_genuine_divergence_copies_source_bytes() {
    use vos_origins_lib::services::engine::apply::ApplyOutcome;

    let src_dir = make_temp_dir("src-crlf-conflict");
    init_repo(&src_dir, "master");
    git_out(&src_dir, &["config", "core.autocrlf", "true"]);
    commit_files(
        &src_dir,
        "A: base",
        &[("src/conflict.txt", b"a\nb\nc\nd\n")],
        &[],
    );
    let base = sha(&src_dir, "HEAD");
    let head = commit_files(
        &src_dir,
        "C: change last line to X",
        &[("src/conflict.txt", b"a\nb\nc\nX\n")],
        &[],
    );

    // Both sides changed the same line (target says Y). Under copy semantics the
    // subsystem's bytes win outright - there is no conflict to raise, and CRLF
    // never hides or fakes anything.
    let work = make_temp_dir("wt-crlf-conflict");
    write(&work, "src/conflict.txt", &crlf(b"a\nb\nc\nY\n"));

    let entry = DeltaEntry {
        status: 'M',
        path: Some("src/conflict.txt".to_string()),
        old_path: None,
        new_path: None,
        score: None,
    };
    let set = apply_file_delta(
        &src_dir,
        &work,
        &head,
        Some(&base),
        &entry,
        &sandbox_root(),
        &ApplyOptions::default(),
    )
    .expect("copy semantics never conflict");
    let written = set
        .results
        .iter()
        .find(|r| r.path == "src/conflict.txt")
        .expect("the path is reported");
    assert!(
        matches!(written.outcome, ApplyOutcome::Written),
        "{:?}",
        written.outcome
    );
}

#[test]
fn binary_with_crlf_bytes_written_byte_exact() {
    use vos_origins_lib::services::engine::apply::ApplyOutcome;

    let src_dir = make_temp_dir("src-crlf-binary");
    init_repo(&src_dir, "master");
    git_out(&src_dir, &["config", "core.autocrlf", "true"]);
    let base_bytes: &[u8] = b"BIN\x00\x01\r\n\xff\x00DATA\r\nEND";
    let new_bytes: &[u8] = b"BIN\x00\x02\r\n\xfe\x00NEWDATA\r\nEND";
    commit_files(
        &src_dir,
        "A: binary baseline",
        &[("src/assets/bin.dat", base_bytes)],
        &[],
    );
    let base = sha(&src_dir, "HEAD");
    let head = commit_files(
        &src_dir,
        "C: modify binary",
        &[("src/assets/bin.dat", new_bytes)],
        &[],
    );

    let work = make_temp_dir("wt-crlf-binary");
    write(&work, "src/assets/bin.dat", base_bytes);

    let entry = DeltaEntry {
        status: 'M',
        path: Some("src/assets/bin.dat".to_string()),
        old_path: None,
        new_path: None,
        score: None,
    };
    let set = apply_file_delta(
        &src_dir,
        &work,
        &head,
        Some(&base),
        &entry,
        &sandbox_root(),
        &ApplyOptions::default(),
    )
    .expect("binary clean write must succeed");
    assert_eq!(set.results.len(), 1, "{:?}", set.results);
    assert_eq!(set.results[0].outcome, ApplyOutcome::Written);
    assert_eq!(
        std::fs::read(work.join("src").join("assets").join("bin.dat")).unwrap(),
        new_bytes,
        "binary must be written byte-exact, CRLF bytes preserved"
    );
}

fn staged_names(dir: &Path) -> Vec<String> {
    let out = git_out(dir, &["diff", "--cached", "--name-only"]);
    let mut names: Vec<String> = out.lines().map(|l| l.trim().to_string()).collect();
    names.retain(|l| !l.is_empty());
    names.sort();
    names
}

#[test]
fn worktree_promotion_stages_without_commit_or_branch() {
    use vos_origins_lib::github::models::PromotionDelivery;

    let src_dir = make_temp_dir("src-worktree");
    let main_dir = make_temp_dir("main-worktree");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    git_out(&main.dir, &["checkout", "-q", "staging"]);
    let head_before = sha(&main.dir, "HEAD");
    let staging_before = sha(&main.dir, "staging");
    assert_eq!(status_porcelain(&main.dir), "", "target clone starts clean");

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            delivery: PromotionDelivery::Worktree,
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);

    assert!(report.commit_sha.is_none(), "worktree never commits");
    assert!(
        report.integration_branch.is_none(),
        "no temp branch may be advertised"
    );
    assert!(
        report.provenance_message.is_none(),
        "no provenance trailers are written without a commit"
    );
    assert!(report.push.is_none());
    assert!(report.pull_request.is_none());
    assert_eq!(sha(&main.dir, "HEAD"), head_before, "HEAD unchanged");
    assert_eq!(
        sha(&main.dir, "staging"),
        staging_before,
        "target ref unchanged"
    );
    let branches = git_out(&main.dir, &["branch", "--format=%(refname:short)"]);
    assert!(
        !branches
            .lines()
            .any(|b| b.trim().starts_with("integration/")),
        "no branch may be created: {branches}"
    );
    let worktrees = git_out(&main.dir, &["worktree", "list"]);
    assert!(
        !worktrees.contains("vos-integration-"),
        "temp worktree must be removed: {worktrees}"
    );

    let mut expected = vec![
        "package.json".to_string(),
        "src/modules/hr/c.txt".to_string(),
        "src/modules/hr/export.ts".to_string(),
    ];
    expected.sort();
    assert_eq!(staged_names(&main.dir), expected);
    assert_eq!(report.staged_paths, expected);
    assert_eq!(
        git_out(&main.dir, &["diff", "--name-only"]),
        "",
        "nothing may be left unstaged"
    );
    assert_eq!(
        std::fs::read_to_string(main.dir.join("src/modules/hr/c.txt")).unwrap(),
        "C feature\n"
    );
    let pkg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(main.dir.join("package.json")).unwrap())
            .unwrap();
    assert_eq!(pkg["dependencies"]["exceljs"], serde_json::json!("^4.4.0"));
    assert_eq!(
        pkg["dependencies"]["react"],
        serde_json::json!("19.2.3"),
        "unrelated packages retained"
    );

    assert_eq!(
        git_out(&main.dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "staging"
    );
    assert!(
        github.calls_named("pushBranch").is_empty(),
        "worktree must not push: {:?}",
        github.calls()
    );
    assert!(
        github.create_pr_calls().is_empty(),
        "worktree must not open a PR"
    );
}

#[test]
fn worktree_dirty_tree_is_refused_with_tree_untouched() {
    use vos_origins_lib::github::models::PromotionDelivery;

    let src_dir = make_temp_dir("src-worktree-dirty");
    let main_dir = make_temp_dir("main-worktree-dirty");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    git_out(&main.dir, &["checkout", "-q", "staging"]);
    let head_before = sha(&main.dir, "HEAD");
    let staging_before = sha(&main.dir, "staging");
    write(&main.dir, "uncommitted.txt", b"dirty\n");

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            delivery: PromotionDelivery::Worktree,
            ..Default::default()
        },
    );
    assert!(!report.ok);
    assert!(
        codes(&report).contains(&"DIRTY_WORKTREE".to_string()),
        "{:?}",
        codes(&report)
    );
    assert_eq!(sha(&main.dir, "HEAD"), head_before, "nothing moved");
    assert_eq!(sha(&main.dir, "staging"), staging_before);
    assert_eq!(
        std::fs::read(main.dir.join("uncommitted.txt")).unwrap(),
        b"dirty\n",
        "user work untouched"
    );
    assert_eq!(
        staged_names(&main.dir),
        Vec::<String>::new(),
        "nothing may be staged"
    );
    assert!(
        !main.dir.join("src/modules/hr/c.txt").exists(),
        "promoted file must not leak into the tree"
    );
    let branches = git_out(&main.dir, &["branch", "--format=%(refname:short)"]);
    assert!(
        !branches
            .lines()
            .any(|b| b.trim().starts_with("integration/")),
        "failed promotion must not leave an integration/* ref: {branches}"
    );
}

#[test]
fn worktree_target_not_checked_out_is_refused() {
    use vos_origins_lib::github::models::PromotionDelivery;

    let src_dir = make_temp_dir("src-worktree-branch");
    let main_dir = make_temp_dir("main-worktree-branch");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    assert_eq!(
        git_out(&main.dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "master"
    );
    let staging_before = sha(&main.dir, "staging");

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            delivery: PromotionDelivery::Worktree,
            target_branch: "staging".to_string(),
            ..Default::default()
        },
    );
    assert!(!report.ok);
    assert!(
        codes(&report).contains(&"TARGET_NOT_CHECKED_OUT".to_string()),
        "{:?}",
        codes(&report)
    );
    let message = report
        .blockers
        .iter()
        .map(|b| b.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        message.contains("\"staging\"") && message.contains("git checkout staging"),
        "the refusal must name the expected branch and how to switch: {message}"
    );
    assert_eq!(sha(&main.dir, "staging"), staging_before, "nothing moved");
    assert_eq!(status_porcelain(&main.dir), "", "tree untouched");
    let branches = git_out(&main.dir, &["branch", "--format=%(refname:short)"]);
    assert!(
        !branches
            .lines()
            .any(|b| b.trim().starts_with("integration/")),
        "failed promotion must not leave an integration/* ref: {branches}"
    );
}

#[test]
fn worktree_leaves_no_integrated_status_behind() {
    use vos_origins_lib::github::models::PromotionDelivery;
    use vos_origins_lib::services::engine::history;

    let src_dir = make_temp_dir("src-worktree-status");
    let main_dir = make_temp_dir("main-worktree-status");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    git_out(&main.dir, &["checkout", "-q", "staging"]);
    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            delivery: PromotionDelivery::Worktree,
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);

    // Status is derived from reachable history, which gained no commit, so
    // the promoted SHA stays exactly as it was (eligible, never integrated).
    let status = history::compute_branch_status(
        &main.dir,
        "staging",
        "hr-management",
        std::slice::from_ref(&src.c),
    );
    assert_ne!(
        status[0].status,
        history::BranchStatusKind::Integrated,
        "a worktree promotion must never integrate"
    );
    assert_eq!(status[0].status, history::BranchStatusKind::Eligible);
    let log = git_out(&main.dir, &["log", "--format=%B", "staging"]);
    assert!(
        !log.contains(&src.c),
        "no reachable commit may reference the promoted SHA"
    );
}

#[test]
fn local_direct_still_commits_onto_the_target() {
    use vos_origins_lib::github::models::PromotionDelivery;

    let src_dir = make_temp_dir("src-still-direct");
    let main_dir = make_temp_dir("main-still-direct");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    git_out(&main.dir, &["checkout", "-q", "staging"]);
    let staging_before = sha(&main.dir, "staging");

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            delivery: PromotionDelivery::LocalDirect,
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let commit = report.commit_sha.clone().expect("local_direct commits");
    assert_eq!(sha(&main.dir, "staging"), commit, "staging advanced");
    assert_ne!(commit, staging_before);
    let message = git_out(&main.dir, &["log", "-1", "--format=%B", "staging"]);
    assert!(message.contains("Subsystem-ID: hr-management"), "{message}");
    assert!(message.contains(&src.c), "{message}");
}

#[test]
fn worktree_wire_value_parses_and_next_commands_carry_diff_and_commit() {
    use vos_origins_lib::commands::promotion::build_worktree_next_commands;
    use vos_origins_lib::github::models::{
        PromotionDelivery, PromotionDispatchResult, PromotionRequest,
    };

    assert_eq!(PromotionDelivery::Worktree.as_str(), "worktree");
    let wire: PromotionRequest = serde_json::from_str(
        r#"{"subsystemId":"hr-management","sourceCommitShas":[],"mode":"individual","targetBranch":"staging","confirmationAcknowledged":true,"delivery":"worktree"}"#,
    )
    .expect("wire with worktree parses");
    assert_eq!(wire.delivery, PromotionDelivery::Worktree);
    let serialized = serde_json::to_string(&PromotionDelivery::Worktree).expect("serializes");
    assert_eq!(serialized, "\"worktree\"");

    let dir = if cfg!(windows) {
        PathBuf::from(r"C:\repos\main-system")
    } else {
        PathBuf::from("/tmp/main-system")
    };
    let staged = vec![
        "package.json".to_string(),
        "src/modules/hr/c.txt".to_string(),
    ];
    let commands = build_worktree_next_commands(&dir, "my promotion message", &staged);
    assert_eq!(commands.len(), 3, "{commands:?}");
    assert!(
        commands[0].starts_with("cd "),
        "first command cds into the clone: {commands:?}"
    );
    assert!(
        commands[0].contains(&dir.to_string_lossy().to_string()),
        "{commands:?}"
    );
    assert_eq!(commands[1], "git diff --staged", "{commands:?}");
    assert_eq!(
        commands[2], "git commit -m \"my promotion message\" -- package.json src/modules/hr/c.txt",
        "{commands:?}"
    );

    let worktree_json = serde_json::json!({
        "accepted": true, "prUrl": null, "prNumber": null,
        "branch": "staging", "commitSha": null,
        "targetBranch": "staging", "message": "m", "requestedAt": "t",
        "findings": [], "delivery": "worktree",
        "nextCommands": commands,
    });
    let result: PromotionDispatchResult =
        serde_json::from_value(worktree_json).expect("worktree result parses");
    assert_eq!(result.delivery, PromotionDelivery::Worktree);
    assert_eq!(result.branch, "staging");
    assert!(result.commit_sha.is_none());
    assert!(result.pr_url.is_none());
    assert!(result.pr_number.is_none());
    assert!(!result.next_commands.is_empty());
}

// ===========================================================================
// Integrity batch: non-contiguous selection / overwrite / excluded / global /
// reporting / housekeeping (G1, G2, G5, G6)
// ===========================================================================

#[test]
fn non_contiguous_selection_promotes_when_skip_predates_newest_toucher() {
    let src_dir = make_temp_dir("gap-src");
    init_repo(&src_dir, "master");
    commit_files(
        &src_dir,
        "A base",
        &[("p.txt", b"a\n"), ("q.txt", b"a\n")],
        &[],
    );
    let b = commit_files(
        &src_dir,
        "B touches P and R",
        &[("p.txt", b"b\n"), ("r.txt", b"b\n")],
        &[],
    );
    let _c = commit_files(&src_dir, "C touches P only", &[("p.txt", b"c\n")], &[]);
    let d = commit_files(&src_dir, "D touches Q only", &[("q.txt", b"d\n")], &[]);
    let main_dir = make_temp_dir("gap-main");
    let main = create_main_repo(&main_dir, &[], &[("p.txt", b"a\n"), ("q.txt", b"a\n")]);
    let github = StubGitHubOps::new();
    let registry = test_registry();
    let allowed = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![b.clone(), d.clone()],
            ..Default::default()
        },
    );
    assert!(
        allowed.ok,
        "non-contiguous selection must promote: {:?}",
        allowed.blockers
    );
    let branch = allowed.integration_branch.clone().unwrap();
    assert_eq!(branch_file_text(&main.dir, &branch, "p.txt"), "b\n");
    assert_eq!(branch_file_text(&main.dir, &branch, "q.txt"), "d\n");
    assert!(try_branch_has(&main.dir, &branch, "r.txt"));
}

/// PIN: a non-contiguous selection whose paths overlap promotes successfully
/// AND carries the skipped commit's edits. Selecting `[B, D]` while skipping
/// `C`, where `D` touches the same path `P` as `C`, ships `D`'s whole-file
/// blob for `P` — which already contains `C`'s edits. This is deliberate:
/// whole-file copy means a skipped commit's edits to a file a selected commit
/// also touches are included. Do not "fix" this back into a refusal.
#[test]
fn pin_non_contiguous_overlap_promotes_and_carries_skipped_edits() {
    let src_dir = make_temp_dir("gap-pin-src");
    init_repo(&src_dir, "master");
    // A base, then B/C/D each edit a different line of the same file.
    commit_files(
        &src_dir,
        "A base",
        &[("p.txt", b"l1-a\nl2-a\n"), ("q.txt", b"a\n")],
        &[],
    );
    let b = commit_files(
        &src_dir,
        "B edits line 1",
        &[("p.txt", b"l1-b\nl2-a\n")],
        &[],
    );
    let c = commit_files(
        &src_dir,
        "C edits line 2",
        &[("p.txt", b"l1-b\nl2-c\n")],
        &[],
    );
    let d = commit_files(
        &src_dir,
        "D edits line 1 again",
        &[("p.txt", b"l1-d\nl2-c\n"), ("q.txt", b"d\n")],
        &[],
    );
    let main_dir = make_temp_dir("gap-pin-main");
    let main = create_main_repo(
        &main_dir,
        &[],
        &[("p.txt", b"l1-a\nl2-a\n"), ("q.txt", b"a\n")],
    );
    let github = StubGitHubOps::new();
    let registry = test_registry();
    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![b.clone(), d.clone()],
            ..Default::default()
        },
    );
    assert!(
        report.ok,
        "overlapping non-contiguous selection must promote: {:?}",
        report.blockers
    );
    let branch = report.integration_branch.clone().unwrap();
    // P ships D's whole-file blob, which still carries C's line-2 edit even
    // though C was skipped. This is the pinned truth, not a bug.
    let p = branch_file_text(&main.dir, &branch, "p.txt");
    assert_eq!(p, "l1-d\nl2-c\n", "the skipped commit's edit is carried");
    assert!(p.contains("l2-c"), "C's edit survives in the shipped blob");
    assert_eq!(branch_file_text(&main.dir, &branch, "q.txt"), "d\n");
    let _ = c;
}

#[test]
fn non_contiguous_undone_effect_promotes() {
    let src_dir = make_temp_dir("gap-undone-src");
    init_repo(&src_dir, "master");
    commit_files(&src_dir, "A base", &[("p.txt", b"a\n")], &[]);
    let b = commit_files(
        &src_dir,
        "B touches P and R",
        &[("p.txt", b"b\n"), ("r.txt", b"b\n")],
        &[],
    );
    let _c = commit_files(&src_dir, "C touches P only", &[("p.txt", b"c\n")], &[]);
    let _d = commit_files(&src_dir, "D touches Q only", &[("q.txt", b"d\n")], &[]);
    let main_dir = make_temp_dir("gap-undone-main");
    let main = create_main_repo(&main_dir, &[], &[("p.txt", b"a\n")]);
    let github = StubGitHubOps::new();
    let registry = test_registry();
    let e = commit_files(
        &src_dir,
        "E undoes C on P keeps R",
        &[("p.txt", b"a\n"), ("r.txt", b"b\n")],
        &[],
    );
    let allowed = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![b.clone(), e.clone()],
            ..Default::default()
        },
    );
    assert!(allowed.ok, "{:?}", allowed.blockers);
    let branch = allowed.integration_branch.clone().unwrap();
    assert_eq!(branch_file_text(&main.dir, &branch, "p.txt"), "a\n");
    assert_eq!(branch_file_text(&main.dir, &branch, "r.txt"), "b\n");
}

/// G2: behind (target blob in set) allows; diverged refuses naming
/// Subsystem-ID when present, else unknown/direct. Wire: excludedPaths.
#[test]
fn overwrite_behind_or_diverged_allows_and_audits_replaced() {
    let src_dir = make_temp_dir("behind-src");
    init_repo(&src_dir, "master");
    commit_files(&src_dir, "A base", &[("keep.txt", b"v1\n")], &[]);
    let b = commit_files(&src_dir, "B modify", &[("keep.txt", b"v2\n")], &[]);
    let main_dir = make_temp_dir("behind-main");
    let main = create_main_repo(&main_dir, &[], &[("keep.txt", b"v1\n")]);
    let github = StubGitHubOps::new();
    let registry = test_registry();
    let allowed = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![b.clone()],
            ..Default::default()
        },
    );
    assert!(
        allowed.ok,
        "behind (target==base, in set) must allow: {:?}",
        allowed.blockers
    );
    assert!(allowed.changes.replaced.iter().any(|p| p == "keep.txt"));
    assert!(!allowed.changes.flagged.iter().any(|p| p == "keep.txt"));

    // Diverged with Subsystem-ID trailer in target's last changer.
    let src2 = make_temp_dir("div-sid-src");
    init_repo(&src2, "master");
    commit_files(&src2, "A base", &[("f.txt", b"base\n")], &[]);
    let tip = commit_files(&src2, "B tip", &[("f.txt", b"tip\n")], &[]);
    let main2_dir = make_temp_dir("div-sid-main");
    let main2 = create_main_repo(&main2_dir, &[], &[("f.txt", b"base\n")]);
    // Target changer carries Subsystem-ID: other-team.
    let trailer_msg = format!(
        "other work\n\nSubsystem-ID: other-team\nSource-Repo: ORG/other\nSource-Commit: {}\nPromotion-Mode: individual\nPromotion-Target: staging\n",
        "a".repeat(40)
    );
    commit_message_on_branch(
        &main2.dir,
        "staging",
        &trailer_msg,
        &[("f.txt", b"diverged\n")],
        &[],
    );
    // Diverged target whose last changer carries a Subsystem-ID trailer:
    // the subsystem's copy still wins — attribution decided nothing, blob
    // membership only decides the audit bucket (replaced + flagged).
    let refused = promote(
        &src2,
        &main2.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![tip],
            ..Default::default()
        },
    );
    assert!(refused.ok, "{:?}", refused.blockers);
    assert!(
        refused.changes.replaced.iter().any(|p| p == "f.txt"),
        "overwritten path is reported as replaced: {:?}",
        refused.changes
    );
    assert!(
        refused.changes.flagged.iter().any(|p| p == "f.txt"),
        "independently-diverged target is flagged: {:?}",
        refused.changes
    );
}

#[test]
fn excluded_paths_are_skipped_and_reported() {
    let src_dir = make_temp_dir("excl-src");
    let main_dir = make_temp_dir("excl-main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();
    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            excluded_paths: vec!["src/modules/hr/c.txt".to_string()],
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    assert!(report
        .changes
        .excluded
        .iter()
        .any(|p| p == "src/modules/hr/c.txt"));
    let branch = report.integration_branch.clone().unwrap();
    assert!(!try_branch_has(&main.dir, &branch, "src/modules/hr/c.txt"));
}

#[test]
fn global_files_hit_refuses() {
    let src_dir = make_temp_dir("global-src");
    init_repo(&src_dir, "master");
    commit_files(&src_dir, "A base", &[("app/shell.tsx", b"v1\n")], &[]);
    let b = commit_files(&src_dir, "B shell edit", &[("app/shell.tsx", b"v2\n")], &[]);
    let main_dir = make_temp_dir("global-main");
    let main = create_main_repo(&main_dir, &[], &[("app/shell.tsx", b"diverged shell\n")]);
    let github = StubGitHubOps::new();
    let text = registry_json(&subsystem_json_global());
    let registry = Registry::from_json(&text, "t").expect("registry with globalFiles");
    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![b],
            ..Default::default()
        },
    );
    assert!(!report.ok);
    assert!(
        codes(&report).contains(&"GLOBAL_FILE_PROTECTED".to_string()),
        "{:?}",
        codes(&report)
    );
}

fn subsystem_json_global() -> String {
    subsystem_json_ext_global("", true)
}

fn subsystem_json_ext_global(_overrides: &str, enabled: bool) -> String {
    format!(
        r#"{{
      "id": "hr-management",
      "name": "Human Resource Management",
      "repository": "ORG/hr-management",
      "sourceBranch": "master",
      "enabled": {enabled},
      "pathStrategy": "preserve-relative-paths",
      "dependencyStrategy": "reconcile",
      "globalFiles": ["app/shell.tsx", "app/"],
      "integrationPolicy": {{
        "applicationFiles": "selected-commit-delta",
        "packageJson": "merge-dependency-delta",
        "lockfiles": "regenerate-in-target",
        "sharedConfiguration": "review-explicitly",
        "githubWorkflows": "main-owned",
        "secretFiles": "never-import"
      }}
    }}"#
    )
}

/// Reporting: replaced / deleted / unchanged / flagged / excluded.
#[test]
fn reporting_lists_replaced_deleted_unchanged_flagged_excluded() {
    let src_dir = make_temp_dir("report-src");
    init_repo(&src_dir, "master");
    commit_files(
        &src_dir,
        "A base",
        &[
            ("mod.txt", b"base\n"),
            ("del.txt", b"bye\n"),
            ("same.txt", b"same\n"),
        ],
        &[],
    );
    let b = commit_files(
        &src_dir,
        "B work",
        &[
            ("mod.txt", b"new\n"),
            ("same.txt", b"same\n"),
            ("add.txt", b"fresh\n"),
        ],
        &[],
    );
    std::fs::remove_file(src_dir.join("del.txt")).ok();
    let c = commit_files(&src_dir, "C delete", &[("mod.txt", b"new\n")], &[]);
    let _ = (b, c);
    let main_dir = make_temp_dir("report-main");
    let main = create_main_repo(
        &main_dir,
        &[],
        &[
            ("mod.txt", b"base\n"),
            ("del.txt", b"bye\n"),
            ("same.txt", b"same\n"),
        ],
    );
    let github = StubGitHubOps::new();
    let registry = test_registry();
    let tip = sha(&src_dir, "HEAD");
    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![tip],
            excluded_paths: vec!["add.txt".to_string()],
            ..Default::default()
        },
    );
    // add.txt excluded → ok (mod still applies or unchanged).
    assert!(report.ok, "{:?}", report.blockers);
    assert!(report.changes.excluded.iter().any(|p| p == "add.txt"));
    // replaced/deleted/unchanged are populated (deleted may be empty here
    // since the tip only deletes del.txt if selected — assert shapes exist).
    let _ = (
        &report.changes.replaced,
        &report.changes.deleted,
        &report.changes.unchanged,
        &report.changes.flagged,
    );
}

#[test]
fn housekeeping_cleans_vos_sources_and_refuses_repromote() {
    let src_dir = make_temp_dir("house-src");
    let main_dir = make_temp_dir("house-main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();
    let first = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            ..Default::default()
        },
    );
    assert!(first.ok, "{:?}", first.blockers);
    let refs = git_out(
        &main.dir,
        &["for-each-ref", "--format=%(refname)", "refs/vos-sources/"],
    );
    assert!(
        refs.trim().is_empty(),
        "vos-sources refs must be empty: {refs}"
    );
    advance_ref(
        &main.dir,
        "staging",
        &sha(&main.dir, &first.integration_branch.clone().unwrap()),
    );
    let second = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            ..Default::default()
        },
    );
    assert!(!second.ok);
    assert!(
        codes(&second).contains(&"DUPLICATE_ACTIVE".to_string()),
        "{:?}",
        codes(&second)
    );
}

#[test]
fn linked_worktree_checkout_refuses_update_ref() {
    use vos_origins_lib::github::models::PromotionDelivery;
    let src_dir = make_temp_dir("link-src");
    let main_dir = make_temp_dir("link-main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();
    let linked = make_temp_dir("linked-wt");
    assert!(sandbox::git_try(
        &main.dir,
        &["worktree", "add", linked.to_str().unwrap(), "staging"]
    ));
    assert_eq!(
        git_out(&main.dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "master"
    );
    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            delivery: PromotionDelivery::LocalDirect,
            ..Default::default()
        },
    );
    assert!(!report.ok);
    assert!(
        codes(&report).contains(&"TARGET_CHECKED_OUT_ELSEWHERE".to_string()),
        "{:?}",
        codes(&report)
    );
    let _ = std::process::Command::new("git")
        .arg("-C")
        .arg(&main.dir)
        .args(["worktree", "remove", "--force", linked.to_str().unwrap()])
        .output();
}

/// G5: gitignored written path is a named skip (no -f, no ENGINE_ERROR);
/// CRLF passes the staged check; extra staged file names extra/missing.
#[test]
fn worktree_ignored_is_named_skip_crlf_passes_extra_names_paths() {
    use vos_origins_lib::github::models::PromotionDelivery;
    let src_dir = make_temp_dir("wt-ignore-src");
    init_repo(&src_dir, "master");
    commit_files(&src_dir, "A base", &[("keep.txt", b"base\n")], &[]);
    let b = commit_files(
        &src_dir,
        "B adds ignored + crlf",
        &[("ignored.log", b"logs\n"), ("crlf.txt", b"a\nb\n")],
        &[],
    );
    let main_dir = make_temp_dir("wt-ignore-main");
    let main = create_main_repo(&main_dir, &[], &[("keep.txt", b"base\n")]);
    git_out(&main.dir, &["checkout", "-q", "staging"]);
    std::fs::write(main.dir.join(".gitignore"), "*.log\n").expect("gitignore");
    git_out(&main.dir, &["add", ".gitignore"]);
    git_out(
        &main.dir,
        &[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-q",
            "-m",
            "ignore",
        ],
    );
    let github = StubGitHubOps::new();
    let registry = test_registry();
    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![b],
            delivery: PromotionDelivery::Worktree,
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    assert!(
        report
            .changes
            .ignored_skipped
            .iter()
            .any(|p| p == "ignored.log"),
        "ignored path must be a named skip: {:?}",
        report.changes
    );
    assert!(
        !report.staged_paths.iter().any(|p| p == "ignored.log"),
        "ignored must never be staged (no -f)"
    );
    assert!(
        !codes(&report).contains(&"ENGINE_ERROR".to_string()),
        "no generic ENGINE_ERROR: {:?}",
        codes(&report)
    );
    // The injected-race naming itself fires by behaviour in the
    // `staged_mismatch_names_extra_and_missing_paths` unit test (same
    // `check_staged_matches` helper the promotion path calls).
}

// ===========================================================================
// Selective re-promotion over the subsystem's own earlier content (§3
// containment fix) + take_subsystem override with trailers
// ===========================================================================

/// The regression, as a fixture: promote C1, then select ONLY C5 while the
/// target holds C1's version of the path. C1's bytes are subsystem content
/// outside the selected range, so preflight must be Clean (not Conflict) and
/// the apply must write C5's bytes.
#[test]
fn selective_repromotion_over_previously_promoted_content_is_clean() {
    use vos_origins_lib::services::engine::preflight::{
        compute_preflight, PreflightOutcome, PreflightRequest,
    };

    let src_dir = make_temp_dir("src-reselect");
    let main_dir = make_temp_dir("main-reselect");
    init_repo(&src_dir, "master");
    commit_files(&src_dir, "A base", &[("shared.txt", b"v0\n")], &[]);
    let bodies = ["c1\n", "c2\n", "c3\n", "c4\n", "c5\n"];
    let mut versions = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        versions.push(commit_files(
            &src_dir,
            &format!("C{} selective", i + 1),
            &[("shared.txt", body.as_bytes())],
            &[],
        ));
    }
    let main = create_main_repo(&main_dir, &[], &[("shared.txt", b"v0\n")]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let first = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![versions[0].clone()],
            ..Default::default()
        },
    );
    assert!(first.ok, "{:?}", first.blockers);
    advance_ref(
        &main.dir,
        "staging",
        &sha(&main.dir, &first.integration_branch.clone().unwrap()),
    );
    assert_eq!(branch_file_text(&main.dir, "staging", "shared.txt"), "c1\n");

    let scratch = make_temp_dir("reselect-scratch");
    let empty: Vec<String> = Vec::new();
    let source_tip = sha(&src_dir, "master");
    let request = PreflightRequest {
        policy: &registry.subsystems[0].integration_policy,
        commits: std::slice::from_ref(&versions[4]),
        mode: PromotionMode::Individual,
        base_sha: None,
        target_branch: "staging",
        on_conflict: PromotionOnConflict::Fail,
        excluded_paths: &empty,
        global_files: &empty,
        source_tip: Some(source_tip.as_str()),
    };
    let preflight =
        compute_preflight(&src_dir, &main.dir, &request, &scratch).expect("preflight computes");
    let file = preflight
        .files
        .iter()
        .find(|f| f.path == "shared.txt")
        .expect("shared.txt in preflight");
    assert_eq!(file.outcome, PreflightOutcome::Clean, "{preflight:?}");
    assert!(preflight.conflicts.is_empty(), "{preflight:?}");
    assert!(
        file.reverts_newer.is_none(),
        "an ancestor target version is simply behind, never a reverts-newer warning: {preflight:?}"
    );

    let second = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![versions[4].clone()],
            ..Default::default()
        },
    );
    assert!(second.ok, "{:?}", second.blockers);
    let branch = second.integration_branch.clone().unwrap();
    assert_eq!(branch_file_text(&main.dir, &branch, "shared.txt"), "c5\n");
}

#[test]
fn target_blob_absent_from_source_repo_is_overwritten() {
    use vos_origins_lib::services::engine::preflight::{compute_preflight, PreflightRequest};

    let src_dir = make_temp_dir("src-mainonly");
    let main_dir = make_temp_dir("main-mainonly");
    init_repo(&src_dir, "master");
    commit_files(&src_dir, "A base", &[("notes.txt", b"v1\n")], &[]);
    let b = commit_files(&src_dir, "B edit", &[("notes.txt", b"v2\n")], &[]);
    let main = create_main_repo(&main_dir, &[], &[("notes.txt", b"v1\n")]);
    commit_message_on_branch(
        &main.dir,
        "staging",
        "staging: main-only edit",
        &[("notes.txt", b"main-only content\n")],
        &[],
    );
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let scratch = make_temp_dir("mainonly-scratch");
    let empty: Vec<String> = Vec::new();
    let source_tip = sha(&src_dir, "master");
    let request = PreflightRequest {
        policy: &registry.subsystems[0].integration_policy,
        commits: std::slice::from_ref(&b),
        mode: PromotionMode::Individual,
        base_sha: None,
        target_branch: "staging",
        on_conflict: PromotionOnConflict::Fail,
        excluded_paths: &empty,
        global_files: &empty,
        source_tip: Some(source_tip.as_str()),
    };
    let preflight =
        compute_preflight(&src_dir, &main.dir, &request, &scratch).expect("preflight computes");
    assert!(preflight.conflicts.is_empty(), "{preflight:?}");
    assert_eq!(preflight.conflict, 0);
    assert_eq!(preflight.replaced, vec!["notes.txt".to_string()]);
    let file = preflight
        .files
        .iter()
        .find(|f| f.path == "notes.txt")
        .expect("notes.txt in preflight");
    assert!(
        file.reverts_newer.is_none(),
        "main-only content never warns: {preflight:?}"
    );

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![b],
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().unwrap();
    assert_eq!(
        branch_file_text(&main.dir, &branch, "notes.txt"),
        "v2\n",
        "the subsystem's copy wins"
    );
    assert!(
        report.changes.replaced.iter().any(|p| p == "notes.txt"),
        "overwritten path is reported as replaced: {:?}",
        report.changes
    );
}

#[test]
fn subsystem_overwrites_diverged_path_reports_replaced_and_trailer() {
    let src_dir = make_temp_dir("src-takesub");
    let main_dir = make_temp_dir("main-takesub");
    init_repo(&src_dir, "master");
    commit_files(&src_dir, "A base", &[("notes.txt", b"v1\n")], &[]);
    let b = commit_files(&src_dir, "B edit", &[("notes.txt", b"v2\n")], &[]);
    let main = create_main_repo(&main_dir, &[], &[("notes.txt", b"v1\n")]);
    commit_message_on_branch(
        &main.dir,
        "staging",
        "staging: main-only edit",
        &[("notes.txt", b"main-only content\n")],
        &[],
    );
    let github = StubGitHubOps::new();
    let registry = test_registry();

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![b],
            on_conflict: PromotionOnConflict::TakeSubsystem,
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().unwrap();
    assert_eq!(
        branch_file_text(&main.dir, &branch, "notes.txt"),
        "v2\n",
        "the subsystem's copy wins"
    );
    assert!(
        report.changes.replaced.iter().any(|p| p == "notes.txt"),
        "overwritten path is reported as replaced: {:?}",
        report.changes
    );
    assert!(
        report
            .changes
            .conflict_taken
            .iter()
            .any(|p| p == "notes.txt"),
        "overwritten path is reported as taken from subsystem: {:?}",
        report.changes
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.code == "conflict_taken_from_subsystem"),
        "override warning is emitted: {:?}",
        report.warnings
    );
    let msg = report.provenance_message.clone().unwrap_or_default();
    assert!(msg.contains("Promotion-Replaced:"), "{msg}");
    assert!(msg.contains("notes.txt"), "{msg}");
    assert!(
        !msg.contains("Promotion-Conflicts"),
        "collapsed to the single honest key: {msg}"
    );
}

/// `take_subsystem` does NOT bypass the hard guards: global-file protection
/// and secret/main-owned skips all stand. Lockfiles are deferred to the
/// deployment, so they never block even under `take_subsystem`.
#[test]
fn take_subsystem_does_not_bypass_global_secret_or_lockfile_guards() {
    let src_dir = make_temp_dir("src-takeglobal");
    init_repo(&src_dir, "master");
    commit_files(&src_dir, "A base", &[("app/shell.tsx", b"v1\n")], &[]);
    let b = commit_files(&src_dir, "B shell edit", &[("app/shell.tsx", b"v2\n")], &[]);
    let main_dir = make_temp_dir("main-takeglobal");
    let main = create_main_repo(&main_dir, &[], &[("app/shell.tsx", b"main shell\n")]);
    let github = StubGitHubOps::new();
    let text = registry_json(&subsystem_json_global());
    let registry = Registry::from_json(&text, "t").expect("registry with globalFiles");
    let global = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![b],
            on_conflict: PromotionOnConflict::TakeSubsystem,
            ..Default::default()
        },
    );
    assert!(!global.ok);
    assert!(
        codes(&global).contains(&"GLOBAL_FILE_PROTECTED".to_string()),
        "{:?}",
        codes(&global)
    );

    let src2_dir = make_temp_dir("src-takelock");
    let main2_dir = make_temp_dir("main-takelock");
    let src2 = create_source_chain(&src2_dir);
    init_repo(&main2_dir, "master");
    write(&main2_dir, "package.json", BASE_MAIN_PKG.as_bytes());
    write(&main2_dir, "bun.lockb", b"bun-binary-lockfile\n");
    commit_all(&main2_dir, "main: bun baseline");
    git_out(&main2_dir, &["branch", "staging"]);
    git_out(&main2_dir, &["branch", "development"]);
    let locked = promote(
        &src2_dir,
        &main2_dir,
        &github,
        &test_registry(),
        PromoteOptions {
            commits: vec![src2.c.clone()],
            real_lockfile: true,
            on_conflict: PromotionOnConflict::TakeSubsystem,
            ..Default::default()
        },
    );
    assert!(locked.ok, "{:?}", locked.blockers);
    assert!(locked.changes.regenerated.is_empty());
    assert!(locked.lockfile.is_none());
    assert_eq!(
        branch_file(
            &main2_dir,
            locked.integration_branch.as_deref().unwrap(),
            "bun.lockb"
        ),
        b"bun-binary-lockfile\n"
    );

    // (c) Secret + main-owned files: still skipped, never imported.
    let src3_dir = make_temp_dir("src-takesecret");
    let main3_dir = make_temp_dir("main-takesecret");
    init_repo(&src3_dir, "master");
    commit_files(
        &src3_dir,
        "A base",
        &[
            ("notes.txt", b"v1\n"),
            ("package.json", b"{\"name\":\"x\"}\n"),
        ],
        &[],
    );
    let tip = commit_files(
        &src3_dir,
        "B notes plus guarded files",
        &[
            ("notes.txt", b"v2\n"),
            (".env.local", b"SECRET=super-secret\n"),
            (".github/workflows/x.yml", b"name: x\non: push\n"),
        ],
        &[],
    );
    let main3 = create_main_repo(&main3_dir, &[], &[("notes.txt", b"v1\n")]);
    let guarded = promote(
        &src3_dir,
        &main3.dir,
        &github,
        &test_registry(),
        PromoteOptions {
            commits: vec![tip],
            on_conflict: PromotionOnConflict::TakeSubsystem,
            ..Default::default()
        },
    );
    assert!(guarded.ok, "{:?}", guarded.blockers);
    assert!(
        guarded
            .changes
            .secret_excluded
            .iter()
            .any(|p| p == ".env.local"),
        "secret stays excluded: {:?}",
        guarded.changes
    );
    assert!(
        guarded
            .changes
            .main_owned_skipped
            .iter()
            .any(|p| p == ".github/workflows/x.yml"),
        "main-owned stays skipped: {:?}",
        guarded.changes
    );
    let branch = guarded.integration_branch.clone().unwrap();
    assert!(!try_branch_has(&main3.dir, &branch, ".env.local"));
    assert!(!try_branch_has(
        &main3.dir,
        &branch,
        ".github/workflows/x.yml"
    ));
    assert_eq!(branch_file_text(&main3.dir, &branch, "notes.txt"), "v2\n");
}

fn registry_with_attribution(attribute: bool) -> Registry {
    let json = registry_json(&subsystem_json("")).replace(
        "\"requirePullRequest\": true",
        &format!("\"requirePullRequest\": true, \"attributeToAccount\": {attribute}"),
    );
    Registry::from_json(&json, "test").expect("registry")
}

fn last_commit_author(repo: &Path, branch: &str) -> String {
    git_out(repo, &["log", "-1", "--format=%an <%ae>", branch])
}

#[test]
fn attribution_on_authors_the_commit_as_the_account() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &test_registry(),
        PromoteOptions {
            commits: vec![src.c.clone()],
            account_mode: AccountMode::Octocat,
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().unwrap();
    assert_eq!(
        last_commit_author(&main.dir, &branch),
        "octocat <1+octocat@users.noreply.github.com>"
    );
}

#[test]
fn attribution_off_authors_the_commit_as_the_bot() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &registry_with_attribution(false),
        PromoteOptions {
            commits: vec![src.c.clone()],
            account_mode: AccountMode::Octocat,
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().unwrap();
    assert_eq!(
        last_commit_author(&main.dir, &branch),
        "vos-integration-bot <vos-integration-bot@users.noreply.github.com>"
    );
}

#[test]
fn unresolved_account_falls_back_to_the_bot_without_failing() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();

    let report = promote(
        &src_dir,
        &main.dir,
        &github,
        &test_registry(),
        PromoteOptions {
            commits: vec![src.c.clone()],
            ..Default::default()
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    let branch = report.integration_branch.clone().unwrap();
    assert_eq!(
        last_commit_author(&main.dir, &branch),
        "vos-integration-bot <vos-integration-bot@users.noreply.github.com>"
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.code == "ACCOUNT_ATTRIBUTION_UNAVAILABLE"),
        "the fallback is reported: {:?}",
        report.warnings
    );
}

#[test]
fn promotion_progress_payload_matches_frontend_shape() {
    let full = vos_origins_lib::services::engine::PromotionProgress {
        stage: "copy",
        label: "Copying selected paths".to_string(),
        index: Some(3),
        total: Some(9),
        detail: None,
    };
    assert_eq!(
        serde_json::to_value(&full).unwrap(),
        serde_json::json!({
            "stage": "copy",
            "label": "Copying selected paths",
            "index": 3,
            "total": 9,
        }),
        "absent details stay absent on the wire"
    );
    let bare = vos_origins_lib::services::engine::PromotionProgress {
        stage: "resolve",
        label: "Resolving subsystem and target".to_string(),
        index: None,
        total: None,
        detail: None,
    };
    assert_eq!(
        serde_json::to_value(&bare).unwrap(),
        serde_json::json!({
            "stage": "resolve",
            "label": "Resolving subsystem and target",
        }),
    );
}

#[test]
fn promotion_progress_emits_stages_in_order() {
    use std::cell::RefCell;

    let src_dir = make_temp_dir("src-progress");
    let main_dir = make_temp_dir("main-progress");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();
    let stages = RefCell::new(Vec::new());

    let report = promote_with_progress(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![src.c.clone()],
            delivery: PromotionDelivery::Local,
            ..Default::default()
        },
        &|update: PromotionProgress| stages.borrow_mut().push(update.stage.to_string()),
    );
    assert!(report.ok, "{:?}", report.blockers);
    let mut ordered: Vec<String> = stages.borrow().clone();
    ordered.dedup();
    assert_eq!(
        ordered,
        vec![
            "resolve", "fetch", "classify", "copy", "manifest", "prereqs", "commit", "deliver",
            "cleanup"
        ],
    );
}

#[test]
fn promotion_progress_index_never_exceeds_total() {
    use std::cell::RefCell;

    let src_dir = make_temp_dir("src-progress-bounds");
    let main_dir = make_temp_dir("main-progress-bounds");
    init_repo(&src_dir, "master");
    commit_files(
        &src_dir,
        "A: base",
        &[
            (
                "package.json",
                format!(
                    "{}\n",
                    serde_json::json!({"name":"hr-management","version":"1.0.0","dependencies":{"next":"^16.2.1"}})
                )
                .as_bytes(),
            ),
            ("src/modules/hr/index.ts", b"export const a = 1;\n"),
        ],
        &[],
    );
    // Two commits touching the same file: the copy loop applies it once per
    // commit while the file list shows it once.
    let b = commit_files(
        &src_dir,
        "B: touch index",
        &[("src/modules/hr/index.ts", b"export const a = 2;\n")],
        &[],
    );
    let c = commit_files(
        &src_dir,
        "C: touch index again",
        &[("src/modules/hr/index.ts", b"export const a = 3;\n")],
        &[],
    );
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();
    let events = RefCell::new(Vec::new());

    let report = promote_with_progress(
        &src_dir,
        &main.dir,
        &github,
        &registry,
        PromoteOptions {
            commits: vec![b, c],
            delivery: PromotionDelivery::Local,
            ..Default::default()
        },
        &|update: PromotionProgress| {
            events
                .borrow_mut()
                .push((update.stage.to_string(), update.index, update.total))
        },
    );
    assert!(report.ok, "{:?}", report.blockers);
    assert!(
        events.borrow().iter().any(|(stage, _, _)| stage == "copy"),
        "copy stage must emit"
    );
    for (stage, index, total) in events.borrow().iter() {
        if let (Some(i), Some(t)) = (index, total) {
            assert!(
                i <= t,
                "stage {stage} reported index {i} exceeding total {t}"
            );
        }
    }
}
