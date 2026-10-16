#!/usr/bin/env node
// promote-subsystem.mjs — server-side promotion orchestrator (§6.1).
//
// Steps:
//   1. accept subsystem / commits / mode / base_sha / target_branch
//   2. read + validate approved registry/policy from the registry branch (fail closed)
//   3. validate allowlist, source repo, enabled, commit shape, ancestry, target existence,
//      branch policy, caller authorization
//   4. inspect the TARGET branch history + open PRs for duplicates / pending promotions
//   5. fetch the source repo read-only (source branch tip)
//   6. create a uniquely named temp integration branch at the CURRENT TARGET BRANCH TIP
//   7. extract selected commit deltas, classify paths, apply eligible changes
//   8. reconcile package.json (dependency delta only) + regenerate target lockfile
//   9. emit a machine-readable summary + a provenance commit
//  10. validate the result; on any failure preserve diagnostics and change nothing on the target
//  11. push integration branch + create/update a PR with base === target_branch
//
// Nothing is written to a protected branch: the only commits created are on the temporary
// integration branch in a throwaway worktree.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import {
  git,
  isAncestor,
  pathMode,
  fetchSourceBranch,
  worktreeAdd,
  worktreeRemove,
} from './lib/git.mjs';
import {
  loadConfig,
  loadSubsystem,
  evaluateTarget,
  resolveBranchRef,
  assertSafeBranchName,
  PolicyError,
} from './lib/policy.mjs';
import { verifyCommits, parseCommitInput } from './verify-source-commits.mjs';
import { classifyPath } from './handle-special-files.mjs';
import { applyFileDelta, applyModeOps, ApplyConflictError } from './lib/apply.mjs';
import { computeDependencyDelta, reconcileDependencies, checkImportPrerequisites } from './reconcile-dependencies.mjs';
import { buildProvenanceMessage, writeProvenanceCommit } from './write-provenance-commit.mjs';
import { computeBranchStatus, detectBranchDuplicates } from './integration-history.mjs';
import { createGitHubClient, authenticatedRemoteUrl } from './lib/gh.mjs';
import { createReport, blocker, warn, finalize, writeReport } from './lib/report.mjs';

const CODE_EXT = new Set(['.js', '.jsx', '.ts', '.tsx', '.mjs', '.cjs', '.mts', '.cts', '.vue', '.svelte']);

/** Deterministic prefix used both for the integration branch name and duplicate detection. */
export function integrationBranchPrefix({ subsystem, shas }) {
  return `integration/${subsystem}-${shas[0].slice(0, 12)}`;
}

export function integrationBranchName({ subsystem, shas, targetBranch, nonce }) {
  const safeTarget = String(targetBranch).replace(/[^A-Za-z0-9._-]/g, '-');
  return `${integrationBranchPrefix({ subsystem, shas })}-${safeTarget}-${nonce}`;
}

/**
 * Resolve the URL the integration branch is pushed to (§6.1 step 11).
 *
 * Production workflows use `actions/checkout` with `persist-credentials: false`, so the plain
 * `origin` URL has no credentials and a push to it fails. We therefore:
 *   1. honour an explicit target (`request.pushRemoteUrl`, or the client's `VOS_PUSH_TARGET`),
 *   2. otherwise build an AUTHENTICATED HTTPS URL from the bot token for the real client,
 *   3. and only fall back to the local `origin` URL for local/stub clients (tests).
 *
 * @returns {{ url: string|null, source: string }}
 */
export function resolvePushRemote({ request = {}, ghClient = null, repoSlug = null, repoDir = null, pushToken = null } = {}) {
  if (request.pushRemoteUrl) return { url: request.pushRemoteUrl, source: 'explicit' };
  if (ghClient && ghClient.pushTarget) return { url: ghClient.pushTarget, source: 'push-target' };
  if (ghClient && ghClient.mode === 'real') {
    const token = pushToken || ghClient.token || process.env.GITHUB_TOKEN || process.env.GH_TOKEN || null;
    if (!token) return { url: null, source: 'missing-token' };
    return { url: authenticatedRemoteUrl(repoSlug, token), source: 'authenticated-token' };
  }
  const urlOut = git(repoDir, ['remote', 'get-url', 'origin'], { allowFailure: true });
  const url = urlOut.status === 0 ? String(urlOut.stdout).trim() || null : null;
  return { url, source: url ? 'origin' : 'no-remote' };
}

