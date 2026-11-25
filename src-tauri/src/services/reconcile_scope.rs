//! Per-subsystem saved directory scopes for reconciliation (`%APPDATA%\vos-origins\reconcile-scopes.json`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::services::registry::assert_subsystem_id;

const CURRENT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScopeStore {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    scopes: BTreeMap<String, Vec<String>>,
}

impl Default for ScopeStore {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            scopes: BTreeMap::new(),
        }
    }
}

fn default_version() -> u32 {
    CURRENT_VERSION
}

pub fn default_scope_path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(PathBuf::from)?;
    Some(base.join("vos-origins").join("reconcile-scopes.json"))
}

pub fn normalize_folder(raw: &str) -> AppResult<String> {
    if raw.contains('\0') {
        return Err(
            AppError::invalid_request("Folder path contains a NUL byte.")
                .with_remedy("Use relative repository directory paths such as `src/modules/hrm`."),
        );
    }
    let replaced = raw.trim().replace('\\', "/");
    let mut trimmed = replaced.as_str();
    while let Some(stripped) = trimmed.strip_prefix("./") {
        trimmed = stripped;
    }
    trimmed = trimmed.trim_end_matches('/');

    if trimmed.is_empty() || trimmed == "." {
        return Err(
            AppError::invalid_request("Folder path must not be empty or `.`.").with_remedy(
                "Select a specific directory inside the repository instead of the repository root.",
            ),
        );
    }
    if trimmed.starts_with('/') {
        return Err(AppError::invalid_request(format!(
            "Folder path '{raw}' must be relative to the repository root."
        )));
    }
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(AppError::invalid_request(format!(
            "Folder path '{raw}' must not contain a drive letter."
        )));
    }
    let segments: Vec<&str> = trimmed.split('/').collect();
    if segments.contains(&"..") || segments.contains(&".") {
        return Err(AppError::invalid_request(format!(
            "Folder path '{raw}' contains traversal segments."
        )));
    }
    if segments.iter().any(|segment| segment.is_empty()) {
        return Err(AppError::invalid_request(format!(
            "Folder path '{raw}' contains empty path segments."
        )));
    }
    Ok(trimmed.to_string())
}

/// Normalize, deduplicate, sort lexicographically, and collapse any path that
/// is a strict descendant of an already-included ancestor directory.
pub fn normalize_folders(raw: &[String]) -> AppResult<Vec<String>> {
    let mut normalized: Vec<String> = Vec::with_capacity(raw.len());
    for entry in raw {
        normalized.push(normalize_folder(entry)?);
    }
    normalized.sort();
    normalized.dedup();

    let mut collapsed: Vec<String> = Vec::with_capacity(normalized.len());
    for candidate in normalized {
        let covered = collapsed
            .iter()
            .any(|ancestor| candidate.starts_with(&format!("{ancestor}/")));
        if !covered {
            collapsed.push(candidate);
        }
    }
    Ok(collapsed)
}

fn read_store(path: &Path) -> AppResult<ScopeStore> {
    if !path.exists() {
        return Ok(ScopeStore::default());
    }
    let text = std::fs::read_to_string(path).map_err(|err| {
        AppError::invalid_request(format!(
            "Could not read reconcile scopes at {}: {err}",
            path.display()
        ))
        .with_remedy("Check file permissions on reconcile-scopes.json or remove it to reset.")
    })?;
    if text.trim().is_empty() {
        return Ok(ScopeStore::default());
    }
    serde_json::from_str::<ScopeStore>(&text).map_err(|err| {
        AppError::invalid_request(format!(
            "Reconcile scopes file {} is invalid JSON: {err}",
            path.display()
        ))
        .with_remedy("Fix or delete reconcile-scopes.json, then save your folder selection again.")
    })
}

