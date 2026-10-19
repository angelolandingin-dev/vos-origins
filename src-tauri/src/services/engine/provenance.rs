//! Provenance trailers (spec §8.1/§8.3). Only the trailing block is
//! authoritative, so prose mentions never create phantom SHAs.

use std::collections::BTreeMap;
use std::path::Path;

use crate::github::gh_cli::GitHubAccount;

use super::git;

pub const BOT_NAME: &str = "vos-integration-bot";
pub const BOT_EMAIL: &str = "vos-integration-bot@users.noreply.github.com";

pub const TRAILER_KEYS: [&str; 8] = [
    "Subsystem-ID",
    "Source-Repo",
    "Source-Commit",
    "Promotion-Mode",
    "Promotion-Target",
    "Reverts-Integration",
    "Restores-Integration",
    "Promotion-Replaced",
];

fn sanitize_subject(subject: &str, max: usize) -> String {
    let single = subject.split_whitespace().collect::<Vec<_>>().join(" ");
    let single = if single.is_empty() {
        "integration".to_string()
    } else {
        single
    };
    if single.chars().count() > max {
        let mut truncated: String = single.chars().take(max - 1).collect();
        truncated.push('…');
        truncated
    } else {
        single
    }
}

pub struct CommitAuthor {
    pub name: String,
    pub email: String,
}

/// Picks the provenance commit's author. Attribution off, or an account that
/// could not be resolved, both fall back to the bot identity so a missing
/// login/id never fails a promotion.
pub fn commit_author(attribute_to_account: bool, account: Option<&GitHubAccount>) -> CommitAuthor {
    match (attribute_to_account, account) {
        (true, Some(account)) => CommitAuthor {
            name: account.login.clone(),
            email: account.noreply_email(),
        },
        _ => CommitAuthor {
            name: BOT_NAME.to_string(),
            email: BOT_EMAIL.to_string(),
        },
    }
}

pub struct ProvenanceInput<'a> {
    pub subsystem_id: &'a str,
    pub source_repo: &'a str,
    pub source_commits: &'a [String],
    pub mode: &'a str,
    pub target: &'a str,
    pub subject: &'a str,
}

pub fn build_provenance_message(input: &ProvenanceInput<'_>) -> String {
    let header = format!(
        "integrate({}): {}",
        input.subsystem_id,
        sanitize_subject(input.subject, 72)
    );
    let mut trailer_lines = vec![
        format!("Subsystem-ID: {}", input.subsystem_id),
        format!("Source-Repo: {}", input.source_repo),
    ];
    for sha in input.source_commits {
        trailer_lines.push(format!("Source-Commit: {sha}"));
    }
    trailer_lines.push(format!("Promotion-Mode: {}", input.mode));
    trailer_lines.push(format!("Promotion-Target: {}", input.target));
    format!("{header}\n\n{}\n", trailer_lines.join("\n"))
}

pub fn build_revert_message(
    subsystem_id: &str,
    source_commits: &[String],
    reverts_integration_sha: &str,
    subject: &str,
) -> String {
    let header = format!("revert(integrate): undo {}", sanitize_subject(subject, 72));
    let mut trailer_lines = vec![
        format!("Reverts-Integration: {reverts_integration_sha}"),
        format!("Subsystem-ID: {subsystem_id}"),
    ];
    for sha in source_commits {
        trailer_lines.push(format!("Source-Commit: {sha}"));
    }
    format!("{header}\n\n{}\n", trailer_lines.join("\n"))
}

#[derive(Debug, Clone, Default)]
pub struct ParsedTrailers {
    pub trailers: BTreeMap<String, Vec<String>>,
    pub order: Vec<String>,
}

