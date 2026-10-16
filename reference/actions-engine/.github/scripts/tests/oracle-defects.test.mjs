// Regression tests for the 13 concrete defects found by the Oracle review of the promotion
// engine. Each test is named for the finding it locks down.
//
// Findings → tests:
//   1  scratch files never committed          → "integration commit contains NO scratch files"
//   2  authenticated push URL                 → "push target is authenticated (token path)"
//   3  ordinary GitHub revert                 → "ordinary GitHub revert → Reverted, re-promotion allowed"
//   4  root-commit delta                      → "root-commit promotion imports the expected files"
//   5  binary modify where target == base     → "binary modify where target == base overwrites"
//   6  per-SHA open-PR matching               → "subset open PR blocks a new request"
//   7  symlinked target parent                → "symlinked target parent directory is rejected"
//   8  workflow_dispatch approvals            → "shared-config approval is grantable via workflow input"
//   9  PR lookup failure + pagination         → "PR-lookup failure yields PR_LOOKUP_FAILED" / "paginates"
//  10  registry ref missing in CI             → "registry ref missing under CI fails closed"
//  11  unsupported lockfile manager           → "unsupported lockfile manager emits the blocker"
//  12  trailer block scoping                  → "embedded Source-Commit does NOT create phantom trailers"
//  13  unsafe sourceBranch                    → "unsafe sourceBranch is rejected"

import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  runPromotion,
  stubLockfileGenerator,
  defaultLockfileGenerator,
  resolvePushRemote,
  envBool,
} from '../promote-subsystem.mjs';
import { createGitHubClient, authenticatedRemoteUrl } from '../lib/gh.mjs';
import { computeBranchStatus, collectEvents, extractSourceCommits } from '../integration-history.mjs';
import { parseTrailers } from '../write-provenance-commit.mjs';
import { nameStatusDelta } from '../lib/git.mjs';
import { applyFileDelta, ApplyConflictError } from '../lib/apply.mjs';
import { loadConfig, loadSubsystem, isCI, PolicyError } from '../lib/policy.mjs';
import {
  createSourceChain,
  createMainRepo,
  makeTempDir,
  uniqueNonce,
  initRepo,
  commitFiles,
  commitAll,
  commitMessageOnBranch,
  advanceRef,
  sha,
  g,
  gOut,
  tryBranchHas,
  branchFile,
  branchFileText,
  initBare,
  refIn,
  configObject,
  subsystemObject,
  writeRegistry,
  write,
} from './helpers/sandbox.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, '..', '..', '..');
const WORKFLOW_FILE = path.join(REPO_ROOT, '.github', 'workflows', 'promote-subsystem.yml');

const p = (...parts) => path.join(makeTempDir('oracle'), ...parts);

const BASE_PKG = {
  name: 'main-system',
  version: '0.1.0',
  private: true,
  dependencies: { next: '^16.2.1', react: '19.2.3' },
  devDependencies: { typescript: '^5' },
};

async function promote(source, main, overrides = {}) {
  const ghClient = overrides.ghClient || createGitHubClient({ mode: 'stub' });
  const request = {
    mainRepoDir: main.dir,
    subsystemId: overrides.subsystemId || 'hr-management',
    commits: overrides.commits,
    mode: overrides.mode || 'individual',
    baseSha: overrides.baseSha || null,
    targetBranch: overrides.targetBranch || 'staging',
    actor: overrides.actor || null,
    allowSharedConfig: overrides.allowSharedConfig || false,
    allowRemovals: overrides.allowRemovals || false,
    push: overrides.push ?? false,
    createPr: overrides.createPr ?? false,
    pushRemoteUrl: overrides.pushRemoteUrl || null,
    nonce: uniqueNonce('oracle'),
  };
  const report = await runPromotion(request, {
    ghClient,
    generateLockfile: overrides.generateLockfile || stubLockfileGenerator,
    resolveSourceUrl: () => source.dir,
  });
  return { report, ghClient, branch: report.integrationBranch && report.integrationBranch.name };
}

