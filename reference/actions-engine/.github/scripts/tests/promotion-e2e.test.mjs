import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

import { runPromotion, stubLockfileGenerator, integrationBranchPrefix } from '../promote-subsystem.mjs';
import { createGitHubClient } from '../lib/gh.mjs';
import { computeBranchStatus } from '../integration-history.mjs';
import { buildRevertMessage, parseTrailers } from '../write-provenance-commit.mjs';
import {
  createSourceChain,
  createMainRepo,
  createBulkSource,
  createBulkMain,
  createMissingDepSource,
  makeTempDir,
  uniqueNonce,
  initRepo,
  commitFiles,
  commitMessageOnBranch,
  advanceRef,
  sha,
  gOut,
  g,
  tryBranchHas,
  branchFile,
  branchFileText,
  treeMode,
  isAncestor,
  initBare,
  refIn,
  configObject,
  subsystemObject,
  writeRegistry,
} from './helpers/sandbox.mjs';

const p = (...parts) => path.join(makeTempDir('e2e'), ...parts);

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
    createPr: overrides.createPr ?? true,
    pushRemoteUrl: overrides.pushRemoteUrl || null,
    nonce: uniqueNonce('promo'),
  };
  const report = await runPromotion(request, {
    ghClient,
    generateLockfile: stubLockfileGenerator,
    resolveSourceUrl: () => source.dir,
  });
  return { report, ghClient, branch: report.integrationBranch && report.integrationBranch.name };
}

function blockerCodes(report) {
  return report.blockers.map((b) => b.code);
}

// --- Acceptance test 1 / 8: selective commit + new dependency ----------------

test('#1 selective commit: promote only C; B and D absent; exceljs added', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'));
  const stagingTip = sha(main.dir, 'staging');

  const { report, branch } = await promote(src, main, { commits: [src.C] });
  assert.equal(report.ok, true, JSON.stringify(report.blockers));
  assert.equal(report.result, 'promoted');

  assert.equal(tryBranchHas(main.dir, branch, 'src/modules/hr/c.txt'), true);
  assert.equal(tryBranchHas(main.dir, branch, 'src/modules/hr/b.txt'), false, 'B changes must be absent');
  assert.equal(tryBranchHas(main.dir, branch, 'src/modules/hr/d.txt'), false, 'D changes must be absent');

  const pkg = JSON.parse(branchFileText(main.dir, branch, 'package.json'));
  assert.equal(pkg.dependencies.exceljs, '^4.4.0');
  assert.equal(pkg.dependencies.react, '19.2.3', 'unrelated target packages retained');
  assert.equal(pkg.devDependencies.typescript, '^5');
});

test('#3 correct base: integration branch starts at the TARGET tip; PR base is the target', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'));
  // Diverge the branches so "started from staging" is distinguishable from "started from master".
  commitMessageOnBranch(main.dir, 'staging', 'staging: only on staging', { 'staging-only.txt': 'staging\n' });
  commitMessageOnBranch(main.dir, 'master', 'master: only on master', { 'master-only.txt': 'master\n' });
  const stagingTip = sha(main.dir, 'staging');
  const masterTip = sha(main.dir, 'master');
  assert.notEqual(stagingTip, masterTip);

  const { report, ghClient, branch } = await promote(src, main, { commits: [src.C], targetBranch: 'staging' });
  assert.equal(report.ok, true);
  assert.equal(gOut(main.dir, ['merge-base', 'staging', branch]), stagingTip, 'integration branch starts at the staging tip');
  assert.equal(tryBranchHas(main.dir, branch, 'staging-only.txt'), true, 'inherits staging history');
  assert.equal(tryBranchHas(main.dir, branch, 'master-only.txt'), false, 'does NOT inherit master history');
  assert.equal(isAncestor(main.dir, masterTip, branch), false);
  const prCall = ghClient.calls.find((c) => c.name === 'createPullRequest');
  assert.equal(prCall.payload.base, 'staging', 'PR base must equal the requested target branch, not --ref');
  assert.equal(report.integrationBranch.startPoint, stagingTip);
});

