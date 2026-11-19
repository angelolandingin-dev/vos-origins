//! Path classification (spec §6.3/§6.4): runs before any write,
//! so unsafe/main-owned paths fail in pre-flight, never half-applied.

use crate::services::registry::{IntegrationPolicy, PackageJsonPolicy};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PathStatus {
    Apply,
    DependencyMerge,
    Regenerate,
    LockfileDeferred,
    MainOwned,
    SecretExcluded,
    ExplicitReview,
    Rejected,
}

impl PathStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            PathStatus::Apply => "apply",
            PathStatus::DependencyMerge => "dependencyMerge",
            PathStatus::Regenerate => "regenerate",
            PathStatus::LockfileDeferred => "lockfileDeferred",
            PathStatus::MainOwned => "mainOwned",
            PathStatus::SecretExcluded => "secretExcluded",
            PathStatus::ExplicitReview => "explicitReview",
            PathStatus::Rejected => "rejected",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Classification {
    pub path: String,
    pub status: PathStatus,
    pub reason: String,
}

pub fn inspect_path_safety(raw_path: &str) -> Result<String, String> {
    if raw_path.is_empty() {
        return Err("empty path".to_string());
    }
    if raw_path.contains('\0') {
        return Err("path contains NUL byte".to_string());
    }
    let mut p = raw_path.replace('\\', "/");
    if p.starts_with('/') {
        return Err("absolute path".to_string());
    }
    let bytes = p.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err("drive-letter path".to_string());
    }
    let segments: Vec<&str> = p.split('/').collect();
    if segments.contains(&"..") {
        return Err("path traversal (\"..\")".to_string());
    }
    if segments.iter().any(|s| s.is_empty()) {
        return Err("empty path segment".to_string());
    }
    while let Some(stripped) = p.strip_prefix("./") {
        p = stripped.to_string();
    }
    if p == "." || p.is_empty() {
        return Err("empty normalized path".to_string());
    }
    Ok(p)
}

pub fn classify_path(
    raw_path: &str,
    policy: &IntegrationPolicy,
    mode: Option<&str>,
) -> Classification {
    let file_path = match inspect_path_safety(raw_path) {
        Ok(path) => path,
        Err(reason) => {
            return Classification {
                path: raw_path.to_string(),
                status: PathStatus::Rejected,
                reason,
            };
        }
    };

    if mode == Some("120000") {
        return Classification {
            path: file_path,
            status: PathStatus::Rejected,
            reason: "symlink entry is not accepted".to_string(),
        };
    }
    if mode == Some("160000") {
        return Classification {
            path: file_path,
            status: PathStatus::Rejected,
            reason: "git submodule/gitlink entry is not accepted".to_string(),
        };
    }

    let basename = file_path
        .rsplit('/')
        .next()
        .unwrap_or(&file_path)
        .to_string();
    let lower = basename.to_ascii_lowercase();

    if is_lockfile(&lower) {
        return Classification {
            path: file_path,
            status: PathStatus::LockfileDeferred,
            reason: "lockfile is deferred to the deployment, which regenerates it; never copied"
                .to_string(),
        };
    }
    if file_path == "package.json" {
        if policy.package_json == PackageJsonPolicy::ExplicitReview {
            return Classification {
                path: file_path,
                status: PathStatus::ExplicitReview,
                reason: "registry policy requires explicit review of package.json".to_string(),
            };
        }
        return Classification {
            path: file_path,
            status: PathStatus::DependencyMerge,
            reason: "only the dependency delta is merged".to_string(),
        };
    }
    if file_path.ends_with("/package.json") {
        return Classification {
            path: file_path,
            status: PathStatus::ExplicitReview,
            reason: "nested/workspace manifest requires explicit review (only the root manifest is reconciled)".to_string(),
        };
    }
    if file_path.starts_with(".github/workflows/")
        || file_path.starts_with(".github/actions/")
        || file_path == ".github/CODEOWNERS"
        || file_path == ".github/dependabot.yml"
    {
        return Classification {
            path: file_path,
            status: PathStatus::MainOwned,
            reason: "main-owned: never automatically overwritten".to_string(),
        };
    }
    if is_secret(&lower) {
        return Classification {
            path: file_path,
            status: PathStatus::SecretExcluded,
            reason: "secret-looking file is never imported automatically".to_string(),
        };
    }
    if is_shared_config(&lower) {
        return Classification {
            path: file_path,
            status: PathStatus::ExplicitReview,
            reason: "shared/global config requires controlled merge or explicit review".to_string(),
        };
    }
    Classification {
        path: file_path,
        status: PathStatus::Apply,
        reason: "subsystem-owned application file".to_string(),
    }
}

fn is_lockfile(lower_basename: &str) -> bool {
    matches!(
        lower_basename,
        "package-lock.json"
            | "npm-shrinkwrap.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "bun.lockb"
            | "bun.lock"
    )
}

fn is_lettered_config(base: &str, stem: &str) -> bool {
    match base.strip_prefix(stem) {
        Some(rest) => match rest.strip_prefix('.') {
            Some(ext) => !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphabetic()),
            None => false,
        },
        None => false,
    }
}

fn is_prefixed_json(base: &str, prefix: &str) -> bool {
    let exact = format!("{prefix}.json");
    if base == exact {
        return true;
    }
    if let Some(rest) = base.strip_prefix(prefix) {
        if let Some(rest) = rest.strip_prefix('.') {
            if let Some(mid) = rest.strip_suffix(".json") {
                return !mid.is_empty()
                    && mid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
            }
        }
    }
    false
}

fn is_rc_like(base: &str, name: &str) -> bool {
    if base == name {
        return true;
    }
    if let Some(rest) = base.strip_prefix(name) {
        if let Some(rest) = rest.strip_prefix('.') {
            return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        }
    }
    false
}

