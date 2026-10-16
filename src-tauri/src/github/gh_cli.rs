//! `gh` detection, auth tokens, and engine prerequisites. Explicit argv only;
//! credentials stay in the CLI/OS store, never in state/logs.

use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::github::models::AuthStatus;
use crate::github::models::DataSource;
use crate::services::engine::lockfile;

pub const DEFAULT_HOST: &str = "github.com";

pub struct GhProbe {
    pub installed: bool,
    pub version: Option<String>,
    pub authenticated: bool,
    pub login: Option<String>,
    pub name: Option<String>,
    pub scopes: Vec<String>,
    pub token_source: Option<String>,
    pub token_available: bool,
    pub git_installed: bool,
    pub git_version: Option<String>,
    pub package_manager: Option<String>,
    pub package_manager_version: Option<String>,
    pub hostname: String,
    pub message: String,
    pub remedy: Option<String>,
}

fn run_gh(args: &[&str]) -> (bool, String) {
    match Command::new("gh").args(args).output() {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.trim().is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&stderr);
            }
            (output.status.success(), text)
        }
        Err(_) => (false, String::new()),
    }
}

fn extract_version(output: &str) -> Option<String> {
    output
        .lines()
        .find(|line| line.starts_with("gh version"))
        .and_then(|line| line.split_whitespace().nth(2))
        .map(str::to_string)
}

fn extract_after(output: &str, marker: &str) -> Option<String> {
    let index = output.find(marker)?;
    let rest = &output[index + marker.len()..];
    rest.split_whitespace().next().map(|token| {
        token
            .trim_matches(|c: char| c == '(' || c == ')' || c == ',')
            .to_string()
    })
}

fn extract_scopes(output: &str) -> Vec<String> {
    let Some(start) = output.find("Token scopes:") else {
        return Vec::new();
    };
    output[start + "Token scopes:".len()..]
        .lines()
        .next()
        .unwrap_or("")
        .split(',')
        .map(|scope| {
            scope
                .trim()
                .trim_matches('\'')
                .trim_matches('"')
                .to_string()
        })
        .filter(|scope| !scope.is_empty())
        .collect()
}

/// `gh auth token` output is cached for [`TOKEN_TTL`] so a burst of commands
/// (e.g. tab switches) spawns at most one `gh` process. `GH_TOKEN` /
/// `GITHUB_TOKEN` always win and bypass the cache.
const TOKEN_TTL: Duration = Duration::from_secs(300);

fn token_cache() -> &'static Mutex<(Option<String>, Option<Instant>)> {
    static CACHE: OnceLock<Mutex<(Option<String>, Option<Instant>)>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new((None, None)))
}

