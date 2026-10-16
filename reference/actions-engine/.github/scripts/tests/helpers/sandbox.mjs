// Disposable local git sandbox generator for the promotion engine tests.
//
// Creates real git repositories (source + main-system) under a disposable directory so the
// engine can be exercised end-to-end with genuine Git behavior: ancestry, first-parent
// deltas, renames/deletes/binaries, three-way merges, and trailer parsing.

import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

let NONCE = 0;
export function uniqueNonce(prefix = 'test') {
  NONCE += 1;
  return `${prefix}-${process.pid}-${Date.now()}-${NONCE}`;
}

export function sandboxRoot() {
  const base = process.env.VOS_ENGINE_SANDBOX || 'C:\\GithubRepos\\vos-web-integrated\\.tmp\\engine-sandbox';
  fs.mkdirSync(base, { recursive: true });
  return base;
}

export function makeTempDir(prefix = 'vos') {
  return fs.mkdtempSync(path.join(sandboxRoot(), `${prefix}-`));
}

export function g(dir, args, opts = {}) {
  return execFileSync('git', ['-C', dir, ...args], {
    encoding: 'utf8',
    windowsHide: true,
    maxBuffer: 256 * 1024 * 1024,
    ...opts,
  });
}

export function gOut(dir, args) {
  return g(dir, args).trim();
}

export function initRepo(dir, branch = 'master') {
  fs.mkdirSync(dir, { recursive: true });
  g(dir, ['init', '-q', '-b', branch]);
  g(dir, ['config', 'user.email', 'sandbox-bot@example.com']);
  g(dir, ['config', 'user.name', 'Sandbox Bot']);
  g(dir, ['config', 'commit.gpgsign', 'false']);
  g(dir, ['config', 'core.autocrlf', 'false']);
  g(dir, ['config', 'core.safecrlf', 'false']);
  return dir;
}

export function write(dir, rel, content) {
  const abs = path.join(dir, rel);
  fs.mkdirSync(path.dirname(abs), { recursive: true });
  fs.writeFileSync(abs, content);
}

export function commitAll(dir, message, { exec = [], allowEmpty = false } = {}) {
  g(dir, ['add', '-A']);
  for (const p of exec) g(dir, ['update-index', '--chmod=+x', '--', p]);
  const args = ['-c', 'user.name=Sandbox Bot', '-c', 'user.email=sandbox-bot@example.com', 'commit', '--no-verify', '-q', '-m', message];
  if (allowEmpty) args.push('--allow-empty');
  g(dir, args);
  return gOut(dir, ['rev-parse', 'HEAD']);
}

export function commitFiles(dir, message, files, opts = {}) {
  for (const [rel, content] of Object.entries(files)) write(dir, rel, content);
  return commitAll(dir, message, opts);
}

export function sha(dir, rev = 'HEAD') {
  return gOut(dir, ['rev-parse', rev]);
}

export function advanceRef(dir, branch, commit) {
  g(dir, ['update-ref', `refs/heads/${branch}`, commit]);
}

export function mergeInto(dir, branch, commit, message) {
  const current = gOut(dir, ['rev-parse', '--abbrev-ref', 'HEAD']);
  g(dir, ['checkout', '-q', branch]);
  g(dir, ['-c', 'user.name=Sandbox Bot', '-c', 'user.email=sandbox-bot@example.com', 'merge', '--no-ff', '-q', '-m', message, commit]);
  g(dir, ['checkout', '-q', current]);
  return sha(dir, branch);
}

// --- Registry -------------------------------------------------------------------

export function configObject(overrides = {}) {
  return {
    schemaVersion: 1,
    mainRepository: 'ORG/main-system',
    registryBranch: 'master',
    sourceBranchDefault: 'master',
    promotion: {
      allowedTargetBranches: ['development', 'staging', 'master'],
      allowedTargetBranchPatterns: ['release/*'],
      requirePullRequest: true,
      allowDirectPush: false,
      workflowFile: 'promote-subsystem.yml',
    },
    ...overrides,
  };
}

export function subsystemObject(overrides = {}) {
  return {
    schemaVersion: 1,
    id: 'hr-management',
    name: 'Human Resource Management',
    repository: 'ORG/hr-management',
    sourceBranch: 'master',
    enabled: true,
    pathStrategy: 'preserve-relative-paths',
    dependencyStrategy: 'reconcile',
    integrationPolicy: {
      applicationFiles: 'selected-commit-delta',
      packageJson: 'merge-dependency-delta',
      lockfiles: 'regenerate-in-target',
      sharedConfiguration: 'review-explicitly',
      githubWorkflows: 'main-owned',
      secretFiles: 'never-import',
    },
    ...overrides,
  };
}