test('#8 new dependency: lockfile regenerated (stub) for the target, manifest merged', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'));
  const { report } = await promote(src, main, { commits: [src.C] });
  assert.equal(report.dependencies.added.length, 1);
  assert.equal(report.dependencies.added[0].name, 'exceljs');
  assert.deepEqual(report.changes.regenerated, ['npm']);
  assert.equal(report.lockfile.stubbed, true);
});

// --- Acceptance test 4 / 5: multi-select + ancestry --------------------------

test('#4 multi-select: two SHAs applied in order; both recorded in provenance', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'));
  const { report, branch } = await promote(src, main, { commits: [src.B, src.C] });
  assert.equal(report.ok, true);
  assert.equal(tryBranchHas(main.dir, branch, 'src/modules/hr/b.txt'), true);
  assert.equal(tryBranchHas(main.dir, branch, 'src/modules/hr/c.txt'), true);
  const trailers = parseTrailers(gOut(main.dir, ['log', '-1', '--format=%B', branch])).trailers;
  assert.deepEqual(trailers['Source-Commit'], [src.B, src.C]);
  assert.equal(report.verification.order[0], src.B);
});

test('#5 source ancestry: a SHA not reachable from the protected branch is rejected', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'));
  gOut(src.dir, ['checkout', '-q', '-b', 'side', src.A]);
  const side = commitFiles(src.dir, 'side commit', { 'src/side.txt': 'side\n' });
  gOut(src.dir, ['checkout', '-q', 'master']);
  const { report } = await promote(src, main, { commits: [side] });
  assert.equal(report.ok, false);
  assert.ok(blockerCodes(report).includes('COMMIT_VERIFICATION_FAILED'));
});

// --- Acceptance test 6 / 24: registry policy (server-side, bypass-proof) -----

test('#6 registry policy: unregistered subsystem rejected', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'));
  const { report } = await promote(src, main, { commits: [src.C], subsystemId: 'payroll' });
  assert.ok(blockerCodes(report).includes('SUBSYSTEM_NOT_REGISTERED'));
});

test('#6 registry policy: disabled subsystem rejected', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'), { subsystem: subsystemObject({ enabled: false }) });
  const { report } = await promote(src, main, { commits: [src.C] });
  assert.ok(blockerCodes(report).includes('SUBSYSTEM_DISABLED'));
});

test('#6 registry policy: malformed registry JSON rejected', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'));
  fs.writeFileSync(path.join(main.dir, '.integration', 'config.json'), '{ broken json');
  const { report } = await promote(src, main, { commits: [src.C] });
  assert.ok(blockerCodes(report).includes('REGISTRY_INVALID'));
});

test('#6/#24 unauthorized target branch rejected even when the UI is bypassed', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'));
  const notAllowed = await promote(src, main, { commits: [src.C], targetBranch: 'feature/evil' });
  assert.ok(blockerCodes(notAllowed.report).includes('TARGET_NOT_ALLOWED'));
  const unsafe = await promote(src, main, { commits: [src.C], targetBranch: '../../etc/passwd' });
  assert.ok(blockerCodes(unsafe.report).includes('TARGET_INVALID'));
});

test('#6 nonexistent (but policy-allowed) target rejected', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'));
  const { report } = await promote(src, main, { commits: [src.C], targetBranch: 'release/9.9' });
  assert.ok(blockerCodes(report).includes('TARGET_NOT_FOUND'));
});

// --- Acceptance test 7: files, renames, deletes, binaries, main-owned --------

