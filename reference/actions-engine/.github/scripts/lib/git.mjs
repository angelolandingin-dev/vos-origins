// Git plumbing helpers. All git invocations use argument arrays (never shell-assembled
// strings), so filenames with spaces/unicode are safe. Binary content is read/written
// as Buffers.

import { execFileSync, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

export class GitError extends Error {
  constructor(args, status, stderr) {
    super(`git ${args.join(' ')} failed (exit ${status}): ${stderr.trim()}`);
    this.name = 'GitError';
    this.args = args;
    this.status = status;
    this.stderr = stderr;
  }
}

/** Run git in repoDir. Returns stdout (string by default, Buffer if opts.buffer). */
export function git(repoDir, args, opts = {}) {
  if (!repoDir) throw new Error('git(): repoDir is required');
  const fullArgs = ['-C', repoDir, ...args];
  const result = spawnSync('git', fullArgs, {
    input: opts.input,
    encoding: opts.buffer ? 'buffer' : 'utf8',
    maxBuffer: opts.maxBuffer ?? 256 * 1024 * 1024,
    windowsHide: true,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    const stderr = (result.stderr || Buffer.alloc(0)).toString('utf8');
    if (opts.allowFailure) return { status: result.status, stdout: result.stdout, stderr };
    throw new GitError(fullArgs, result.status, stderr);
  }
  if (opts.allowFailure) return { status: 0, stdout: result.stdout, stderr: '' };
  return result.stdout;
}

export function gitBuffer(repoDir, args, opts = {}) {
  return git(repoDir, args, { ...opts, buffer: true });
}

export function revParse(repoDir, rev, opts = {}) {
  try {
    return git(repoDir, ['rev-parse', '--verify', '--quiet', `${rev}^{commit}`], opts).trim();
  } catch {
    return null;
  }
}

export function refExists(repoDir, ref) {
  return revParse(repoDir, ref) !== null;
}

export function isAncestor(repoDir, ancestor, descendant) {
  if (!revParse(repoDir, ancestor) || !revParse(repoDir, descendant)) return false;
  const { status } = git(repoDir, ['merge-base', '--is-ancestor', ancestor, descendant], { allowFailure: true });
  return status === 0;
}

export function mergeBase(repoDir, a, b) {
  const out = git(repoDir, ['merge-base', a, b], { allowFailure: true });
  return out.status === 0 ? String(out.stdout).trim() : null;
}

export function commitSubject(repoDir, sha) {
  return git(repoDir, ['show', '-s', '--format=%s', sha]).trim();
}

export function commitBody(repoDir, sha) {
  return git(repoDir, ['show', '-s', '--format=%B', sha]);
}

export function commitMeta(repoDir, sha) {
  const fmt = '%H%x1f%P%x1f%an%x1f%ae%x1f%aI%x1f%cI%x1f%s';
  const raw = git(repoDir, ['show', '-s', `--format=${fmt}`, sha]);
  const [hash, parents, authorName, authorEmail, authoredAt, committedAt, subject] = raw.split('\x1f');
  return {
    sha: hash,
    parents: parents.split(' ').filter(Boolean),
    authorName,
    authorEmail,
    authoredAt,
    committedAt,
    subject: (subject || '').replace(/\n$/, ''),
  };
}

export function firstParent(repoDir, sha) {
  const parents = git(repoDir, ['rev-list', '--parents', '-n', '1', sha]).trim().split(' ').slice(1);
  return parents.length ? parents[0] : null;
}

/**
 * NUL-delimited name-status delta of a single commit vs its first parent.
 *
 * A root commit has no parent, so its delta is the ENTIRE tree (every path is an add). We use
 * `diff-tree --root` for that case: without it the delta would be empty and a root-commit
 * promotion would be falsely reported as `NOOP_PROMOTION` (§5.2).
 */
export function nameStatusDelta(repoDir, sha) {
  const parent = firstParent(repoDir, sha);
  if (!parent) {
    const buffer = git(repoDir, ['diff-tree', '--root', '--no-commit-id', '--name-status', '-r', '-z', '-M', sha], { buffer: true });
    return { parent: null, entries: parseNameStatusZ(buffer) };
  }
  return { parent, entries: parseNameStatusZ(git(repoDir, ['diff', '--name-status', '-z', '-M', parent, sha], { buffer: true })) };
}

/** NUL-delimited name-status delta between two arbitrary revisions (cumulative mode). */
export function nameStatusRange(repoDir, base, head) {
  return { parent: base, entries: parseNameStatusZ(git(repoDir, ['diff', '--name-status', '-z', '-M', base, head], { buffer: true })) };
}

function parseNameStatusZ(buffer) {
  const text = Buffer.isBuffer(buffer) ? buffer.toString('utf8') : buffer;
  const tokens = text.split('\0');
  const entries = [];
  for (let i = 0; i < tokens.length; i += 1) {
    const status = tokens[i];
    if (!status) continue;
    const letter = status[0];
    if (letter === 'R' || letter === 'C') {
      const oldPath = tokens[i + 1];
      const newPath = tokens[i + 2];
      entries.push({ status: letter, score: Number(status.slice(1)) || 0, oldPath, newPath });
      i += 2;
    } else {
      const filePath = tokens[i + 1];
      entries.push({ status: letter, path: filePath });
      i += 1;
    }
  }
  return entries;
}

/** Read raw blob bytes for `<sha>:<path>`; returns null if the path doesn't exist. */
export function readBlob(repoDir, sha, filePath) {
  const { status, stdout } = git(repoDir, ['cat-file', 'blob', `${sha}:${filePath}`], { buffer: true, allowFailure: true });
  if (status !== 0) return null;
  return stdout;
}

/** Returns '100644' | '100755' | '120000' | '160000' | null for the object mode. */
export function pathMode(repoDir, sha, filePath) {
  const { status, stdout } = git(repoDir, ['ls-tree', '-z', sha, '--', filePath], { allowFailure: true });
  if (status !== 0) return null;
  const text = String(stdout);
  if (!text) return null;
  const mode = text.split(' ')[0];
  return mode || null;
}

export function listTreePaths(repoDir, sha) {
  const out = git(repoDir, ['ls-tree', '-r', '-z', '--name-only', sha], { buffer: true });
  return Buffer.isBuffer(out) ? out.toString('utf8').split('\0').filter(Boolean) : out.split('\0').filter(Boolean);
}

export function listBranches(repoDir, { remote = false } = {}) {
  const args = remote ? ['branch', '-r', '--format=%(refname:short)'] : ['branch', '--format=%(refname:short)'];
  return git(repoDir, args).split('\n').map((l) => l.trim()).filter(Boolean);
}

export function addRemote(repoDir, name, url) {
  git(repoDir, ['remote', 'remove', name], { allowFailure: true });
  git(repoDir, ['remote', 'add', name, url]);
}

/**
 * Fetch a source branch tip into a quarantine ref. Fetching the branch tip makes the
 * entire reachable history (and therefore every selected SHA) available in this object DB.
 */
export function fetchSourceBranch(repoDir, { remoteName, url, sourceBranch }) {
  addRemote(repoDir, remoteName, url);
  const destRef = `refs/vos-sources/${remoteName}/${sourceBranch}`;
  git(repoDir, ['fetch', '--no-tags', '--force', url, `+refs/heads/${sourceBranch}:${destRef}`], { maxBuffer: 512 * 1024 * 1024 });
  return destRef;
}

/** Fetch an explicit SHA (used for cumulative/edge cases where the tip may differ). */
export function fetchSha(repoDir, url, sha) {
  git(repoDir, ['fetch', '--no-tags', '--force', url, sha], { allowFailure: true });
}

export function worktreeAdd(repoDir, worktreeDir, branchName, startPoint) {
  fs.mkdirSync(path.dirname(worktreeDir), { recursive: true });
  git(repoDir, ['worktree', 'add', '-f', '-b', branchName, worktreeDir, startPoint]);
}

export function worktreeRemove(repoDir, worktreeDir, { force = true } = {}) {
  git(repoDir, ['worktree', 'remove', ...(force ? ['--force'] : []), worktreeDir], { allowFailure: true });
}

export function repoRoot(dir) {
  try {
    return git(dir, ['rev-parse', '--show-toplevel']).trim();
  } catch {
    return null;
  }
}

/** `git show <rev>:<path>` text, or null. */
export function showFile(repoDir, rev, filePath) {
  const { status, stdout } = git(repoDir, ['show', `${rev}:${filePath}`], { buffer: true, allowFailure: true });
  if (status !== 0) return null;
  return stdout;
}

export function fileExistsAt(repoDir, rev, filePath) {
  const { status } = git(repoDir, ['cat-file', '-e', `${rev}:${filePath}`], { allowFailure: true });
  return status === 0;
}

export { execFileSync };