export function writeRegistry(mainDir, { config = configObject(), subsystem = subsystemObject(), subsystemId = null } = {}) {
  const id = subsystemId || subsystem.id;
  write(mainDir, '.integration/config.json', `${JSON.stringify(config, null, 2)}\n`);
  write(mainDir, `.integration/subsystems/${id}.json`, `${JSON.stringify(subsystem, null, 2)}\n`);
}

// --- Fixtures -------------------------------------------------------------------

const BASE_MAIN_PKG = {
  name: 'main-system',
  version: '0.1.0',
  private: true,
  dependencies: { next: '^16.2.1', react: '19.2.3' },
  devDependencies: { typescript: '^5' },
};

const LOCKFILE = {
  name: 'main-system',
  version: '0.1.0',
  lockfileVersion: 3,
  requires: true,
  packages: {
    '': { name: 'main-system', version: '0.1.0', dependencies: { next: '^16.2.1', react: '19.2.3' } },
  },
};

/** Canonical source chain A-B-C-D (acceptance tests 1/4/8). */
export function createSourceChain(dir) {
  initRepo(dir, 'master');
  const A = commitFiles(dir, 'A: base', {
    'package.json': `${JSON.stringify({ name: 'hr-management', version: '1.0.0', dependencies: { next: '^16.2.1' } }, null, 2)}\n`,
    'src/modules/hr/index.ts': 'export const a = 1;\n',
  });
  const B = commitFiles(dir, 'B: unrelated change', { 'src/modules/hr/b.txt': 'B unrelated change\n' });
  const C = commitFiles(dir, 'C: employee export feature', {
    'src/modules/hr/c.txt': 'C feature\n',
    'src/modules/hr/export.ts': "import ExcelJS from 'exceljs';\nexport const exportEmployees = () => new ExcelJS.Workbook();\n",
    'package.json': `${JSON.stringify({ name: 'hr-management', version: '1.0.0', dependencies: { next: '^16.2.1', exceljs: '^4.4.0' } }, null, 2)}\n`,
  });
  const D = commitFiles(dir, 'D: later unrelated change', { 'src/modules/hr/d.txt': 'D unrelated change\n' });
  return { dir, A, B, C, D };
}

/** Main-system repo with registry + branches, matching the chain's A baseline. */
export function createMainRepo(dir, { config = configObject(), subsystem = subsystemObject(), extraDependencies = {}, extraFiles = {}, branches = ['staging', 'development'] } = {}) {
  initRepo(dir, 'master');
  const pkg = { ...BASE_MAIN_PKG, dependencies: { ...BASE_MAIN_PKG.dependencies, ...extraDependencies } };
  write(dir, 'package.json', `${JSON.stringify(pkg, null, 2)}\n`);
  write(dir, 'package-lock.json', `${JSON.stringify(LOCKFILE, null, 2)}\n`);
  write(dir, 'src/modules/hr/index.ts', 'export const a = 1;\n');
  write(dir, 'next.config.ts', 'export default {};\n');
  write(dir, 'tsconfig.json', '{\n  "compilerOptions": {}\n}\n');
  for (const [rel, content] of Object.entries(extraFiles)) write(dir, rel, content);
  writeRegistry(dir, { config, subsystem });
  const base = commitAll(dir, 'main: base');
  for (const b of branches) g(dir, ['branch', b]);
  return { dir, base };
}

/** Source repo exercising adds / modifies / deletes / rename-with-space / binary / unicode. */
export function createBulkSource(dir) {
  initRepo(dir, 'master');
  const A = commitFiles(dir, 'A: bulk base', {
    'package.json': `${JSON.stringify({ name: 'hr-management', dependencies: { next: '^16.2.1' } }, null, 2)}\n`,
    'src/old-name.txt': 'old content\n',
    'src/to-delete.txt': 'delete me\n',
    'src/keep.txt': 'keep\n',
  });
  const newFile = 'brand new\n';
  const renamed = 'renamed content\n';
  const imageBytes = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x00, 0x01, 0x02, 0xff, 0xfe]);
  write(dir, 'src/new-file.txt', newFile);
  fs.rmSync(path.join(dir, 'src/old-name.txt'));
  write(dir, 'src/renamed file.txt', renamed);
  fs.rmSync(path.join(dir, 'src/to-delete.txt'));
  write(dir, 'src/assets/image.png', imageBytes);
  write(dir, 'src/émigré.txt', 'unicode name\n');
  write(dir, 'scripts/run.sh', '#!/bin/sh\necho hi\n');
  write(dir, '.github/workflows/evil.yml', 'name: evil\non: push\n');
  write(dir, '.env.local', 'SECRET=super-secret\n');
  const B = commitAll(dir, 'B: bulk rename/delete/add/binary', { exec: ['scripts/run.sh'] });
  return { dir, A, B, newFile, renamed, imageBytes };
}