test('#7 preserve adds/modifies/deletes/renames/binary/unicode/exec; skip main-owned & secrets', async () => {
  const src = createBulkSource(p('source'));
  const main = createBulkMain(p('main'));
  const { report, branch } = await promote(src, main, { commits: [src.B] });
  assert.equal(report.ok, true, JSON.stringify(report.blockers));

  assert.equal(tryBranchHas(main.dir, branch, 'src/new-file.txt'), true);
  assert.equal(branchFileText(main.dir, branch, 'src/new-file.txt'), src.newFile);
  assert.equal(tryBranchHas(main.dir, branch, 'src/old-name.txt'), false, 'rename removes the old path');
  assert.equal(branchFileText(main.dir, branch, 'src/renamed file.txt'), src.renamed);
  assert.equal(tryBranchHas(main.dir, branch, 'src/to-delete.txt'), false, 'delete is honored');
  assert.equal(tryBranchHas(main.dir, branch, 'src/émigré.txt'), true, 'unicode filename handled');
  assert.deepEqual(branchFile(main.dir, branch, 'src/assets/image.png'), src.imageBytes, 'binary preserved byte-for-byte');
  assert.equal(treeMode(main.dir, branch, 'scripts/run.sh'), '100755', 'executable bit preserved');

  assert.equal(tryBranchHas(main.dir, branch, '.github/workflows/evil.yml'), false, 'workflows are main-owned');
  assert.equal(tryBranchHas(main.dir, branch, '.env.local'), false, 'secret files are never imported');
  assert.ok(report.changes.mainOwnedSkipped.some((f) => f.path === '.github/workflows/evil.yml'));
  assert.ok(report.changes.secretExcluded.some((f) => f.path === '.env.local'));
});

// --- Acceptance tests 9 / 10: dependency prerequisites & conflicts -----------

test('#9 missing dependency commit does not auto-include B; validation blocks', async () => {
  const src = createMissingDepSource(p('source'));
  const main = createMainRepo(p('main'));
  const blocked = await promote(src, main, { commits: [src.C] });
  assert.equal(blocked.report.ok, false);
  assert.ok(blockerCodes(blocked.report).includes('MISSING_DEPENDENCY_PREREQUISITE'));

  const both = await promote(src, main, { commits: [src.B, src.C] });
  assert.equal(both.report.ok, true, JSON.stringify(both.report.blockers));
});

test('#10 incompatible dependency range fails closed; no force-install', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'), { extraDependencies: { exceljs: '^3.0.0' } });
  const { report } = await promote(src, main, { commits: [src.C] });
  assert.equal(report.ok, false);
  assert.ok(blockerCodes(report).includes('DEPENDENCY_INCOMPATIBLE_RANGE'));
});

// --- Acceptance test 11: patch conflict --------------------------------------

test('#11 patch conflict stops safely; target branch remains untouched', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'), { extraFiles: { 'src/modules/hr/c.txt': 'divergent target content\n' } });
  const stagingTip = sha(main.dir, 'staging');
  const { report } = await promote(src, main, { commits: [src.C] });
  assert.equal(report.ok, false);
  assert.ok(blockerCodes(report).includes('PATCH_CONFLICT'));
  assert.equal(sha(main.dir, 'staging'), stagingTip, 'target branch tip must be unchanged');
  for (const b of gOut(main.dir, ['branch', '--format=%(refname:short)']).split('\n').map((s) => s.trim()).filter((s) => s.startsWith('integration/'))) {
    assert.equal(sha(main.dir, b), stagingTip, 'no integration commit was made');
  }
});

// --- Acceptance tests 12 / 13 / 14: pending vs integrated --------------------

test('#12/#13 open/pending PR is NOT Integrated; unmerged branch keeps status Eligible', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'));
  const { report } = await promote(src, main, { commits: [src.C] });
  assert.equal(report.ok, true, 'PR "created" (stubbed) but not merged');
  const status = computeBranchStatus(main.dir, { branch: 'staging', subsystem: 'hr-management', shas: [src.C] });
  assert.equal(status.results[0].status, 'Eligible', 'source commit is not Integrated until the PR is merged');
});

test('#12 existing open PR blocks a duplicate dispatch', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'));
  const head = `${integrationBranchPrefix({ subsystem: 'hr-management', shas: [src.C] })}-anything`;
  const ghClient = createGitHubClient({ mode: 'stub', openPrs: [{ number: 42, head, base: 'staging', state: 'open' }] });
  const { report } = await promote(src, main, { commits: [src.C], ghClient });
  assert.ok(blockerCodes(report).includes('PR_ALREADY_OPEN'));
});

