// Content application for selected source deltas.
//
// Design: rather than generating a textual patch and hoping `git apply` accepts it, we take
// the source commit's *objects* (already present in the local object DB after fetching the
// source branch) and apply each file as a three-way merge:
//
//     base = source file at the commit's first parent (SHA^1)
//     theirs = source file at the selected commit (SHA)
//     ours = current target working-tree file
//
// This preserves binary content exactly, preserves executable modes, and correctly detects
// divergence: if both the target and the source changed the same file incompatibly we STOP
// (fail closed) instead of silently overwriting. Deletes and renames are handled explicitly.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { readBlob, pathMode } from './git.mjs';

export class ApplyConflictError extends Error {
  constructor(message, details = {}) {
    super(message);
    this.name = 'ApplyConflictError';
    this.details = details;
  }
}

function isBinary(buf) {
  if (!buf) return false;
  const len = Math.min(buf.length, 8000);
  for (let i = 0; i < len; i += 1) if (buf[i] === 0) return true;
  return false;
}

/**
 * Resolve `relPath` under `dir` and refuse if any component is a symlink.
 *
 * The promotion target is a Git worktree that may contain entries the engine does not fully
 * control. Following a symlinked parent directory (or a symlinked target file) would let a
 * write escape the worktree, so every existing component is checked with `lstatSync` and a
 * resolved-path containment check is applied as a second line of defence.
 */
function assertRealPathWithinWorktree(dir, relPath) {
  const root = path.resolve(dir);
  const abs = path.resolve(root, relPath);
  if (abs !== root && !abs.startsWith(root + path.sep)) {
    throw new ApplyConflictError(`Refusing to touch a path outside the worktree: ${relPath}`, { path: relPath, kind: 'path-escape' });
  }
  const segments = relPath.replace(/\\/g, '/').split('/').filter(Boolean);
  let current = root;
  for (const segment of segments) {
    current = path.join(current, segment);
    let stat = null;
    try {
      stat = fs.lstatSync(current);
    } catch {
      continue; // Component does not exist yet; nothing to follow.
    }
    if (stat.isSymbolicLink()) {
      throw new ApplyConflictError(`Refusing to traverse a symlink in the target worktree: ${relPath}`, { path: relPath, kind: 'symlink-traversal', component: segment });
    }
  }
}

function writeFile(dir, relPath, buffer) {
  assertRealPathWithinWorktree(dir, relPath);
  const abs = path.join(dir, relPath);
  fs.mkdirSync(path.dirname(abs), { recursive: true });
  fs.writeFileSync(abs, buffer);
}

function readWorktreeFile(dir, relPath) {
  assertRealPathWithinWorktree(dir, relPath);
  const abs = path.join(dir, relPath);
  if (!fs.existsSync(abs)) return null;
  if (fs.statSync(abs).isDirectory()) return null;
  return fs.readFileSync(abs);
}

function deleteWorktreeFile(dir, relPath) {
  assertRealPathWithinWorktree(dir, relPath);
  const abs = path.join(dir, relPath);
  if (fs.existsSync(abs)) fs.rmSync(abs, { recursive: false, force: true });
}

function buffersEqual(a, b) {
  if (a === null || b === null) return a === b;
  return Buffer.compare(a, b) === 0;
}

function threeWayMerge(objectsRepo, scratchDir, relPath, ours, base, theirs) {
  if (isBinary(ours) || isBinary(base) || isBinary(theirs)) {
    return { ok: false, conflict: true, reason: 'binary content cannot be three-way merged' };
  }
  const tag = relPath.replace(/[^a-zA-Z0-9._-]/g, '_');
  const oursFile = path.join(scratchDir, `${tag}.ours`);
  const baseFile = path.join(scratchDir, `${tag}.base`);
  const theirsFile = path.join(scratchDir, `${tag}.theirs`);
  fs.writeFileSync(oursFile, ours);
  fs.writeFileSync(baseFile, base);
  fs.writeFileSync(theirsFile, theirs);
  const result = spawnSync('git', ['merge-file', '-p', oursFile, baseFile, theirsFile], {
    maxBuffer: 256 * 1024 * 1024,
    windowsHide: true,
  });
  if (result.error) throw result.error;
  if (result.status < 0) {
    return { ok: false, conflict: true, reason: `git merge-file error (exit ${result.status})` };
  }
  if (result.status > 0) {
    return { ok: false, conflict: true, reason: `${result.status} merge conflict(s)`, merged: result.stdout };
  }
  return { ok: true, content: result.stdout };
}

