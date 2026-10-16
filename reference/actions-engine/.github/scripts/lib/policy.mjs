// Registry + policy loading and evaluation.
//
// Authority model (§4.2): the main repository's configured registryBranch is authoritative.
// The engine re-reads and re-validates policy from that branch on every run and never
// trusts inputs from the desktop control plane. If the promotion target differs from the
// registry branch, policy is read from the registry branch but patches are applied against
// the target branch tip.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { git, refExists, showFile, listBranches, revParse } from './git.mjs';
import { assertSchema, SchemaError } from './json-schema.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
export const SCHEMA_DIR = path.resolve(__dirname, '..', '..', '..', '.integration', 'schema');

export class PolicyError extends Error {
  constructor(message, code = 'POLICY_ERROR') {
    super(message);
    this.name = 'PolicyError';
    this.code = code;
  }
}

function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, 'utf8'));
}

function readSchema(name) {
  return readJson(path.join(SCHEMA_DIR, name));
}

/** True when running under CI (GitHub Actions or a generic CI flag). */
export function isCI(env = process.env) {
  return env.GITHUB_ACTIONS === 'true' || (typeof env.CI === 'string' && env.CI !== '' && env.CI !== 'false');
}

/** Resolve a usable revision for the registry branch, either local or fetched from origin. */
function resolveRegistryRef(mainRepoDir, registryBranch) {
  for (const candidate of [registryBranch, `refs/remotes/origin/${registryBranch}`, `origin/${registryBranch}`]) {
    if (refExists(mainRepoDir, candidate)) return candidate;
  }
  return null;
}

/** Best-effort `git fetch origin <registryBranch>` (no-op when there is no `origin`). */
function tryFetchRegistryBranch(mainRepoDir, registryBranch) {
  try {
    assertSafeBranchName(registryBranch, 'registryBranch');
  } catch {
    return false;
  }
  const remotes = git(mainRepoDir, ['remote'], { allowFailure: true });
  if (remotes.status !== 0 || !String(remotes.stdout).split('\n').map((r) => r.trim()).includes('origin')) return false;
  const fetched = git(mainRepoDir, ['fetch', '--no-tags', 'origin', `refs/heads/${registryBranch}:refs/remotes/origin/${registryBranch}`], { allowFailure: true });
  return fetched.status === 0;
}

/**
 * Fetch a file's content from a git ref, falling back to the working tree.
 * Returns { content, source } where source is `ref:<rev>` or 'worktree'.
 *
 * The registry branch is authoritative (§4.2). A missing local ref is therefore NOT silently
 * replaced by whichever branch the workflow happened to check out: we first try to fetch the
 * registry branch from `origin`, and if it is still unavailable we only fall back to the
 * working-tree copy outside CI. Under CI a missing ref (or a missing file inside the ref) fails
 * closed, so the workflow `--ref` can never masquerade as the registry revision.
 *
 * @param {{allowWorktreeFallback?: boolean|null}} [opts] Force-allow/deny the worktree fallback.
 */
export function readRegistryFile(mainRepoDir, registryBranch, relPath, opts = {}) {
  const { allowWorktreeFallback = null } = opts;
  if (registryBranch) {
    let ref = resolveRegistryRef(mainRepoDir, registryBranch);
    if (!ref && tryFetchRegistryBranch(mainRepoDir, registryBranch)) {
      ref = resolveRegistryRef(mainRepoDir, registryBranch);
    }
    if (ref) {
      const buf = showFile(mainRepoDir, ref, relPath);
      if (buf !== null) return { content: buf.toString('utf8'), source: `ref:${ref}` };
    }
    const fallbackAllowed = allowWorktreeFallback === true || (allowWorktreeFallback !== false && !isCI());
    if (!fallbackAllowed) {
      throw new PolicyError(
        ref
          ? `Registry file ${relPath} is missing from registry branch "${registryBranch}" (${ref}); refusing to fall back to the checked-out worktree under CI.`
          : `Registry branch "${registryBranch}" is not available locally and could not be fetched; refusing to use the checked-out worktree as the registry under CI.`,
        ref ? 'REGISTRY_FILE_MISSING' : 'REGISTRY_REF_MISSING',
      );
    }
  }
  const abs = path.join(mainRepoDir, relPath);
  if (fs.existsSync(abs)) {
    return { content: fs.readFileSync(abs, 'utf8'), source: 'worktree' };
  }
  throw new PolicyError(`Registry file not found: ${relPath} (registryBranch=${registryBranch})`, 'REGISTRY_MISSING');
}

function parseJsonFailClosed(content, label) {
  let parsed;
  try {
    parsed = JSON.parse(content);
  } catch (err) {
    throw new PolicyError(`${label} is not valid JSON: ${err.message}`, 'INVALID_JSON');
  }
  return parsed;
}

/** Load + validate config.json (fail closed). */
export function loadConfig(mainRepoDir, registryBranch = null) {
  const { content, source } = readRegistryFile(mainRepoDir, registryBranch, '.integration/config.json');
  const config = parseJsonFailClosed(content, '.integration/config.json');
  try {
    assertSchema(readSchema('config.schema.json'), config, '.integration/config.json');
  } catch (err) {
    throw new PolicyError(err.message, 'INVALID_CONFIG');
  }
  if (config.promotion.allowDirectPush !== false) {
    throw new PolicyError('allowDirectPush must be false (direct pushes are prohibited)', 'INVALID_CONFIG');
  }
  return { config, source };
}

