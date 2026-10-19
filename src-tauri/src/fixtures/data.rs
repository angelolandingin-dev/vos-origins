use chrono::{DateTime, Duration, Utc};

use super::{relative_time, FailureEffect, IntegrationEvent, IntegrationEventKind};
use crate::github::models::*;

pub const MAIN_REPOSITORY: &str = "organization/main-system";
pub const REGISTRY_BRANCH: &str = "master";
pub const WORKFLOW_FILE: &str = "promote-subsystem.yml";
pub const NAMESPACE: &str = "organization";

pub const DEFAULT_TARGET_BRANCH: &str = "staging";

pub const STATUS_PENDING_NOTE: &str =
    "Status is computed per target branch from Git history and open PRs.";

pub fn ts(now: DateTime<Utc>, ago: Duration) -> String {
    (now - ago).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub struct FileSeed {
    pub path: &'static str,
    pub previous_path: Option<&'static str>,
    pub change: FileChangeKind,
    pub additions: u32,
    pub deletions: u32,
    pub policy: FileIntegrationPolicy,
    pub note: Option<&'static str>,
}

pub struct DepSeed {
    pub name: &'static str,
    pub section: DependencySection,
    pub change: DependencyChangeKind,
    pub from_range: Option<&'static str>,
    pub to_range: Option<&'static str>,
    pub conflict: bool,
    pub note: Option<&'static str>,
}

pub struct CommitSeed {
    pub sha: &'static str,
    pub subject: &'static str,
    pub body: &'static str,
    pub author: &'static str,
    pub login: &'static str,
    pub ago_minutes: i64,
    pub tags: &'static [&'static str],
    pub files_changed: u32,
    pub additions: u32,
    pub deletions: u32,
    pub touches_manifest: bool,
    pub is_merge: bool,
    pub files: &'static [FileSeed],
    pub deps: &'static [DepSeed],
}

pub fn subsystems(now: DateTime<Utc>) -> Vec<SubsystemSummary> {
    let policy = SubsystemPolicy::default();
    let targets = vec![
        "development".to_string(),
        "staging".to_string(),
        "master".to_string(),
        "release/*".to_string(),
    ];

    let row = |id: &str,
               name: &str,
               repo: &str,
               source_branch: &str,
               description: &str,
               connectivity: ConnectivityState,
               message: Option<&str>,
               pr: u64,
               registered_days: i64| SubsystemSummary {
        id: id.to_string(),
        name: name.to_string(),
        repository: format!("{NAMESPACE}/{repo}"),
        source_branch: source_branch.to_string(),
        enabled: true,
        description: description.to_string(),
        connectivity,
        connectivity_message: message.map(str::to_string),
        policy: policy.clone(),
        path_strategy: "preserve-relative-paths".to_string(),
        dependency_strategy: "reconcile".to_string(),
        last_synced_at: ts(now, Duration::minutes(11)),
        registered_at: ts(now, Duration::days(registered_days)),
        registration_pr_number: Some(pr),
        registration_pr_url: Some(format!("https://github.com/{MAIN_REPOSITORY}/pull/{pr}")),
        allowed_target_branches: targets.clone(),
    };

    vec![
        row(
            "hr-management",
            "HR Management",
            "hr-management",
            "master",
            "Employee records, onboarding, and export tooling.",
            ConnectivityState::Connected,
            None,
            341,
            96,
        ),
        row(
            "payroll-management",
            "Payroll Management",
            "payroll-management",
            "master",
            "Payroll runs, payslips, and statutory reporting.",
            ConnectivityState::Connected,
            None,
            342,
            88,
        ),
        row(
            "inventory-management",
            "Inventory Management",
            "inventory-management",
            "master",
            "Stock levels, warehouses, and stock movement audit.",
            ConnectivityState::Connected,
            None,
            355,
            61,
        ),
        row(
            "procurement",
            "Procurement",
            "procurement",
            "master",
            "Purchase requisitions, vendors, and approvals.",
            ConnectivityState::Connected,
            None,
            358,
            54,
        ),
        row(
            "document-management",
            "Document Management",
            "document-management",
            "main",
            "Document storage, versioning, and retention policy.",
            ConnectivityState::Degraded,
            Some("Configured source branch 'main' was last verified 11 minutes ago; branch protection could not be re-checked."),
            360,
            30,
        ),
    ]
}

pub fn target_policy() -> TargetBranchPolicy {
    TargetBranchPolicy {
        allowed_target_branches: vec!["development".into(), "staging".into(), "master".into()],
        allowed_target_branch_patterns: vec!["release/*".into()],
        require_pull_request: true,
        allow_direct_push: false,
        workflow_file: WORKFLOW_FILE.to_string(),
    }
}

pub fn branches() -> Vec<TargetBranch> {
    vec![
        TargetBranch {
            name: "development".into(),
            head_sha: "0d17c4e9a1b2c3d4e5f60718293a4b5c6d7e8f90".into(),
            protected: true,
            is_default: false,
            allowed: true,
            disallowed_reason: None,
            open_pr_count: 0,
            active_promotions: 0,
        },
        TargetBranch {
            name: DEFAULT_TARGET_BRANCH.into(),
            head_sha: "5b83a1f7c2d4e6f8091a2b3c4d5e6f708192a3b4".into(),
            protected: true,
            is_default: false,
            allowed: true,
            disallowed_reason: None,
            open_pr_count: 2,
            active_promotions: 1,
        },
        TargetBranch {
            name: "master".into(),
            head_sha: "c4a90e2b7d1f3e5a6b8c0d2e4f60718293a4b5c6".into(),
            protected: true,
            is_default: true,
            allowed: true,
            disallowed_reason: None,
            open_pr_count: 0,
            active_promotions: 0,
        },
        TargetBranch {
            name: "release/2026.1".into(),
            head_sha: "e7d6c5b4a3928170615243f8e9d0c1b2a3f4e5d6".into(),
            protected: true,
            is_default: false,
            allowed: true,
            disallowed_reason: None,
            open_pr_count: 0,
            active_promotions: 0,
        },
        TargetBranch {
            name: "spike/kb-scratch".into(),
            head_sha: "1a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d".into(),
            protected: false,
            is_default: false,
            allowed: false,
            disallowed_reason: Some(
                "Not listed in allowedTargetBranches and matches no allowedTargetBranchPatterns entry in .integration/config.json.".into(),
            ),
            open_pr_count: 0,
            active_promotions: 0,
        },
    ]
}

/// `master` inherits reachable records without a separate request (spec §5).
pub fn branch_ancestry() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        ("development", vec![]),
        ("staging", vec![]),
        ("master", vec!["development", "staging"]),
        ("release/2026.1", vec!["staging"]),
        ("spike/kb-scratch", vec![]),
    ]
}

