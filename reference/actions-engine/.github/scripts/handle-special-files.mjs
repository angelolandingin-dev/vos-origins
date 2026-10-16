#!/usr/bin/env node
// handle-special-files.mjs — path classification for integration.
//
// Paths are classified BEFORE any content is written to the target tree, per §6.3/§6.4.
// This module is pure (no git/filesystem access beyond reading stdin in CLI mode) so it can
// be unit-tested directly. Executable/rename/delete/binary *application* lives in
// lib/apply.mjs; here we only decide policy.
//
// Classification result actions:
//   apply            — subsystem-owned application file: apply the selected delta
//   dependency-merge — package.json: merge the dependency delta only
//   regenerate       — lockfile: never copy; regenerate for the target
//   main-owned       — .github/workflows/**: never auto-overwritten
//   secret-excluded  — .env*/secret-looking: never imported
//   explicit-review  — shared/global config: controlled merge or human stop
//   rejected         — unsafe path (traversal / absolute / symlink / submodule)
//
// CLI:
//   node handle-special-files.mjs --policy <subsystem.json> --paths-file <newline list>
//   node handle-special-files.mjs --policy <file>   # reads JSON { paths: [...] } from stdin

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const DEFAULT_POLICY = {
  applicationFiles: 'selected-commit-delta',
  packageJson: 'merge-dependency-delta',
  lockfiles: 'regenerate-in-target',
  sharedConfiguration: 'review-explicitly',
  githubWorkflows: 'main-owned',
  secretFiles: 'never-import',
};

const LOCKFILE_NAMES = new Set([
  'package-lock.json',
  'npm-shrinkwrap.json',
  'yarn.lock',
  'pnpm-lock.yaml',
  'bun.lockb',
  'bun.lock',
]);

// Shared / global configuration that must not be blindly overwritten (§6.4).
const SHARED_CONFIG_PATTERNS = [
  /^next\.config\.[a-z]+$/i,
  /^tsconfig(\.[a-z0-9-]+)?\.json$/i,
  /^jsconfig(\.[a-z0-9-]+)?\.json$/i,
  /^postcss\.config\.[a-z]+$/i,
  /^eslint\.config\.[a-z]+$/i,
  /^\.eslintrc(\.[a-z0-9-]+)?$/i,
  /^tailwind\.config\.[a-z]+$/i,
  /^vite\.config\.[a-z]+$/i,
  /^webpack\.config\.[a-z]+$/i,
  /^jest\.config\.[a-z]+$/i,
  /^vitest\.config\.[a-z]+$/i,
  /^babel\.config\.[a-z]+$/i,
  /^\.babelrc(\.[a-z0-9-]+)?$/i,
  /^\.prettierrc(\.[a-z0-9-]+)?$/i,
  /^prettier\.config\.[a-z]+$/i,
  /^components\.json$/i,
  /^\.editorconfig$/i,
  /^\.nvmrc$/i,
  /^\.node-version$/i,
];

const SECRET_FILE_PATTERNS = [
  /^\.env(\..*)?$/i, // .env, .env.local, .env.production ...
  /^\.npmrc$/i,
  /^\.pypirc$/i,
  /^\.netrc$/i,
  /^id_(rsa|dsa|ecdsa|ed25519)(\.pub)?$/i,
  /^\.htpasswd$/i,
  /(^|[._-])secrets?([._-]|$)/i,
  /(^|[._-])credentials?([._-]|$)/i,
  /(^|[._-])tokens?([._-]|$)/i,
  /service[-_]?account.*\.json$/i,
];

const SECRET_EXTENSIONS = new Set(['.pem', '.p12', '.pfx', '.key', '.keystore', '.jks', '.asc']);

const GITHUB_OWNED_PREFIXES = ['.github/workflows/', '.github/actions/'];
const GITHUB_OWNED_FILES = new Set(['.github/CODEOWNERS', '.github/dependabot.yml']);

/** Normalize to a repo-relative POSIX path, or report why it is unsafe. */
export function inspectPathSafety(rawPath) {
  if (typeof rawPath !== 'string' || rawPath.length === 0) {
    return { safe: false, reason: 'empty path' };
  }
  if (rawPath.includes('\0')) return { safe: false, reason: 'path contains NUL byte' };
  let p = rawPath.replace(/\\/g, '/');
  if (p.startsWith('/')) return { safe: false, reason: 'absolute path' };
  if (/^[a-zA-Z]:/.test(p)) return { safe: false, reason: 'drive-letter path' };
  const segments = p.split('/');
  if (segments.some((s) => s === '..')) return { safe: false, reason: 'path traversal ("..")' };
  if (segments.some((s) => s === '')) return { safe: false, reason: 'empty path segment' };
  while (p.startsWith('./')) p = p.slice(2);
  if (p === '.' || p === '') return { safe: false, reason: 'empty normalized path' };
  return { safe: true, path: p };
}

function isSharedConfig(basename) {
  return SHARED_CONFIG_PATTERNS.some((re) => re.test(basename));
}

function isSecret(basename) {
  const ext = path.extname(basename).toLowerCase();
  if (SECRET_EXTENSIONS.has(ext)) return true;
  return SECRET_FILE_PATTERNS.some((re) => re.test(basename));
}