fn env_token() -> Option<String> {
    for key in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// `GH_TOKEN` / `GITHUB_TOKEN` first, then `gh auth token` (cached).
pub fn auth_token() -> Option<String> {
    auth_token_with_provider(&probe_gh_token)
}

fn probe_gh_token() -> Option<String> {
    let out = Command::new("gh")
        .args(["auth", "token", "--hostname", DEFAULT_HOST])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

/// Token resolution with an injectable `gh` seam (tests count provider
/// calls to prove the cache holds across commands).
pub fn auth_token_with_provider(provider: &dyn Fn() -> Option<String>) -> Option<String> {
    if let Some(token) = env_token() {
        return Some(token);
    }
    if let Ok(cached) = token_cache().lock() {
        if let (Some(token), Some(at)) = (cached.0.clone(), cached.1) {
            if at.elapsed() < TOKEN_TTL {
                return Some(token);
            }
        }
    }
    let fresh = provider()?;
    let fresh = fresh.trim().to_string();
    if fresh.is_empty() {
        return None;
    }
    if let Ok(mut cached) = token_cache().lock() {
        *cached = (Some(fresh.clone()), Some(Instant::now()));
    }
    Some(fresh)
}

#[cfg(test)]
pub fn clear_token_cache_for_tests() {
    if let Ok(mut cached) = token_cache().lock() {
        *cached = (None, None);
    }
}

pub fn git_version() -> Option<String> {
    let out = Command::new("git").arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Some(
        text.split_whitespace()
            .nth(2)
            .unwrap_or("unknown")
            .to_string(),
    )
}

pub fn package_manager() -> Option<(String, String)> {
    for program in ["npm", "pnpm", "yarn", "bun"] {
        if let Some(version) = lockfile::cli_available(program) {
            return Some((program.to_string(), version));
        }
    }
    None
}

fn environment_probe() -> (bool, Option<String>, Option<String>, Option<String>) {
    let git = git_version();
    let pm = package_manager();
    (
        git.is_some(),
        git,
        pm.as_ref().map(|(name, _)| name.clone()),
        pm.map(|(_, version)| version),
    )
}

pub fn probe() -> GhProbe {
    let (git_installed, git_version, package_manager, package_manager_version) =
        environment_probe();
    let base = |installed: bool,
                version: Option<String>,
                authenticated: bool,
                login: Option<String>,
                scopes: Vec<String>,
                token_source: Option<String>,
                message: String,
                remedy: Option<String>| {
        GhProbe {
            installed,
            version,
            authenticated,
            login,
            name: None,
            scopes,
            token_source,
            token_available: auth_token().is_some(),
            git_installed,
            git_version: git_version.clone(),
            package_manager: package_manager.clone(),
            package_manager_version: package_manager_version.clone(),
            hostname: DEFAULT_HOST.to_string(),
            message,
            remedy,
        }
    };

    let (version_ok, version_out) = run_gh(&["--version"]);
    if !version_ok {
        return base(
            false,
            None,
            false,
            None,
            Vec::new(),
            None,
            "GitHub CLI (gh) was not found on this machine.".to_string(),
            Some(
                "Install GitHub CLI from https://cli.github.com and run `gh auth login` in a terminal, then refresh. Live GitHub operations stay disabled until then.".to_string(),
            ),
        );
    }

    let version = extract_version(&version_out);
    let (auth_ok, auth_out) = run_gh(&["auth", "status", "--hostname", DEFAULT_HOST]);

    if !auth_ok {
        return base(
            true,
            version,
            false,
            None,
            Vec::new(),
            None,
            "GitHub CLI is installed but not authenticated for github.com.".to_string(),
            Some(
                "Run `gh auth login --hostname github.com` in a terminal, choose HTTPS, and grant at least the 'repo' scope so the tool can push branches and open integration PRs.".to_string(),
            ),
        );
    }

    let login = extract_after(&auth_out, "account ");
    let scopes = extract_scopes(&auth_out);
    let mut warnings: Vec<String> = Vec::new();
    if !scopes.is_empty() && !scopes.iter().any(|scope| scope == "repo") {
        warnings.push("the 'repo' scope is missing".to_string());
    }
    if !git_installed {
        warnings.push(
            "git was not found on PATH (required to build the integration branch)".to_string(),
        );
    }
    if package_manager.is_none() {
        warnings.push("no package manager (npm/pnpm/yarn) was found on PATH (required to regenerate a lockfile)".to_string());
    }

    let message = match warnings.is_empty() {
        true => format!(
            "Authenticated with GitHub CLI{}.",
            login
                .as_ref()
                .map(|l| format!(" as {l}"))
                .unwrap_or_default()
        ),
        false => format!("Authenticated, but {}", warnings.join(" and ")),
    };

    base(
        true,
        version,
        true,
        login,
        scopes,
        Some("gh auth login (secure OS credential store)".to_string()),
        message,
        match warnings.is_empty() {
            true => None,
            false => Some("Install the missing tools or run `gh auth refresh -h github.com -s repo` to add the scope.".to_string()),
        },
    )
}

/// Fail-closed gate for live dispatch.
pub fn require_prerequisites(probe: &GhProbe) -> AppResult<()> {
    if !probe.installed {
        return Err(AppError::new(
            ErrorCode::MissingPrerequisite,
            "GitHub CLI (gh) was not found on this machine.",
        )
        .with_remedy(
            "Install GitHub CLI from https://cli.github.com, run `gh auth login`, then retry.",
        ));
    }
    if !probe.authenticated {
        return Err(AppError::new(
            ErrorCode::Unauthenticated,
            "GitHub CLI is installed but not authenticated.",
        )
        .with_remedy(
            "Run `gh auth login --hostname github.com` in a terminal and grant the 'repo' scope.",
        ));
    }
    if !probe.token_available {
        return Err(AppError::new(
            ErrorCode::Unauthenticated,
            "No GitHub token is available (checked GH_TOKEN, GITHUB_TOKEN and `gh auth token`).",
        )
        .with_remedy("Set GH_TOKEN, or run `gh auth login` so `gh auth token` returns a token."));
    }
    if !probe.git_installed {
        return Err(AppError::new(
            ErrorCode::MissingPrerequisite,
            "git was not found on PATH; the engine builds the integration branch with git.",
        )
        .with_remedy(
            "Install Git (https://git-scm.com) and make sure `git --version` works in a terminal.",
        ));
    }
    if probe.package_manager.is_none() {
        return Err(AppError::new(
            ErrorCode::MissingPrerequisite,
            "No package manager (npm/pnpm/yarn) was found; the engine regenerates the target lockfile with it.",
        )
        .with_remedy("Install Node.js (which includes npm) and make sure `npm --version` works in a terminal."));
    }
    if !probe.scopes.is_empty() && !probe.scopes.iter().any(|scope| scope == "repo") {
        return Err(AppError::new(
            ErrorCode::InsufficientScope,
            "The GitHub login lacks the 'repo' scope required to push a branch and open a pull request.",
        )
        .with_remedy("Run `gh auth refresh -h github.com -s repo` and retry."));
    }
    Ok(())
}

pub fn auth_status(
    probe: GhProbe,
    data_source: DataSource,
    main_repository: &str,
    registry_branch: &str,
    workflow_file: &str,
    organization: &str,
    checked_at: String,
) -> AuthStatus {
    AuthStatus {
        gh_installed: probe.installed,
        gh_version: probe.version,
        authenticated: probe.authenticated,
        hostname: probe.hostname,
        account_login: probe.login,
        account_name: probe.name,
        organization: Some(organization.to_string()),
        scopes: probe.scopes,
        token_source: probe.token_source,
        token_available: probe.token_available,
        git_installed: probe.git_installed,
        git_version: probe.git_version,
        package_manager: probe.package_manager,
        package_manager_version: probe.package_manager_version,
        checked_at,
        message: probe.message,
        remedy: probe.remedy,
        data_source,
        main_repository: main_repository.to_string(),
        registry_branch: registry_branch.to_string(),
        workflow_file: workflow_file.to_string(),
    }
}

/// Allow-listed opener (spec §9.3).
pub fn url_is_allowlisted(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("").to_ascii_lowercase();
    host == "github.com"
        || host.ends_with(".github.com")
        || host == "cli.github.com"
        || host == "docs.github.com"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Env is process-global: save, scrub, run, restore in one test so no
    /// parallel test can observe a half-set state. Holds the crate-wide
    /// `crate::test_support::env_lock` for its whole lifetime so env-mutating
    /// tests in other modules (e.g. `checkouts`, `live_read`) cannot run
    /// concurrently with this one.
    struct EnvGuard {
        gh_token: Option<String>,
        github_token: Option<String>,
        _env: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn scrub() -> Self {
            let guard = Self {
                gh_token: std::env::var("GH_TOKEN").ok(),
                github_token: std::env::var("GITHUB_TOKEN").ok(),
                _env: crate::test_support::env_lock(),
            };
            std::env::remove_var("GH_TOKEN");
            std::env::remove_var("GITHUB_TOKEN");
            clear_token_cache_for_tests();
            guard
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.gh_token.as_deref() {
                Some(value) => std::env::set_var("GH_TOKEN", value),
                None => std::env::remove_var("GH_TOKEN"),
            }
            match self.github_token.as_deref() {
                Some(value) => std::env::set_var("GITHUB_TOKEN", value),
                None => std::env::remove_var("GITHUB_TOKEN"),
            }
            clear_token_cache_for_tests();
        }
    }

    #[test]
    fn token_provider_runs_once_and_env_wins() {
        let _env = EnvGuard::scrub();
        let calls = AtomicU64::new(0);
        let provider = || {
            calls.fetch_add(1, Ordering::Relaxed);
            Some("cached-token".to_string())
        };
        assert_eq!(
            auth_token_with_provider(&provider),
            Some("cached-token".to_string())
        );
        assert_eq!(
            auth_token_with_provider(&provider),
            Some("cached-token".to_string())
        );
        assert_eq!(
            auth_token_with_provider(&provider),
            Some("cached-token".to_string())
        );
        assert_eq!(
            calls.load(Ordering::Relaxed),
            1,
            "the `gh` seam runs once; later commands reuse the cached token"
        );

        // `GH_TOKEN` bypasses the cache and never touches the provider.
        std::env::set_var("GH_TOKEN", "env-token");
        assert_eq!(
            auth_token_with_provider(&provider),
            Some("env-token".to_string())
        );
        assert_eq!(
            calls.load(Ordering::Relaxed),
            1,
            "env tokens short-circuit before the provider"
        );
    }
}