fn write_store_atomic(path: &Path, store: &ScopeStore) -> AppResult<()> {
    let parent = path.parent().ok_or_else(|| {
        AppError::invalid_request(format!("Invalid scope path: {}", path.display()))
    })?;
    std::fs::create_dir_all(parent).map_err(|err| {
        AppError::invalid_request(format!("Could not create {}: {err}", parent.display()))
    })?;

    let payload = serde_json::to_string_pretty(store)
        .map_err(|err| AppError::invalid_request(format!("Could not serialize scopes: {err}")))?;
    let tmp_name = format!(
        ".reconcile-scopes.{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let tmp_path = parent.join(tmp_name);
    std::fs::write(&tmp_path, format!("{payload}\n")).map_err(|err| {
        let _ = std::fs::remove_file(&tmp_path);
        AppError::invalid_request(format!("Could not write {}: {err}", tmp_path.display()))
    })?;
    if let Err(err) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(AppError::invalid_request(format!(
            "Could not replace {}: {err}",
            path.display()
        )));
    }
    Ok(())
}

pub fn load_scope(path: &Path, subsystem_id: &str) -> AppResult<Vec<String>> {
    assert_subsystem_id(subsystem_id).map_err(|err| AppError::invalid_request(err.message))?;
    let store = read_store(path)?;
    match store.scopes.get(subsystem_id) {
        Some(folders) => normalize_folders(folders),
        None => Ok(Vec::new()),
    }
}

pub fn save_scope(path: &Path, subsystem_id: &str, folders: &[String]) -> AppResult<Vec<String>> {
    assert_subsystem_id(subsystem_id).map_err(|err| AppError::invalid_request(err.message))?;
    let canonical = normalize_folders(folders)?;
    let mut store = read_store(path)?;
    store.version = CURRENT_VERSION;
    if canonical.is_empty() {
        store.scopes.remove(subsystem_id);
    } else {
        store
            .scopes
            .insert(subsystem_id.to_string(), canonical.clone());
    }
    write_store_atomic(path, &store)?;
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_scope_file(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vos-reconcile-scope-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        dir.join("reconcile-scopes.json")
    }

    #[test]
    fn missing_file_returns_empty_scope() {
        let path = temp_scope_file("missing");
        let loaded = load_scope(&path, "hrm").expect("missing returns empty");
        assert!(loaded.is_empty());
    }

    #[test]
    fn normalizes_dedups_and_collapses_descendants() {
        let normalized = normalize_folders(&[
            ".\\src\\modules\\hrm\\payroll\\".to_string(),
            "src/app/(hrm)/".to_string(),
            "src/modules/hrm".to_string(),
            "./src/app/(hrm)".to_string(),
            "src/modules/hrm-extra".to_string(),
        ])
        .expect("valid folders");

        assert_eq!(
            normalized,
            vec![
                "src/app/(hrm)".to_string(),
                "src/modules/hrm".to_string(),
                "src/modules/hrm-extra".to_string(),
            ]
        );
    }

    #[test]
    fn rejects_unsafe_or_root_folder_paths() {
        for bad in [
            "",
            ".",
            "./",
            "/",
            "/etc",
            "C:\\repo\\src",
            "src/../secret",
            "src//app",
            "src/\0bad",
        ] {
            assert!(
                normalize_folder(bad).is_err(),
                "expected '{bad}' to be rejected"
            );
        }
    }

    #[test]
    fn save_and_reload_round_trips_per_subsystem_and_removes_when_empty() {
        let path = temp_scope_file("roundtrip");
        let saved_hrm = save_scope(
            &path,
            "hrm",
            &[
                "src/app/api/hrm/".to_string(),
                "src/modules/human-resource-management".to_string(),
            ],
        )
        .expect("save hrm");
        assert_eq!(
            saved_hrm,
            vec![
                "src/app/api/hrm".to_string(),
                "src/modules/human-resource-management".to_string(),
            ]
        );

        save_scope(&path, "fm", &["src/app/api/fm".to_string()]).expect("save fm");
        assert_eq!(load_scope(&path, "hrm").unwrap(), saved_hrm);
        assert_eq!(
            load_scope(&path, "fm").unwrap(),
            vec!["src/app/api/fm".to_string()]
        );

        let cleared = save_scope(&path, "hrm", &[]).expect("clear hrm");
        assert!(cleared.is_empty());
        assert!(load_scope(&path, "hrm").unwrap().is_empty());
        assert_eq!(
            load_scope(&path, "fm").unwrap(),
            vec!["src/app/api/fm".to_string()]
        );

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