test('#14 merged integration: reachable trailers make status Integrated', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'));
  const { report, branch } = await promote(src, main, { commits: [src.C] });
  advanceRef(main.dir, 'staging', sha(main.dir, branch));
  const status = computeBranchStatus(main.dir, { branch: 'staging', subsystem: 'hr-management', shas: [src.C] });
  assert.equal(status.results[0].status, 'Integrated');
  assert.equal(status.results[0].integrationCommit, report.provenance.commit);
});

// --- Acceptance tests 2 / 16 / 17 / 18: target independence ------------------

test('#2/#16 duplicate active on the same target is blocked', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'));
  const first = await promote(src, main, { commits: [src.C] });
  advanceRef(main.dir, 'staging', sha(main.dir, first.branch));
  const second = await promote(src, main, { commits: [src.C], targetBranch: 'staging' });
  assert.ok(blockerCodes(second.report).includes('DUPLICATE_ACTIVE'));
});

test('#2/#17 same SHA to a different target remains allowed and independent', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'));
  const staging = await promote(src, main, { commits: [src.C], targetBranch: 'staging' });
  advanceRef(main.dir, 'staging', sha(main.dir, staging.branch));

  const master = await promote(src, main, { commits: [src.C], targetBranch: 'master' });
  assert.equal(master.report.ok, true, JSON.stringify(master.report.blockers));
  assert.equal(tryBranchHas(main.dir, master.branch, 'src/modules/hr/c.txt'), true);

  const onDevelopment = computeBranchStatus(main.dir, { branch: 'development', subsystem: 'hr-management', shas: [src.C] });
  assert.equal(onDevelopment.results[0].status, 'Eligible', 'development is independent');
});

test('#18 inherited promotion: branch merge makes status Integrated', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'));
  const { branch } = await promote(src, main, { commits: [src.C], targetBranch: 'staging' });
  advanceRef(main.dir, 'staging', sha(main.dir, branch));
  advanceRef(main.dir, 'master', sha(main.dir, 'staging'));
  const status = computeBranchStatus(main.dir, { branch: 'master', subsystem: 'hr-management', shas: [src.C] });
  assert.equal(status.results[0].status, 'Integrated');
});

// --- Acceptance test 19: revert + controlled re-promotion --------------------

test('#19 revert yields Reverted; re-promotion is allowed and restores Integrated', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'));
  const { report, branch } = await promote(src, main, { commits: [src.C] });
  const integrationCommit = report.provenance.commit;
  advanceRef(main.dir, 'staging', sha(main.dir, branch));

  const revertMsg = buildRevertMessage({ subsystemId: 'hr-management', sourceCommits: [src.C], revertsIntegrationSha: integrationCommit, subject: 'C feature' });
  commitMessageOnBranch(
    main.dir,
    'staging',
    revertMsg,
    { 'package.json': `${JSON.stringify({ name: 'main-system', version: '0.1.0', private: true, dependencies: { next: '^16.2.1', react: '19.2.3' }, devDependencies: { typescript: '^5' } }, null, 2)}\n` },
    { remove: ['src/modules/hr/c.txt', 'src/modules/hr/export.ts'] },
  );
  const reverted = computeBranchStatus(main.dir, { branch: 'staging', subsystem: 'hr-management', shas: [src.C] });
  assert.equal(reverted.results[0].status, 'Reverted');

  const again = await promote(src, main, { commits: [src.C], targetBranch: 'staging' });
  assert.equal(again.report.ok, true, 're-promotion after revert must be allowed');
});

// --- Acceptance tests 20 / 23: authorization + no ledger -------------------

test('#20 unauthorized caller is rejected', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'));
  const ghClient = createGitHubClient({ mode: 'stub', authorized: false });
  const { report } = await promote(src, main, { commits: [src.C], ghClient, actor: 'mallory' });
  assert.ok(blockerCodes(report).includes('UNAUTHORIZED_CALLER'));
});

