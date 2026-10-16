//! Tool-local registry: allowlist + policy, validated fail closed (spec §1/§3).
//! Never written into a target repo; mirrors reference `lib/policy.mjs`.
//! Unknown keys/enums or unsafe names reject the whole document.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

pub const BUNDLED_REGISTRY: &str = include_str!("../../registry.json");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Registry {
    pub main_repository: String,
    pub promotion: PromotionPolicy,
    pub subsystems: Vec<SubsystemEntry>,
    /// Absolute path to the user's local clone of the main repository,
    /// used for `local` delivery. `None` ⇒ resolve via `VOS_TARGET_REPO_DIR`
    /// or the tool-managed checkout cache.
    #[serde(default)]
    pub local_main_repository_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromotionPolicy {
    pub allowed_target_branches: Vec<String>,
    #[serde(default)]
    pub allowed_target_branch_patterns: Vec<String>,
    /// Must stay `true`: the engine only ever opens a PR.
    pub require_pull_request: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubsystemEntry {
    pub id: String,
    pub name: String,
    pub repository: String,
    pub source_branch: String,
    pub enabled: bool,
    pub path_strategy: PathStrategy,
    pub dependency_strategy: DependencyStrategy,
    pub integration_policy: IntegrationPolicy,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PathStrategy {
    PreserveRelativePaths,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyStrategy {
    Reconcile,
    #[serde(rename = "none")]
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntegrationPolicy {
    pub application_files: ApplicationFiles,
    pub package_json: PackageJsonPolicy,
    pub lockfiles: LockfilePolicy,
    pub shared_configuration: SharedConfigPolicy,
    pub github_workflows: GithubWorkflowPolicy,
    pub secret_files: SecretFilePolicy,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ApplicationFiles {
    SelectedCommitDelta,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PackageJsonPolicy {
    MergeDependencyDelta,
    ExplicitReview,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LockfilePolicy {
    RegenerateInTarget,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SharedConfigPolicy {
    ReviewExplicitly,
    MergeControlled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum GithubWorkflowPolicy {
    MainOwned,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SecretFilePolicy {
    NeverImport,
}

impl Registry {
    pub fn from_json(text: &str, source_label: &str) -> AppResult<Self> {
        let registry: Registry = serde_json::from_str(text).map_err(|err| {
            AppError::invalid_registry(format!("{source_label} is not a valid registry document: {err}"))
                .with_remedy(
                    "Fix the registry JSON. Unknown keys and unknown enum values are rejected rather than ignored.",
                )
        })?;
        registry.validate().map_err(|err| {
            AppError::invalid_registry(format!("{source_label} failed validation: {}", err.message))
                .with_remedy(err.remedy.unwrap_or_else(|| {
                    "Correct the registry entry; the tool refuses to run with an unsafe policy."
                        .to_string()
                }))
        })?;
        Ok(registry)
    }

    pub fn load(override_path: Option<&Path>) -> AppResult<Self> {
        if let Some(path) = override_path {
            if path.exists() {
                let text = std::fs::read_to_string(path).map_err(|err| {
                    AppError::invalid_registry(format!("Registry override {} could not be read: {err}", path.display()))
                        .with_remedy("Fix the file permissions or remove the override to fall back to the bundled registry.")
                })?;
                return Self::from_json(&text, &path.display().to_string());
            }
        }
        Self::from_json(BUNDLED_REGISTRY, "bundled registry.json")
    }

    pub fn app_data_override_path() -> Option<PathBuf> {
        let base = std::env::var_os("APPDATA")
            .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
            .map(PathBuf::from)?;
        Some(base.join("vos-origin").join("registry.json"))
    }

    pub fn find(&self, subsystem_id: &str) -> Option<&SubsystemEntry> {
        self.subsystems
            .iter()
            .find(|entry| entry.id == subsystem_id)
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        require_repository_slug(&self.main_repository, "mainRepository")?;

        if let Some(path) = self.local_main_repository_path.as_deref() {
            if path.trim().is_empty() {
                return Err(ValidationError::new(
                    "localMainRepositoryPath must not be blank",
                    "Set localMainRepositoryPath to the absolute path of your local main-system clone, or remove it to use VOS_TARGET_REPO_DIR / the managed checkout.",
                ));
            }
            if !PathBuf::from(path).is_absolute() {
                return Err(ValidationError::new(
                    format!("localMainRepositoryPath must be absolute, got {path:?}"),
                    "Set localMainRepositoryPath to the absolute path of your local main-system clone, e.g. C:\\repos\\main-system.",
                ));
            }
        }

        if !self.promotion.require_pull_request {
            return Err(ValidationError::new(
                "promotion.requirePullRequest must be true; the engine never direct-pushes.",
                "Set promotion.requirePullRequest to true.",
            ));
        }
        for branch in &self.promotion.allowed_target_branches {
            assert_safe_branch_name(branch, "allowedTargetBranches[]")?;
        }
        for pattern in &self.promotion.allowed_target_branch_patterns {
            assert_safe_glob(pattern, "allowedTargetBranchPatterns[]")?;
        }

        let mut seen = std::collections::HashSet::new();
        for subsystem in &self.subsystems {
            assert_subsystem_id(&subsystem.id)?;
            require_repository_slug(&subsystem.repository, "subsystems[].repository")?;
            assert_safe_branch_name(&subsystem.source_branch, "subsystems[].sourceBranch")?;
            if subsystem.name.trim().is_empty() {
                return Err(ValidationError::new(
                    format!("subsystems[{}].name must not be empty", subsystem.id),
                    "Give the subsystem a human-readable name.",
                ));
            }
            if !seen.insert(subsystem.id.clone()) {
                return Err(ValidationError::new(
                    format!("duplicate subsystem id: {}", subsystem.id),
                    "Subsystem ids must be unique across the registry.",
                ));
            }
        }
        Ok(())
    }

    pub fn is_target_allowed(&self, branch: &str) -> TargetDecision {
        if self
            .promotion
            .allowed_target_branches
            .iter()
            .any(|b| b == branch)
        {
            return TargetDecision {
                allowed: true,
                reason: "listed".to_string(),
            };
        }
        for pattern in &self.promotion.allowed_target_branch_patterns {
            if match_glob(pattern, branch) {
                return TargetDecision {
                    allowed: true,
                    reason: format!("pattern:{pattern}"),
                };
            }
        }
        TargetDecision {
            allowed: false,
            reason: "not in allowedTargetBranches or allowedTargetBranchPatterns".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TargetDecision {
    pub allowed: bool,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct ValidationError {
    pub message: String,
    pub remedy: Option<String>,
}

impl ValidationError {
    fn new(message: impl Into<String>, remedy: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            remedy: Some(remedy.into()),
        }
    }

    fn bare(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            remedy: None,
        }
    }
}

/// Guards against injection / ambiguous refs.
pub fn assert_safe_branch_name(name: &str, label: &str) -> Result<(), ValidationError> {
    if name.is_empty() {
        return Err(ValidationError::new(
            format!("{label} must be a non-empty string"),
            "Provide a valid branch name.",
        ));
    }
    if name.len() > 255 {
        return Err(ValidationError::new(
            format!("{label} is too long"),
            "Shorten the branch name.",
        ));
    }
    let first = name.chars().next().unwrap_or(' ');
    let last = name.chars().last().unwrap_or(' ');
    if first == '-' || first == '/' || last == '/' || last == '.' {
        return Err(ValidationError::new(
            format!("{label} has an illegal form: {name}"),
            "Use a normal branch name such as release/2026-01.",
        ));
    }
    if name.contains("..") || name.contains("@{") || name.contains("//") {
        return Err(ValidationError::new(
            format!("{label} has an illegal form: {name}"),
            "Branch names may not contain '..', '@{' or '//'.",
        ));
    }
    if name.chars().any(is_branch_unsafe_char) {
        return Err(ValidationError::new(
            format!("{label} contains characters that are not permitted: {name}"),
            "Remove whitespace and the characters ~ ^ : ? * [ \\ from the branch name.",
        ));
    }
    Ok(())
}

fn is_branch_unsafe_char(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\n' | '\r' | '~' | '^' | ':' | '?' | '*' | '[' | '\\'
    ) || (c as u32) < 0x20
        || c as u32 == 0x7f
}

/// A target-branch glob must stay a *glob*: only `*`, `**`, `?` and safe literal chars.
pub fn assert_safe_glob(pattern: &str, label: &str) -> Result<(), ValidationError> {
    if pattern.is_empty() {
        return Err(ValidationError::new(
            format!("{label} must be non-empty"),
            "Remove the empty pattern.",
        ));
    }
    for c in pattern.chars() {
        let ok = c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.' | '*' | '?');
        if !ok {
            return Err(ValidationError::new(
                format!("{label} contains an unsupported character '{c}': {pattern}"),
                "Allowed glob characters are letters, digits, / - _ . * and ?.",
            ));
        }
    }
    Ok(())
}

/// Require a full 40/64-char lowercase hex SHA.
pub fn assert_full_sha(sha: &str, label: &str) -> Result<(), ValidationError> {
    let ok_len = sha.len() == 40 || sha.len() == 64;
    let ok_chars = sha
        .chars()
        .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
    if !ok_len || !ok_chars {
        return Err(ValidationError::new(
            format!("{label} must be a full 40/64-char lowercase hex SHA, got {sha:?}"),
            "Use full SHAs. Abbreviated SHAs are display-only and cannot be promoted.",
        ));
    }
    Ok(())
}

pub fn assert_subsystem_id(id: &str) -> Result<(), ValidationError> {
    let bytes = id.as_bytes();
    if bytes.len() < 2 || bytes.len() > 64 {
        return Err(ValidationError::new(
            format!("Invalid subsystem id shape: {id:?}"),
            "Subsystem ids are 2-64 characters: lowercase letters, digits and dashes, starting alphanumeric.",
        ));
    }
    let first = bytes[0];
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err(ValidationError::bare(format!(
            "Invalid subsystem id shape: {id:?}"
        )));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(ValidationError::bare(format!(
            "Invalid subsystem id shape: {id:?}"
        )));
    }
    Ok(())
}

pub fn require_repository_slug(value: &str, label: &str) -> Result<(), ValidationError> {
    let mut parts = value.split('/');
    let owner = parts.next().unwrap_or("");
    let repo = parts.next().unwrap_or("");
    let extra = parts.next().is_some();
    let ok_part = |p: &str| {
        !p.is_empty()
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
    };
    if !ok_part(owner) || !ok_part(repo) || extra {
        return Err(ValidationError::new(
            format!("{label} must be an OWNER/REPO slug, got {value:?}"),
            "Use the GitHub OWNER/REPO form, for example ORG/main-system.",
        ));
    }
    Ok(())
}

/// Never compiled to a regex, so metacharacters match literally.
pub fn match_glob(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    glob_here(&p, &n)
}

fn glob_here(p: &[char], n: &[char]) -> bool {
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star_p, mut star_n): (Option<usize>, usize) = (None, 0);
    let mut star_crosses_segments = false;

    while ni < n.len() {
        if pi < p.len() {
            match p[pi] {
                '*' => {
                    let double = pi + 1 < p.len() && p[pi + 1] == '*';
                    star_p = Some(pi);
                    star_n = ni;
                    star_crosses_segments = double;
                    pi += if double { 2 } else { 1 };
                    continue;
                }
                '?' => {
                    if n[ni] != '/' {
                        pi += 1;
                        ni += 1;
                        continue;
                    }
                }
                c => {
                    if c == n[ni] {
                        pi += 1;
                        ni += 1;
                        continue;
                    }
                }
            }
        }
        match star_p {
            Some(sp) => {
                if !star_crosses_segments && n[star_n] == '/' {
                    return false;
                }
                star_n += 1;
                ni = star_n;
                pi = sp + if star_crosses_segments { 2 } else { 1 };
            }
            None => return false,
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_registry_is_valid() {
        let registry =
            Registry::from_json(BUNDLED_REGISTRY, "bundled").expect("bundled registry validates");
        assert_eq!(registry.main_repository, "ORG/main-system");
        assert!(registry.find("hr-management").is_some());
    }

    #[test]
    fn unknown_key_fails_closed() {
        let text = r#"{"mainRepository":"ORG/x","promotion":{"allowedTargetBranches":["master"],"requirePullRequest":true},"subsystems":[],"surprise":1}"#;
        assert!(Registry::from_json(text, "t").is_err());
    }

    #[test]
    fn require_pull_request_false_is_rejected() {
        let text = r#"{"mainRepository":"ORG/x","promotion":{"allowedTargetBranches":["master"],"requirePullRequest":false},"subsystems":[]}"#;
        let err = Registry::from_json(text, "t").unwrap_err();
        assert!(
            err.message.contains("requirePullRequest"),
            "{}",
            err.message
        );
    }

    #[test]
    fn duplicate_subsystem_id_is_rejected() {
        let text = r#"{"mainRepository":"ORG/x","promotion":{"allowedTargetBranches":["master"],"requirePullRequest":true},"subsystems":[
            {"id":"hr","name":"HR","repository":"ORG/a","sourceBranch":"master","enabled":true,"pathStrategy":"preserve-relative-paths","dependencyStrategy":"reconcile","integrationPolicy":{"applicationFiles":"selected-commit-delta","packageJson":"merge-dependency-delta","lockfiles":"regenerate-in-target","sharedConfiguration":"review-explicitly","githubWorkflows":"main-owned","secretFiles":"never-import"}},
            {"id":"hr","name":"HR2","repository":"ORG/b","sourceBranch":"master","enabled":true,"pathStrategy":"preserve-relative-paths","dependencyStrategy":"reconcile","integrationPolicy":{"applicationFiles":"selected-commit-delta","packageJson":"merge-dependency-delta","lockfiles":"regenerate-in-target","sharedConfiguration":"review-explicitly","githubWorkflows":"main-owned","secretFiles":"never-import"}}
        ]}"#;
        assert!(Registry::from_json(text, "t").is_err());
    }

    #[test]
    fn unknown_enum_value_fails_closed() {
        let text = r#"{"mainRepository":"ORG/x","promotion":{"allowedTargetBranches":["master"],"requirePullRequest":true},"subsystems":[
            {"id":"hr","name":"HR","repository":"ORG/a","sourceBranch":"master","enabled":true,"pathStrategy":"preserve-relative-paths","dependencyStrategy":"bogus","integrationPolicy":{"applicationFiles":"selected-commit-delta","packageJson":"merge-dependency-delta","lockfiles":"regenerate-in-target","sharedConfiguration":"review-explicitly","githubWorkflows":"main-owned","secretFiles":"never-import"}}
        ]}"#;
        assert!(Registry::from_json(text, "t").is_err());
    }

    #[test]
    fn unsafe_source_branch_is_rejected() {
        for bad in ["evil branch", "a..b", "-x", "x\ty"] {
            assert!(
                assert_safe_branch_name(bad, "sourceBranch").is_err(),
                "must reject {bad}"
            );
        }
        assert!(assert_safe_branch_name("release/2026-01", "b").is_ok());
    }

    #[test]
    fn full_sha_shape() {
        let valid = "a".repeat(40);
        assert!(assert_full_sha(&valid, "sha").is_ok());
        // SHA-256 is accepted exactly like SHA-1 (the engine and the Tauri
        // promotion gate share this check).
        assert!(assert_full_sha(&"b".repeat(64), "sha").is_ok());
        assert!(assert_full_sha(&valid[..7], "sha").is_err());
        assert!(assert_full_sha(&valid.to_uppercase(), "sha").is_err());
    }

    #[test]
    fn glob_semantics_are_not_regex() {
        assert!(match_glob("release/*", "release/1.0"));
        assert!(!match_glob("release/*", "release/1/0"));
        assert!(!match_glob("release/*", "development"));
        assert!(match_glob("release/**", "release/a/b"));
        assert!(match_glob("release/v?.0", "release/v1.0"));
        assert!(!match_glob("a.b", "axb"));
        assert!(match_glob("a.b", "a.b"));
    }

    #[test]
    fn target_allowance() {
        let registry = Registry::from_json(BUNDLED_REGISTRY, "b").unwrap();
        assert!(registry.is_target_allowed("staging").allowed);
        assert!(registry.is_target_allowed("release/2026-01").allowed);
        assert!(!registry.is_target_allowed("feature/x").allowed);
    }
}
