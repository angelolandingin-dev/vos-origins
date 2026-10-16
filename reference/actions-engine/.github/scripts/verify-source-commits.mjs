#!/usr/bin/env node
// verify-source-commits.mjs — validate the requested source commit set.
//
// Guarantees:
//  * `commits` must be a JSON array of one or more FULL lowercase hex SHAs (40 or 64).
//    Anything else is rejected (fail closed). No abbreviated SHAs, no ranges, no refs.
//  * Duplicates are rejected.
//  * Every SHA must be REACHABLE from the subsystem's configured protected source branch.
//  * The user-supplied order must be a valid topological (ancestry-consistent) order.
//    A deliberate, displayed order is preserved; an impossible order is rejected.
//  * individual mode → first-parent delta (SHA^1..SHA) for each SHA.
//    cumulative mode → base_sha must be a verified ancestor of the single endpoint;
//    the delta is base_sha..endpoint.
//
// CLI: echo '{"repoDir":..., "sourceBaseRef":..., "commits":[...], "mode":"individual"}'
//      | node verify-source-commits.mjs

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  nameStatusDelta,
  nameStatusRange,
  commitMeta,
  firstParent,
  isAncestor,
  revParse,
} from './lib/git.mjs';
import { assertFullSha, PolicyError } from './lib/policy.mjs';

export class VerifyError extends Error {
  constructor(message, details = {}) {
    super(message);
    this.name = 'VerifyError';
    this.details = details;
  }
}

/** Parse the `commits` workflow input. Fail closed on anything that isn't a JSON SHA array. */
export function parseCommitInput(raw, { label = 'commits' } = {}) {
  if (typeof raw !== 'string' || raw.trim() === '') {
    throw new VerifyError(`${label} is required and must be a JSON array of full SHAs`);
  }
  let parsed;
  try {
    parsed = JSON.parse(raw);
  } catch (err) {
    throw new VerifyError(`${label} is not valid JSON: ${err.message}`);
  }
  if (!Array.isArray(parsed) || parsed.length === 0) {
    throw new VerifyError(`${label} must be a non-empty JSON array of full SHAs`);
  }
  const seen = new Set();
  for (const sha of parsed) {
    assertFullSha(sha, `${label}[]`);
    if (seen.has(sha)) throw new VerifyError(`${label} contains a duplicate SHA: ${sha}`);
    seen.add(sha);
  }
  return parsed.slice();
}

/** Sort a SHA set into a deterministic ancestry (topological) order. */
export function ancestryOrder(repoDir, shas) {
  const remaining = new Set(shas);
  const ordered = [];
  // Repeatedly take a commit that has no ancestor still remaining.
  let guard = shas.length * shas.length + 10;
  while (remaining.size > 0 && guard-- > 0) {
    for (const candidate of shas) {
      if (!remaining.has(candidate)) continue;
      const hasAncestorRemaining = [...remaining].some((other) => other !== candidate && isAncestor(repoDir, other, candidate));
      if (!hasAncestorRemaining) {
        ordered.push(candidate);
        remaining.delete(candidate);
        break;
      }
    }
  }
  if (remaining.size > 0) {
    throw new VerifyError('Could not establish an ancestry order for the selected commits', { remaining: [...remaining] });
  }
  return ordered;
}

/**
 * @param {object} req
 * @param {string} req.repoDir            object DB containing the fetched source history
 * @param {string} req.sourceBaseRef      verified ref for the protected source branch
 * @param {string[]} req.commits          requested full SHAs (order preserved)
 * @param {'individual'|'cumulative'} req.mode
 * @param {string} [req.baseSha]          required for cumulative
 */
