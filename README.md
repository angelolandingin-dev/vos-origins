# VOS Origins

Desktop app for moving code between a main system repository and its subsystem
repositories, in both directions, without adding a single file to either one.

Promotion goes subsystem → trunk. Distribution goes trunk → subsystem. Both run
on your machine against your own clones and leave a commit or a pull request you
can review, amend, or throw away.

## What it is not

- No GitHub Actions workflow, no bot account, no server. The app does the work.
- No ledger file and no database in the target repositories. State is read from
  git history (provenance trailers) and open pull requests.
- Not a deployment tool. It writes commits and opens PRs; merging and releasing
  stay where they already are.

## Requirements

| Piece      | Version / note                                                                                         |
| ---------- | ------------------------------------------------------------------------------------------------------ |
| Windows    | 10 or 11, WebView2 runtime (installed by default)                                                      |
| Node.js    | 20.19+, 22.12+, or 24                                                                                  |
| Rust       | 1.100.0 — `rust-toolchain.toml` pins it, rustup installs it on demand                                  |
| Git        | any recent version on `PATH`                                                                           |
| GitHub CLI | `gh auth login --scopes repo`; the app reads the token scopes and shows a remedy when they are missing |

The app is Windows-only today. The Rust layer is portable enough to compile
elsewhere, but the release pipeline ships NSIS and MSI installers only.

## Quick start

```bash
cd vos-origins
npm install
npm run tauri dev
```

Build an installer:

```bash
npm run tauri build
```

Output lands in `src-tauri/target/release/bundle/`:
`VOS Origins_<version>_x64-setup.exe` (NSIS, the one auto-updates use) and
`VOS Origins_<version>_x64_en-US.msi`.

Opening the Vite dev server in a browser instead of the Tauri window shows a
"desktop backend unavailable" screen. That is expected: every screen reads from
the Rust command surface.

## Concepts

**Registry.** One JSON file at `%APPDATA%\vos-origins\registry.json`, bootstrapped
from `src-tauri/registry.json` on first run. It names the main repository, the
branches a promotion may target, and the subsystem entries (id, name, repository,
source branch, path strategy, dependency strategy, integration policy). The
Settings screen edits it; nothing about a repository is hardcoded.

**Checkout cache.** Disposable clones live under
`%APPDATA%\vos-origins\checkouts\`. A configured local path wins over a managed
clone, so you can point a subsystem at a working copy you already have. The cache
refreshes a branch only when the remote is ahead of it, and Settings has a button
to clear it.

**Provenance.** A promotion commit records where it came from in git trailers:

```
integrate(<subsystem>): <subject>

Subsystem-ID: hr-management
Source-Repo: organization/hr-management
Source-Commit: <sha>
Promotion-Mode: copy
Promotion-Target: staging
```

The app reads those trailers back to decide what is already integrated on a given
branch. There is no second source of truth.

**Delivery.** Promotion offers four modes, chosen per promotion:

| Mode                     | What happens                                                                  |
| ------------------------ | ----------------------------------------------------------------------------- |
| `local_direct` (default) | Commits onto the target branch in your clone, no push, working tree untouched |
| `local`                  | Writes the files and stops, so you can inspect and commit yourself            |
| `worktree`               | Does the work in a linked worktree and leaves it in place                     |
| `push_pr`                | Pushes a branch and opens a pull request                                      |

Distribution always uses a pull request, because it writes into someone else's
repository. The branch is `distribute-<trunk-branch>`, the base is the
subsystem's source branch, and an existing branch is reported as a blocker
instead of being reused.

## Screens

| Screen              | Job                                                                   |
| ------------------- | --------------------------------------------------------------------- |
| Commit Browser      | Source history of a subsystem, with per-branch integration status     |
| Subsystem Diff      | Files missing from, behind, or only present on the trunk              |
| Promote Commits     | Pick exact SHAs, confirm the target branch, run the promotion         |
| Distribute          | Pick trunk commits, then open PRs in the subsystems that consume them |
| Integration History | Merged, pending, reverted and restored promotions per branch          |
| Settings            | Registry, CLI status, cache, diagnostics, updates                     |

## Promotion rules worth knowing

- A promotion copies whole files from the newest selected commit that touched
  each path. Skipping a commit does not hide its edits to files another selected
  commit also touched; the skipped commit's edits to files nothing else touches
  are simply not written.
- Removals are honoured. A bundle that adds and then deletes the same file lands
  as a removal; a bundle whose net effect is nothing is refused rather than
  producing an empty commit.
- Excluding a path clears its blocker. Exclusions are the operator's answer to
  "I know, ship it anyway".
- Dependency changes only touch `package.json` sections the subsystem owns
  (`dependencies`, `devDependencies`, …). Scripts, workspaces, engines and
  overrides belong to the trunk and are never rewritten.
- Lockfiles are never generated. The commit message says which lockfile command
  to run, because regenerating one offline produces a lockfile nobody trusts.

## Distribution rules worth knowing

- The trunk commit list is first-parent only. Merge commits and side-branch work
  are visible through the commit that brought them in.
- Only paths the selected commits actually touched are offered. Paths that do not
  exist in the subsystem are skipped and listed with a reason instead of being
  invented, so a new directory arrives only when the subsystem's code imports
  something from it.
- Dependencies follow imports. A package the distributed files or the subsystem's
  own code import is written from the trunk's manifest; everything else is left
  alone. Range equality is checked semantically, so `^4.17.21` and `4.17.21` are
  not a change.
- An incompatible range is a warning, not a blocker: the trunk is the source of
  truth on this path. The subsystem's CI is where a real break surfaces.
- Always a PR, never a push to the base branch.

## Safety

Copied paths are resolved against the clone and refused when they escape it.
Symlinks, Windows junctions, submodules, workflow files, secret-looking files and
shared configs are classified before anything is written: some are blocked, some
need an explicit acknowledgement, some are deferred and named in the commit
message. A failed run leaves no half-written commit.

## Repository layout

```text
vos-origins/
├── src/                       React + TypeScript UI
│   ├── app/                   routing, providers, app context
│   ├── api/                   typed wrappers over the command surface
│   ├── components/            shared UI, promotion dialog, commit table
│   └── pages/                 one file per screen
└── src-tauri/
    ├── registry.json          bootstrap registry
    ├── deny.toml              dependency policy for cargo-deny
    └── src/
        ├── commands/          Tauri command surface (25 commands)
        ├── services/
        │   ├── engine/        promotion and distribution engines
        │   ├── live_read.rs   live reads: commits, diffs, statuses
        │   ├── checkouts.rs   disposable clone cache
        │   └── subsystem_diff.rs
        ├── github/            gh CLI detection and REST calls
        └── fixtures/          sample data, used only when VOS_USE_FIXTURES=1