impl ParsedTrailers {
    pub fn get(&self, key: &str) -> &[String] {
        self.trailers.get(key).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn first(&self, key: &str) -> Option<&str> {
        self.trailers
            .get(key)
            .and_then(|v| v.first())
            .map(|s| s.as_str())
    }
}

fn split_trailer(line: &str) -> Option<(String, String)> {
    let idx = line.find(':')?;
    if idx == 0 {
        return None;
    }
    let key = &line[..idx];
    if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    Some((key.to_string(), line[idx + 1..].trim().to_string()))
}

pub fn parse_trailers(message: &str) -> ParsedTrailers {
    let lines: Vec<&str> = message.split('\n').collect();
    let mut end = lines.len();
    while end > 0 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    let mut block: Vec<&str> = Vec::new();
    let mut i = end;
    while i > 0 {
        if split_trailer(lines[i - 1]).is_none() {
            break;
        }
        block.push(lines[i - 1]);
        i -= 1;
    }
    block.reverse();

    let mut result = ParsedTrailers::default();
    for line in block {
        if let Some((key, value)) = split_trailer(line) {
            if TRAILER_KEYS.contains(&key.as_str()) {
                // `Source-Commit:` values are only meaningful as full SHAs
                // (40/64 hex, as `extract_source_commits` enforces for PR
                // bodies). Anything else is dropped here so commit-history
                // collection can never mint phantom SHAs from typos or prose.
                let value = if key == "Source-Commit" {
                    let candidate = value.trim();
                    let ok = (candidate.len() == 40 || candidate.len() == 64)
                        && candidate.chars().all(|c| c.is_ascii_hexdigit());
                    if !ok {
                        continue;
                    }
                    candidate.to_ascii_lowercase()
                } else {
                    value
                };
                if !result.trailers.contains_key(&key) {
                    result.order.push(key.clone());
                }
                result.trailers.entry(key).or_default().push(value);
            }
        }
    }
    result
}

pub fn write_provenance_commit(
    worktree_dir: &Path,
    message: &str,
    message_file: &Path,
    author_name: &str,
    author_email: &str,
    mode_ops: &[super::apply::ModeOp],
) -> Result<String, String> {
    git::add_all(worktree_dir).map_err(|err| err.to_string())?;
    super::apply::apply_mode_ops(worktree_dir, mode_ops);
    if git::status_porcelain(worktree_dir).trim().is_empty() {
        return Err("no changes staged; refusing to record a no-op integration".to_string());
    }
    std::fs::write(message_file, message)
        .map_err(|err| format!("could not write commit message file: {err}"))?;
    let result = git::commit_with_message(worktree_dir, message_file, author_name, author_email);
    let _ = std::fs::remove_file(message_file);
    result.map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_source_commit_order_is_preserved() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let msg = build_provenance_message(&ProvenanceInput {
            subsystem_id: "hr-management",
            source_repo: "ORG/hr-management",
            source_commits: &[a.clone(), b.clone()],
            mode: "individual",
            target: "staging",
            subject: "employee export",
        });
        let parsed = parse_trailers(&msg);
        assert_eq!(parsed.get("Source-Commit"), &[a, b]);
        assert_eq!(parsed.first("Subsystem-ID"), Some("hr-management"));
        assert_eq!(parsed.first("Promotion-Target"), Some("staging"));
    }

    #[test]
    fn embedded_source_commit_does_not_create_phantom() {
        let real = "a".repeat(40);
        let phantom = "dead".repeat(10);
        let msg = [
            "integrate(hr-management): subject",
            "",
            &format!("Source-Commit: {phantom}"),
            "",
            "Prose follows, then the authoritative trailer block:",
            "",
            "Subsystem-ID: hr-management",
            &format!("Source-Commit: {real}"),
            "Promotion-Target: staging",
            "",
        ]
        .join("\n");
        let parsed = parse_trailers(&msg);
        assert_eq!(parsed.get("Source-Commit"), &[real]);
        assert!(!parsed.get("Source-Commit").iter().any(|s| s == &phantom));
    }

    #[test]
    fn invalid_source_commit_trailers_are_dropped_and_normalized() {
        let valid = "b".repeat(40);
        let upper = "C".repeat(40);
        let msg = [
            "integrate(hr-management): subject",
            "",
            "Subsystem-ID: hr-management",
            "Source-Commit: not-a-sha",
            "Source-Commit: abc123",
            &format!("Source-Commit: {valid}"),
            &format!("Source-Commit: {upper}"),
            "Promotion-Target: staging",
            "",
        ]
        .join("\n");
        let parsed = parse_trailers(&msg);
        assert_eq!(
            parsed.get("Source-Commit"),
            &[valid, "c".repeat(40)],
            "only 40/64-hex survives, normalized to lowercase"
        );
    }

    #[test]
    fn commit_author_uses_the_account_when_attribution_is_on() {
        let account = GitHubAccount {
            login: "angelolandingin-dev".to_string(),
            id: 4242,
        };
        let author = commit_author(true, Some(&account));
        assert_eq!(author.name, "angelolandingin-dev");
        assert_eq!(
            author.email,
            "4242+angelolandingin-dev@users.noreply.github.com"
        );
    }

    #[test]
    fn commit_author_falls_back_to_the_bot_when_off_or_unresolved() {
        let account = GitHubAccount {
            login: "someone".to_string(),
            id: 7,
        };
        for author in [
            commit_author(false, Some(&account)),
            commit_author(true, None),
            commit_author(false, None),
        ] {
            assert_eq!(author.name, BOT_NAME);
            assert_eq!(author.email, BOT_EMAIL);
        }
    }
}
