//! Dependency reconciliation (spec §7): merge only the delta, fail closed.
//! Flags imports added by no selected commit (unselected prerequisite).

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::semver::{compare_ranges, ComparisonKind};
use super::Blocker;

pub const DEP_SECTIONS: [&str; 4] = [
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
];

const MAIN_OWNED_MANIFEST_KEYS: [&str; 7] = [
    "scripts",
    "workspaces",
    "engines",
    "packageManager",
    "overrides",
    "resolutions",
    "peerDependenciesMeta",
];

const IGNORED_MANIFEST_KEYS: [&str; 19] = [
    "name",
    "version",
    "description",
    "author",
    "license",
    "private",
    "type",
    "main",
    "module",
    "exports",
    "types",
    "publishConfig",
    "files",
    "repository",
    "bugs",
    "homepage",
    "keywords",
    "packageManager",
    "engines",
];

const CODE_EXTENSIONS: [&str; 10] = [
    ".js", ".jsx", ".ts", ".tsx", ".mjs", ".cjs", ".mts", ".cts", ".vue", ".svelte",
];

const NODE_BUILTINS: [&str; 45] = [
    "assert",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "constants",
    "crypto",
    "dgram",
    "diagnostics_channel",
    "dns",
    "domain",
    "events",
    "fs",
    "http",
    "http2",
    "https",
    "inspector",
    "module",
    "net",
    "os",
    "path",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "repl",
    "stream",
    "string_decoder",
    "sys",
    "test",
    "timers",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
    "assert/strict",
    "fs/promises",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepAdd {
    pub section: String,
    pub name: String,
    pub range: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepUpdate {
    pub section: String,
    pub name: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepRemove {
    pub section: String,
    pub name: String,
    pub range: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepMove {
    pub name: String,
    pub from_section: String,
    pub to_section: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManifestChanges {
    pub main_owned: Vec<String>,
    pub ignored: Vec<String>,
    pub other: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DependencyDelta {
    pub added: Vec<DepAdd>,
    pub updated: Vec<DepUpdate>,
    pub removed: Vec<DepRemove>,
    pub moved: Vec<DepMove>,
    pub manifest_changes: ManifestChanges,
    /// Non-string dependency values (or other unmergeable shapes) found while
    /// diffing; surfaced fail-closed by `reconcile_dependencies`.
    pub blockers: Vec<Blocker>,
}

fn section_object<'a>(
    value: &'a Value,
    section: &str,
) -> Option<&'a serde_json::Map<String, Value>> {
    value.get(section).and_then(Value::as_object)
}

fn range_of(object: &serde_json::Map<String, Value>, name: &str) -> Option<String> {
    object.get(name).map(|v| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

pub fn compute_dependency_delta(old_pkg: &Value, new_pkg: &Value) -> DependencyDelta {
    let mut delta = DependencyDelta::default();

    let mut names: Vec<String> = Vec::new();
    let mut name_seen: HashSet<String> = HashSet::new();
    for section in DEP_SECTIONS {
        for pkg in [old_pkg, new_pkg] {
            if let Some(obj) = section_object(pkg, section) {
                for key in obj.keys() {
                    if name_seen.insert(key.clone()) {
                        names.push(key.clone());
                    }
                }
            }
        }
    }

    for name in names {
        let mut presences: Vec<(String, String)> = Vec::new();
        let mut hads: Vec<(String, String)> = Vec::new();
        // A present-but-non-string value (e.g. an object) is not a version
        // range the merger understands: fail closed instead of ignoring it.
        let mut shape_flagged = false;
        for section in DEP_SECTIONS {
            for pkg in [old_pkg, new_pkg] {
                if let Some(value) = pkg.get(section).and_then(|s| s.get(&name)) {
                    if !value.is_string() && !shape_flagged {
                        shape_flagged = true;
                        delta.blockers.push(Blocker::new(
                            "MANIFEST_SHAPE_REQUIRES_REVIEW",
                            format!("Selected commit(s) set dependency \"{name}\" in \"{section}\" to a non-string value, which is not applied automatically; explicit review required."),
                        ));
                    }
                }
            }
            if let Some(value) = new_pkg
                .get(section)
                .and_then(|s| s.get(&name))
                .and_then(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    _ => None,
                })
            {
                presences.push((section.to_string(), value));
            }
            if let Some(value) = old_pkg
                .get(section)
                .and_then(|s| s.get(&name))
                .and_then(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    _ => None,
                })
            {
                hads.push((section.to_string(), value));
            }
        }

        if hads.is_empty() && !presences.is_empty() {
            for (section, range) in presences {
                delta.added.push(DepAdd {
                    section,
                    name: name.clone(),
                    range,
                });
            }
            continue;
        }
        if presences.is_empty() && !hads.is_empty() {
            for (section, range) in hads {
                delta.removed.push(DepRemove {
                    section,
                    name: name.clone(),
                    range,
                });
            }
            continue;
        }
        if !presences.is_empty() && !hads.is_empty() {
            let from = &hads[0];
            let to = &presences[0];
            if from.0 == to.0 {
                if from.1 != to.1 {
                    delta.updated.push(DepUpdate {
                        section: from.0.clone(),
                        name: name.clone(),
                        from: from.1.clone(),
                        to: to.1.clone(),
                    });
                }
            } else {
                delta.moved.push(DepMove {
                    name: name.clone(),
                    from_section: from.0.clone(),
                    to_section: to.0.clone(),
                    from: from.1.clone(),
                    to: to.1.clone(),
                });
            }
        }
    }

    let mut changed_top_level: Vec<String> = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    for pkg in [old_pkg, new_pkg] {
        if let Some(obj) = pkg.as_object() {
            for key in obj.keys() {
                if !keys.contains(key) {
                    keys.push(key.clone());
                }
            }
        }
    }
    for key in keys {
        if DEP_SECTIONS.contains(&key.as_str()) {
            continue;
        }
        if old_pkg.get(&key) != new_pkg.get(&key) {
            changed_top_level.push(key);
        }
    }

    delta.manifest_changes = ManifestChanges {
        main_owned: changed_top_level
            .iter()
            .filter(|k| MAIN_OWNED_MANIFEST_KEYS.contains(&k.as_str()))
            .cloned()
            .collect(),
        ignored: changed_top_level
            .iter()
            .filter(|k| IGNORED_MANIFEST_KEYS.contains(&k.as_str()))
            .cloned()
            .collect(),
        other: changed_top_level
            .iter()
            .filter(|k| {
                !MAIN_OWNED_MANIFEST_KEYS.contains(&k.as_str())
                    && !IGNORED_MANIFEST_KEYS.contains(&k.as_str())
            })
            .cloned()
            .collect(),
    };

    delta
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedDep {
    pub name: String,
    pub previous_range: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovedDep {
    pub name: String,
    pub from_section: String,
    pub to_section: String,
    pub range: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewNote {
    pub name: String,
    pub section: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default)]
pub struct ReconcileResult {
    pub manifest: Value,
    pub added: Vec<DepAdd>,
    pub updated: Vec<DepUpdate>,
    pub removed: Vec<RemovedDep>,
    pub moved: Vec<MovedDep>,
    pub review: Vec<ReviewNote>,
    pub blockers: Vec<Blocker>,
    pub noop: Vec<String>,
}

#[derive(Debug, Clone)]
struct Event {
    kind: EventKind,
    section: String,
    to_section: String,
    name: String,
    range: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventKind {
    Add,
    Update,
    Move,
    Remove,
}

impl Event {
    fn key(&self) -> String {
        // Moves keep their historical `to_section:name` key so delta
        // de-duplication is unchanged now that `section` carries the source.
        if self.kind == EventKind::Move {
            return format!("{}:{}", self.to_section, self.name);
        }
        let section = if !self.section.is_empty() {
            &self.section
        } else {
            &self.to_section
        };
        format!("{section}:{}", self.name)
    }
}

pub fn reconcile_dependencies(
    deltas: &[DependencyDelta],
    target_manifest: &Value,
    allow_removals: bool,
) -> ReconcileResult {
    let mut manifest = target_manifest.clone();
    if !manifest.is_object() {
        manifest = Value::Object(serde_json::Map::new());
    }
    if let Some(root) = manifest.as_object_mut() {
        for section in DEP_SECTIONS {
            if !root.get(section).map(Value::is_object).unwrap_or(false) {
                root.insert(section.to_string(), Value::Object(serde_json::Map::new()));
            }
        }
    }

    let mut result = ReconcileResult {
        manifest: manifest.clone(),
        ..Default::default()
    };
    let mut order: Vec<String> = Vec::new();
    let mut seen: HashMap<String, Event> = HashMap::new();
    for delta in deltas {
        let mut push = |event: Event| {
            let key = event.key();
            if !seen.contains_key(&key) {
                order.push(key.clone());
            }
            seen.insert(key, event);
        };
        for a in &delta.added {
            push(Event {
                kind: EventKind::Add,
                section: a.section.clone(),
                to_section: String::new(),
                name: a.name.clone(),
                range: a.range.clone(),
            });
        }
        for u in &delta.updated {
            push(Event {
                kind: EventKind::Update,
                section: u.section.clone(),
                to_section: String::new(),
                name: u.name.clone(),
                range: u.to.clone(),
            });
        }
        for m in &delta.moved {
            push(Event {
                kind: EventKind::Move,
                section: m.from_section.clone(),
                to_section: m.to_section.clone(),
                name: m.name.clone(),
                range: m.to.clone(),
            });
        }
        for r in &delta.removed {
            push(Event {
                kind: EventKind::Remove,
                section: r.section.clone(),
                to_section: String::new(),
                name: r.name.clone(),
                range: r.range.clone(),
            });
        }
        for key in &delta.manifest_changes.main_owned {
            result.blockers.push(Blocker::new(
                "MANIFEST_KEY_REQUIRES_REVIEW",
                format!("Selected commit(s) change a reviewed package.json key: \"{key}\". This requires explicit human review and is not applied automatically."),
            ));
        }
        for blocker in &delta.blockers {
            result.blockers.push(blocker.clone());
        }
        for key in &delta.manifest_changes.other {
            result.blockers.push(Blocker::new(
                "MANIFEST_SHAPE_REQUIRES_REVIEW",
                format!("Selected commit(s) change non-dependency package.json field \"{key}\", which is not applied automatically; explicit review required."),
            ));
        }
        if !delta.manifest_changes.ignored.is_empty() {
            result.noop.push(format!(
                "main-owned manifest fields ignored: {}",
                delta.manifest_changes.ignored.join(", ")
            ));
        }
    }

    let find_anywhere = |manifest: &Value, name: &str| -> Option<(String, String)> {
        for section in DEP_SECTIONS {
            if let Some(obj) = section_object(manifest, section) {
                if obj.contains_key(name) {
                    return Some((section.to_string(), range_of(obj, name).unwrap_or_default()));
                }
            }
        }
        None
    };

    for key in order {
        let Some(event) = seen.get(&key).cloned() else {
            continue;
        };

        match event.kind {
            EventKind::Remove => {
                let Some((_, current_range)) = find_anywhere(&manifest, &event.name) else {
                    result.noop.push(format!(
                        "removal requested but not present in target: {}",
                        event.name
                    ));
                    continue;
                };
                if !allow_removals {
                    result.blockers.push(Blocker::new(
                        "DEPENDENCY_REMOVAL_REQUIRES_REVIEW",
                        format!(
                            "Selected commit(s) remove \"{}\", but the main system and other active modules may still require it. Removal requires explicit review.",
                            event.name
                        ),
                    ));
                    continue;
                }
                if let Some(root) = manifest.as_object_mut() {
                    for section in DEP_SECTIONS {
                        if let Some(obj) = root.get_mut(section).and_then(Value::as_object_mut) {
                            obj.remove(&event.name);
                        }
                    }
                }
                result.removed.push(RemovedDep {
                    name: event.name.clone(),
                    previous_range: current_range,
                });
            }
            EventKind::Move => {
                if let Some((_, current_range)) = find_anywhere(&manifest, &event.name) {
                    if current_range != event.range {
                        let cmp = compare_ranges(&current_range, &event.range);
                        if cmp.kind == ComparisonKind::Incompatible {
                            result.blockers.push(Blocker::new(
                                "DEPENDENCY_INCOMPATIBLE_RANGE",
                                format!(
                                    "Cannot relocate \"{}\" to {}: target range {} is incompatible with source range {}.",
                                    event.name, event.to_section, current_range, event.range
                                ),
                            ));
                            continue;
                        }
                    }
                }
                if let Some(root) = manifest.as_object_mut() {
                    for section in DEP_SECTIONS {
                        if let Some(obj) = root.get_mut(section).and_then(Value::as_object_mut) {
                            obj.remove(&event.name);
                        }
                    }
                    if let Some(dest) = root
                        .get_mut(&event.to_section)
                        .and_then(Value::as_object_mut)
                    {
                        dest.insert(event.name.clone(), Value::String(event.range.clone()));
                    }
                }
                result.moved.push(MovedDep {
                    name: event.name.clone(),
                    from_section: event.section.clone(),
                    to_section: event.to_section.clone(),
                    range: event.range.clone(),
                });
            }
            EventKind::Add | EventKind::Update => {
                let source_range = event.range.clone();
                let target_section = event.section.clone();
                let Some((current_section, current_range)) = find_anywhere(&manifest, &event.name)
                else {
                    if let Some(root) = manifest.as_object_mut() {
                        if let Some(dest) =
                            root.get_mut(&target_section).and_then(Value::as_object_mut)
                        {
                            dest.insert(event.name.clone(), Value::String(source_range.clone()));
                        }
                    }
                    result.added.push(DepAdd {
                        section: target_section,
                        name: event.name.clone(),
                        range: source_range,
                    });
                    continue;
                };

                let cmp = compare_ranges(&current_range, &source_range);
                match cmp.kind {
                    ComparisonKind::Identical => {
                        result.noop.push(format!(
                            "target already satisfies an identical range: {}",
                            event.name
                        ));
                    }
                    ComparisonKind::Incompatible => {
                        result.blockers.push(Blocker::new(
                            "DEPENDENCY_INCOMPATIBLE_RANGE",
                            format!(
                                "Incompatible dependency range for \"{}\": target has {}, selected source requires {}. Fail closed — human decision required.",
                                event.name, current_range, source_range
                            ),
                        ));
                    }
                    ComparisonKind::Unknown => {
                        result.blockers.push(Blocker::new(
                            "DEPENDENCY_RANGE_UNKNOWN",
                            format!(
                                "Cannot prove compatibility for \"{}\" (target {} vs source {}: {}). Fail closed.",
                                event.name,
                                current_range,
                                source_range,
                                cmp.reason.as_deref().unwrap_or("unknown")
                            ),
                        ));
                    }
                    ComparisonKind::Compatible => {
                        if current_section != target_section {
                            if let Some(root) = manifest.as_object_mut() {
                                if let Some(src) = root
                                    .get_mut(&current_section)
                                    .and_then(Value::as_object_mut)
                                {
                                    src.remove(&event.name);
                                }
                                if let Some(dest) =
                                    root.get_mut(&target_section).and_then(Value::as_object_mut)
                                {
                                    dest.insert(
                                        event.name.clone(),
                                        Value::String(source_range.clone()),
                                    );
                                }
                            }
                            result.moved.push(MovedDep {
                                name: event.name.clone(),
                                from_section: current_section,
                                to_section: target_section.clone(),
                                range: source_range.clone(),
                            });
                        } else if let Some(root) = manifest.as_object_mut() {
                            if let Some(dest) =
                                root.get_mut(&target_section).and_then(Value::as_object_mut)
                            {
                                dest.insert(
                                    event.name.clone(),
                                    Value::String(source_range.clone()),
                                );
                            }
                        }
                        result.updated.push(DepUpdate {
                            section: target_section.clone(),
                            name: event.name.clone(),
                            from: current_range.clone(),
                            to: source_range.clone(),
                        });
                        result.review.push(ReviewNote {
                            name: event.name.clone(),
                            section: target_section,
                            from: current_range,
                            to: source_range,
                        });
                    }
                }
            }
        }
    }

    result.manifest = manifest;
    result
}

pub fn extract_import_specifiers(content: &str) -> Vec<String> {
    // Strip comments first so commented-out `from`/`require` tokens and
    // prose inside JSDoc never produce false-positive specifiers. The strip
    // is quote-aware, so markers inside a string literal (e.g. a URL) survive.
    let stripped = strip_comments(content);
    let chars: Vec<char> = stripped.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut i = 0usize;
    let word_char = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' || c == '"' || c == '`' {
            let mut j = i + 1;
            // Backtick template literals may span lines; quoted strings end
            // at the newline like before.
            while j < chars.len() && chars[j] != c && (c == '`' || chars[j] != '\n') {
                j += 1;
            }
            if j < chars.len() && chars[j] == c {
                let inner: String = chars[i + 1..j].iter().collect();
                if !inner.is_empty() {
                    let mut k = i;
                    while k > 0 && chars[k - 1].is_whitespace() {
                        k -= 1;
                    }
                    let mut accept = false;
                    if k > 0 && chars[k - 1] == '(' {
                        let mut k2 = k - 1;
                        while k2 > 0 && chars[k2 - 1].is_whitespace() {
                            k2 -= 1;
                        }
                        let mut w2 = k2;
                        while w2 > 0 && word_char(chars[w2 - 1]) {
                            w2 -= 1;
                        }
                        let word2: String = chars[w2..k2].iter().collect();
                        accept = word2 == "require" || word2 == "import";
                    } else {
                        let mut w = k;
                        while w > 0 && word_char(chars[w - 1]) {
                            w -= 1;
                        }
                        let word: String = chars[w..k].iter().collect();
                        if word == "from" {
                            accept = true;
                        } else if word == "import" {
                            let mut p = w;
                            while p > 0 && (chars[p - 1] == ' ' || chars[p - 1] == '\t') {
                                p -= 1;
                            }
                            accept = p == 0 || chars[p - 1] == '\n';
                        }
                    }
                    if accept && seen.insert(inner.clone()) {
                        out.push(inner);
                    }
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Removes `//` and `/* */` comments in one pass. A `/*` inside a `//`
/// comment never opens a block (the line comment eats its line first), and
/// markers inside string literals are copied through. `'`/`"` strings end
/// at a newline (only backticks span lines), so a quote inside a regex
/// cannot desync the rest of the file. Newlines inside blocks are kept so
/// line shape is unchanged.
fn strip_comments(content: &str) -> String {
    let chars: Vec<char> = content.chars().collect();
    let mut out = String::with_capacity(content.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' || c == '"' || c == '`' {
            out.push(c);
            i += 1;
            while i < chars.len() {
                let d = chars[i];
                // Only backticks span lines: an unterminated `'`/`"` string
                // is already a syntax error, so a newline ends it before a
                // quote inside a regex can desync the rest of the file.
                if d == '\n' && c != '`' {
                    break;
                }
                out.push(d);
                i += 1;
                if d == '\\' {
                    if i < chars.len() {
                        out.push(chars[i]);
                        i += 1;
                    }
                } else if d == c {
                    break;
                }
            }
        } else if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            i += 2;
            while i < chars.len()
                && !(chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '/')
            {
                if chars[i] == '\n' {
                    out.push('\n');
                }
                i += 1;
            }
            i += 2;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

pub fn to_package_name(specifier: &str) -> Option<String> {
    if specifier.is_empty() {
        return None;
    }
    // Prose is not a package: npm names never contain whitespace.
    if specifier.chars().any(char::is_whitespace) {
        return None;
    }
    if specifier.starts_with('.')
        || specifier.starts_with('/')
        || specifier.starts_with('#')
        || specifier.starts_with("node:")
        || specifier.starts_with("@/")
        || specifier.starts_with('~')
        || specifier.starts_with("bun:")
    {
        return None;
    }
    let parts: Vec<&str> = specifier.split('/').collect();
    if specifier.starts_with('@') {
        if parts.len() >= 2 {
            Some(format!("{}/{}", parts[0], parts[1]))
        } else {
            None
        }
    } else if parts.first().map(|p| !p.is_empty()).unwrap_or(false) {
        Some(parts[0].to_string())
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingDependency {
    pub name: String,
    pub refs: Vec<ImportRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRef {
    pub path: String,
    pub specifier: String,
}

pub struct CodeFile {
    pub path: String,
    pub content: String,
}

pub fn is_code_path(path: &str) -> bool {
    let ext = path
        .rsplit_once('.')
        .map(|(_, e)| format!(".{}", e.to_ascii_lowercase()))
        .unwrap_or_default();
    CODE_EXTENSIONS.contains(&ext.as_str())
}

pub fn check_import_prerequisites(
    files: &[CodeFile],
    target_manifest: &Value,
    delta_added_names: &[String],
) -> Vec<MissingDependency> {
    let mut available: HashSet<String> = HashSet::new();
    for section in DEP_SECTIONS {
        if let Some(obj) = section_object(target_manifest, section) {
            for name in obj.keys() {
                available.insert(name.clone());
            }
        }
    }
    for name in delta_added_names {
        available.insert(name.clone());
    }

    let mut missing: HashMap<String, Vec<ImportRef>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut seen_refs: HashSet<(String, String)> = HashSet::new();
    for file in files {
        let ext = file
            .path
            .rsplit_once('.')
            .map(|(_, e)| format!(".{}", e.to_ascii_lowercase()))
            .unwrap_or_default();
        if !CODE_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        for spec in extract_import_specifiers(&file.content) {
            let Some(name) = to_package_name(&spec) else {
                continue;
            };
            let bare = name.strip_prefix("node:").unwrap_or(&name);
            if NODE_BUILTINS.contains(&bare) {
                continue;
            }
            if available.contains(&name) {
                continue;
            }
            if !missing.contains_key(&name) {
                order.push(name.clone());
            }
            if !seen_refs.insert((file.path.clone(), spec.clone())) {
                continue;
            }
            missing.entry(name.clone()).or_default().push(ImportRef {
                path: file.path.clone(),
                specifier: spec,
            });
        }
    }

    order
        .into_iter()
        .map(|name| {
            let refs = missing.remove(&name).unwrap_or_default();
            MissingDependency { name, refs }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn delta_add_update_move_remove() {
        let old = json!({ "dependencies": { "a": "^1.0.0", "b": "^2.0.0" }, "devDependencies": { "t": "^1.0.0" } });
        let new = json!({
            "dependencies": { "a": "^1.5.0", "c": "^3.0.0" },
            "devDependencies": { "b": "^2.0.0" },
            "peerDependencies": { "p": "^1.0.0" }
        });
        let d = compute_dependency_delta(&old, &new);
        assert_eq!(
            d.added,
            vec![
                DepAdd {
                    section: "dependencies".into(),
                    name: "c".into(),
                    range: "^3.0.0".into()
                },
                DepAdd {
                    section: "peerDependencies".into(),
                    name: "p".into(),
                    range: "^1.0.0".into()
                },
            ]
        );
        assert_eq!(d.updated.len(), 1);
        assert_eq!(d.moved.len(), 1);
        assert_eq!(d.removed.len(), 1);
    }

    #[test]
    fn reconcile_add_retains_unrelated_packages() {
        let target = json!({ "name": "main", "dependencies": { "next": "^16.2.1" }, "devDependencies": { "typescript": "^5" } });
        let delta = compute_dependency_delta(
            &json!({ "dependencies": { "next": "^16.2.1" } }),
            &json!({ "dependencies": { "next": "^16.2.1", "exceljs": "^4.4.0" } }),
        );
        let res = reconcile_dependencies(&[delta], &target, false);
        assert!(res.blockers.is_empty());
        assert_eq!(res.manifest["dependencies"]["exceljs"], json!("^4.4.0"));
        assert_eq!(res.manifest["dependencies"]["next"], json!("^16.2.1"));
        assert_eq!(res.manifest["devDependencies"]["typescript"], json!("^5"));
    }

    #[test]
    fn reconcile_incompatible_fails_closed_untouched() {
        let delta = compute_dependency_delta(
            &json!({ "dependencies": { "a": "^3.0.0" } }),
            &json!({ "dependencies": { "a": "^4.4.0" } }),
        );
        let res = reconcile_dependencies(
            &[delta],
            &json!({ "dependencies": { "a": "^3.0.0" } }),
            false,
        );
        assert_eq!(res.blockers.len(), 1);
        assert_eq!(res.blockers[0].code, "DEPENDENCY_INCOMPATIBLE_RANGE");
        assert_eq!(res.manifest["dependencies"]["a"], json!("^3.0.0"));
    }

    #[test]
    fn import_prerequisite_flags_unselected_dependency() {
        let files = vec![CodeFile {
            path: "src/use.ts".to_string(),
            content: "import leftPad from 'left-pad';\nexport default leftPad;\n".to_string(),
        }];
        let missing = check_import_prerequisites(
            &files,
            &json!({ "dependencies": { "next": "^16.2.1" } }),
            &[],
        );
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].name, "left-pad");
        let ok = check_import_prerequisites(
            &files,
            &json!({ "dependencies": {} }),
            &["left-pad".to_string()],
        );
        assert!(ok.is_empty());
    }

    #[test]
    fn package_name_extraction() {
        assert_eq!(to_package_name("./local"), None);
        assert_eq!(to_package_name("node:fs"), None);
        assert_eq!(to_package_name("@/components/x"), None);
        assert_eq!(to_package_name("lodash/fp").as_deref(), Some("lodash"));
        assert_eq!(
            to_package_name("@scope/pkg/sub").as_deref(),
            Some("@scope/pkg")
        );
        assert_eq!(
            extract_import_specifiers("import a from 'x';\nconst b = require('y');\nimport('z');"),
            vec!["x".to_string(), "y".to_string(), "z".to_string()]
        );
    }

    #[test]
    fn reconciled_move_carries_from_section() {
        let delta = compute_dependency_delta(
            &json!({ "dependencies": { "a": "^1.0.0" } }),
            &json!({ "devDependencies": { "a": "^1.0.0" } }),
        );
        assert_eq!(delta.moved.len(), 1);
        assert_eq!(delta.moved[0].from_section, "dependencies");
        let res = reconcile_dependencies(
            &[delta],
            &json!({ "dependencies": { "a": "^1.0.0" } }),
            false,
        );
        assert!(res.blockers.is_empty(), "{:?}", res.blockers);
        assert_eq!(res.moved.len(), 1);
        assert_eq!(res.moved[0].from_section, "dependencies");
        assert_eq!(res.moved[0].to_section, "devDependencies");
    }

    #[test]
    fn non_string_dependency_value_blocks_fail_closed() {
        let old = json!({ "dependencies": { "a": "^1.0.0" } });
        let new = json!({ "dependencies": { "a": { "version": "^1.0.0" } } });
        let delta = compute_dependency_delta(&old, &new);
        assert!(
            delta
                .blockers
                .iter()
                .any(|b| b.code == "MANIFEST_SHAPE_REQUIRES_REVIEW"),
            "{:?}",
            delta.blockers
        );
        let res = reconcile_dependencies(&[delta], &old, false);
        assert!(
            res.blockers
                .iter()
                .any(|b| b.code == "MANIFEST_SHAPE_REQUIRES_REVIEW"),
            "{:?}",
            res.blockers
        );
        assert_eq!(res.manifest["dependencies"]["a"], json!("^1.0.0"));
    }

    #[test]
    fn import_scan_ignores_line_comments_and_accepts_backticks() {
        assert_eq!(
            extract_import_specifiers(
                "// import ghost from 'ghost-pkg';\nimport real from 'real-pkg'; // trailing require('trailing-pkg');\n"
            ),
            vec!["real-pkg".to_string()]
        );
        assert_eq!(
            extract_import_specifiers("const m = await import(`backtick-pkg`);\n"),
            vec!["backtick-pkg".to_string()]
        );
        // `//` inside a string literal is code, not a comment.
        assert_eq!(
            extract_import_specifiers("import u from 'https://cdn.example/x';\n"),
            vec!["https://cdn.example/x".to_string()]
        );
    }

    #[test]
    fn import_scan_ignores_block_comments_and_prose() {
        // JSDoc prose that once blocked promotions.
        assert_eq!(
            extract_import_specifiers(
                "/**\n * Reads one eagerly loaded boundary file from `public/geojson`.\n */\n"
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            extract_import_specifiers(
                "/**\n * Decode JWT payload (NO VERIFY) and extract numeric userId from `sub`.\n */\n"
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            extract_import_specifiers("/* import ghost from 'ghost-pkg'; */\n"),
            Vec::<String>::new()
        );
        // A `//` inside a block comment stays inside it.
        assert_eq!(
            extract_import_specifiers(
                "/*\n// import ghost from 'ghost-pkg';\n*/\nimport real from 'real-pkg';\n"
            ),
            vec!["real-pkg".to_string()]
        );
        // A `/*` inside a line comment never opens a block.
        assert_eq!(
            extract_import_specifiers("// /* still a line comment\nimport real from 'real-pkg';\n"),
            vec!["real-pkg".to_string()]
        );
        // Markers inside string literals survive untouched.
        assert_eq!(
            extract_import_specifiers("import u from 'pkg/*not-a-comment*/sub';\n"),
            vec!["pkg/*not-a-comment*/sub".to_string()]
        );
    }

    #[test]
    fn import_scan_recovers_when_a_regex_holds_a_quote() {
        // The 1675493 shape: the regex's `'` must not swallow the JSDoc below it.
        let content = r#"import fs from 'node:fs';
const parts = s.replace(/[().,'"]/g, " ");
/**
 * Reads one eagerly loaded boundary file from `public/geojson`.
 */
import area from '../utils/area-match';
"#;
        assert_eq!(
            extract_import_specifiers(content),
            vec!["node:fs".to_string(), "../utils/area-match".to_string()]
        );
        // An escaped newline inside a quoted string is still one string.
        assert_eq!(
            extract_import_specifiers("import x from 'a\\nb';\n"),
            vec!["a\\nb".to_string()]
        );
        // A legitimate multi-line backtick template stays one string.
        assert_eq!(
            extract_import_specifiers(
                "const t = `line1\nline2 // not a comment`;\nimport r from 'real-pkg';\n"
            ),
            vec!["real-pkg".to_string()]
        );
    }

    #[test]
    fn package_name_rejects_prose_but_keeps_valid_shapes() {
        assert_eq!(to_package_name("my filter excluded everything"), None);
        assert_eq!(to_package_name("final_approved extra"), None);
        assert_eq!(to_package_name("name").as_deref(), Some("name"));
        assert_eq!(
            to_package_name("@scope/name").as_deref(),
            Some("@scope/name")
        );
        assert_eq!(to_package_name("name/sub").as_deref(), Some("name"));
        assert_eq!(to_package_name("name.js").as_deref(), Some("name.js"));
        assert_eq!(
            to_package_name("name-with-dashes").as_deref(),
            Some("name-with-dashes")
        );
    }

    #[test]
    fn import_refs_dedupe_same_path_and_specifier() {
        let files = vec![
            CodeFile {
                path: "src/a.ts".to_string(),
                content: "import x from 'missing-pkg';\n".to_string(),
            },
            CodeFile {
                path: "src/a.ts".to_string(),
                content: "import x from 'missing-pkg';\n".to_string(),
            },
            CodeFile {
                path: "src/b.ts".to_string(),
                content: "import x from 'missing-pkg';\n".to_string(),
            },
        ];
        let missing = check_import_prerequisites(&files, &json!({ "dependencies": {} }), &[]);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].name, "missing-pkg");
        let mut paths: Vec<&str> = missing[0].refs.iter().map(|r| r.path.as_str()).collect();
        paths.sort_unstable();
        assert_eq!(paths, vec!["src/a.ts", "src/b.ts"]);
    }
}
