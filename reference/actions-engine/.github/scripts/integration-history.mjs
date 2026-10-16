#!/usr/bin/env node
// integration-history.mjs — derive per-branch integration status from Git ancestry.
//
// §8: there is NO ledger. Status comes from structured trailers on commits REACHABLE from
// the queried main-system branch, plus (separately) GitHub's open PRs for pending states.
//
// §8.3: integration / revert / restore events are replayed in chronological-topological
// order so a revert produces `Reverted` and a later re-promotion restores `Integrated`,
// while the ORIGINAL integration commit is never erased from history.
//
// Revert recognition covers BOTH forms:
//   * CANONICAL: a `Reverts-Integration: <sha>` trailer (see `buildRevertMessage`).
//   * ORDINARY GITHUB: `Revert "<integration subject>"` with a `This reverts commit <sha>.`
//     body — resolved against the integration commit SHA (or prefix) or the quoted subject.
// If neither can be tied to a known integration the event is ignored rather than guessed.
//
// Statuses are always per (subsystem, source SHA, target branch). Because we only walk
// commits reachable from the queried branch, a branch that INHERITED an integration commit
// through a main-system branch merge automatically reports `Integrated` too.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { git } from './lib/git.mjs';
import { parseTrailers } from './write-provenance-commit.mjs';
import { assertSafeBranchName, PolicyError } from './lib/policy.mjs';

const REC = '\x1e';
const FIELD = '\x1f';

/**
 * Recognise an ORDINARY GitHub revert commit:
 *   subject: Revert "integrate(hr-management): employee export"
 *   body:    This reverts commit <FULL_MAIN_INTEGRATION_COMMIT_SHA>.
 * Returns { sha, subject } (either may be null) or null when the subject is not a revert.
 */