function blockerCodes(report) {
  return report.blockers.map((b) => b.code);
}

// --- Finding 1: scratch merge files must never reach the integration commit -----------------

test('finding #1: integration commit contains NO scratch files', async () => {
  const sourceDir = p('src-scratch');
  initRepo(sourceDir);
  commitFiles(sourceDir, 'A: baseline', { 'src/merge.txt': 'a\nb\nc\nd\n' });
  const C = commitFiles(sourceDir, 'C: modify a different line', { 'src/merge.txt': 'a\nb\nc\nX\n' });

  const main = createMainRepo(p('main-scratch'), { extraFiles: { 'src/merge.txt': 'a\nY\nc\nd\n' } });
  const { report, branch } = await promote({ dir: sourceDir }, main, { commits: [C] });

  assert.equal(report.ok, true, JSON.stringify(report.blockers));
  assert.equal(report.changes.merged.some((m) => m.path === 'src/merge.txt'), true, 'a real three-way merge happened');

  const names = gOut(main.dir, ['show', '--name-only', '--format=', report.provenance.commit])
    .split('\n')
    .map((l) => l.trim())
    .filter(Boolean);
  const scratch = names.filter((n) => /\.(ours|base|theirs)$/.test(n));
  assert.deepEqual(scratch, [], `integration commit must not contain scratch files: ${JSON.stringify(names)}`);
  assert.equal(tryBranchHas(main.dir, branch, 'src/merge.txt'), true);
});

// --- Finding 2: push must not use the unauthenticated origin --------------------------------

test('finding #2: push target is authenticated (token path), not bare origin', () => {
  const main = createMainRepo(p('main-push'));

  const real = createGitHubClient({ mode: 'real', token: 'ghs_secret_token', pushTarget: null });
  const resolved = resolvePushRemote({ request: {}, ghClient: real, repoSlug: 'ORG/main-system', repoDir: main.dir });
  assert.equal(resolved.source, 'authenticated-token');
  assert.equal(resolved.url, authenticatedRemoteUrl('ORG/main-system', 'ghs_secret_token'));
  assert.match(resolved.url, /x-access-token:ghs_secret_token@github\.com\/ORG\/main-system\.git/);

  // A missing token must NOT silently fall back to the credential-less origin under the real client.
  const noToken = createGitHubClient({ mode: 'real', token: 'x', pushTarget: null });
  noToken.token = null;
  const missing = resolvePushRemote({ request: {}, ghClient: noToken, repoSlug: 'ORG/main-system', repoDir: main.dir });
  assert.equal(missing.url, null);
  assert.equal(missing.source, 'missing-token');

  // An explicit target (local tests / VOS_PUSH_TARGET) always wins.
  const stub = createGitHubClient({ mode: 'stub', pushTarget: 'C:/tmp/bare.git' });
  assert.deepEqual(resolvePushRemote({ ghClient: stub, repoSlug: 'ORG/main-system', repoDir: main.dir }), { url: 'C:/tmp/bare.git', source: 'push-target' });
});

test('finding #2: an explicit push target is honoured end-to-end and reported', async () => {
  const src = createSourceChain(p('src-push2'));
  const main = createMainRepo(p('main-push2'));
  const bare = initBare(p('remote-push2'));
  const ghClient = createGitHubClient({ mode: 'stub', pushTarget: bare });
  const { report, branch } = await promote(src, main, { commits: [src.C], ghClient, push: true });
  assert.equal(report.ok, true, JSON.stringify(report.blockers));
  assert.equal(report.push.source, 'push-target');
  assert.notEqual(refIn(bare, `refs/heads/${branch}`), null);
});

// --- Finding 3: ordinary GitHub reverts count as revert events ------------------------------

