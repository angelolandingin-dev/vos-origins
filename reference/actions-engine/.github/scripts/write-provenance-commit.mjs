#!/usr/bin/env node
// write-provenance-commit.mjs — build the standardized trailer block and create the commit.
//
// §8.1: every integration leaves a commit with structured trailers in the main repository's
// history. For multi-commit promotions there is ONE `Source-Commit:` line per source SHA,
// preserving order. `Promotion-Target` records the originally requested target for audit;
// current status is always derived from the queried branch's reachable history.
//
// §8.3: revert / restore events are also normalized here so history parsing has one shape.
//
// CANONICAL REVERT-MESSAGE REQUIREMENT (canonical vs. ordinary):
//   * CANONICAL — produced by `buildRevertMessage`, carrying a `Reverts-Integration: <sha>`
//     trailer (plus optional `Subsystem-ID:`/`Source-Commit:`). This is the preferred form and
//     is the only form that is self-describing without consulting other commits.
//   * ORDINARY GITHUB REVERT — the message GitHub writes when you click "Revert" on the PR:
//         Revert "integrate(hr-management): employee export"
//         <blank line>
//         This reverts commit <FULL_MAIN_INTEGRATION_COMMIT_SHA>.
//     `integration-history.mjs` recognizes this form and normalizes it into the same revert
//     event (resolved by the referenced commit SHA or the quoted integration subject). No
//     trailers are required for that path, but the quoted subject MUST be the integration
//     commit's subject verbatim (or the body MUST name the integration commit SHA), otherwise
//     the event cannot be tied to a known integration and is ignored rather than guessed.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { git } from './lib/git.mjs';

export const TRAILER_KEYS = ['Subsystem-ID', 'Source-Repo', 'Source-Commit', 'Promotion-Mode', 'Promotion-Target', 'Reverts-Integration', 'Restores-Integration'];

function sanitizeSubject(subject, max = 72) {
  const single = String(subject || 'integration').replace(/\s+/g, ' ').trim();
  return single.length > max ? `${single.slice(0, max - 1)}…` : single;
}

/** Build the integration commit message (subject + trailer block). */
export function buildProvenanceMessage({ subsystemId, sourceRepo, sourceCommits, mode, target, subject }) {
  if (!subsystemId) throw new Error('buildProvenanceMessage: subsystemId is required');
  if (!Array.isArray(sourceCommits) || sourceCommits.length === 0) throw new Error('buildProvenanceMessage: at least one source commit is required');
  const header = `integrate(${subsystemId}): ${sanitizeSubject(subject)}`;
  const trailerLines = [
    `Subsystem-ID: ${subsystemId}`,
    `Source-Repo: ${sourceRepo}`,
    ...sourceCommits.map((sha) => `Source-Commit: ${sha}`),
    `Promotion-Mode: ${mode}`,
    `Promotion-Target: ${target}`,
  ];
  return `${header}\n\n${trailerLines.join('\n')}\n`;
}

/** Build a revert-event message referencing the integration commit it reverses. */
export function buildRevertMessage({ subsystemId, sourceCommits, revertsIntegrationSha, subject }) {
  const header = `revert(integrate): undo ${sanitizeSubject(subject)}`;
  const trailerLines = [
    `Reverts-Integration: ${revertsIntegrationSha}`,
    `Subsystem-ID: ${subsystemId}`,
    ...(sourceCommits || []).map((sha) => `Source-Commit: ${sha}`),
  ];
  return `${header}\n\n${trailerLines.join('\n')}\n`;
}

/**
 * Parse trailers from a commit message. Repeated keys become arrays.
 *
 * Only the TRAILING CONSECUTIVE TRAILER BLOCK is considered (the same region
 * `git interpret-trailers` operates on): we start at the end of the message, skip trailing
 * blank lines, and collect consecutive `Key: value` lines upward until the first line that is
 * not a trailer. This prevents a mention such as `Source-Commit: deadbeef…` embedded in the
 * message body from creating phantom SHAs.
 */
export function parseTrailers(message) {
  const result = { trailers: {}, order: [] };
  const lines = String(message || '').split('\n');
  const isTrailerLine = (line) => /^([A-Za-z0-9-]+):\s*(.*)$/.test(line);
  let end = lines.length - 1;
  while (end >= 0 && lines[end].trim() === '') end -= 1;
  const block = [];
  for (let i = end; i >= 0; i -= 1) {
    if (!isTrailerLine(lines[i])) break;
    block.push(lines[i]);
  }
  block.reverse();
  const collected = [];
  for (const line of block) {
    const m = line.match(/^([A-Za-z0-9-]+):\s*(.*)$/);
    if (m && TRAILER_KEYS.includes(m[1])) collected.push([m[1], m[2].trim()]);
  }
  for (const [key, value] of collected) {
    if (!result.trailers[key]) {
      result.trailers[key] = [];
      result.order.push(key);
    }
    result.trailers[key].push(value);
  }
  return result;
}

/** True if `worktreeDir` has staged or unstaged changes. */
export function hasChanges(worktreeDir) {
  const out = git(worktreeDir, ['status', '--porcelain']);
  return out.trim().length > 0;
}

/**
 * Stage everything in the worktree and create the provenance commit.
 * Returns { sha, message }. Throws if there is nothing to commit.
 */
export function writeProvenanceCommit({ worktreeDir, message, authorName = 'vos-integration-bot', authorEmail = 'vos-integration-bot@users.noreply.github.com', allowEmpty = false, stageHook = null }) {
  git(worktreeDir, ['add', '-A']);
  if (typeof stageHook === 'function') stageHook(worktreeDir); // e.g. fix executable bits via update-index
  if (!allowEmpty && !hasChanges(worktreeDir)) {
    throw new Error('writeProvenanceCommit: no changes staged; refusing to record a no-op integration (see §6.3)');
  }
  const msgFile = path.join(os.tmpdir(), `vos-msg-${process.pid}-${Date.now()}.txt`);
  fs.writeFileSync(msgFile, message, 'utf8');
  try {
    git(worktreeDir, [
      '-c', `user.name=${authorName}`,
      '-c', `user.email=${authorEmail}`,
      'commit', '--no-verify', '--allow-empty', '-F', msgFile,
    ]);
  } finally {
    fs.rmSync(msgFile, { force: true });
  }
  const sha = git(worktreeDir, ['rev-parse', 'HEAD']).trim();
  return { sha, message };
}

// --- CLI ------------------------------------------------------------------------

function main() {
  const raw = fs.readFileSync(0, 'utf8').trim();
  const req = JSON.parse(raw);
  if (req.action === 'parse') {
    process.stdout.write(`${JSON.stringify(parseTrailers(req.message), null, 2)}\n`);
    return;
  }
  const message = buildProvenanceMessage(req);
  const out = writeProvenanceCommit({ worktreeDir: req.worktreeDir, message });
  process.stdout.write(`${JSON.stringify(out, null, 2)}\n`);
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