/** Main repo baseline that matches the bulk source's A commit. */
export function createBulkMain(dir) {
  initRepo(dir, 'master');
  write(dir, 'package.json', `${JSON.stringify(BASE_MAIN_PKG, null, 2)}\n`);
  write(dir, 'package-lock.json', `${JSON.stringify(LOCKFILE, null, 2)}\n`);
  write(dir, 'src/old-name.txt', 'old content\n');
  write(dir, 'src/to-delete.txt', 'delete me\n');
  write(dir, 'src/keep.txt', 'keep\n');
  writeRegistry(dir, {});
  const base = commitAll(dir, 'main: bulk base');
  g(dir, ['branch', 'staging']);
  return { dir, base };
}

/** Source chain where C2 imports a package added by the UNSELECTED B2. */
export function createMissingDepSource(dir) {
  initRepo(dir, 'master');
  const A = commitFiles(dir, 'A: base', {
    'package.json': `${JSON.stringify({ name: 'hr-management', dependencies: { next: '^16.2.1' } }, null, 2)}\n`,
    'src/modules/hr/index.ts': 'export const a = 1;\n',
  });
  const B = commitFiles(dir, 'B: add left-pad dependency', {
    'package.json': `${JSON.stringify({ name: 'hr-management', dependencies: { next: '^16.2.1', 'left-pad': '^1.3.0' } }, null, 2)}\n`,
  });
  const C = commitFiles(dir, 'C: use left-pad', {
    'src/modules/hr/use-leftpad.ts': "import leftPad from 'left-pad';\nexport const pad = () => leftPad('x', 5);\n",
  });
  return { dir, A, B, C };
}

// --- Engine driver --------------------------------------------------------------

export function integrationBranchCandidates(repoDir) {
  return g(repoDir, ['branch', '--format=%(refname:short)'])
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.startsWith('integration/'));
}

export function branchFile(repoDir, branch, rel) {
  const out = execFileSync('git', ['-C', repoDir, 'show', `${branch}:${rel}`], { windowsHide: true });
  return out;
}

export function branchFileText(repoDir, branch, rel) {
  return branchFile(repoDir, branch, rel).toString('utf8');
}

export function branchHas(repoDir, branch, rel) {
  const r = execFileSync('git', ['-C', repoDir, 'cat-file', '-e', `${branch}:${rel}`], { windowsHide: true, stdio: 'ignore' , encoding: 'utf8'});
  return r === '';
}

export function tryBranchHas(repoDir, branch, rel) {
  try {
    execFileSync('git', ['-C', repoDir, 'cat-file', '-e', `${branch}:${rel}`], { windowsHide: true, stdio: 'ignore' });
    return true;
  } catch {
    return false;
  }
}

export function treeMode(repoDir, branch, rel) {
  const out = g(repoDir, ['ls-tree', branch, '--', rel]).trim();
  return out ? out.split(/\s+/)[0] : null;
}

export function isAncestor(repoDir, a, b) {
  try {
    g(repoDir, ['merge-base', '--is-ancestor', a, b]);
    return true;
  } catch {
    return false;
  }
}

/** Commit a message (with optional files) onto a branch without leaving the worktree moved. */
export function commitMessageOnBranch(dir, branch, message, files = {}, { remove = [] } = {}) {
  const cur = gOut(dir, ['rev-parse', '--abbrev-ref', 'HEAD']);
  g(dir, ['checkout', '-q', branch]);
  for (const rel of remove) fs.rmSync(path.join(dir, rel), { force: true });
  for (const [rel, content] of Object.entries(files || {})) write(dir, rel, content);
  commitAll(dir, message, { allowEmpty: true });
  const s = sha(dir, branch);
  g(dir, ['checkout', '-q', cur]);
  return s;
}

/** Create a bare git repository to act as a local push remote. */
export function initBare(dir) {
  fs.mkdirSync(dir, { recursive: true });
  execFileSync('git', ['init', '--bare', '-q', dir], { windowsHide: true });
  return dir;
}

export function refIn(repoDir, ref) {
  try {
    return gOut(repoDir, ['rev-parse', '--verify', ref]);
  } catch {
    return null;
  }
}