export function parseOrdinaryGithubRevert(subject, message) {
  const s = String(subject || '');
  if (!/^Revert\s+"/.test(s)) return null;
  const m = s.match(/^Revert\s+"([\s\S]*)"\s*$/);
  const bodyMatch = String(message || '').match(/This reverts commit ([0-9a-fA-F]{7,64})\.?/);
  return { sha: bodyMatch ? bodyMatch[1].toLowerCase() : null, subject: m ? m[1] : null };
}

/** Extract `Source-Commit: <sha>` lines from any text (PR body, commit message, ...). */
export function extractSourceCommits(text) {
  const out = [];
  const re = /^[ \t>*-]*Source-Commit:\s*([0-9a-fA-F]{40,64})\s*$/gim;
  let m;
  while ((m = re.exec(String(text || ''))) !== null) out.push(m[1].toLowerCase());
  return out;
}

/** Walk the reachable history of `ref` oldest→newest and extract trailer events. */
export function collectEvents(repoDir, ref) {
  const fmt = `--format=%H${FIELD}%ct${FIELD}%cI${FIELD}%s${FIELD}%B${REC}`;
  const raw = git(repoDir, ['log', '--reverse', '--topo-order', fmt, ref], { maxBuffer: 512 * 1024 * 1024 });
  const chunks = raw.split(REC).map((c) => c.replace(/^\n+/, '')).filter((c) => c.trim().length > 0);
  const events = [];
  for (const chunk of chunks) {
    const [sha, ct, cIso, subject, message] = chunk.split(FIELD);
    const parsed = parseTrailers(message);
    const trailers = parsed.trailers;
    const base = { commit: sha, committedAt: Number(ct), committedIso: cIso, subject, trailers };
    if (trailers['Reverts-Integration'] && trailers['Reverts-Integration'].length) {
      events.push({
        type: 'revert',
        ...base,
        revertsIntegration: trailers['Reverts-Integration'][0],
        subsystem: (trailers['Subsystem-ID'] || [])[0] || null,
        sourceCommits: trailers['Source-Commit'] || [],
      });
    } else if (trailers['Restores-Integration'] && trailers['Restores-Integration'].length) {
      events.push({
        type: 'restore',
        ...base,
        restoresIntegration: trailers['Restores-Integration'][0],
        subsystem: (trailers['Subsystem-ID'] || [])[0] || null,
        sourceCommits: trailers['Source-Commit'] || [],
      });
    } else {
      // §8.3: an ordinary GitHub "Revert ..." PR carries no trailers. Normalise it as a revert
      // event referencing either the named integration commit SHA or the quoted subject.
      const ghRevert = parseOrdinaryGithubRevert(subject, message);
      if (ghRevert && (ghRevert.sha || ghRevert.subject)) {
        events.push({
          type: 'revert',
          ...base,
          revertsIntegration: ghRevert.sha,
          revertsSubject: ghRevert.subject,
          subsystem: (trailers['Subsystem-ID'] || [])[0] || null,
          sourceCommits: trailers['Source-Commit'] || [],
          ordinary: true,
        });
      } else if ((trailers['Subsystem-ID'] || []).length && (trailers['Source-Commit'] || []).length) {
        events.push({
          type: 'integrate',
          ...base,
          subsystem: trailers['Subsystem-ID'][0],
          sourceRepo: (trailers['Source-Repo'] || [])[0] || null,
          sourceCommits: trailers['Source-Commit'],
          mode: (trailers['Promotion-Mode'] || [])[0] || null,
          target: (trailers['Promotion-Target'] || [])[0] || null,
        });
      }
    }
  }
  return events;
}

/**
 * Replay events → status map keyed by `${subsystem}\x1f${sha}`.
 * Returns { statuses: Map, integrationIndex: Map(commit->{subsystem,sourceCommits}), events }.
 */
export function replayEvents(events) {
  const statuses = new Map();
  const integrationIndex = new Map();
  const history = [];

  for (const event of events) {
    if (event.type === 'integrate') {
      integrationIndex.set(event.commit, { subsystem: event.subsystem, sourceCommits: event.sourceCommits, target: event.target, subject: event.subject });
      for (const sha of event.sourceCommits) {
        const key = `${event.subsystem}${FIELD}${sha}`;
        statuses.set(key, {
          subsystem: event.subsystem,
          sha,
          status: 'integrated',
          integrationCommit: event.commit,
          promotionTarget: event.target,
          at: event.committedAt,
          revertedBy: null,
        });
      }
      history.push({ ...event });
    } else if (event.type === 'revert') {
      // Resolve the integration this revert undoes: exact SHA → abbreviated SHA prefix →
      // quoted integration subject (ordinary GitHub revert).
      let resolvedIntegration = event.revertsIntegration || null;
      let target = resolvedIntegration ? integrationIndex.get(resolvedIntegration) || null : null;
      if (!target && resolvedIntegration) {
        for (const [commit, info] of integrationIndex) {
          if (commit.startsWith(resolvedIntegration)) {
            target = info;
            resolvedIntegration = commit;
            break;
          }
        }
      }
      if (!target && event.revertsSubject) {
        for (const [commit, info] of integrationIndex) {
          if (info.subject === event.revertsSubject) {
            target = info;
            resolvedIntegration = commit;
            break;
          }
        }
      }
      const affected = target ? target.sourceCommits : event.sourceCommits;
      const subsystem = event.subsystem || (target && target.subsystem) || null;
      if (!target && (!subsystem || !affected.length)) {
        // Unresolvable ordinary revert (does not reference a known integration); ignore it
        // rather than fabricating a status change.
        continue;
      }
      for (const sha of affected) {
        if (!subsystem) continue;
        const key = `${subsystem}${FIELD}${sha}`;
        const prev = statuses.get(key);
        statuses.set(key, {
          subsystem,
          sha,
          status: 'reverted',
          integrationCommit: prev ? prev.integrationCommit : resolvedIntegration,
          promotionTarget: prev ? prev.promotionTarget : null,
          at: event.committedAt,
          revertedBy: event.commit,
          originalIntegration: prev ? prev.integrationCommit : resolvedIntegration,
        });
      }
      history.push({ ...event, revertsIntegration: resolvedIntegration, resolvedSubsystem: subsystem, resolvedSourceCommits: affected });
    } else if (event.type === 'restore') {
      const target = integrationIndex.get(event.restoresIntegration) || null;
      const affected = target ? target.sourceCommits : event.sourceCommits;
      const subsystem = event.subsystem || (target && target.subsystem) || null;
      for (const sha of affected) {
        if (!subsystem) continue;
        const key = `${subsystem}${FIELD}${sha}`;
        statuses.set(key, {
          subsystem,
          sha,
          status: 'integrated',
          integrationCommit: event.commit,
          promotionTarget: target ? target.target : null,
          at: event.committedAt,
          revertedBy: null,
          restoredFrom: event.restoresIntegration,
        });
      }
      history.push({ ...event, resolvedSubsystem: subsystem, resolvedSourceCommits: affected });
    }
  }
  return { statuses, integrationIndex, history };
}

function lookup(statuses, subsystem, sha) {
  return statuses.get(`${subsystem}${FIELD}${sha}`) || null;
}

/**
 * Compute branch-specific status for requested SHAs.
 * @returns {{ branch, subsystem, results: Array }}
 */
export function computeBranchStatus(repoDir, { branch, subsystem, shas }) {
  assertSafeBranchName(branch, 'branch');
  const ref = resolveRef(repoDir, branch);
  if (!ref) {
    return { branch, subsystem, ref: null, results: shas.map((sha) => ({ sha, status: 'Unknown' })) };
  }
  const events = collectEvents(repoDir, ref);
  const { statuses } = replayEvents(events);
  const results = shas.map((sha) => {
    const rec = lookup(statuses, subsystem, sha);
    if (!rec) return { sha, status: 'Eligible' };
    return {
      sha,
      status: rec.status === 'integrated' ? 'Integrated' : rec.status === 'reverted' ? 'Reverted' : 'Eligible',
      integrationCommit: rec.integrationCommit || null,
      promotionTarget: rec.promotionTarget || null,
      revertedBy: rec.revertedBy || null,
      at: rec.at || null,
    };
  });
  return { branch, subsystem, ref, results };
}

function resolveRef(repoDir, branch) {
  for (const candidate of [`refs/heads/${branch}`, `refs/remotes/origin/${branch}`, branch]) {
    const out = git(repoDir, ['rev-parse', '--verify', '--quiet', `${candidate}^{commit}`], { allowFailure: true });
    if (out.status === 0) return candidate;
  }
  return null;
}

/** Full integration history for a branch (optionally filtered by subsystem). */
export function listIntegrationHistory(repoDir, { branch, subsystem = null }) {
  assertSafeBranchName(branch, 'branch');
  const ref = resolveRef(repoDir, branch);
  if (!ref) throw new PolicyError(`Branch not found: ${branch}`, 'BRANCH_NOT_FOUND');
  const events = collectEvents(repoDir, ref);
  const replayed = replayEvents(events);
  const items = replayed.history
    .map((e) => ({
      type: e.type,
      commit: e.commit,
      committedIso: e.committedIso,
      subject: e.subject,
      subsystem: e.subsystem || e.resolvedSubsystem || null,
      sourceCommits: e.sourceCommits || e.resolvedSourceCommits || [],
      promotionTarget: e.target || null,
      revertsIntegration: e.revertsIntegration || null,
      restoresIntegration: e.restoresIntegration || null,
    }))
    .filter((e) => !subsystem || e.subsystem === subsystem);
  const active = [...replayed.statuses.values()].filter((s) => s.status === 'integrated' && (!subsystem || s.subsystem === subsystem));
  const reverted = [...replayed.statuses.values()].filter((s) => s.status === 'reverted' && (!subsystem || s.subsystem === subsystem));
  return { branch, ref, items, active, reverted };
}

/** Does an open PR reference any of the requested source SHAs (§8.3)? */
function prReferencesRequestedShas(pr, { subsystem, shas }) {
  const bodyShas = new Set(extractSourceCommits(pr && pr.body));
  if (shas.some((s) => bodyShas.has(s))) return true;
  // Fallback: the deterministic branch-name convention encodes every selected SHA.
  const head = String((pr && pr.head) || '');
  return shas.some((s) => head.includes(`${subsystem}-${s.slice(0, 12)}`));
}

/**
 * Duplicate / pending detection for a promotion request on a specific target branch (§8.3).
 *
 * Open PRs are matched against EACH requested SHA (via `Source-Commit:` metadata in the PR body,
 * falling back to the per-SHA branch-name convention) rather than a first-SHA prefix, so an open
 * PR for `[B, C]` blocks a new `[C]` request and vice versa.
 *
 * A PR-lookup failure is surfaced separately as `prLookupError` (the caller reports
 * `PR_LOOKUP_FAILED`) instead of being mislabeled as an already-open PR (§12).
 *
 * @param {object} ghClient - open PR lookup (may be a stub)
 */
export async function detectBranchDuplicates({ repoDir, branch, subsystem, shas, repoSlug, ghClient = null }) {
  const status = computeBranchStatus(repoDir, { branch, subsystem, shas });
  const activeShas = status.results.filter((r) => r.status === 'Integrated').map((r) => r.sha);
  const revertedShas = status.results.filter((r) => r.status === 'Reverted').map((r) => r.sha);

  let openPrs = [];
  let prLookupError = null;
  if (ghClient && repoSlug) {
    try {
      const prs = await ghClient.listOpenPullRequests(repoSlug, { base: branch });
      openPrs = (prs || []).filter((pr) => prReferencesRequestedShas(pr, { subsystem, shas }));
    } catch (err) {
      prLookupError = { message: String((err && err.message) || err), base: branch };
    }
  }
  return { status, activeShas, revertedShas, openPrs, prLookupError };
}

// --- CLI ------------------------------------------------------------------------

async function main(argv) {
  const args = parseArgs(argv);
  const repoDir = args.repo || process.cwd();
  if (args.branch && args.subsystem && args.commits) {
    const shas = JSON.parse(args.commits);
    const result = computeBranchStatus(repoDir, { branch: args.branch, subsystem: args.subsystem, shas });
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
    return;
  }
  if (args.branch) {
    const result = listIntegrationHistory(repoDir, { branch: args.branch, subsystem: args.subsystem || null });
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
    return;
  }
  process.stderr.write('usage: integration-history.mjs --branch <b> [--subsystem <id>] [--commits <json>] [--repo <dir>]\n');
  process.exitCode = 2;
}

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i].startsWith('--')) {
      const key = argv[i].slice(2);
      const next = argv[i + 1];
      if (next && !next.startsWith('--')) {
        out[key] = next;
        i += 1;
      } else out[key] = true;
    }
  }
  return out;
}

const invokedDirectly = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedDirectly) {
  main(process.argv.slice(2)).catch((err) => {
    process.stderr.write(`${err.name}: ${err.message}\n`);
    process.exitCode = 1;
  });
}