const HR_SEEDS: &[CommitSeed] = &[
    CommitSeed {
        sha: "a81f2c3e5d9c4b7a2f1e8d3c6b9a0f4e2d7c1b85",
        subject: "Add employee export feature",
        body: "Add functionality to export employees to Excel format with custom filters.\n\nThe export reuses the shared filter type so that the department and employment-status\nfilters behave the same as the list screen.",
        author: "Dev User",
        login: "dev.user",
        ago_minutes: 120,
        tags: &["feature", "export", "employees"],
        files_changed: 4,
        additions: 38,
        deletions: 6,
        touches_manifest: true,
        is_merge: false,
        files: &[
            FileSeed {
                path: "src/modules/hr/export.ts",
                previous_path: None,
                change: FileChangeKind::Modified,
                additions: 12,
                deletions: 5,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
            FileSeed {
                path: "src/modules/hr/exportUtils.ts",
                previous_path: None,
                change: FileChangeKind::Added,
                additions: 14,
                deletions: 0,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
            FileSeed {
                path: "src/modules/hr/types.ts",
                previous_path: None,
                change: FileChangeKind::Modified,
                additions: 8,
                deletions: 1,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
            FileSeed {
                path: "package.json",
                previous_path: None,
                change: FileChangeKind::Modified,
                additions: 1,
                deletions: 0,
                policy: FileIntegrationPolicy::ReviewRequired,
                note: Some("Manifest delta is merged into the target manifest; the deployment regenerates the target lockfile, never copied."),
            },
        ],
        deps: &[DepSeed {
            name: "exceljs",
            section: DependencySection::Dependencies,
            change: DependencyChangeKind::Added,
            from_range: None,
            to_range: Some("^4.4.0"),
            conflict: false,
            note: Some("New runtime dependency introduced by this commit."),
        }],
    },
    CommitSeed {
        sha: "c71e392b48ac7f2e1d90b3a5c6e418d7f2a09b31",
        subject: "Update profile validation",
        body: "Reject empty display names and normalise whitespace before persisting the employee profile.",
        author: "Dev User",
        login: "dev.user",
        ago_minutes: 2880,
        tags: &["validation"],
        files_changed: 2,
        additions: 21,
        deletions: 9,
        touches_manifest: false,
        is_merge: false,
        files: &[
            FileSeed {
                path: "src/modules/hr/profile.ts",
                previous_path: None,
                change: FileChangeKind::Modified,
                additions: 17,
                deletions: 8,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
            FileSeed {
                path: "src/modules/hr/profile.test.ts",
                previous_path: None,
                change: FileChangeKind::Modified,
                additions: 4,
                deletions: 1,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
        ],
        deps: &[],
    },
    CommitSeed {
        sha: "f12b301d7e5a4c9382b16f0d3c7e8459a1b2c3d4",
        subject: "Fix employee list bug",
        body: "The employee list dropped the last page when the total was an exact multiple of the page size.",
        author: "Priya Raman",
        login: "priya.raman",
        ago_minutes: 5760,
        tags: &["bugfix"],
        files_changed: 1,
        additions: 6,
        deletions: 2,
        touches_manifest: false,
        is_merge: false,
        files: &[FileSeed {
            path: "src/modules/hr/list.ts",
            previous_path: None,
            change: FileChangeKind::Modified,
            additions: 6,
            deletions: 2,
            policy: FileIntegrationPolicy::Apply,
            note: None,
        }],
        deps: &[],
    },
    CommitSeed {
        sha: "9d4e112a3c8b7f6e5d4c3b2a1908f7e6d5c4b3a2",
        subject: "Refactor HR routes",
        body: "Split the single HR route module into employee, department, and export routers.",
        author: "Dev User",
        login: "dev.user",
        ago_minutes: 10080,
        tags: &["refactor"],
        files_changed: 5,
        additions: 142,
        deletions: 118,
        touches_manifest: false,
        is_merge: false,
        files: &[
            FileSeed {
                path: "src/modules/hr/routes/index.ts",
                previous_path: Some("src/modules/hr/routes.ts"),
                change: FileChangeKind::Renamed,
                additions: 4,
                deletions: 2,
                policy: FileIntegrationPolicy::Apply,
                note: Some("Rename is preserved when applying the delta."),
            },
            FileSeed {
                path: "src/modules/hr/routes/employees.ts",
                previous_path: None,
                change: FileChangeKind::Added,
                additions: 61,
                deletions: 0,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
            FileSeed {
                path: "src/modules/hr/routes/departments.ts",
                previous_path: None,
                change: FileChangeKind::Added,
                additions: 58,
                deletions: 0,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
            FileSeed {
                path: "src/modules/hr/routes/export.ts",
                previous_path: None,
                change: FileChangeKind::Added,
                additions: 19,
                deletions: 0,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
            FileSeed {
                path: "src/modules/hr/routes.ts",
                previous_path: None,
                change: FileChangeKind::Deleted,
                additions: 0,
                deletions: 116,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
        ],
        deps: &[],
    },
    CommitSeed {
        sha: "3ac9d7e1b2c3d4e5f60718293a4b5c6d7e8f9012",
        subject: "Add department filter",
        body: "Add a department filter to the employee table, including the shared filter chip component.\n\nThis commit touches a large shared component and is used to exercise bounded diff rendering.",
        author: "Alex Chen",
        login: "alex.chen",
        ago_minutes: 10080,
        tags: &["feature", "ui"],
        files_changed: 2,
        additions: 214,
        deletions: 96,
        touches_manifest: false,
        is_merge: false,
        files: &[
            FileSeed {
                path: "src/modules/hr/components/EmployeeTable.tsx",
                previous_path: None,
                change: FileChangeKind::Modified,
                additions: 208,
                deletions: 94,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
            FileSeed {
                path: "src/modules/hr/components/FilterChips.tsx",
                previous_path: None,
                change: FileChangeKind::Added,
                additions: 6,
                deletions: 2,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
        ],
        deps: &[],
    },
    CommitSeed {
        sha: "72b1e4fa1b2c3d4e5f6071829304a5b6c7d8e9f0",
        subject: "UI improvements",
        body: "Spacing, empty states, and keyboard focus rings for the HR screens.",
        author: "Dev User",
        login: "dev.user",
        ago_minutes: 20160,
        tags: &["ui"],
        files_changed: 3,
        additions: 44,
        deletions: 27,
        touches_manifest: false,
        is_merge: false,
        files: &[
            FileSeed {
                path: "src/modules/hr/components/EmptyState.tsx",
                previous_path: None,
                change: FileChangeKind::Added,
                additions: 22,
                deletions: 0,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
            FileSeed {
                path: "src/modules/hr/components/EmployeeTable.tsx",
                previous_path: None,
                change: FileChangeKind::Modified,
                additions: 14,
                deletions: 18,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
            FileSeed {
                path: "src/modules/hr/styles/table.css",
                previous_path: None,
                change: FileChangeKind::Modified,
                additions: 8,
                deletions: 9,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
        ],
        deps: &[],
    },
    CommitSeed {
        sha: "b4e8a11f0c9d8e7f6a5b4c3d2e1f001122334455",
        subject: "Add payroll stub to employee onboarding",
        body: "Creates a payroll stub record when an employee completes onboarding.",
        author: "Priya Raman",
        login: "priya.raman",
        ago_minutes: 30240,
        tags: &["feature"],
        files_changed: 1,
        additions: 31,
        deletions: 0,
        touches_manifest: false,
        is_merge: false,
        files: &[FileSeed {
            path: "src/modules/hr/onboarding.ts",
            previous_path: None,
            change: FileChangeKind::Modified,
            additions: 31,
            deletions: 0,
            policy: FileIntegrationPolicy::Apply,
            note: None,
        }],
        deps: &[],
    },
    CommitSeed {
        sha: "5f0a2bc3d4e5f6071829304a5b6c7d8e9f001122",
        subject: "Remove legacy CSV shim",
        body: "Drops the deprecated CSV helper that overlapped with the shared exporter.",
        author: "Alex Chen",
        login: "alex.chen",
        ago_minutes: 40320,
        tags: &["cleanup"],
        files_changed: 2,
        additions: 3,
        deletions: 96,
        touches_manifest: false,
        is_merge: false,
        files: &[
            FileSeed {
                path: "src/modules/hr/csv.ts",
                previous_path: None,
                change: FileChangeKind::Deleted,
                additions: 0,
                deletions: 93,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
            FileSeed {
                path: "src/modules/hr/export.ts",
                previous_path: None,
                change: FileChangeKind::Modified,
                additions: 3,
                deletions: 3,
                policy: FileIntegrationPolicy::Apply,
                note: Some("This hunk no longer applies: the target branch changed the same lines after this commit."),
            },
        ],
        deps: &[],
    },
];

const PAYROLL_SEEDS: &[CommitSeed] = &[
    CommitSeed {
        sha: "d42f7a1b9c8e7d6f5a4b3c2d1e0f998877665544",
        subject: "Add payslip PDF export",
        body: "Generate payslips as PDF using the shared document service.",
        author: "Mei Lin",
        login: "mei.lin",
        ago_minutes: 1440,
        tags: &["feature", "export"],
        files_changed: 2,
        additions: 76,
        deletions: 4,
        touches_manifest: false,
        is_merge: false,
        files: &[
            FileSeed {
                path: "src/modules/payroll/payslip.ts",
                previous_path: None,
                change: FileChangeKind::Modified,
                additions: 62,
                deletions: 4,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
            FileSeed {
                path: "src/modules/payroll/templates/payslip.ts",
                previous_path: None,
                change: FileChangeKind::Added,
                additions: 14,
                deletions: 0,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
        ],
        deps: &[],
    },
    CommitSeed {
        sha: "1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f70819293",
        subject: "Fix statutory tax rounding",
        body: "Round statutory deductions using banker's rounding to match the finance ledger.",
        author: "Mei Lin",
        login: "mei.lin",
        ago_minutes: 10080,
        tags: &["bugfix"],
        files_changed: 1,
        additions: 9,
        deletions: 5,
        touches_manifest: false,
        is_merge: false,
        files: &[FileSeed {
            path: "src/modules/payroll/tax.ts",
            previous_path: None,
            change: FileChangeKind::Modified,
            additions: 9,
            deletions: 5,
            policy: FileIntegrationPolicy::Apply,
            note: None,
        }],
        deps: &[],
    },
    CommitSeed {
        sha: "9e2a471c3b4d5e6f708192a3b4c5d6e7f8091a2b",
        subject: "Add bank file format v2",
        body: "Support the v2 bank transfer file required by the new provider.",
        author: "Mei Lin",
        login: "mei.lin",
        ago_minutes: 20160,
        tags: &["feature"],
        files_changed: 1,
        additions: 54,
        deletions: 12,
        touches_manifest: false,
        is_merge: false,
        files: &[FileSeed {
            path: "src/modules/payroll/bankFile.ts",
            previous_path: None,
            change: FileChangeKind::Modified,
            additions: 54,
            deletions: 12,
            policy: FileIntegrationPolicy::Apply,
            note: None,
        }],
        deps: &[],
    },
];

const INVENTORY_SEEDS: &[CommitSeed] = &[
    CommitSeed {
        sha: "7f6e5d4c3b2a1908f7e6d5c4b3a2918374655647",
        subject: "Add low stock alert thresholds",
        body: "Per-warehouse low stock thresholds with an alert summary endpoint.",
        author: "Sara Novak",
        login: "sara.novak",
        ago_minutes: 4320,
        tags: &["feature"],
        files_changed: 2,
        additions: 88,
        deletions: 10,
        touches_manifest: false,
        is_merge: false,
        files: &[
            FileSeed {
                path: "src/modules/inventory/stock.ts",
                previous_path: None,
                change: FileChangeKind::Modified,
                additions: 74,
                deletions: 10,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
            FileSeed {
                path: "src/modules/inventory/thresholds.ts",
                previous_path: None,
                change: FileChangeKind::Added,
                additions: 14,
                deletions: 0,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
        ],
        deps: &[],
    },
    CommitSeed {
        sha: "2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e",
        subject: "Fix warehouse transfer audit log",
        body: "Record the acting user on transfer audit entries.",
        author: "Sara Novak",
        login: "sara.novak",
        ago_minutes: 15840,
        tags: &["bugfix"],
        files_changed: 1,
        additions: 7,
        deletions: 3,
        touches_manifest: false,
        is_merge: false,
        files: &[FileSeed {
            path: "src/modules/inventory/transfer.ts",
            previous_path: None,
            change: FileChangeKind::Modified,
            additions: 7,
            deletions: 3,
            policy: FileIntegrationPolicy::Apply,
            note: None,
        }],
        deps: &[],
    },
];

const PROCUREMENT_SEEDS: &[CommitSeed] = &[
    CommitSeed {
        sha: "3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f",
        subject: "Add vendor approval workflow",
        body: "Two-step vendor approval with a configurable second approver.",
        author: "Tom Becker",
        login: "tom.becker",
        ago_minutes: 2160,
        tags: &["feature"],
        files_changed: 3,
        additions: 132,
        deletions: 18,
        touches_manifest: false,
        is_merge: false,
        files: &[
            FileSeed {
                path: "src/modules/procurement/approval.ts",
                previous_path: None,
                change: FileChangeKind::Modified,
                additions: 98,
                deletions: 18,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
            FileSeed {
                path: "src/modules/procurement/vendor.ts",
                previous_path: None,
                change: FileChangeKind::Modified,
                additions: 24,
                deletions: 0,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
            FileSeed {
                path: "api/procurement/approval/route.ts",
                previous_path: None,
                change: FileChangeKind::Added,
                additions: 10,
                deletions: 0,
                policy: FileIntegrationPolicy::Apply,
                note: None,
            },
        ],
        deps: &[],
    },
    CommitSeed {
        sha: "4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f70",
        subject: "Tighten purchase order permissions",
        body: "Only approvers may exceed the configured purchase order ceiling.",
        author: "Tom Becker",
        login: "tom.becker",
        ago_minutes: 17280,
        tags: &["security"],
        files_changed: 1,
        additions: 11,
        deletions: 6,
        touches_manifest: false,
        is_merge: false,
        files: &[FileSeed {
            path: "src/modules/procurement/permissions.ts",
            previous_path: None,
            change: FileChangeKind::Modified,
            additions: 11,
            deletions: 6,
            policy: FileIntegrationPolicy::Apply,
            note: None,
        }],
        deps: &[],
    },
];

const DOCUMENT_SEEDS: &[CommitSeed] = &[CommitSeed {
    sha: "5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f7081",
    subject: "Add retention policy evaluation",
    body: "Evaluate retention policy nightly and flag documents past their retention window.",
    author: "Nadia Farah",
    login: "nadia.farah",
    ago_minutes: 720,
    tags: &["feature"],
    files_changed: 2,
    additions: 64,
    deletions: 5,
    touches_manifest: false,
    is_merge: false,
    files: &[
        FileSeed {
            path: "src/modules/documents/retention.ts",
            previous_path: None,
            change: FileChangeKind::Modified,
            additions: 58,
            deletions: 5,
            policy: FileIntegrationPolicy::Apply,
            note: None,
        },
        FileSeed {
            path: "src/modules/documents/policy.ts",
            previous_path: None,
            change: FileChangeKind::Added,
            additions: 6,
            deletions: 0,
            policy: FileIntegrationPolicy::Apply,
            note: None,
        },
    ],
    deps: &[],
}];

pub fn commit_seeds_for(subsystem_id: &str) -> &'static [CommitSeed] {
    match subsystem_id {
        "hr-management" => HR_SEEDS,
        "payroll-management" => PAYROLL_SEEDS,
        "inventory-management" => INVENTORY_SEEDS,
        "procurement" => PROCUREMENT_SEEDS,
        "document-management" => DOCUMENT_SEEDS,
        _ => &[],
    }
}

pub fn commit_summaries(subsystem_id: &str, now: DateTime<Utc>) -> Vec<CommitSummary> {
    commit_seeds_for(subsystem_id)
        .iter()
        .map(|seed| {
            let committed_at = now - Duration::minutes(seed.ago_minutes);
            let authored_at = committed_at - Duration::minutes(37);
            CommitSummary {
                sha: seed.sha.to_string(),
                short_sha: seed.sha.chars().take(7).collect(),
                subject: seed.subject.to_string(),
                author_name: seed.author.to_string(),
                author_login: Some(seed.login.to_string()),
                authored_at: authored_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                committed_at: committed_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                relative_time: relative_time(committed_at, now),
                status: CommitStatus::unknown(DEFAULT_TARGET_BRANCH, STATUS_PENDING_NOTE),
                parents: if seed.is_merge {
                    vec![
                        format!("{}0000000000000000000000000000000000", &seed.sha[..8]),
                        format!("{}1111111111111111111111111111111111", &seed.sha[..8]),
                    ]
                } else {
                    vec![format!(
                        "{}ffffffffffffffffffffffffffffffff",
                        &seed.sha[..8]
                    )]
                },
                is_merge: seed.is_merge,
                eligible: true,
                files_changed: seed.files_changed,
                additions: seed.additions,
                deletions: seed.deletions,
                tags: seed.tags.iter().map(|t| t.to_string()).collect(),
                touches_manifest: seed.touches_manifest,
            }
        })
        .collect()
}

pub fn commit_details(
    subsystem_id: &str,
    repository: &str,
    now: DateTime<Utc>,
) -> Vec<CommitDetails> {
    commit_seeds_for(subsystem_id)
        .iter()
        .map(|seed| {
            let committed_at = now - Duration::minutes(seed.ago_minutes);
            let authored_at = committed_at - Duration::minutes(37);
            let files: Vec<ChangedFile> = seed
                .files
                .iter()
                .map(|file| ChangedFile {
                    path: file.path.to_string(),
                    previous_path: file.previous_path.map(str::to_string),
                    change: file.change,
                    additions: file.additions,
                    deletions: file.deletions,
                    binary: false,
                    policy: file.policy,
                    policy_note: file.note.map(str::to_string),
                })
                .collect();

            let dependencies: Vec<DependencyChange> = seed
                .deps
                .iter()
                .map(|dep| DependencyChange {
                    name: dep.name.to_string(),
                    section: dep.section,
                    change: dep.change,
                    from_range: dep.from_range.map(str::to_string),
                    to_range: dep.to_range.map(str::to_string),
                    conflict: dep.conflict,
                    note: dep.note.map(str::to_string),
                })
                .collect();

            CommitDetails {
                subsystem_id: subsystem_id.to_string(),
                target_branch: DEFAULT_TARGET_BRANCH.to_string(),
                sha: seed.sha.to_string(),
                short_sha: seed.sha.chars().take(7).collect(),
                subject: seed.subject.to_string(),
                body: seed.body.to_string(),
                author_name: seed.author.to_string(),
                author_login: Some(seed.login.to_string()),
                authored_at: authored_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                committed_at: committed_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                relative_time: relative_time(committed_at, now),
                status: CommitStatus::unknown(DEFAULT_TARGET_BRANCH, STATUS_PENDING_NOTE),
                tags: seed.tags.iter().map(|t| t.to_string()).collect(),
                files,
                files_changed: seed.files_changed,
                additions: seed.additions,
                deletions: seed.deletions,
                dependencies,
                related_pull_requests: vec![PullRequestRef {
                    number: 200 + (seed.ago_minutes % 90) as u64,
                    title: seed.subject.to_string(),
                    state: PrState::Merged,
                    base_branch: "master".to_string(),
                    html_url: format!(
                        "https://github.com/{repository}/pull/{}",
                        200 + (seed.ago_minutes % 90)
                    ),
                    repository: repository.to_string(),
                }],
                parents: vec![format!(
                    "{}ffffffffffffffffffffffffffffffff",
                    &seed.sha[..8]
                )],
                is_merge: seed.is_merge,
                source_url: format!("https://github.com/{repository}/commit/{}", seed.sha),
                note: None,
            }
        })
        .collect()
}

fn line(kind: DiffLineKind, old_line: Option<u32>, new_line: Option<u32>, text: &str) -> DiffLine {
    DiffLine {
        kind,
        old_line,
        new_line,
        text: text.to_string(),
    }
}

pub fn hand_written_diffs() -> Vec<CommitDiff> {
    let sha = "a81f2c3e5d9c4b7a2f1e8d3c6b9a0f4e2d7c1b85";

    let export_ts = CommitDiff {
        subsystem_id: "hr-management".into(),
        sha: sha.into(),
        file_path: "src/modules/hr/export.ts".into(),
        language: "typescript".into(),
        hunks: vec![
            DiffHunk {
                header: "@@ -1,7 +1,10 @@".into(),
                lines: vec![
                    line(DiffLineKind::Context, Some(1), Some(1), "import { getEmployees } from './employeeRepo';"),
                    line(DiffLineKind::Context, Some(2), Some(2), "import { generateCSV } from './csv';"),
                    line(DiffLineKind::Removed, Some(3), None, "import type { Employee } from './types';"),
                    line(DiffLineKind::Added, None, Some(3), "import ExcelJS from 'exceljs';"),
                    line(DiffLineKind::Added, None, Some(4), "import type { Employee, ExportFilters } from './types';"),
                    line(DiffLineKind::Added, None, Some(5), "import { buildWorkbook } from './exportUtils';"),
                    line(DiffLineKind::Context, Some(4), Some(6), ""),
                    line(DiffLineKind::Context, Some(5), Some(7), "/**"),
                    line(DiffLineKind::Context, Some(6), Some(8), " * Employee export entry points."),
                    line(DiffLineKind::Context, Some(7), Some(9), " */"),
                ],
            },
            DiffHunk {
                header: "@@ -10,6 +10,8 @@".into(),
                lines: vec![
                    line(DiffLineKind::Removed, Some(10), None, "export async function exportEmployees() {"),
                    line(DiffLineKind::Removed, Some(11), None, "  const employees = await getEmployees();"),
                    line(DiffLineKind::Removed, Some(12), None, "  return generateCSV(employees);"),
                    line(DiffLineKind::Removed, Some(13), None, "}"),
                    line(DiffLineKind::Added, None, Some(10), "export async function exportEmployees(filters?: ExportFilters) {"),
                    line(DiffLineKind::Added, None, Some(11), "  const employees = await getEmployees(filters);"),
                    line(DiffLineKind::Added, None, Some(12), "  const workbook = new ExcelJS.Workbook();"),
                    line(DiffLineKind::Added, None, Some(13), "  const sheet = workbook.addWorksheet('Employees');"),
                    line(DiffLineKind::Added, None, Some(14), "  return await generateExcel(employees, sheet);"),
                    line(DiffLineKind::Added, None, Some(15), "}"),
                    line(DiffLineKind::Context, Some(14), Some(16), ""),
                    line(DiffLineKind::Context, Some(15), Some(17), "export async function generateExcel("),
                ],
            },
        ],
        total_lines: 20,
        additions: 12,
        deletions: 5,
        binary: false,
        truncated: false,
        source_url: format!("https://github.com/{NAMESPACE}/hr-management/commit/{sha}#diff-src/modules/hr/export.ts"),
        note: None,
    };

    let export_utils_ts = CommitDiff {
        subsystem_id: "hr-management".into(),
        sha: sha.into(),
        file_path: "src/modules/hr/exportUtils.ts".into(),
        language: "typescript".into(),
        hunks: vec![DiffHunk {
            header: "@@ -0,0 +1,14 @@".into(),
            lines: vec![
                line(DiffLineKind::Added, None, Some(1), "import ExcelJS from 'exceljs';"),
                line(DiffLineKind::Added, None, Some(2), "import type { Employee } from './types';"),
                line(DiffLineKind::Added, None, Some(3), ""),
                line(DiffLineKind::Added, None, Some(4), "const COLUMNS = ['id', 'name', 'department', 'status'] as const;"),
                line(DiffLineKind::Added, None, Some(5), ""),
                line(DiffLineKind::Added, None, Some(6), "export function buildWorkbook(employees: Employee[]): ExcelJS.Workbook {"),
                line(DiffLineKind::Added, None, Some(7), "  const workbook = new ExcelJS.Workbook();"),
                line(DiffLineKind::Added, None, Some(8), "  const sheet = workbook.addWorksheet('Employees');"),
                line(DiffLineKind::Added, None, Some(9), "  sheet.columns = COLUMNS.map((key) => ({ header: key, key }));"),
                line(DiffLineKind::Added, None, Some(10), "  sheet.addRows(employees);"),
                line(DiffLineKind::Added, None, Some(11), "  return workbook;"),
                line(DiffLineKind::Added, None, Some(12), "}"),
                line(DiffLineKind::Added, None, Some(13), ""),
                line(DiffLineKind::Added, None, Some(14), "export const EXPORT_MIME = 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet';"),
            ],
        }],
        total_lines: 14,
        additions: 14,
        deletions: 0,
        binary: false,
        truncated: false,
        source_url: format!("https://github.com/{NAMESPACE}/hr-management/commit/{sha}#diff-src/modules/hr/exportUtils.ts"),
        note: Some("New file. Applied at the same relative path in the main-system tree.".into()),
    };

    let types_ts = CommitDiff {
        subsystem_id: "hr-management".into(),
        sha: sha.into(),
        file_path: "src/modules/hr/types.ts".into(),
        language: "typescript".into(),
        hunks: vec![DiffHunk {
            header: "@@ -12,6 +12,13 @@".into(),
            lines: vec![
                line(DiffLineKind::Context, Some(12), Some(12), "export interface Employee {"),
                line(DiffLineKind::Context, Some(13), Some(13), "  id: string;"),
                line(DiffLineKind::Context, Some(14), Some(14), "  name: string;"),
                line(DiffLineKind::Added, None, Some(15), "  departmentId: string;"),
                line(DiffLineKind::Context, Some(15), Some(16), "  status: EmploymentStatus;"),
                line(DiffLineKind::Context, Some(16), Some(17), "}"),
                line(DiffLineKind::Added, None, Some(18), ""),
                line(DiffLineKind::Added, None, Some(19), "export interface ExportFilters {"),
                line(DiffLineKind::Added, None, Some(20), "  departmentId?: string;"),
                line(DiffLineKind::Added, None, Some(21), "  status?: EmploymentStatus;"),
                line(DiffLineKind::Added, None, Some(22), "  search?: string;"),
                line(DiffLineKind::Added, None, Some(23), "}"),
            ],
        }],
        total_lines: 12,
        additions: 8,
        deletions: 1,
        binary: false,
        truncated: false,
        source_url: format!("https://github.com/{NAMESPACE}/hr-management/commit/{sha}#diff-src/modules/hr/types.ts"),
        note: None,
    };

    let package_json = CommitDiff {
        subsystem_id: "hr-management".into(),
        sha: sha.into(),
        file_path: "package.json".into(),
        language: "json".into(),
        hunks: vec![DiffHunk {
            header: "@@ -18,6 +18,7 @@".into(),
            lines: vec![
                line(DiffLineKind::Context, Some(18), Some(18), "  \"dependencies\": {"),
                line(DiffLineKind::Context, Some(19), Some(19), "    \"next\": \"15.5.0\","),
                line(DiffLineKind::Context, Some(20), Some(20), "    \"react\": \"19.1.0\","),
                line(DiffLineKind::Added, None, Some(21), "    \"exceljs\": \"^4.4.0\","),
                line(DiffLineKind::Context, Some(21), Some(22), "    \"zod\": \"^3.24.1\""),
                line(DiffLineKind::Context, Some(22), Some(23), "  },"),
            ],
        }],
        total_lines: 6,
        additions: 1,
        deletions: 0,
        binary: false,
        truncated: false,
        source_url: format!("https://github.com/{NAMESPACE}/hr-management/commit/{sha}#diff-package.json"),
        note: Some("Manifest delta only: the main-system manifest is updated with the added dependency and the deployment regenerates the target lockfile. The subsystem lockfile is never copied.".into()),
    };

    let fix_bug = CommitDiff {
        subsystem_id: "hr-management".into(),
        sha: "f12b301d7e5a4c9382b16f0d3c7e8459a1b2c3d4".into(),
        file_path: "src/modules/hr/list.ts".into(),
        language: "typescript".into(),
        hunks: vec![DiffHunk {
            header: "@@ -24,9 +24,13 @@".into(),
            lines: vec![
                line(DiffLineKind::Context, Some(24), Some(24), "export function paginate(rows: Employee[], page: number, size: number) {"),
                line(DiffLineKind::Removed, Some(25), None, "  const offset = page * size;"),
                line(DiffLineKind::Added, None, Some(25), "  const offset = (page - 1) * size;"),
                line(DiffLineKind::Context, Some(26), Some(26), "  const pageRows = rows.slice(offset, offset + size);"),
                line(DiffLineKind::Removed, Some(27), None, "  return { rows: pageRows };"),
                line(DiffLineKind::Added, None, Some(27), "  return {"),
                line(DiffLineKind::Added, None, Some(28), "    rows: pageRows,"),
                line(DiffLineKind::Added, None, Some(29), "    total: rows.length,"),
                line(DiffLineKind::Added, None, Some(30), "  };"),
                line(DiffLineKind::Context, Some(28), Some(31), "}"),
            ],
        }],
        total_lines: 10,
        additions: 6,
        deletions: 2,
        binary: false,
        truncated: false,
        source_url: format!("https://github.com/{NAMESPACE}/hr-management/commit/f12b301d7e5a4c9382b16f0d3c7e8459a1b2c3d4#diff-src/modules/hr/list.ts"),
        note: None,
    };

    let blocked_cleanup = CommitDiff {
        subsystem_id: "hr-management".into(),
        sha: "5f0a2bc3d4e5f6071829304a5b6c7d8e9f001122".into(),
        file_path: "src/modules/hr/export.ts".into(),
        language: "typescript".into(),
        hunks: vec![DiffHunk {
            header: "@@ -30,9 +30,6 @@".into(),
            lines: vec![
                line(DiffLineKind::Context, Some(30), Some(30), "export async function exportEmployeesCsv() {"),
                line(DiffLineKind::Removed, Some(31), None, "  const employees = await getEmployees();"),
                line(DiffLineKind::Removed, Some(32), None, "  return generateCSV(employees);"),
                line(DiffLineKind::Removed, Some(33), None, "}"),
                line(DiffLineKind::Context, Some(34), Some(31), ""),
                line(DiffLineKind::Context, Some(35), Some(32), "export async function exportDepartmentsCsv() {"),
            ],
        }],
        total_lines: 6,
        additions: 3,
        deletions: 3,
        binary: false,
        truncated: false,
        source_url: format!("https://github.com/{NAMESPACE}/hr-management/commit/5f0a2bc3d4e5f6071829304a5b6c7d8e9f001122#diff-src/modules/hr/export.ts"),
        note: Some("Patch does not apply on the staging tip: these lines changed after this commit. The promotion is blocked until an integrator resolves it.".into()),
    };

    vec![
        export_ts,
        export_utils_ts,
        types_ts,
        package_json,
        fix_bug,
        blocked_cleanup,
    ]
}

pub fn large_generated_diff() -> CommitDiff {
    let sha = "3ac9d7e1b2c3d4e5f60718293a4b5c6d7e8f9012";
    let mut lines: Vec<DiffLine> = Vec::with_capacity(420);
    let mut old_line = 1_u32;
    let mut new_line = 1_u32;

    for block in 0..28_u32 {
        for i in 0..4 {
            lines.push(line(
                DiffLineKind::Context,
                Some(old_line),
                Some(new_line),
                &format!("  const row{block}_{i} = useMemo(() => buildRow({block}, {i}), [rows]);"),
            ));
            old_line += 1;
            new_line += 1;
        }
        lines.push(line(
            DiffLineKind::Removed,
            Some(old_line),
            None,
            &format!("  const filtered{block} = rows.filter(matchesLegacyFilter{block});"),
        ));
        old_line += 1;
        lines.push(line(
            DiffLineKind::Added,
            None,
            Some(new_line),
            &format!("  const filtered{block} = rows.filter((row) => matchesFilters(row, filters, {block}));"),
        ));
        new_line += 1;
        for i in 0..5 {
            lines.push(line(
                DiffLineKind::Context,
                Some(old_line),
                Some(new_line),
                &format!("  const column{block}_{i} = COLUMNS[{i}].render(filtered{block}[{i}]);"),
            ));
            old_line += 1;
            new_line += 1;
        }
    }

    let additions = lines
        .iter()
        .filter(|l| l.kind == DiffLineKind::Added)
        .count() as u32;
    let deletions = lines
        .iter()
        .filter(|l| l.kind == DiffLineKind::Removed)
        .count() as u32;
    let total = lines.len() as u32;

    CommitDiff {
        subsystem_id: "hr-management".into(),
        sha: sha.into(),
        file_path: "src/modules/hr/components/EmployeeTable.tsx".into(),
        language: "tsx".into(),
        hunks: vec![
            DiffHunk {
                header: "@@ -1,18 +1,22 @@".into(),
                lines: vec![
                    line(DiffLineKind::Context, Some(1), Some(1), "import { useMemo, useState } from 'react';"),
                    line(DiffLineKind::Context, Some(2), Some(2), "import type { Employee } from '../types';"),
                    line(DiffLineKind::Added, None, Some(3), "import { FilterChips } from './FilterChips';"),
                    line(DiffLineKind::Context, Some(3), Some(4), ""),
                    line(DiffLineKind::Removed, Some(4), None, "export function EmployeeTable({ rows }: Props) {"),
                    line(DiffLineKind::Added, None, Some(5), "export function EmployeeTable({ rows, filters }: Props) {"),
                    line(DiffLineKind::Context, Some(5), Some(6), "  return ("),
                    line(DiffLineKind::Added, None, Some(7), "    <FilterChips filters={filters} />"),
                    line(DiffLineKind::Context, Some(6), Some(8), "  );"),
                ],
            },
            DiffHunk {
                header: format!("@@ -40,{total} +44,{} @@", total + 4),
                lines,
            },
        ],
        total_lines: total,
        additions,
        deletions,
        binary: false,
        truncated: false,
        source_url: format!("https://github.com/{NAMESPACE}/hr-management/commit/{sha}#diff-src/modules/hr/components/EmployeeTable.tsx"),
        note: Some("Large change rendered through a bounded, virtualized side-by-side viewer; the full file is fetched on demand.".into()),
    }
}

pub fn events(now: DateTime<Utc>) -> Vec<IntegrationEvent> {
    let event = |id: &str,
                 kind: IntegrationEventKind,
                 subsystem: &str,
                 shas: Vec<&str>,
                 target: &str,
                 main_sha: &str,
                 pr: u64,
                 ago: Duration,
                 message: &str,
                 reverts: Option<&str>| IntegrationEvent {
        id: id.to_string(),
        kind,
        subsystem_id: subsystem.to_string(),
        source_shas: shas.into_iter().map(str::to_string).collect(),
        target_branch: target.to_string(),
        main_commit_sha: main_sha.to_string(),
        merged_at: ts(now, ago),
        message: message.to_string(),
        pull_request_number: Some(pr),
        actor: "main.reviewer".to_string(),
        reverts_event_id: reverts.map(str::to_string),
    };

    vec![
        event(
            "evt-hr-f12",
            IntegrationEventKind::Integrate,
            "hr-management",
            vec!["f12b301d7e5a4c9382b16f0d3c7e8459a1b2c3d4"],
            "staging",
            "7a1f9c2e4b6d8f0a2c4e6b8d0f2a4c6e8b0d2f4a",
            409,
            Duration::days(3),
            "integrate(hr-management): fix employee list bug",
            None,
        ),
        event(
            "evt-hr-72b",
            IntegrationEventKind::Integrate,
            "hr-management",
            vec!["72b1e4fa1b2c3d4e5f6071829304a5b6c7d8e9f0"],
            "development",
            "3c5e7a9b1d3f5a7c9e1b3d5f7a9c1e3b5d7f9a1c",
            388,
            Duration::days(12),
            "integrate(hr-management): UI improvements",
            None,
        ),
        event(
            "evt-hr-b4e",
            IntegrationEventKind::Integrate,
            "hr-management",
            vec!["b4e8a11f0c9d8e7f6a5b4c3d2e1f001122334455"],
            "staging",
            "9e8d7c6b5a4938271605142f3e4d5c6b7a8f9e0d",
            398,
            Duration::days(18),
            "integrate(hr-management): add payroll stub to employee onboarding",
            None,
        ),
        event(
            "evt-hr-b4e-revert",
            IntegrationEventKind::Revert,
            "hr-management",
            vec!["b4e8a11f0c9d8e7f6a5b4c3d2e1f001122334455"],
            "staging",
            "2d4f6a8c0e2b4d6f8a0c2e4b6d8f0a2c4e6b8d0f",
            413,
            Duration::days(16),
            "revert(integrate): undo HR payroll stub on onboarding",
            Some("evt-hr-b4e"),
        ),
        event(
            "evt-pay-d42",
            IntegrationEventKind::Integrate,
            "payroll-management",
            vec!["d42f7a1b9c8e7d6f5a4b3c2d1e0f998877665544"],
            "staging",
            "6b8d0f2a4c6e8b0d2f4a6c8e0b2d4f6a8c0e2b4d",
            407,
            Duration::days(1),
            "integrate(payroll-management): add payslip PDF export",
            None,
        ),
        event(
            "evt-pay-9e2",
            IntegrationEventKind::Integrate,
            "payroll-management",
            vec!["9e2a471c3b4d5e6f708192a3b4c5d6e7f8091a2b"],
            "staging",
            "1f3e5d7c9b8a70615243f8e9d0c1b2a3f4e5d6c7",
            371,
            Duration::days(21),
            "integrate(payroll-management): bank file format v2",
            None,
        ),
        event(
            "evt-inv-7f6",
            IntegrationEventKind::Integrate,
            "inventory-management",
            vec!["7f6e5d4c3b2a1908f7e6d5c4b3a2918374655647"],
            "staging",
            "8a0c2e4b6d8f0a2c4e6b8d0f2a4c6e8b0d2f4a6c",
            405,
            Duration::days(2),
            "integrate(inventory-management): low stock alert thresholds",
            None,
        ),
    ]
}

pub fn failure_effects() -> Vec<FailureEffect> {
    vec![
        FailureEffect {
            subsystem_id: "hr-management".into(),
            source_shas: vec!["5f0a2bc3d4e5f6071829304a5b6c7d8e9f001122".into()],
            target_branch: "staging".into(),
            kind: CommitStatusKind::Blocked,
            reason: "Patch conflict: src/modules/hr/export.ts hunk @@ -30,9 +30,6 @@ no longer applies to the staging tip. Target branch left unchanged.".into(),
        },
        FailureEffect {
            subsystem_id: "procurement".into(),
            source_shas: vec!["4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f70".into()],
            target_branch: "staging".into(),
            kind: CommitStatusKind::Failed,
            reason: "Main-system CI failed: unit tests rejected the tightened purchase order permissions.".into(),
        },
    ]
}

pub fn prs(now: DateTime<Utc>) -> Vec<IntegrationPr> {
    let pr = |number: u64,
              title: &str,
              state: PrState,
              review: ReviewState,
              base: &str,
              head: &str,
              subsystem: &str,
              shas: Vec<&'static str>,
              created: Duration,
              updated: Duration,
              merged: Option<Duration>,
              merged_by: Option<&str>,
              merge_commit: Option<&str>,
              author: &str,
              provenance_ok: bool,
              provenance_note: Option<&str>| IntegrationPr {
        number,
        title: title.to_string(),
        state,
        draft: false,
        review_state: review,
        base_branch: base.to_string(),
        head_branch: head.to_string(),
        subsystem_id: subsystem.to_string(),
        source_shas: shas.into_iter().map(str::to_string).collect(),
        created_at: ts(now, created),
        updated_at: ts(now, updated),
        merged_at: merged.map(|d| ts(now, d)),
        merged_by: merged_by.map(str::to_string),
        merge_commit_sha: merge_commit.map(str::to_string),
        author: author.to_string(),
        html_url: format!("https://github.com/{MAIN_REPOSITORY}/pull/{number}"),
        provenance_ok,
        provenance_note: provenance_note.map(str::to_string),
    };

    vec![
        pr(
            412,
            "integrate(hr-management): update profile validation",
            PrState::Open,
            ReviewState::Required,
            "staging",
            "main-system-promote/hr-management-c71e392",
            "hr-management",
            vec!["c71e392b48ac7f2e1d90b3a5c6e418d7f2a09b31"],
            Duration::hours(3),
            Duration::hours(2),
            None,
            None,
            None,
            "dev.user",
            true,
            None,
        ),
        pr(
            411,
            "integrate(inventory-management): low stock alert thresholds",
            PrState::Open,
            ReviewState::ChangesRequested,
            "staging",
            "main-system-promote/inventory-management-7f6e5d4",
            "inventory-management",
            vec!["7f6e5d4c3b2a1908f7e6d5c4b3a2918374655647"],
            Duration::days(2),
            Duration::hours(9),
            None,
            None,
            None,
            "sara.novak",
            true,
            None,
        ),
        pr(
            409,
            "integrate(hr-management): fix employee list bug",
            PrState::Merged,
            ReviewState::Approved,
            "staging",
            "main-system-promote/hr-management-f12b301",
            "hr-management",
            vec!["f12b301d7e5a4c9382b16f0d3c7e8459a1b2c3d4"],
            Duration::days(3),
            Duration::days(3),
            Some(Duration::days(3) - Duration::minutes(30)),
            Some("main.reviewer"),
            Some("7a1f9c2e4b6d8f0a2c4e6b8d0f2a4c6e8b0d2f4a"),
            "dev.user",
            true,
            None,
        ),
        pr(
            407,
            "integrate(payroll-management): add payslip PDF export",
            PrState::Merged,
            ReviewState::Approved,
            "staging",
            "main-system-promote/payroll-management-d42f7a1",
            "payroll-management",
            vec!["d42f7a1b9c8e7d6f5a4b3c2d1e0f998877665544"],
            Duration::days(1),
            Duration::days(1),
            Some(Duration::days(1) - Duration::minutes(12)),
            Some("main.reviewer"),
            Some("6b8d0f2a4c6e8b0d2f4a6c8e0b2d4f6a8c0e2b4d"),
            "mei.lin",
            false,
            Some("Squash merge detected; the Source-Commit trailer was preserved by the merge template, but the main-system commit SHA differs from the integration branch head. Verified manually."),
        ),
        pr(
            388,
            "integrate(hr-management): UI improvements",
            PrState::Merged,
            ReviewState::Approved,
            "development",
            "main-system-promote/hr-management-72b1e4f",
            "hr-management",
            vec!["72b1e4fa1b2c3d4e5f6071829304a5b6c7d8e9f0"],
            Duration::days(12),
            Duration::days(12),
            Some(Duration::days(12) - Duration::hours(2)),
            Some("main.reviewer"),
            Some("3c5e7a9b1d3f5a7c9e1b3d5f7a9c1e3b5d7f9a1c"),
            "dev.user",
            true,
            None,
        ),
        pr(
            377,
            "integrate(hr-management): remove legacy CSV shim",
            PrState::Closed,
            ReviewState::ChangesRequested,
            "staging",
            "main-system-promote/hr-management-5f0a2bc",
            "hr-management",
            vec!["5f0a2bc3d4e5f6071829304a5b6c7d8e9f001122"],
            Duration::days(6),
            Duration::days(5),
            None,
            None,
            None,
            "alex.chen",
            true,
            Some("Closed without merge: not recorded as integrated."),
        ),
    ]
}

pub fn history(now: DateTime<Utc>) -> Vec<IntegrationHistoryEntry> {
    let entry = |id: &str,
                 kind: IntegrationHistoryKind,
                 subsystem: &str,
                 shas: Vec<&'static str>,
                 target: &str,
                 main_sha: Option<&str>,
                 pr: Option<u64>,
                 actor: &str,
                 ago: Duration,
                 detail: &str,
                 inherited: Option<&str>| IntegrationHistoryEntry {
        id: id.to_string(),
        kind,
        subsystem_id: subsystem.to_string(),
        source_shas: shas.into_iter().map(str::to_string).collect(),
        target_branch: target.to_string(),
        main_commit_sha: main_sha.map(str::to_string),
        pull_request_number: pr,
        actor: actor.to_string(),
        occurred_at: ts(now, ago),
        detail: detail.to_string(),
        inherited_from_branch: inherited.map(str::to_string),
    };

    vec![
        entry(
            "hist-pay-d42",
            IntegrationHistoryKind::Merged,
            "payroll-management",
            vec!["d42f7a1b9c8e7d6f5a4b3c2d1e0f998877665544"],
            "staging",
            Some("6b8d0f2a4c6e8b0d2f4a6c8e0b2d4f6a8c0e2b4d"),
            Some(407),
            "main.reviewer",
            Duration::days(1),
            "Merged into staging with standard provenance trailers (Subsystem-ID, Source-Commit, Promotion-Target).",
            None,
        ),
        entry(
            "hist-inv-7f6",
            IntegrationHistoryKind::PendingPr,
            "inventory-management",
            vec!["7f6e5d4c3b2a1908f7e6d5c4b3a2918374655647"],
            "staging",
            None,
            Some(411),
            "sara.novak",
            Duration::days(2),
            "Integration PR open with changes requested. Not integrated.",
            None,
        ),
        entry(
            "hist-hr-f12",
            IntegrationHistoryKind::Merged,
            "hr-management",
            vec!["f12b301d7e5a4c9382b16f0d3c7e8459a1b2c3d4"],
            "staging",
            Some("7a1f9c2e4b6d8f0a2c4e6b8d0f2a4c6e8b0d2f4a"),
            Some(409),
            "main.reviewer",
            Duration::days(3),
            "Merged into staging. Reachable from master through the routine staging -> master branch merge.",
            None,
        ),
        entry(
            "hist-hr-f12-master",
            IntegrationHistoryKind::Merged,
            "hr-management",
            vec!["f12b301d7e5a4c9382b16f0d3c7e8459a1b2c3d4"],
            "master",
            Some("7a1f9c2e4b6d8f0a2c4e6b8d0f2a4c6e8b0d2f4a"),
            Some(409),
            "main.reviewer",
            Duration::days(2),
            "Inherited by master through a main-system branch merge. No separate promotion request exists for master.",
            Some("staging"),
        ),
        entry(
            "hist-hr-72b",
            IntegrationHistoryKind::Merged,
            "hr-management",
            vec!["72b1e4fa1b2c3d4e5f6071829304a5b6c7d8e9f0"],
            "development",
            Some("3c5e7a9b1d3f5a7c9e1b3d5f7a9c1e3b5d7f9a1c"),
            Some(388),
            "main.reviewer",
            Duration::days(12),
            "Merged into development.",
            None,
        ),
        entry(
            "hist-hr-b4e-revert",
            IntegrationHistoryKind::Reverted,
            "hr-management",
            vec!["b4e8a11f0c9d8e7f6a5b4c3d2e1f001122334455"],
            "staging",
            Some("2d4f6a8c0e2b4d6f8a0c2e4b6d8f0a2c4e6b8d0f"),
            Some(413),
            "main.reviewer",
            Duration::days(16),
            "Reverted via a revert PR. The original integration commit is still in history and is normalized as an event.",
            None,
        ),
        entry(
            "hist-hr-5f0-failed",
            IntegrationHistoryKind::Failed,
            "hr-management",
            vec!["5f0a2bc3d4e5f6071829304a5b6c7d8e9f001122"],
            "staging",
            None,
            Some(377),
            "alex.chen",
            Duration::days(5),
            "Promotion blocked by a patch conflict, then the PR was closed without merge. Not integrated.",
            None,
        ),
    ]
}