test('finding #3: ordinary GitHub revert → status Reverted, then re-promotion allowed', async () => {
  const src = createSourceChain(p('src-revert'));
  const main = createMainRepo(p('main-revert'));
  const basePkgText = fs.readFileSync(path.join(main.dir, 'package.json'), 'utf8');

  const first = await promote(src, main, { commits: [src.C] });
  assert.equal(first.report.ok, true, JSON.stringify(first.report.blockers));
  const integrationCommit = first.report.provenance.commit;
  advanceRef(main.dir, 'staging', sha(main.dir, first.branch));

  const subject = gOut(main.dir, ['log', '-1', '--format=%s', integrationCommit]);
  assert.match(subject, /^integrate\(hr-management\):/);
  const revertMsg = `Revert "${subject}"\n\nThis reverts commit ${integrationCommit}.\n`;
  commitMessageOnBranch(
    main.dir,
    'staging',
    revertMsg,
    { 'package.json': basePkgText },
    { remove: ['src/modules/hr/c.txt', 'src/modules/hr/export.ts'] },
  );

  const reverted = computeBranchStatus(main.dir, { branch: 'staging', subsystem: 'hr-management', shas: [src.C] });
  assert.equal(reverted.results[0].status, 'Reverted');
  assert.notEqual(reverted.results[0].revertedBy, null);

  const again = await promote(src, main, { commits: [src.C], targetBranch: 'staging' });
  assert.equal(again.report.ok, true, `re-promotion after an ordinary revert must be allowed: ${JSON.stringify(again.report.blockers)}`);
});

// --- Finding 4: root commits produce a real delta -------------------------------------------

test('finding #4: root-commit promotion imports the expected files', async () => {
  const sourceDir = p('src-root');
  initRepo(sourceDir);
  const ROOT = commitFiles(sourceDir, 'root commit', {
    'docs/readme.md': '# root\n',
    'src/root-only.txt': 'root only\n',
  });

  const delta = nameStatusDelta(sourceDir, ROOT);
  assert.equal(delta.parent, null);
  assert.equal(delta.entries.length, 2, 'a root commit diffs against the empty tree');
  assert.deepEqual(delta.entries.map((e) => e.status), ['A', 'A']);

  const main = createMainRepo(p('main-root'));
  const { report } = await promote({ dir: sourceDir }, main, { commits: [ROOT] });
  assert.equal(report.ok, true, JSON.stringify(report.blockers));
  assert.equal(blockerCodes(report).includes('NOOP_PROMOTION'), false);
  assert.equal(tryBranchHas(main.dir, report.integrationBranch.name, 'src/root-only.txt'), true);
  assert.equal(tryBranchHas(main.dir, report.integrationBranch.name, 'docs/readme.md'), true);
});

// --- Finding 5: binary modify when the target equals the source baseline --------------------

test('finding #5: binary modify where target == base overwrites (no false conflict)', async () => {
  const sourceDir = p('src-bin');
  initRepo(sourceDir);
  const baseBytes = Buffer.from([0x00, 0x01, 0x02, 0xff, 0x00, 0x10]);
  const newBytes = Buffer.from([0x00, 0x09, 0x00, 0xfe]);
  commitFiles(sourceDir, 'A: binary baseline', { 'src/assets/bin.dat': baseBytes });
  const C = commitFiles(sourceDir, 'C: modify binary', { 'src/assets/bin.dat': newBytes });

  const main = createMainRepo(p('main-bin'), { extraFiles: { 'src/assets/bin.dat': baseBytes } });
  const { report, branch } = await promote({ dir: sourceDir }, main, { commits: [C] });
  assert.equal(report.ok, true, JSON.stringify(report.blockers));
  assert.deepEqual(branchFile(main.dir, branch, 'src/assets/bin.dat'), newBytes, 'binary source change applied byte-for-byte');
});

// --- Finding 6: open-PR duplicate detection matches EACH requested SHA ----------------------