/**
 * Apply one name-status delta entry into the target working tree.
 *
 * `scratchDir` is accepted for backwards compatibility but is IGNORED: merge scratch files are
 * always created in the OS temporary directory (never inside the worktree) and removed before
 * returning, so no `.ours`/`.base`/`.theirs` junk can ever be staged by `git add -A` (§6.3/§8.1).
 *
 * @returns {{ results: Array, modeOps: Array<{path:string, mode:string}> }}
 */
export function applyFileDelta({ objectsRepo, worktreeDir, sourceSha, baseSha, entry, scratchDir = null }) {
  const results = [];
  const modeOps = [];
  const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'vos-merge-'));

  const applyOne = (relPath, kind) => {
    // Fail closed before reading or writing anything if a symlink could redirect this path.
    assertRealPathWithinWorktree(worktreeDir, relPath);
    const mode = pathMode(objectsRepo, sourceSha, relPath);
    if (mode === '120000' || mode === '160000') {
      throw new ApplyConflictError(`Refusing to materialize a symlink/submodule entry: ${relPath}`, { path: relPath, mode });
    }
    const theirs = readBlob(objectsRepo, sourceSha, relPath);
    const base = baseSha ? readBlob(objectsRepo, baseSha, relPath) : null;
    const ours = readWorktreeFile(worktreeDir, relPath);

    if (theirs === null) {
      // Deletion in the source delta.
      if (ours === null) {
        results.push({ path: relPath, outcome: 'unchanged', reason: 'already absent' });
        return;
      }
      if (base !== null && buffersEqual(ours, base)) {
        deleteWorktreeFile(worktreeDir, relPath);
        results.push({ path: relPath, outcome: 'deleted' });
        return;
      }
      throw new ApplyConflictError(
        `Target file diverged from the source baseline; refusing to delete ${relPath}`,
        { path: relPath, kind: 'delete-diverge' },
      );
    }

    if (buffersEqual(ours, theirs)) {
      results.push({ path: relPath, outcome: 'unchanged', reason: 'target already matches source' });
      if (mode) modeOps.push({ path: relPath, mode });
      return;
    }

    if (ours === null) {
      writeFile(worktreeDir, relPath, theirs);
      results.push({ path: relPath, outcome: 'written', reason: base === null ? 'added' : 'restored-add' });
      if (mode) modeOps.push({ path: relPath, mode });
      return;
    }

    if (base === null) {
      throw new ApplyConflictError(
        `Target already contains a different file at ${relPath}; cannot apply an add over divergent content`,
        { path: relPath, kind: 'add-over-diverge' },
      );
    }

    // Fast path: the target is still byte-for-byte at the source baseline, so the source change
    // applies cleanly with no merge. This also covers BINARY files, which `git merge-file`
    // refuses, without ever reporting a false conflict (§6.4).
    if (buffersEqual(ours, base)) {
      writeFile(worktreeDir, relPath, theirs);
      results.push({ path: relPath, outcome: 'written', reason: 'target matched the source baseline; source change applied' });
      if (mode) modeOps.push({ path: relPath, mode });
      return;
    }

    const merged = threeWayMerge(objectsRepo, scratch, relPath, ours, base, theirs);
    if (!merged.ok) {
      throw new ApplyConflictError(`Three-way merge failed for ${relPath}: ${merged.reason}`, {
        path: relPath,
        kind: 'merge-conflict',
        reason: merged.reason,
      });
    }
    if (buffersEqual(merged.content, ours)) {
      results.push({ path: relPath, outcome: 'unchanged', reason: 'merge produced no net change' });
    } else {
      writeFile(worktreeDir, relPath, merged.content);
      results.push({ path: relPath, outcome: 'merged' });
    }
    if (mode) modeOps.push({ path: relPath, mode });
  };

  try {
    if (entry.status === 'D') {
      applyOne(entry.path, 'delete');
    } else if (entry.status === 'R' || entry.status === 'C') {
      applyOne(entry.oldPath, 'delete');
      applyOne(entry.newPath, 'add');
    } else {
      applyOne(entry.path, 'add');
    }
  } finally {
    fs.rmSync(scratch, { recursive: true, force: true });
  }

  return { results, modeOps };
}

/** Apply mode bits recorded during application (cross-platform safe). */
export function applyModeOps(worktreeDir, modeOps) {
  for (const { path: relPath, mode } of modeOps) {
    const chmod = mode === '100755' ? '+x' : '-x';
    spawnSync('git', ['-C', worktreeDir, 'update-index', `--chmod=${chmod}`, '--', relPath], { windowsHide: true });
  }
}
