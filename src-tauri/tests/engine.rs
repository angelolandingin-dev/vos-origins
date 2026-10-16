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

use subsystem_integration_manager_lib::github::models::PromotionMode;
use subsystem_integration_manager_lib::services::engine::apply::{apply_file_delta, ApplyConflict};
use subsystem_integration_manager_lib::services::engine::git::DeltaEntry;
use subsystem_integration_manager_lib::services::engine::github_ops::{
    CreatePrRequest, GitHubMode, GitHubOps, PullRequestInfo, PushOutcome, StubGitHubOps,
};
use subsystem_integration_manager_lib::services::engine::lockfile::{
    LockfileGenerator, RealLockfileGenerator, StubLockfileGenerator,
};
use subsystem_integration_manager_lib::services::engine::{
    run_promotion, EngineDeps, EngineOptions, EngineRequest, PromotionReport,
};
use subsystem_integration_manager_lib::services::registry::Registry;

// ===========================================================================
// Sandbox
// ===========================================================================

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

// ===========================================================================
// Fixtures
// ===========================================================================

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

// ===========================================================================
// Promotion driver
// ===========================================================================

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
    let request = EngineRequest {
        subsystem_id: options.subsystem_id,
        commits: options.commits,
        mode: options.mode,
        base_sha: options.base_sha,
        target_branch: options.target_branch,
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
        move |_entry: &subsystem_integration_manager_lib::services::registry::SubsystemEntry| {
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

    let deps = EngineDeps {
        github,
        lockfile,
        source_url: &source_url,
    };
    run_promotion(registry, &request, &engine_options, &deps)
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
    let parsed =
        subsystem_integration_manager_lib::services::engine::provenance::parse_trailers(&message);
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
fn case5b_non_ancestry_order_is_rejected() {
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
            commits: vec![src.c.clone(), src.b.clone()],
            ..Default::default()
        },
    );
    assert!(!report.ok);
    assert!(codes(&report).contains(&"COMMIT_VERIFICATION_FAILED".to_string()));
}