test('finding #6: subset open PR ([B,C]) blocks a new [C] request (and vice-versa)', async () => {
  const src = createSourceChain(p('src-pr-subset'));
  const main = createMainRepo(p('main-pr-subset'));

  const bcBody = `Subsystem-ID: hr-management\nSource-Commit: ${src.B}\nSource-Commit: ${src.C}\nPromotion-Target: staging\n`;
  const ghClient = createGitHubClient({
    mode: 'stub',
    openPrs: [{ number: 41, head: `integration/hr-management-${src.B.slice(0, 12)}-staging-1`, base: 'staging', state: 'open', body: bcBody }],
  });
  const subset = await promote(src, main, { commits: [src.C], ghClient });
  assert.ok(blockerCodes(subset.report).includes('PR_ALREADY_OPEN'), 'an open [B,C] PR must block a new [C] request');

  // The reverse direction: an open PR for [C] blocks a new [B,C] request.
  const cBody = `Subsystem-ID: hr-management\nSource-Commit: ${src.C}\nPromotion-Target: staging\n`;
  const ghClient2 = createGitHubClient({
    mode: 'stub',
    openPrs: [{ number: 42, head: 'integration/hr-management-deadbeefcafe-staging-9', base: 'staging', state: 'open', body: cBody }],
  });
  const superset = await promote(src, main, { commits: [src.B, src.C], ghClient: ghClient2 });
  assert.ok(blockerCodes(superset.report).includes('PR_ALREADY_OPEN'), 'an open [C] PR must block a new [B,C] request');

  // A PR that references neither SHA must not block.
  const unrelated = createGitHubClient({
    mode: 'stub',
    openPrs: [{ number: 43, head: 'integration/hr-management-aaaaaaaaaaaa-staging-9', base: 'staging', state: 'open', body: `Source-Commit: ${'f'.repeat(40)}\n` }],
  });
  const allowed = await promote(src, main, { commits: [src.C], ghClient: unrelated });
  assert.equal(blockerCodes(allowed.report).includes('PR_ALREADY_OPEN'), false);
});

test('finding #6: extractSourceCommits reads Source-Commit metadata from PR bodies', () => {
  const body = `Some PR text\n- \`${'a'.repeat(40)}\`\n\nSource-Commit: ${'b'.repeat(40)}\nSource-Commit: ${'c'.repeat(40)}\n`;
  assert.deepEqual(extractSourceCommits(body), ['b'.repeat(40), 'c'.repeat(40)]);
});

// --- Finding 7: symlinked target parents are refused ----------------------------------------

test('finding #7: symlinked target parent directory is rejected', (t) => {
  const src = createSourceChain(p('src-symlink'));
  const worktreeDir = p('wt-symlink');
  fs.mkdirSync(worktreeDir, { recursive: true });
  const outside = p('outside-symlink');
  fs.mkdirSync(outside, { recursive: true });

  try {
    fs.symlinkSync(outside, path.join(worktreeDir, 'src'), 'junction');
  } catch (err) {
    t.skip(`symlink/junction creation not permitted here: ${err.message}`);
    return;
  }
  assert.equal(fs.lstatSync(path.join(worktreeDir, 'src')).isSymbolicLink(), true);

  assert.throws(
    () => applyFileDelta({
      objectsRepo: src.dir,
      worktreeDir,
      sourceSha: src.C,
      baseSha: src.B,
      entry: { status: 'M', path: 'src/modules/hr/c.txt' },
    }),
    (err) => err instanceof ApplyConflictError && err.details && err.details.kind === 'symlink-traversal',
  );
  // Nothing escaped into the symlinked directory.
  assert.deepEqual(fs.readdirSync(outside), []);
});

// --- Finding 8: workflow_dispatch can grant shared-config / removals approval ---------------