test('#23 no JSON ledger is written; audit lives in git trailers', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'));
  const { branch } = await promote(src, main, { commits: [src.C] });
  assert.equal(tryBranchHas(main.dir, branch, '.integration/integrated-commits.json'), false);
  assert.equal(fs.existsSync(path.join(main.dir, '.integration', 'integrated-commits.json')), false);
  const trailers = parseTrailers(gOut(main.dir, ['log', '-1', '--format=%B', branch])).trailers;
  assert.deepEqual(trailers['Subsystem-ID'], ['hr-management']);
  assert.deepEqual(trailers['Source-Commit'], [src.C]);
  assert.deepEqual(trailers['Promotion-Mode'], ['individual']);
  assert.deepEqual(trailers['Promotion-Target'], ['staging']);
});

// --- Guard rails: no-op + shared config -------------------------------------

test('no-op promotion (only main-owned changes) is refused', async () => {
  const sourceDir = path.join(makeTempDir('src'), 'noop');
  initRepo(sourceDir);
  commitFiles(sourceDir, 'A base', { 'package.json': '{"name":"x"}\n' });
  commitFiles(sourceDir, 'B only a workflow', { '.github/workflows/x.yml': 'name: x\non: push\n' });
  const main = createMainRepo(p('main'));
  const { report } = await promote({ dir: sourceDir }, main, { commits: [sha(sourceDir, 'master')] });
  assert.equal(report.ok, false);
  assert.ok(blockerCodes(report).includes('NOOP_PROMOTION'));
});

test('shared config blocks by default and is applied only under explicit approval', async () => {
  const sourceDir = path.join(makeTempDir('src'), 'shared');
  initRepo(sourceDir);
  commitFiles(sourceDir, 'A', { 'tsconfig.json': '{"a":1}\n' });
  const B = commitFiles(sourceDir, 'B modify shared config', { 'tsconfig.json': '{"a":2}\n' });
  const main = createMainRepo(p('main'), { extraFiles: { 'tsconfig.json': '{"a":1}\n' } });

  const blocked = await promote({ dir: sourceDir }, main, { commits: [B] });
  assert.ok(blockerCodes(blocked.report).includes('SHARED_CONFIG_REQUIRES_REVIEW'));

  const allowed = await promote({ dir: sourceDir }, main, { commits: [B], allowSharedConfig: true });
  assert.equal(allowed.report.ok, true, JSON.stringify(allowed.report.blockers));
  assert.equal(branchFileText(main.dir, allowed.branch, 'tsconfig.json'), '{"a":2}\n');
});

// --- Push + PR (stubbed transport, real local push) --------------------------

test('push + PR creation: branch is pushed to the remote and PR base is the target (stub transport)', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'));
  const bare = initBare(p('remote.git'));
  const ghClient = createGitHubClient({ mode: 'stub', pushTarget: bare });
  const { report, branch } = await promote(src, main, { commits: [src.C], ghClient, push: true });
  assert.equal(report.ok, true, JSON.stringify(report.blockers));
  assert.equal(refIn(bare, `refs/heads/${branch}`) !== null, true, 'integration branch must exist on the remote');
  const prCall = ghClient.calls.find((c) => c.name === 'createPullRequest');
  assert.equal(prCall.payload.base, 'staging');
  assert.match(prCall.payload.body, /base \*\*`staging`\*\*/);
});

// --- Cumulative mode ---------------------------------------------------------

test('cumulative mode: net range applied from a verified baseline; provenance lists the endpoint', async () => {
  const src = createSourceChain(p('source'));
  const main = createMainRepo(p('main'));
  const { report, branch } = await promote(src, main, { commits: [src.C], mode: 'cumulative', baseSha: src.A });
  assert.equal(report.ok, true, JSON.stringify(report.blockers));
  assert.equal(tryBranchHas(main.dir, branch, 'src/modules/hr/b.txt'), true, 'cumulative includes intervening B');
  assert.equal(tryBranchHas(main.dir, branch, 'src/modules/hr/c.txt'), true);
  const trailers = parseTrailers(gOut(main.dir, ['log', '-1', '--format=%B', branch])).trailers;
  assert.deepEqual(trailers['Source-Commit'], [src.C]);
  assert.deepEqual(trailers['Promotion-Mode'], ['cumulative']);
});