#[test]
fn case6_registry_policy_rejects_unregistered_disabled_and_bad_targets() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(&main_dir, &[], &[]);
    let github = StubGitHubOps::new();
    let registry = test_registry();

    // Unregistered subsystem.
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

    // Disabled subsystem.
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

    // Malformed registry JSON.
    assert!(Registry::from_json("{ broken json", "t").is_err());

    // Target branch not permitted by policy.
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

    // Unsafe target branch name.
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

    // Policy-allowed but nonexistent target.
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
fn case8_new_dependency_merges_delta_and_regenerates_lockfile() {
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
    assert_eq!(report.dependencies.added.len(), 1);
    assert_eq!(report.dependencies.added[0].name, "exceljs");
    assert_eq!(report.changes.regenerated, vec!["npm".to_string()]);
    assert!(report.lockfile.as_ref().map(|l| l.stubbed).unwrap_or(false));
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
fn case11_patch_conflict_leaves_the_target_untouched() {
    let src_dir = make_temp_dir("src");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    let main = create_main_repo(
        &main_dir,
        &[],
        &[("src/modules/hr/c.txt", b"divergent target content\n")],
    );
    let staging_tip = sha(&main.dir, "staging");
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
        codes(&report).contains(&"PATCH_CONFLICT".to_string()),
        "{:?}",
        codes(&report)
    );
    assert_eq!(
        sha(&main.dir, "staging"),
        staging_tip,
        "target tip must be unchanged"
    );

    for branch in git_out(&main.dir, &["branch", "--format=%(refname:short)"]).lines() {
        if branch.starts_with("integration/") {
            assert_eq!(
                sha(&main.dir, branch.trim()),
                staging_tip,
                "no integration commit was made"
            );
        }
    }
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
fn case12_scratch_files_are_never_committed() {
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
    assert!(
        report.changes.merged.iter().any(|p| p == "src/merge.txt"),
        "a real three-way merge should have happened: {:?}",
        report.changes
    );

    let commit = report.commit_sha.clone().unwrap();
    let names = git_out(&main.dir, &["show", "--name-only", "--format=", &commit]);
    let scratch: Vec<&str> = names
        .lines()
        .map(|l| l.trim())
        .filter(|l| l.ends_with(".ours") || l.ends_with(".base") || l.ends_with(".theirs"))
        .collect();
    assert!(
        scratch.is_empty(),
        "integration commit must not contain scratch files: {names:?}"
    );
}

#[test]
fn case13_phantom_source_commit_in_body_is_ignored() {
    use subsystem_integration_manager_lib::services::engine::history;

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
    use subsystem_integration_manager_lib::services::engine::github_ops::PullRequestInfo;
    use subsystem_integration_manager_lib::services::engine::{
        integration_branch_prefix, run_promotion,
    };

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

    // An open PR referencing the SHA must block a fresh dispatch.
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
    let source_url =
        move |_e: &subsystem_integration_manager_lib::services::registry::SubsystemEntry| {
            Some(source_dir.to_string_lossy().replace('\\', "/"))
        };
    let stub_lock = StubLockfileGenerator;
    let deps = EngineDeps {
        github: &open_pr_github,
        lockfile: &stub_lock,
        source_url: &source_url,
    };
    let blocked = run_promotion(&registry, &request, &engine_options, &deps);
    assert!(
        codes(&blocked).contains(&"PR_ALREADY_OPEN".to_string()),
        "{:?}",
        codes(&blocked)
    );
}

#[test]
fn case16_unsupported_lockfile_manager_fails_closed() {
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
    assert!(!report.ok);
    assert!(
        codes(&report).contains(&"LOCKFILE_MANAGER_UNSUPPORTED".to_string()),
        "{:?}",
        codes(&report)
    );
}

#[test]
fn case17_revert_then_repromotion_is_allowed() {
    use subsystem_integration_manager_lib::services::engine::history;
    use subsystem_integration_manager_lib::services::engine::provenance::build_revert_message;

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
    let parsed =
        subsystem_integration_manager_lib::services::engine::provenance::parse_trailers(&message);
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
    let conflict: ApplyConflict =
        apply_file_delta(&src_dir, &worktree, &c, Some(&a), &entry, &sandbox_root())
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
    ) -> Result<
        Vec<subsystem_integration_manager_lib::services::engine::github_ops::BranchInfo>,
        String,
    > {
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
    let source_url =
        move |_e: &subsystem_integration_manager_lib::services::registry::SubsystemEntry| {
            Some(source_dir.to_string_lossy().replace('\\', "/"))
        };
    let github = FailingPrOps;
    let lockfile = StubLockfileGenerator;
    let deps = EngineDeps {
        github: &github,
        lockfile: &lockfile,
        source_url: &source_url,
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
    use subsystem_integration_manager_lib::services::engine::history;

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
    // Divergent old path: rename handling would merge/rewrite it, copy
    // handling must leave it alone.
    write(&work, "src/old.txt", b"worktree divergent content\n");

    let entry = DeltaEntry {
        status: 'C',
        path: None,
        old_path: Some("src/old.txt".to_string()),
        new_path: Some("src/new.txt".to_string()),
        score: None,
    };
    let set = apply_file_delta(&src_dir, &work, &head, None, &entry, &sandbox_root())
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
fn oracle_watchout_failed_promotion_deletes_integration_ref() {
    let src_dir = make_temp_dir("src-watchout");
    let main_dir = make_temp_dir("main");
    let src = create_source_chain(&src_dir);
    // Divergent target content forces PATCH_CONFLICT after the temporary
    // integration branch has been created.
    let main = create_main_repo(
        &main_dir,
        &[],
        &[("src/modules/hr/c.txt", b"divergent target content\n")],
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
    assert!(!report.ok);
    assert!(
        codes(&report).contains(&"PATCH_CONFLICT".to_string()),
        "{:?}",
        codes(&report)
    );
    assert!(
        report.integration_branch.is_some(),
        "the branch existed before the bail"
    );
    let branches = git_out(&main.dir, &["branch", "--format=%(refname:short)"]);
    assert!(
        !branches.lines().any(|b| b.contains("integration/")),
        "failed promotion must not leave an integration/* ref: {branches}"
    );
}