test('finding #8: shared-config approval is grantable via the workflow input', async () => {
  const yml = fs.readFileSync(WORKFLOW_FILE, 'utf8');
  assert.match(yml, /allow_shared_config:/, 'workflow_dispatch input allow_shared_config must exist');
  assert.match(yml, /allow_removals:/, 'workflow_dispatch input allow_removals must exist');
  assert.match(yml, /VOS_ALLOW_SHARED_CONFIG:\s*\$\{\{\s*inputs\.allow_shared_config\s*\}\}/, 'input must map to the env flag');
  assert.match(yml, /VOS_ALLOW_REMOVALS:\s*\$\{\{\s*inputs\.allow_removals\s*\}\}/, 'input must map to the env flag');

  // The input→env→flag chain behaves: envBool parses the serialized boolean.
  const before = process.env.VOS_ALLOW_SHARED_CONFIG;
  process.env.VOS_ALLOW_SHARED_CONFIG = 'true';
  try {
    assert.equal(envBool('VOS_ALLOW_SHARED_CONFIG'), true);
    process.env.VOS_ALLOW_SHARED_CONFIG = 'false';
    assert.equal(envBool('VOS_ALLOW_SHARED_CONFIG'), false);

    process.env.VOS_ALLOW_SHARED_CONFIG = 'true';
    const sourceDir = p('src-shared');
    initRepo(sourceDir);
    commitFiles(sourceDir, 'A', { 'tsconfig.json': '{"a":1}\n' });
    const B = commitFiles(sourceDir, 'B: modify shared config', { 'tsconfig.json': '{"a":2}\n' });
    const main = createMainRepo(p('main-shared'), { extraFiles: { 'tsconfig.json': '{"a":1}\n' } });

    const granted = await promote({ dir: sourceDir }, main, { commits: [B], allowSharedConfig: envBool('VOS_ALLOW_SHARED_CONFIG') });
    assert.equal(granted.report.ok, true, JSON.stringify(granted.report.blockers));
    assert.equal(branchFileText(main.dir, granted.branch, 'tsconfig.json'), '{"a":2}\n');
  } finally {
    if (before === undefined) delete process.env.VOS_ALLOW_SHARED_CONFIG;
    else process.env.VOS_ALLOW_SHARED_CONFIG = before;
  }
});

// --- Finding 9: PR lookup failure is distinct; listing paginates ----------------------------

test('finding #9: PR-lookup failure yields PR_LOOKUP_FAILED, not PR_ALREADY_OPEN', async () => {
  const src = createSourceChain(p('src-prfail'));
  const main = createMainRepo(p('main-prfail'));
  const failingClient = {
    mode: 'stub',
    pushTarget: null,
    async listOpenPullRequests() {
      throw new Error('network exploded');
    },
    async createPullRequest() {
      throw new Error('should not be called');
    },
  };
  const { report } = await promote(src, main, { commits: [src.C], ghClient: failingClient });
  assert.ok(blockerCodes(report).includes('PR_LOOKUP_FAILED'), JSON.stringify(report.blockers));
  assert.equal(blockerCodes(report).includes('PR_ALREADY_OPEN'), false);
});

test('finding #9: real client paginates PR listing (per_page=100 & page=N)', async () => {
  const urls = [];
  const pages = [
    [{ number: 1 }, { number: 2 }],
    [{ number: 3 }],
  ];
  const client = createGitHubClient({
    mode: 'real',
    token: 't',
    apiBase: 'https://api.test',
    fetchImpl: async (url) => {
      urls.push(String(url));
      const page = Number(new URL(String(url)).searchParams.get('page')) || 1;
      const body = pages[page - 1] || [];
      return { ok: true, status: 200, text: async () => JSON.stringify(body) };
    },
  });
  const prs = await client.listOpenPullRequests('ORG/main', { base: 'staging', perPage: 2 });
  assert.equal(prs.length, 3);
  assert.equal(urls.length, 2, 'must request the second page when the first is full');
  assert.match(urls[0], /per_page=2/);
  assert.match(urls[0], /page=1/);
  assert.match(urls[1], /page=2/);
});

// --- Finding 10: registry ref missing under CI fails closed ---------------------------------