```

## Development

```bash
npm run check        # typecheck, eslint, prettier, npm audit
npm run check:rust   # rustfmt --check, clippy -D warnings
npm run test:rust    # cargo test
npm run deny         # cargo-deny (install it: cargo install cargo-deny)
npm run check:all    # all of the above
```

`cargo test` builds disposable git repositories under the system temp directory.
Set `VOS_ENGINE_SANDBOX` to put them somewhere else. Nothing in the suite talks
to the network: GitHub access is stubbed.

The frontend talks to Rust through `src/api/invoke.ts`; every command returns
`ApiError` (`code`, `message`, `remedy`, `details`, `retryable`) instead of a bare
string, so screens can show an actionable error.

## Releasing

`.github/workflows/release.yml` fires on a tag matching `app-v*`, builds the
installers on a Windows runner, signs the update artifacts, and publishes a draft
release with `latest.json`.

```bash
# bump the version in src-tauri/tauri.conf.json (it drives both tag and updater)
git tag app-v0.1.2
git push origin app-v0.1.2
```

The updater reads
`https://github.com/<owner>/vos-origins/releases/latest/download/latest.json`, so
the repository has to be public and the release has to be published, not left as
a draft. Signing uses `TAURI_SIGNING_PRIVATE_KEY` (a repository secret in CI, a
local environment variable when building by hand). If that key is lost, installed
copies can no longer be updated and must be reinstalled manually.

Installs built before 0.1.1 predate the updater. Replace those once by hand.

## Troubleshooting

**"desktop backend unavailable"** — you opened the Vite URL in a browser. Use
`npm run tauri dev`.

**A run refuses with a blocker** — the message names the path and the reason.
Shared configs and workflow files need an explicit acknowledgement; secret-looking
files never ship. Excluding a path clears its blocker.

**A clone looks stale** — Settings → Storage → refresh or clear checkouts. The
cache is disposable; the app rebuilds it on demand.

**`gh` not found or unauthenticated** — the top bar shows the detected version and
the missing scopes. `gh auth login --scopes repo` is the fix.

**The commit button is disabled** — read the line under the confirmation checkbox.
It names the missing input, usually the commit message.

## Known limits

- Windows only for packaged builds.
- A rebuild from scratch takes several minutes and about 15 GB in
  `src-tauri/target`; delete it when you are not building.
- Distribution does not write lockfiles, so a subsystem whose CI runs `npm ci`
  stays red until someone commits the lockfile.
- `chrono` is used for fixture timestamps only and is flagged as unmaintained by
  `cargo-deny`; the advisory is pinned in `src-tauri/deny.toml` with its reason.
