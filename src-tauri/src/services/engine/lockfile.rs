//! Lockfile regeneration (spec §6.4/§7): never ship a stale lockfile.
//! Unsupported managers / missing lockfiles report `skipped`; fail closed.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockfileOutcome {
    pub manager: Option<String>,
    pub command: Option<String>,
    pub ok: bool,
    pub skipped: bool,
    pub supported: bool,
    pub stubbed: bool,
    pub reason: Option<String>,
    pub output: Option<String>,
}

impl LockfileOutcome {
    fn skipped(manager: Option<&str>, supported: bool, reason: &str) -> Self {
        Self {
            manager: manager.map(str::to_string),
            command: None,
            ok: false,
            skipped: true,
            supported,
            stubbed: false,
            reason: Some(reason.to_string()),
            output: None,
        }
    }
}

pub trait LockfileGenerator: Send + Sync {
    fn generate(&self, worktree_dir: &Path) -> LockfileOutcome;
}

fn has(dir: &Path, name: &str) -> bool {
    dir.join(name).exists()
}

pub fn manager_for_dir(dir: &Path) -> Option<&'static str> {
    if has(dir, "pnpm-lock.yaml") {
        Some("pnpm")
    } else if has(dir, "yarn.lock") {
        Some("yarn")
    } else if has(dir, "package-lock.json") {
        Some("npm")
    } else if has(dir, "bun.lockb") || has(dir, "bun.lock") {
        Some("bun")
    } else {
        None
    }
}

/// Resolve a CLI name to a runnable path. Node CLIs are `.cmd` shims on Windows and
/// `Command::new("npm")` cannot find them (Command does not apply PATHEXT), so search PATH.
fn resolve_program(program: &str) -> PathBuf {
    #[cfg(windows)]
    {
        if Path::new(program).extension().is_none() {
            if let Some(paths) = std::env::var_os("PATH") {
                for dir in std::env::split_paths(&paths) {
                    for ext in [".exe", ".cmd", ".bat", ".com"] {
                        let candidate = dir.join(format!("{program}{ext}"));
                        if candidate.is_file() {
                            return candidate;
                        }
                    }
                }
            }
        }
    }
    PathBuf::from(program)
}

pub fn cli_available(program: &str) -> Option<String> {
    let output = crate::platform::hidden(resolve_program(program))
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text.lines().next().unwrap_or("").trim().to_string();
    Some(if version.is_empty() {
        "unknown".to_string()
    } else {
        version
    })
}

pub struct RealLockfileGenerator;

impl LockfileGenerator for RealLockfileGenerator {
    fn generate(&self, worktree_dir: &Path) -> LockfileOutcome {
        let (manager, program, args) = if has(worktree_dir, "pnpm-lock.yaml") {
            ("pnpm", "pnpm", vec!["install", "--lockfile-only"])
        } else if has(worktree_dir, "yarn.lock") {
            ("yarn", "yarn", vec!["install", "--mode", "update-lockfile"])
        } else if has(worktree_dir, "package-lock.json") {
            (
                "npm",
                "npm",
                vec![
                    "install",
                    "--package-lock-only",
                    "--ignore-scripts",
                    "--no-audit",
                    "--no-fund",
                ],
            )
        } else if has(worktree_dir, "bun.lockb") || has(worktree_dir, "bun.lock") {
            return LockfileOutcome::skipped(
                Some("bun"),
                false,
                "bun lockfiles are not supported by the engine (npm/pnpm/yarn only); a human must regenerate and update the lockfile",
            );
        } else {
            return LockfileOutcome::skipped(None, false, "no target lockfile present");
        };

        let command_line = format!("{program} {}", args.join(" "));
        let output = crate::platform::hidden(resolve_program(program))
            .args(&args)
            .current_dir(worktree_dir)
            .output();
        match output {
            Ok(out) => {
                let combined = format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
                let tail: String = {
                    let chars: Vec<char> = combined.chars().collect();
                    let start = chars.len().saturating_sub(4000);
                    chars[start..].iter().collect()
                };
                LockfileOutcome {
                    manager: Some(manager.to_string()),
                    command: Some(command_line),
                    ok: out.status.success(),
                    skipped: false,
                    supported: true,
                    stubbed: false,
                    reason: None,
                    output: Some(tail),
                }
            }
            Err(err) => LockfileOutcome {
                manager: Some(manager.to_string()),
                command: Some(command_line),
                ok: false,
                skipped: false,
                supported: true,
                stubbed: false,
                reason: Some(format!("{program} could not be started: {err}")),
                output: None,
            },
        }
    }
}

pub struct StubLockfileGenerator;

impl LockfileGenerator for StubLockfileGenerator {
    fn generate(&self, worktree_dir: &Path) -> LockfileOutcome {
        if !has(worktree_dir, "pnpm-lock.yaml")
            && !has(worktree_dir, "yarn.lock")
            && !has(worktree_dir, "package-lock.json")
        {
            if has(worktree_dir, "bun.lockb") || has(worktree_dir, "bun.lock") {
                let mut outcome = LockfileOutcome::skipped(
                    Some("bun"),
                    false,
                    "STUB: bun lockfile is unsupported by the engine",
                );
                outcome.stubbed = true;
                return outcome;
            }
            let mut outcome =
                LockfileOutcome::skipped(None, false, "STUB: no target lockfile present");
            outcome.stubbed = true;
            return outcome;
        }
        let manager = if has(worktree_dir, "pnpm-lock.yaml") {
            "pnpm"
        } else if has(worktree_dir, "yarn.lock") {
            "yarn"
        } else {
            "npm"
        };
        LockfileOutcome {
            manager: Some(manager.to_string()),
            command: None,
            ok: true,
            skipped: false,
            supported: true,
            stubbed: true,
            reason: Some("STUB: lockfile not regenerated (no network in local tests)".to_string()),
            output: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_detection() {
        let dir = std::env::temp_dir().join(format!("vos-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(manager_for_dir(&dir), None);
        std::fs::write(dir.join("package-lock.json"), "{}").unwrap();
        assert_eq!(manager_for_dir(&dir), Some("npm"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_program_finds_real_clis_and_passes_through_unknown_names() {
        #[cfg(windows)]
        {
            let resolved = resolve_program("cmd");
            assert!(
                resolved.is_file(),
                "expected `cmd` to resolve, got {resolved:?}"
            );
            // The reported bug: Node CLIs are `.cmd` shims. If Node is installed, the
            // resolver must find `npm.cmd` (not the extensionless shell script).
            let npm_cmd_on_path = std::env::var_os("PATH")
                .map(|paths| std::env::split_paths(&paths).any(|d| d.join("npm.cmd").is_file()))
                .unwrap_or(false);
            if npm_cmd_on_path {
                let npm = resolve_program("npm");
                assert!(
                    npm.is_file() && npm.to_string_lossy().to_lowercase().ends_with(".cmd"),
                    "npm.cmd is on PATH but resolve_program returned {npm:?}"
                );
            }
        }
        // A name that is not on PATH is returned unchanged.
        let missing = resolve_program("vos-no-such-cli-xyz");
        assert_eq!(missing, PathBuf::from("vos-no-such-cli-xyz"));
    }
}