export function verifyCommits(req) {
  const { repoDir, sourceBaseRef, commits, mode = 'individual', baseSha = null } = req;
  if (mode !== 'individual' && mode !== 'cumulative') {
    throw new VerifyError(`Unsupported mode: ${mode}`);
  }
  const baseTip = revParse(repoDir, sourceBaseRef);
  if (!baseTip) throw new VerifyError(`Source branch ref not found: ${sourceBaseRef}`);

  if (mode === 'cumulative') {
    if (commits.length !== 1) {
      throw new VerifyError('cumulative mode requires exactly one endpoint SHA in commits');
    }
    if (!baseSha) throw new VerifyError('cumulative mode requires base_sha');
    assertFullSha(baseSha, 'base_sha');
    const endpoint = commits[0];
    if (!revParse(repoDir, endpoint)) throw new VerifyError(`endpoint SHA not found in fetched history: ${endpoint}`);
    if (!isAncestor(repoDir, endpoint, baseTip)) {
      throw new VerifyError(`endpoint ${endpoint} is not reachable from source branch ${sourceBaseRef}`);
    }
    if (!isAncestor(repoDir, baseSha, baseTip)) {
      throw new VerifyError(`base_sha ${baseSha} is not reachable from source branch ${sourceBaseRef}`);
    }
    if (!isAncestor(repoDir, baseSha, endpoint)) {
      throw new VerifyError(`base_sha ${baseSha} is not an ancestor of ${endpoint}; cumulative range is invalid`);
    }
    const range = nameStatusRange(repoDir, baseSha, endpoint);
    return {
      ok: true,
      mode,
      sourceTip: baseTip,
      order: [baseSha, endpoint],
      ancestryOrder: [baseSha, endpoint],
      provenanceCommits: [endpoint],
      userOrderValid: true,
      cumulative: { baseSha, endpoint },
      plan: [
        {
          kind: 'cumulative',
          sha: endpoint,
          baseSha,
          subject: commitMeta(repoDir, endpoint).subject,
          meta: commitMeta(repoDir, endpoint),
          entries: range.entries,
        },
      ],
    };
  }

  for (const sha of commits) {
    if (!revParse(repoDir, sha)) throw new VerifyError(`SHA not found in fetched history: ${sha}`);
    if (!isAncestor(repoDir, sha, baseTip)) {
      throw new VerifyError(`SHA ${sha} is not reachable from the configured protected source branch (${sourceBaseRef})`);
    }
  }

  // Validate the caller's order is ancestry-consistent.
  for (let i = 0; i < commits.length; i += 1) {
    for (let j = i + 1; j < commits.length; j += 1) {
      if (isAncestor(repoDir, commits[j], commits[i])) {
        throw new VerifyError(
          `Commit order is not ancestry-consistent: ${commits[i]} is a descendant of ${commits[j]} but was listed first`,
          { orderedAncestry: ancestryOrder(repoDir, commits) },
        );
      }
    }
  }

  const plan = commits.map((sha) => {
    const { parent, entries } = nameStatusDelta(repoDir, sha);
    return {
      kind: 'individual',
      sha,
      baseSha: parent,
      subject: commitMeta(repoDir, sha).subject,
      meta: commitMeta(repoDir, sha),
      entries,
    };
  });

  return {
    ok: true,
    mode,
    sourceTip: baseTip,
    order: commits.slice(),
    ancestryOrder: ancestryOrder(repoDir, commits),
    provenanceCommits: commits.slice(),
    userOrderValid: true,
    plan,
  };
}

// --- CLI ------------------------------------------------------------------------

function main() {
  const raw = fs.readFileSync(0, 'utf8').trim();
  const req = JSON.parse(raw);
  const result = verifyCommits(req);
  process.stdout.write(`${JSON.stringify({ ...result, plan: result.plan.map((p) => ({ sha: p.sha, baseSha: p.baseSha, subject: p.subject, changedFiles: p.entries.length })) }, null, 2)}\n`);
}

const invokedDirectly = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedDirectly) {
  try {
    main();
  } catch (err) {
    process.stderr.write(`${err.name}: ${err.message}\n`);
    process.exitCode = 1;
  }
}