test('finding #10: registry ref missing in CI fails closed', () => {
  const dir = p('cfg-ci');
  initRepo(dir);
  writeRegistry(dir, { config: configObject({ registryBranch: 'registry-does-not-exist' }) });
  commitAll(dir, 'registry');
  // The boot read (registryBranch=null) always uses the worktree and must still succeed.
  assert.equal(loadConfig(dir, null).source, 'worktree');

  const before = process.env.GITHUB_ACTIONS;
  process.env.GITHUB_ACTIONS = 'true';
  try {
    assert.throws(
      () => loadConfig(dir, 'registry-does-not-exist'),
      (err) => err instanceof PolicyError && err.code === 'REGISTRY_REF_MISSING',
    );
  } finally {
    if (before === undefined) delete process.env.GITHUB_ACTIONS;
    else process.env.GITHUB_ACTIONS = before;
  }

  if (!isCI()) {
    // Outside CI the documented worktree fallback is retained.
    assert.equal(loadConfig(dir, 'registry-does-not-exist').source, 'worktree');
  }
});

// --- Finding 11: unsupported lockfile manager is not silently skipped -----------------------

test('finding #11: unsupported lockfile manager emits LOCKFILE_MANAGER_UNSUPPORTED', async () => {
  const src = createSourceChain(p('src-bun'));
  const mainDir = p('main-bun');
  initRepo(mainDir, 'master');
  write(mainDir, 'package.json', `${JSON.stringify(BASE_PKG, null, 2)}\n`);
  write(mainDir, 'bun.lockb', 'bun-binary-lockfile\n');
  writeRegistry(mainDir, {});
  commitAll(mainDir, 'main: bun baseline');
  g(mainDir, ['branch', 'staging']);
  g(mainDir, ['branch', 'development']);

  const { report } = await promote(src, { dir: mainDir }, { commits: [src.C], generateLockfile: defaultLockfileGenerator });
  assert.ok(blockerCodes(report).includes('LOCKFILE_MANAGER_UNSUPPORTED'), JSON.stringify(report.blockers));
  assert.equal(report.ok, false);
});

// --- Finding 12: only the trailing trailer block counts -------------------------------------

test('finding #12: embedded Source-Commit in a message body does NOT create phantom trailers', () => {
  const real = 'a'.repeat(40);
  const phantom = 'dead'.repeat(10);
  const msg = [
    'integrate(hr-management): subject',
    '',
    `Source-Commit: ${phantom}`,
    '',
    'Prose follows, then the authoritative trailer block:',
    '',
    'Subsystem-ID: hr-management',
    `Source-Commit: ${real}`,
    'Promotion-Target: staging',
    '',
  ].join('\n');
  const parsed = parseTrailers(msg);
  assert.deepEqual(parsed.trailers['Source-Commit'], [real]);
  assert.equal(parsed.trailers['Source-Commit'].includes(phantom), false);
});

test('finding #12: collectEvents ignores a phantom Source-Commit in the body', () => {
  const src = createSourceChain(p('src-phantom'));
  const main = createMainRepo(p('main-phantom'));
  const phantom = 'dead'.repeat(10);
  const msg = [
    'integrate(hr-management): phantom subject',
    '',
    `Source-Commit: ${phantom}`,
    '',
    'Prose follows, then the authoritative trailer block:',
    '',
    'Subsystem-ID: hr-management',
    `Source-Commit: ${src.C}`,
    'Promotion-Target: staging',
    '',
  ].join('\n');
  commitMessageOnBranch(main.dir, 'staging', msg, { 'src/modules/hr/c.txt': 'C feature\n' });

  const integrate = collectEvents(main.dir, 'staging').find((e) => e.type === 'integrate');
  assert.ok(integrate);
  assert.deepEqual(integrate.sourceCommits, [src.C]);
});

// --- Finding 13: sourceBranch is validated before use in a refspec --------------------------

test('finding #13: unsafe sourceBranch is rejected by loadSubsystem', () => {
  for (const bad of ['evil branch', 'a..b', '-x', 'x\ty']) {
    const dir = p('sub-bad');
    initRepo(dir);
    writeRegistry(dir, { subsystem: subsystemObject({ sourceBranch: bad }) });
    assert.throws(
      () => loadSubsystem(dir, null, 'hr-management'),
      (err) => err instanceof PolicyError && err.code === 'INVALID_BRANCH',
      `sourceBranch ${JSON.stringify(bad)} must be rejected`,
    );
  }
});
