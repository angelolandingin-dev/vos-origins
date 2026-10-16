//! Registration validation (spec §4.1). Authoritative check re-runs in CI;
//! fixture mode reports branch protection as unverified, never assumed.

use crate::fixtures::FixtureWorld;
use crate::github::models::*;

pub fn slug_is_valid(id: &str) -> bool {
    let mut previous_dash = true;
    if id.is_empty() || id.len() > 48 {
        return false;
    }
    for ch in id.chars() {
        match ch {
            'a'..='z' | '0'..='9' => previous_dash = false,
            '-' if !previous_dash => previous_dash = true,
            _ => return false,
        }
    }
    !previous_dash
}

pub fn repository_is_valid(repository: &str) -> bool {
    let mut parts = repository.split('/');
    let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let valid_segment = |segment: &str| {
        !segment.is_empty()
            && segment.len() <= 100
            && segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    valid_segment(owner) && valid_segment(name)
}

pub fn branch_is_valid(branch: &str) -> bool {
    !branch.is_empty()
        && branch.len() <= 120
        && !branch.starts_with('/')
        && !branch.ends_with('/')
        && branch
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
}

pub fn validate(
    request: &SubsystemRegistrationRequest,
    world: &FixtureWorld,
) -> Vec<ValidationFinding> {
    let mut findings: Vec<ValidationFinding> = Vec::new();

    if !slug_is_valid(&request.id) {
        findings.push(ValidationFinding::error(
            "invalid_slug",
            format!(
                "'{}' is not a valid subsystem id (lowercase letters, digits and single dashes).",
                request.id
            ),
            "Use a slug such as 'hr-management'. The id becomes .integration/subsystems/<id>.json.",
        ));
    }
    if request.name.trim().is_empty() {
        findings.push(ValidationFinding::error(
            "invalid_name",
            "Display name is required.",
            "Enter the human-readable subsystem name shown in the picker.",
        ));
    }
    if !repository_is_valid(&request.repository) {
        findings.push(ValidationFinding::error(
            "invalid_repository",
            format!("'{}' is not a valid OWNER/REPO slug.", request.repository),
            "Provide the GitHub repository as OWNER/REPO, for example organization/hr-management.",
        ));
    }
    if !branch_is_valid(&request.source_branch) {
        findings.push(ValidationFinding::error(
            "invalid_source_branch",
            format!("'{}' is not a valid branch name.", request.source_branch),
            "Use the protected branch that approved commits land on (usually 'master').",
        ));
    }

    if let Some(existing) = world.subsystems.iter().find(|s| s.id == request.id) {
        findings.push(ValidationFinding::error(
            "duplicate_id",
            format!(
                "Subsystem id '{}' is already registered as {}.",
                request.id, existing.repository
            ),
            "Open the existing registration or choose a different id.",
        ));
    }
    if let Some(existing) = world
        .subsystems
        .iter()
        .find(|s| s.repository.eq_ignore_ascii_case(&request.repository))
    {
        findings.push(ValidationFinding::error(
            "duplicate_repository",
            format!("{} is already registered as subsystem '{}'.", request.repository, existing.id),
            "A repository may only back one registry entry. Inspect or disable the existing entry instead.",
        ));
    }

    findings.push(ValidationFinding::warning(
        "branch_protection_unverified",
        format!(
            "Branch protection for {}@{} could not be verified from the desktop client.",
            request.repository, request.source_branch
        ),
        "The registration PR's CI job re-checks branch protection with server-side permissions before merging.",
    ));

    findings.push(ValidationFinding::info_with_remedy(
        "policy_defaults",
        "Integration policy defaults applied: preserve-relative-paths, reconcile dependencies, regenerate lockfiles in the target, never import secret files.",
        "Override individual policy keys in the registration PR if this subsystem needs different semantics.",
    ));

    findings
}

pub fn has_errors(findings: &[ValidationFinding]) -> bool {
    findings.iter().any(|f| f.level == FindingLevel::Error)
}

pub fn branch_name_for(id: &str) -> String {
    format!("integration/register-{id}")
}