/**
 * Classify a single repo-relative path.
 * @param {string} rawPath
 * @param {{policy?: object, mode?: string|null}} [opts] mode = git object mode ("120000" symlink, "160000" submodule)
 */
export function classifyPath(rawPath, opts = {}) {
  const policy = { ...DEFAULT_POLICY, ...(opts.policy || {}) };
  const safety = inspectPathSafety(rawPath);
  if (!safety.safe) {
    return { path: rawPath, category: 'unsafe', action: 'rejected', status: 'rejected', reason: safety.reason };
  }
  const filePath = safety.path;

  if (opts.mode === '120000') {
    return { path: filePath, category: 'unsafe', action: 'rejected', status: 'rejected', reason: 'symlink entry is not accepted' };
  }
  if (opts.mode === '160000') {
    return { path: filePath, category: 'unsafe', action: 'rejected', status: 'rejected', reason: 'git submodule/gitlink entry is not accepted' };
  }

  const basename = filePath.split('/').pop();

  if (LOCKFILE_NAMES.has(basename)) {
    return { path: filePath, category: 'lockfile', action: policy.lockfiles, status: 'regenerate', reason: 'lockfile is regenerated for the target, never copied' };
  }
  if (filePath === 'package.json') {
    if (policy.packageJson === 'explicit-review') {
      return { path: filePath, category: 'package-json', action: 'explicit-review', status: 'explicit-review', reason: 'registry policy requires explicit review of package.json' };
    }
    return { path: filePath, category: 'package-json', action: 'dependency-merge', status: 'dependency-merge', reason: 'only the dependency delta is merged' };
  }
  if (filePath.endsWith('/package.json')) {
    return { path: filePath, category: 'package-json', action: 'explicit-review', status: 'explicit-review', reason: 'nested/workspace manifest requires explicit review (only the root manifest is reconciled)' };
  }
  if (GITHUB_OWNED_PREFIXES.some((prefix) => filePath.startsWith(prefix)) || GITHUB_OWNED_FILES.has(filePath)) {
    return { path: filePath, category: 'github-owned', action: policy.githubWorkflows, status: 'main-owned', reason: 'main-owned: never automatically overwritten' };
  }
  if (isSecret(basename)) {
    return { path: filePath, category: 'secret', action: policy.secretFiles, status: 'secret-excluded', reason: 'secret-looking file is never imported automatically' };
  }
  if (isSharedConfig(basename)) {
    return { path: filePath, category: 'shared-config', action: policy.sharedConfiguration, status: 'explicit-review', reason: 'shared/global config requires controlled merge or explicit review' };
  }
  return { path: filePath, category: 'application', action: 'apply', status: 'apply', reason: 'subsystem-owned application file' };
}

/**
 * Classify a list of delta entries. Each entry is { status, path } or
 * { status, oldPath, newPath } (rename/copy). Returns an array of classified entries
 * keyed by the *effective* path(s).
 */
export function classifyEntries(entries, opts = {}) {
  const out = [];
  for (const entry of entries) {
    if (entry.status === 'R' || entry.status === 'C') {
      const from = classifyPath(entry.oldPath, { ...opts, mode: null });
      const to = classifyPath(entry.newPath, { ...opts, mode: opts.newMode ?? null });
      out.push({ ...entry, classified: { from, to }, status_: to.status === 'apply' && from.status === 'apply' ? 'apply' : to.status });
    } else {
      const classified = classifyPath(entry.path, { ...opts, mode: entry.mode ?? null });
      out.push({ ...entry, classified, status_: classified.status });
    }
  }
  return out;
}

/** Group classified entries into a policy summary the orchestrator reports on. */
export function summarizeClassifications(classified) {
  const groups = {
    apply: [],
    'dependency-merge': [],
    regenerate: [],
    'main-owned': [],
    'secret-excluded': [],
    'explicit-review': [],
    rejected: [],
  };
  for (const entry of classified) {
    const status = entry.status_ || 'rejected';
    (groups[status] || (groups[status] = [])).push(entry);
  }
  return groups;
}

// --- CLI ------------------------------------------------------------------------

function main(argv) {
  const args = parseArgs(argv);
  let policy = DEFAULT_POLICY;
  if (args.policy) {
    const raw = JSON.parse(fs.readFileSync(args.policy, 'utf8'));
    policy = raw.integrationPolicy ? { ...DEFAULT_POLICY, ...raw.integrationPolicy } : { ...DEFAULT_POLICY, ...raw };
  }
  let paths = [];
  if (args['paths-file']) {
    paths = fs.readFileSync(args['paths-file'], 'utf8').split('\n').map((l) => l.trim()).filter(Boolean);
  } else {
    const stdin = fs.readFileSync(0, 'utf8').trim();
    if (stdin) {
      const parsed = JSON.parse(stdin);
      paths = Array.isArray(parsed) ? parsed : parsed.paths || [];
    }
  }
  const result = paths.map((p) => classifyPath(p, { policy }));
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
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
      } else {
        out[key] = true;
      }
    }
  }
  return out;
}

const invokedDirectly = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedDirectly) {
  main(process.argv.slice(2));
}