fn is_shared_config(lower_basename: &str) -> bool {
    if is_prefixed_json(lower_basename, "tsconfig") || is_prefixed_json(lower_basename, "jsconfig")
    {
        return true;
    }
    if lower_basename == "components.json"
        || lower_basename == ".editorconfig"
        || lower_basename == ".nvmrc"
        || lower_basename == ".node-version"
    {
        return true;
    }
    for stem in [
        "next.config",
        "postcss.config",
        "eslint.config",
        "tailwind.config",
        "vite.config",
        "webpack.config",
        "jest.config",
        "vitest.config",
        "babel.config",
        "prettier.config",
    ] {
        if is_lettered_config(lower_basename, stem) {
            return true;
        }
    }
    for rc in [".eslintrc", ".babelrc", ".prettierrc"] {
        if is_rc_like(lower_basename, rc) {
            return true;
        }
    }
    false
}

fn has_delimited_word(base: &str, word: &str) -> bool {
    let is_delim = |c: char| c == '.' || c == '_' || c == '-';
    let chars: Vec<char> = base.chars().collect();
    let needle: Vec<char> = word.chars().collect();
    if needle.is_empty() || chars.len() < needle.len() {
        return false;
    }
    for start in 0..=(chars.len() - needle.len()) {
        if chars[start..start + needle.len()] == needle[..] {
            let before_ok = start == 0 || is_delim(chars[start - 1]);
            let after = start + needle.len();
            let after_ok = after == chars.len() || is_delim(chars[after]);
            if before_ok && after_ok {
                return true;
            }
        }
    }
    false
}

fn is_secret(lower_basename: &str) -> bool {
    let ext = match lower_basename.rsplit_once('.') {
        Some((_, ext)) => format!(".{ext}"),
        None => String::new(),
    };
    if matches!(
        ext.as_str(),
        ".pem" | ".p12" | ".pfx" | ".key" | ".keystore" | ".jks" | ".asc"
    ) {
        return true;
    }
    if lower_basename.starts_with(".env") {
        return true;
    }
    if matches!(
        lower_basename,
        ".npmrc" | ".pypirc" | ".netrc" | ".htpasswd"
    ) {
        return true;
    }
    for id in ["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519"] {
        if lower_basename == id || lower_basename == format!("{id}.pub") {
            return true;
        }
    }
    for word in [
        "secret",
        "secrets",
        "credential",
        "credentials",
        "token",
        "tokens",
    ] {
        if has_delimited_word(lower_basename, word) {
            return true;
        }
    }
    let stem = lower_basename.strip_suffix(".json").unwrap_or("");
    !stem.is_empty() && stem.starts_with("service") && stem.contains("account")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_policy() -> IntegrationPolicy {
        crate::services::registry::Registry::from_json(
            crate::services::registry::BUNDLED_REGISTRY,
            "b",
        )
        .unwrap()
        .subsystems[0]
            .integration_policy
            .clone()
    }

    #[test]
    fn policy_classification_matches_reference() {
        let policy = default_policy();
        let status = |p: &str| classify_path(p, &policy, None).status;
        assert_eq!(status("src/modules/hr/index.ts"), PathStatus::Apply);
        assert_eq!(status("public/logo.png"), PathStatus::Apply);
        assert_eq!(status("package.json"), PathStatus::DependencyMerge);
        assert_eq!(status("package-lock.json"), PathStatus::LockfileDeferred);
        assert_eq!(status("pnpm-lock.yaml"), PathStatus::LockfileDeferred);
        assert_eq!(status(".github/workflows/ci.yml"), PathStatus::MainOwned);
        assert_eq!(status(".github/CODEOWNERS"), PathStatus::MainOwned);
        assert_eq!(status(".env.local"), PathStatus::SecretExcluded);
        assert_eq!(
            status("config/credentials.json"),
            PathStatus::SecretExcluded
        );
        assert_eq!(status("certs/server.pem"), PathStatus::SecretExcluded);
        assert_eq!(status("tsconfig.json"), PathStatus::ExplicitReview);
        assert_eq!(status("next.config.ts"), PathStatus::ExplicitReview);
        assert_eq!(
            status("packages/app/package.json"),
            PathStatus::ExplicitReview
        );
    }

    #[test]
    fn unsafe_paths_and_special_modes_are_rejected() {
        let policy = default_policy();
        let status = |p: &str, m: Option<&str>| classify_path(p, &policy, m).status;
        assert_eq!(status("../evil.txt", None), PathStatus::Rejected);
        assert_eq!(status("/etc/passwd", None), PathStatus::Rejected);
        assert_eq!(status("C:/windows/system32", None), PathStatus::Rejected);
        assert_eq!(status("a/../../b", None), PathStatus::Rejected);
        assert_eq!(status("link", Some("120000")), PathStatus::Rejected);
        assert_eq!(status("submodule", Some("160000")), PathStatus::Rejected);
    }

    #[test]
    fn backslashes_are_normalized() {
        assert_eq!(
            inspect_path_safety("src\\modules\\hr\\a.ts").unwrap(),
            "src/modules/hr/a.ts"
        );
    }

    #[test]
    fn lockfile_deferred_serializes_camel_case() {
        assert_eq!(PathStatus::LockfileDeferred.as_str(), "lockfileDeferred");
        assert_eq!(
            serde_json::to_value(PathStatus::LockfileDeferred).unwrap(),
            serde_json::json!("lockfileDeferred")
        );
        let round: PathStatus =
            serde_json::from_value(serde_json::json!("lockfileDeferred")).unwrap();
        assert_eq!(round, PathStatus::LockfileDeferred);
    }
}