/** Load + validate one subsystem registry entry (fail closed). */
export function loadSubsystem(mainRepoDir, registryBranch, subsystemId) {
  if (!/^[a-z0-9][a-z0-9-]{1,63}$/.test(subsystemId)) {
    throw new PolicyError(`Invalid subsystem id shape: ${JSON.stringify(subsystemId)}`, 'INVALID_SUBSYSTEM_ID');
  }
  const relPath = `.integration/subsystems/${subsystemId}.json`;
  const { content, source } = readRegistryFile(mainRepoDir, registryBranch, relPath);
  const subsystem = parseJsonFailClosed(content, relPath);
  try {
    assertSchema(readSchema('subsystem.schema.json'), subsystem, relPath);
  } catch (err) {
    throw new PolicyError(err.message, 'INVALID_SUBSYSTEM');
  }
  if (subsystem.id !== subsystemId) {
    throw new PolicyError(
      `${relPath} declares id="${subsystem.id}" but the filename/id requested was "${subsystemId}"`,
      'SUBSYSTEM_ID_MISMATCH',
    );
  }
  // Defence in depth (§11): the source branch is interpolated into a git fetch refspec, so
  // reject injection-shaped names here even though the GitHub ref namespace already forbids them.
  assertSafeBranchName(subsystem.sourceBranch, 'sourceBranch');
  return { subsystem, source };
}

// --- Branch-name and glob safety ------------------------------------------------

const BRANCH_UNSAFE = /[\s~^:?*[\\\x00-\x1f\x7f]/;
const SHA_RE = /^[0-9a-f]{40}$|^[0-9a-f]{64}$/;

/** Strict branch-name validation (defence against injection / ambiguous refs). */
export function assertSafeBranchName(name, label = 'branch') {
  if (typeof name !== 'string' || name.length === 0) {
    throw new PolicyError(`${label} must be a non-empty string`, 'INVALID_BRANCH');
  }
  if (name.length > 255) throw new PolicyError(`${label} is too long`, 'INVALID_BRANCH');
  if (name.startsWith('-') || name.startsWith('/') || name.endsWith('/') || name.endsWith('.')) {
    throw new PolicyError(`${label} has an illegal form: ${name}`, 'INVALID_BRANCH');
  }
  if (name.includes('..') || name.includes('@{') || name.includes('//')) {
    throw new PolicyError(`${label} has an illegal form: ${name}`, 'INVALID_BRANCH');
  }
  if (BRANCH_UNSAFE.test(name)) {
    throw new PolicyError(`${label} contains characters that are not permitted: ${name}`, 'INVALID_BRANCH');
  }
  return name;
}

export function assertFullSha(sha, label = 'sha') {
  if (typeof sha !== 'string' || !SHA_RE.test(sha)) {
    throw new PolicyError(`${label} must be a full 40/64-char lowercase hex SHA, got ${JSON.stringify(sha)}`, 'INVALID_SHA');
  }
  return sha;
}

/**
 * Safe glob matcher for allowedTargetBranchPatterns. Supports `*` (within a path
 * segment), `**` (across segments), and `?`. NOT regex — user input is never compiled
 * into a pattern. Anchored full-string match.
 */
export function matchGlob(pattern, name) {
  let re = '^';
  for (let i = 0; i < pattern.length; i += 1) {
    const ch = pattern[i];
    if (ch === '*') {
      if (pattern[i + 1] === '*') {
        re += '.*';
        i += 1;
      } else {
        re += '[^/]*';
      }
    } else if (ch === '?') {
      re += '[^/]';
    } else {
      re += ch.replace(/[.+^${}()|[\]\\]/g, '\\$&');
    }
  }
  re += '$';
  return new RegExp(re, 'u').test(name);
}

/** Is `branch` allowed by policy (exact list or safe glob pattern)? */
export function isTargetAllowed(config, branch) {
  const p = config.promotion;
  if ((p.allowedTargetBranches || []).includes(branch)) return { allowed: true, reason: 'listed' };
  for (const pattern of p.allowedTargetBranchPatterns || []) {
    if (matchGlob(pattern, branch)) return { allowed: true, reason: `pattern:${pattern}` };
  }
  return { allowed: false, reason: 'not in allowedTargetBranches or allowedTargetBranchPatterns' };
}

/** Evaluate whether a target branch exists and is authorized at execution time. */
export function evaluateTarget(mainRepoDir, config, targetBranch, { ghClient = null } = {}) {
  assertSafeBranchName(targetBranch, 'target_branch');
  const policy = isTargetAllowed(config, targetBranch);
  const existsLocally = !!revParse(mainRepoDir, `refs/heads/${targetBranch}`) || !!revParse(mainRepoDir, `refs/remotes/origin/${targetBranch}`);
  return { targetBranch, ...policy, existsLocally, existsRemotely: existsLocally ? true : null, ghClient };
}

export function anyBranchExists(mainRepoDir, branch) {
  const local = listBranches(mainRepoDir);
  if (local.includes(branch)) return true;
  const remote = listBranches(mainRepoDir, { remote: true });
  return remote.some((r) => r === `origin/${branch}` || r.endsWith(`/${branch}`));
}

export function resolveBranchRef(mainRepoDir, branch) {
  for (const candidate of [`refs/heads/${branch}`, `refs/remotes/origin/${branch}`, branch]) {
    const sha = revParse(mainRepoDir, candidate);
    if (sha) return { ref: candidate, sha };
  }
  return null;
}

export { SHA_RE };