function readText(file) {
  return fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : null;
}

function stringifyManifest(manifest, originalText) {
  const indentMatch = originalText && originalText.match(/\n([ \t]+)"/);
  const indent = indentMatch ? indentMatch[1] : '  ';
  return `${JSON.stringify(manifest, null, indent)}\n`;
}

/** Real lockfile regeneration for the target branch's package manager. */
export function defaultLockfileGenerator(worktreeDir) {
  const has = (name) => fs.existsSync(path.join(worktreeDir, name));
  let command;
  let manager;
  if (has('pnpm-lock.yaml')) {
    manager = 'pnpm';
    command = ['pnpm', 'install', '--lockfile-only'];
  } else if (has('yarn.lock')) {
    manager = 'yarn';
    command = ['yarn', 'install', '--mode', 'update-lockfile'];
  } else if (has('package-lock.json')) {
    manager = 'npm';
    command = ['npm', 'install', '--package-lock-only', '--ignore-scripts', '--no-audit', '--no-fund'];
  } else if (has('bun.lockb') || has('bun.lock')) {
    // §7 only defines npm/pnpm/yarn behaviour. Rather than silently skipping (which would ship a
    // stale lockfile next to a changed manifest) this is reported as an unsupported manager.
    return {
      manager: 'bun',
      skipped: true,
      supported: false,
      reason: 'bun lockfiles are not supported by the engine (npm/pnpm/yarn only); a human must regenerate and update the lockfile',
    };
  } else {
    return { manager: null, skipped: true, supported: false, reason: 'no target lockfile present' };
  }
  const result = spawnSync(command[0], command.slice(1), { cwd: worktreeDir, encoding: 'utf8', windowsHide: true, maxBuffer: 64 * 1024 * 1024 });
  return {
    manager,
    command: command.join(' '),
    ok: result.status === 0,
    status: result.status,
    output: `${result.stdout || ''}${result.stderr || ''}`.slice(-4000),
  };
}

/** Explicit stub used by local tests: records the call, never touches the network. */
export function stubLockfileGenerator(worktreeDir) {
  const has = (name) => fs.existsSync(path.join(worktreeDir, name));
  if (!has('pnpm-lock.yaml') && !has('yarn.lock') && !has('package-lock.json')) {
    if (has('bun.lockb') || has('bun.lock')) {
      return { manager: 'bun', stubbed: true, skipped: true, supported: false, note: 'STUB: bun lockfile is unsupported by the engine' };
    }
    return { manager: null, stubbed: true, skipped: true, supported: false, note: 'STUB: no target lockfile present' };
  }
  const manager = has('pnpm-lock.yaml') ? 'pnpm' : has('yarn.lock') ? 'yarn' : 'npm';
  return { manager, ok: true, stubbed: true, note: 'STUB: lockfile not regenerated (no network in local tests)' };
}

/** Classify a single delta entry; returns { status, classified } where status drives handling. */
export function classifyEntry(entry, policy, { objectsRepo, sha, baseSha }) {
  const statusOf = (filePath, mode) => classifyPath(filePath, { policy, mode });

  if (entry.status === 'D') {
    const mode = baseSha ? pathMode(objectsRepo, baseSha, entry.path) : null;
    const c = statusOf(entry.path, mode);
    let status = c.status;
    if (status === 'dependency-merge') status = 'rejected'; // deleting the root manifest is never automatic
    return { entry, classified: c, status };
  }
  if (entry.status === 'R' || entry.status === 'C') {
    const modeFrom = baseSha ? pathMode(objectsRepo, baseSha, entry.oldPath) : null;
    const modeTo = pathMode(objectsRepo, sha, entry.newPath);
    const from = statusOf(entry.oldPath, modeFrom);
    const to = statusOf(entry.newPath, modeTo);
    let status = to.status;
    if (from.status === 'rejected' || to.status === 'rejected') status = 'rejected';
    return { entry, classified: { from, to }, status };
  }
  const mode = pathMode(objectsRepo, sha, entry.path);
  const c = statusOf(entry.path, mode);
  return { entry, classified: c, status: c.status };
}

function manifestPair(objectsRepo, sha, baseSha) {
  const readPkg = (rev) => {
    try {
      const buf = git(objectsRepo, ['show', `${rev}:package.json`], { buffer: true, allowFailure: true });
      if (buf.status !== 0) return null;
      return JSON.parse(buf.stdout.toString('utf8'));
    } catch {
      return null;
    }
  };
  const newPkg = readPkg(sha);
  const oldPkg = baseSha ? readPkg(baseSha) : null;
  return { sha, oldPkg: oldPkg || {}, newPkg: newPkg || {} };
}

/**
 * Run a promotion. Returns a finalized report. Never throws for expected/policy conditions
 * (they become blockers); only unexpected programming errors propagate.
 */
export async function runPromotion(request, deps = {}) {
  const repoDir = request.mainRepoDir;
  const ghClient = deps.ghClient || createGitHubClient({ mode: 'stub' });
  const generateLockfile = deps.generateLockfile || defaultLockfileGenerator;
  const resolveSourceUrl = deps.resolveSourceUrl;
  const nonce = request.nonce || String(Date.now());
  const log = deps.log || (() => {});

  const report = createReport({
    request: {
      subsystem: request.subsystemId,
      commits: request.commits,
      mode: request.mode,
      baseSha: request.baseSha || null,
      targetBranch: request.targetBranch,
    },
    githubClientMode: ghClient.mode,
  });

  let worktreeDir = null;
  try {
    // --- 2/3. Registry & policy (fail closed) -----------------------------------
    let bootConfig;
    try {
      bootConfig = loadConfig(repoDir, null);
    } catch (err) {
      blocker(report, 'REGISTRY_INVALID', err.message);
      return finalize(report);
    }
    const registryBranch = request.registryBranchOverride || bootConfig.config.registryBranch;
    report.registry = { branch: registryBranch };
    try {
      const loaded = loadConfig(repoDir, registryBranch);
      report.registry.configSource = loaded.source;
      request.config = loaded.config;
    } catch (err) {
      blocker(report, 'REGISTRY_INVALID', err.message);
      return finalize(report);
    }
    const config = request.config;

    if (config.promotion.requirePullRequest !== true) {
      blocker(report, 'POLICY_PR_REQUIRED', 'promotion.requirePullRequest must be true; the engine never direct-pushes.');
    }
    if (config.promotion.allowDirectPush !== false) {
      blocker(report, 'POLICY_DIRECT_PUSH', 'promotion.allowDirectPush must be false.');
    }

    let subsystem;
    try {
      const loaded = loadSubsystem(repoDir, registryBranch, request.subsystemId);
      subsystem = loaded.subsystem;
      report.registry.subsystemSource = loaded.source;
    } catch (err) {
      blocker(report, err.code === 'REGISTRY_MISSING' ? 'SUBSYSTEM_NOT_REGISTERED' : 'SUBSYSTEM_INVALID', err.message);
      return finalize(report);
    }
    if (!subsystem.enabled) {
      blocker(report, 'SUBSYSTEM_DISABLED', `Subsystem "${subsystem.id}" is registered but disabled.`);
      return finalize(report);
    }
    report.subsystem = { id: subsystem.id, repository: subsystem.repository, sourceBranch: subsystem.sourceBranch };

    // --- 4a. Target branch policy + existence -----------------------------------
    let target;
    try {
      assertSafeBranchName(request.targetBranch, 'target_branch');
      target = evaluateTarget(repoDir, config, request.targetBranch);
    } catch (err) {
      blocker(report, 'TARGET_INVALID', err.message);
      return finalize(report);
    }
    if (!target.allowed) {
      blocker(report, 'TARGET_NOT_ALLOWED', `Target branch "${request.targetBranch}" is not permitted by policy (${target.reason}).`);
      return finalize(report);
    }
    const targetRef = resolveBranchRef(repoDir, request.targetBranch);
    if (!targetRef) {
      const remote = await ghClient.getBranch(config.mainRepository, request.targetBranch);
      if (!remote || remote.exists !== true) {
        blocker(report, 'TARGET_NOT_FOUND', `Target branch "${request.targetBranch}" does not exist locally or remotely.`);
        return finalize(report);
      }
      blocker(report, 'TARGET_NOT_LOCAL', `Target branch "${request.targetBranch}" exists remotely but is not available locally for patching.`);
      return finalize(report);
    }
    report.target = { branch: request.targetBranch, tip: targetRef.sha, ref: targetRef.ref };

    // --- 3b. Caller authorization (best effort) ---------------------------------
    if (request.actor) {
      const authorized = await ghClient.isAuthorizedUser(config.mainRepository, request.actor);
      if (authorized === false) {
        blocker(report, 'UNAUTHORIZED_CALLER', `Actor "${request.actor}" is not authorized to dispatch promotions on ${config.mainRepository}.`);
        return finalize(report);
      }
    }

    // --- 4b. Duplicate / pending detection (target-branch specific) -------------
    const dupes = await detectBranchDuplicates({
      repoDir,
      branch: request.targetBranch,
      subsystem: subsystem.id,
      shas: request.commits,
      repoSlug: config.mainRepository,
      ghClient,
    });
    report.duplicates = { active: dupes.activeShas, reverted: dupes.revertedShas, openPrs: dupes.openPrs.length };
    if (dupes.activeShas.length) {
      blocker(report, 'DUPLICATE_ACTIVE', `SHA(s) already integrated on "${request.targetBranch}": ${dupes.activeShas.join(', ')}.`, { shas: dupes.activeShas });
      return finalize(report);
    }
    if (dupes.prLookupError) {
      blocker(report, 'PR_LOOKUP_FAILED', `Could not list open pull requests for base "${request.targetBranch}"; refusing to guess whether a promotion is already pending (${dupes.prLookupError.message}).`, dupes.prLookupError);
      return finalize(report);
    }
    if (dupes.openPrs.length) {
      blocker(report, 'PR_ALREADY_OPEN', `An open integration PR already targets "${request.targetBranch}" for these SHA(s).`, { prs: dupes.openPrs });
      return finalize(report);
    }

    // --- 5. Fetch source (read-only) --------------------------------------------
    let sourceUrl;
    if (typeof resolveSourceUrl === 'function') {
      sourceUrl = resolveSourceUrl(subsystem);
    } else if (request.sourceUrlOverride) {
      sourceUrl = request.sourceUrlOverride;
    } else if (ghClient.mode === 'real') {
      sourceUrl = authenticatedRemoteUrl(subsystem.repository, deps.sourceToken || process.env.SOURCE_READ_TOKEN);
    } else {
      blocker(report, 'SOURCE_URL_UNRESOLVED', 'No source URL resolver configured for a non-real GitHub client; refusing to guess a remote.');
      return finalize(report);
    }
    report.source = { repository: subsystem.repository, url: String(sourceUrl).replace(/x-access-token:[^@]+@/, 'x-access-token:***@') };

    const sourceRef = fetchSourceBranch(repoDir, { remoteName: subsystem.id, url: sourceUrl, sourceBranch: subsystem.sourceBranch });

    // --- 3c + 5b. Verify selected commits (shape, ancestry, order, deltas) ------
    let verification;
    try {
      verification = verifyCommits({
        repoDir,
        sourceBaseRef: sourceRef,
        commits: request.commits,
        mode: request.mode,
        baseSha: request.baseSha || null,
      });
    } catch (err) {
      blocker(report, 'COMMIT_VERIFICATION_FAILED', err.message, err.details || {});
      return finalize(report);
    }
    report.verification = { mode: verification.mode, order: verification.order, ancestryOrder: verification.ancestryOrder };

    // --- 7. Build + DISPLAY the deliberate plan ---------------------------------
    const plan = verification.plan.map((p) => {
      const entries = p.entries.map((entry) => {
        const { status, classified } = classifyEntry(entry, subsystem.integrationPolicy, {
          objectsRepo: repoDir,
          sha: p.sha,
          baseSha: p.baseSha,
        });
        return { status: entry.status, path: entry.path || entry.newPath, oldPath: entry.oldPath || null, classification: status, detail: classified };
      });
      return { sha: p.sha, baseSha: p.baseSha, subject: p.subject, entries };
    });
    report.plan = plan;

    // Pre-flight blockers from classification (fail closed BEFORE creating a worktree).
    const sharedConfigEntries = [];
    for (const p of plan) {
      for (const e of p.entries) {
        if (e.classification === 'rejected') {
          blocker(report, 'UNSAFE_PATH', `Rejected unsafe path "${e.path}": ${e.detail.reason || e.detail.status || 'unsafe'}`);
        } else if (e.classification === 'explicit-review') {
          sharedConfigEntries.push(e.path);
          if (!request.allowSharedConfig) {
            blocker(report, 'SHARED_CONFIG_REQUIRES_REVIEW', `Shared/global config changed: "${e.path}". Requires explicit review (set allowSharedConfig to proceed).`);
          }
        }
      }
    }
    if (sharedConfigEntries.length && request.allowSharedConfig) {
      warn(report, 'SHARED_CONFIG_EXPLICITLY_ALLOWED', `Shared config applied under explicit approval: ${sharedConfigEntries.join(', ')}`);
    }
    if (report.blockers.length) return finalize(report);

    // --- 6. Worktree at the target tip ------------------------------------------
    const branchName = integrationBranchName({
      subsystem: subsystem.id,
      shas: request.commits,
      targetBranch: request.targetBranch,
      nonce,
    });
    worktreeDir = path.join(os.tmpdir(), `vos-integration-${nonce}`);
    worktreeAdd(repoDir, worktreeDir, branchName, targetRef.sha);
    report.integrationBranch = { name: branchName, startPoint: targetRef.sha };
    log(`integration branch ${branchName} created at target tip ${targetRef.sha}`);

    // --- 7b. Apply the deltas (in deliberate order) -----------------------------
    const modeOps = [];
    const lockfileTouched = new Set();
    const pkgPairs = [];
    for (const p of plan) {
      for (const e of p.entries) {
        const rawEntry = e.oldPath ? { status: e.status, oldPath: e.oldPath, newPath: e.path } : { status: e.status, path: e.path };
        switch (e.classification) {
          case 'apply': {
            const { results, modeOps: ops } = applyFileDelta({
              objectsRepo: repoDir,
              worktreeDir,
              sourceSha: p.sha,
              baseSha: p.baseSha,
              entry: rawEntry,
            });
            modeOps.push(...ops);
            for (const r of results) {
              if (r.outcome === 'written') report.changes.applied.push({ path: r.path, reason: r.reason });
              else if (r.outcome === 'merged') report.changes.merged.push({ path: r.path });
              else if (r.outcome === 'deleted') report.changes.deleted.push({ path: r.path });
            }
            break;
          }
          case 'dependency-merge':
            pkgPairs.push(manifestPair(repoDir, p.sha, p.baseSha));
            break;
          case 'regenerate':
            lockfileTouched.add(path.basename(e.path));
            break;
          case 'main-owned':
            report.changes.mainOwnedSkipped.push({ path: e.path, reason: 'main-owned; not overwritten' });
            warn(report, 'MAIN_OWNED_SKIPPED', `Not imported (main-owned): ${e.path}`);
            break;
          case 'secret-excluded':
            report.changes.secretExcluded.push({ path: e.path });
            warn(report, 'SECRET_EXCLUDED', `Not imported (secret-looking): ${e.path}`);
            break;
          case 'explicit-review': {
            // Only reachable when request.allowSharedConfig === true.
            const { results, modeOps: ops } = applyFileDelta({
              objectsRepo: repoDir,
              worktreeDir,
              sourceSha: p.sha,
              baseSha: p.baseSha,
              entry: rawEntry,
            });
            modeOps.push(...ops);
            report.changes.applied.push(...results.filter((r) => r.outcome === 'written' || r.outcome === 'merged').map((r) => ({ path: r.path, reason: 'shared-config (explicitly approved)' })));
            break;
          }
          default:
            break;
        }
      }
    }

    // --- 8. Dependency reconciliation -------------------------------------------
    const manifestPath = path.join(worktreeDir, 'package.json');
    const originalManifestText = readText(manifestPath);
    const targetManifest = originalManifestText ? JSON.parse(originalManifestText) : {};
    let depsChanged = false;
    let deltaNames = [];
    if (pkgPairs.length) {
      const deltas = pkgPairs.map((pair) => computeDependencyDelta(pair.oldPkg, pair.newPkg));
      const reconciled = reconcileDependencies({ deltas, targetManifest, allowRemovals: !!request.allowRemovals });
      report.dependencies = {
        added: reconciled.added,
        updated: reconciled.updated,
        removed: reconciled.removed,
        moved: reconciled.moved,
        review: reconciled.review,
        noop: reconciled.noop,
      };
      for (const b of reconciled.blockers) blocker(report, b.code, b.message, b.details);
      if (reconciled.blockers.length) return finalize(report);
      depsChanged = reconciled.added.length + reconciled.updated.length + reconciled.removed.length + reconciled.moved.length > 0;
      if (depsChanged) fs.writeFileSync(manifestPath, stringifyManifest(reconciled.manifest, originalManifestText), 'utf8');
      deltaNames = [...reconciled.added.map((a) => a.name), ...reconciled.moved.map((m) => m.name)];
    }

    // Import prerequisite heuristic (runs even when the selected commit does NOT touch
    // package.json — that is exactly the "missing dependency commit" case, §7).
    {
      const codeFiles = [];
      for (const p of plan) {
        for (const e of p.entries) {
          if (e.classification !== 'apply') continue;
          const pth = e.path;
          if (!CODE_EXT.has(path.extname(pth).toLowerCase())) continue;
          const buf = git(repoDir, ['show', `${p.sha}:${pth}`], { buffer: true, allowFailure: true });
          if (buf.status !== 0) continue;
          codeFiles.push({ path: pth, content: buf.stdout.toString('utf8') });
        }
      }
      const missing = checkImportPrerequisites({ files: codeFiles, targetManifest, deltaAddedNames: deltaNames });
      for (const m of missing) {
        blocker(
          report,
          'MISSING_DEPENDENCY_PREREQUISITE',
          `Selected commit(s) import "${m.name}" which is neither in the target manifest nor added by the selected delta. An unselected prerequisite commit (e.g. one that added "${m.name}") likely must be selected explicitly.`,
          { package: m.name, refs: m.refs },
        );
      }
      if (report.blockers.length) return finalize(report);
    }

    // --- 8b. Lockfile regeneration ----------------------------------------------
    const shouldRegenerate = depsChanged || lockfileTouched.size > 0;
    if (shouldRegenerate) {
      const gen = generateLockfile(worktreeDir, { subsystem, targetBranch: request.targetBranch });
      report.lockfile = gen;
      if (gen && gen.ok === false && !gen.skipped) {
        blocker(report, 'LOCKFILE_REGENERATION_FAILED', `Lockfile regeneration failed (${gen.manager || 'unknown'}): ${(gen.output || '').slice(0, 800)}`);
        return finalize(report);
      }
      if (gen && gen.skipped) {
        // §6.4/§7: never silently skip a lockfile that had to be regenerated.
        blocker(
          report,
          'LOCKFILE_MANAGER_UNSUPPORTED',
          `The selected delta changes dependencies but the target lockfile could not be regenerated (${gen.manager || 'unknown manager'}): ${gen.reason || 'unsupported or absent target lockfile'}.`,
          { manager: gen.manager || null, reason: gen.reason || null },
        );
        return finalize(report);
      }
      report.changes.regenerated.push(gen.manager || 'none');
    }

    // --- No-op guard (§6.3) ------------------------------------------------------
    const anyChange =
      report.changes.applied.length + report.changes.merged.length + report.changes.deleted.length > 0 ||
      depsChanged ||
      (shouldRegenerate && report.lockfile && !report.lockfile.skipped && !report.lockfile.stubbed);
    if (!anyChange) {
      blocker(report, 'NOOP_PROMOTION', 'The selected commit delta produced no applicable changes; refusing to record a no-op integration.');
      return finalize(report);
    }

    // --- 9. Provenance commit ---------------------------------------------------
    const lastPlan = plan[plan.length - 1];
    const message = buildProvenanceMessage({
      subsystemId: subsystem.id,
      sourceRepo: subsystem.repository,
      sourceCommits: verification.provenanceCommits || verification.order,
      mode: verification.mode,
      target: request.targetBranch,
      subject: lastPlan.subject,
    });
    report.provenance = { message };
    const commit = writeProvenanceCommit({ worktreeDir, message, stageHook: (dir) => applyModeOps(dir, modeOps) });
    report.provenance.commit = commit.sha;
    report.integrationBranch.head = commit.sha;
    log(`provenance commit ${commit.sha} on ${branchName}`);

    // --- 10. Post-commit sanity: branch must be ahead of target, not touching it -
    if (!isAncestor(repoDir, targetRef.sha, commit.sha)) {
      blocker(report, 'INTEGRITY_BRANCH_ANCESTRY', 'Integration commit is not descended from the target tip; aborting.');
      return finalize(report);
    }

    // --- 11. Push + PR (base === target_branch) --------------------------------
    const repoSlug = request.repoSlugOverride || config.mainRepository;
    if (request.push !== false) {
      try {
        const resolved = resolvePushRemote({ request, ghClient, repoSlug, repoDir, pushToken: deps.pushToken });
        if (!resolved.url) {
          blocker(report, 'PUSH_FAILED', `No authenticated push target is available (${resolved.source}); refusing to push to an unauthenticated remote.`);
          return finalize(report);
        }
        report.push = { ...ghClient.pushBranch({ repoDir, branch: branchName, remoteUrl: resolved.url }), source: resolved.source };
      } catch (err) {
        blocker(report, 'PUSH_FAILED', err.message);
        return finalize(report);
      }
    }

    const prBody = buildPrBody({ report, subsystem, verification, targetBranch: request.targetBranch, branchName });
    if (request.createPr !== false) {
      try {
        const pr = await ghClient.createPullRequest(repoSlug, {
          head: branchName,
          base: request.targetBranch,
          title: `integrate(${subsystem.id}): ${lastPlan.subject}`,
          body: prBody,
        });
        report.pullRequest = pr;
      } catch (err) {
        blocker(report, 'PR_CREATION_FAILED', err.message);
        return finalize(report);
      }
    }

    report.result = 'promoted';
    return finalize(report);
  } catch (err) {
    if (err instanceof ApplyConflictError) {
      blocker(report, 'PATCH_CONFLICT', err.message, err.details);
    } else if (err instanceof PolicyError) {
      blocker(report, err.code || 'POLICY_ERROR', err.message);
    } else {
      blocker(report, 'ENGINE_ERROR', `${err.name}: ${err.message}`);
    }
    return finalize(report);
  } finally {
    if (worktreeDir) {
      try {
        worktreeRemove(repoDir, worktreeDir, { force: true });
      } catch {
        /* best effort cleanup */
      }
    }
  }
}

export function buildPrBody({ report, subsystem, verification, targetBranch, branchName }) {
  const lines = [];
  lines.push(`## Subsystem promotion: \`${subsystem.id}\``);
  lines.push('');
  lines.push(`Integration branch \`${branchName}\` → base **\`${targetBranch}\`** (PR base equals the requested target branch, not the workflow \`--ref\`).`);
  lines.push('');
  lines.push(`**Mode:** ${verification.mode} · **Source repo:** ${subsystem.repository} · **Source branch:** ${subsystem.sourceBranch}`);
  lines.push('');
  lines.push('### Source commits (in applied order)');
  for (const sha of verification.order) lines.push(`- \`${sha}\``);
  lines.push('');
  // Machine-readable metadata block: `integration-history.mjs` reads these `Source-Commit:`
  // lines to match an open PR against EACH requested source SHA (§8.3), not just the first.
  lines.push(`Subsystem-ID: ${subsystem.id}`);
  lines.push(`Source-Repo: ${subsystem.repository}`);
  for (const sha of verification.order) lines.push(`Source-Commit: ${sha}`);
  lines.push(`Promotion-Target: ${targetBranch}`);
  lines.push('');
  if (report.dependencies && (report.dependencies.added.length || report.dependencies.updated.length || report.dependencies.removed.length || report.dependencies.moved.length)) {
    lines.push('### Dependency changes');
    for (const d of report.dependencies.added) lines.push(`- added \`${d.name}\` ${d.range}`);
    for (const d of report.dependencies.updated) lines.push(`- updated \`${d.name}\`: ${d.from} → ${d.to}`);
    for (const d of report.dependencies.moved) lines.push(`- moved \`${d.name}\`: ${d.fromSection} → ${d.toSection} (${d.range})`);
    for (const d of report.dependencies.removed) lines.push(`- removed \`${d.name}\` (was ${d.previousRange})`);
    lines.push('');
  }
  if (report.changes.mainOwnedSkipped.length) {
    lines.push('### Not imported (main-owned)');
    for (const f of report.changes.mainOwnedSkipped) lines.push(`- \`${f.path}\``);
    lines.push('');
  }
  if (report.changes.secretExcluded.length) {
    lines.push('### Not imported (secret-looking)');
    for (const f of report.changes.secretExcluded) lines.push(`- \`${f.path}\``);
    lines.push('');
  }
  lines.push('> This PR only integrates the explicitly selected source commits. Unselected earlier/later commits are never imported automatically.');
  return lines.join('\n');
}

// --- CLI ------------------------------------------------------------------------

export function envBool(name, def = false) {
  const v = process.env[name];
  if (v === undefined) return def;
  const normalized = String(v).trim().toLowerCase();
  return normalized === '1' || normalized === 'true';
}

async function main() {
  const mainRepoDir = process.env.GITHUB_WORKSPACE || process.cwd();
  const mode = (process.env.PROMOTION_MODE || 'individual').trim();
  const commits = parseCommitInput(process.env.SOURCE_COMMITS_JSON);
  const request = {
    mainRepoDir,
    subsystemId: process.env.SOURCE_SUBSYSTEM,
    commits,
    mode,
    baseSha: process.env.BASE_SHA || null,
    targetBranch: process.env.TARGET_BRANCH,
    actor: process.env.GITHUB_ACTOR || null,
    allowSharedConfig: envBool('VOS_ALLOW_SHARED_CONFIG'),
    allowRemovals: envBool('VOS_ALLOW_REMOVALS'),
    registryBranchOverride: process.env.VOS_REGISTRY_BRANCH || null,
    sourceUrlOverride: process.env.VOS_SOURCE_URL || null,
    push: envBool('VOS_PUSH', true),
    createPr: envBool('VOS_CREATE_PR', true),
    nonce: process.env.VOS_NONCE || `${Date.now()}`,
  };
  const ghMode = process.env.VOS_GH_MODE || (process.env.GITHUB_ACTIONS ? 'real' : 'stub');
  const ghClient = createGitHubClient({
    mode: ghMode,
    token: process.env.GITHUB_TOKEN || process.env.GH_TOKEN,
    pushTarget: process.env.VOS_PUSH_TARGET || null,
  });
  const useStubLockfile = envBool('VOS_STUB_LOCKFILE', ghMode !== 'real');

  const report = await runPromotion(request, {
    ghClient,
    generateLockfile: useStubLockfile ? stubLockfileGenerator : defaultLockfileGenerator,
    log: (m) => process.stderr.write(`[engine] ${m}\n`),
  });

  writeReport(report, {
    filePath: process.env.VOS_REPORT_PATH || null,
    stepSummaryPath: process.env.GITHUB_STEP_SUMMARY || null,
  });
  process.exitCode = report.ok ? 0 : 1;
}

const invokedDirectly = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedDirectly) {
  main().catch((err) => {
    process.stderr.write(`FATAL ${err.name}: ${err.message}\n${err.stack || ''}\n`);
    process.exitCode = 1;
  });
}
