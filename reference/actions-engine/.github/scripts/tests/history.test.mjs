import { test } from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';

import { computeBranchStatus, listIntegrationHistory, collectEvents, replayEvents } from '../integration-history.mjs';
import { buildProvenanceMessage, buildRevertMessage, parseTrailers } from '../write-provenance-commit.mjs';
import {
  createSourceChain,
  createMainRepo,
  makeTempDir,
  commitMessageOnBranch,
  advanceRef,
  sha,
} from './helpers/sandbox.mjs';

function fixture() {
  const src = createSourceChain(path.join(makeTempDir('src'), 'hist'));
  const main = createMainRepo(path.join(makeTempDir('main'), 'hist'));
  return { src, main };
}

test('parseTrailers: preserves repeated Source-Commit order', () => {
  const msg = buildProvenanceMessage({
    subsystemId: 'hr-management',
    sourceRepo: 'ORG/hr-management',
    sourceCommits: ['a'.repeat(40), 'b'.repeat(40)],
    mode: 'individual',
    target: 'staging',
    subject: 'employee export',
  });
  const parsed = parseTrailers(msg);
  assert.deepEqual(parsed.trailers['Source-Commit'], ['a'.repeat(40), 'b'.repeat(40)]);
  assert.deepEqual(parsed.trailers['Subsystem-ID'], ['hr-management']);
  assert.deepEqual(parsed.trailers['Promotion-Target'], ['staging']);
});

test('status is per (subsystem, SHA, branch): integrated on staging, eligible on master', () => {
  const { src, main } = fixture();
  const integrateMsg = buildProvenanceMessage({
    subsystemId: 'hr-management',
    sourceRepo: 'ORG/hr-management',
    sourceCommits: [src.C],
    mode: 'individual',
    target: 'staging',
    subject: 'C feature',
  });
  commitMessageOnBranch(main.dir, 'staging', integrateMsg, { 'src/modules/hr/c.txt': 'C feature\n' });

  const staging = computeBranchStatus(main.dir, { branch: 'staging', subsystem: 'hr-management', shas: [src.C] });
  assert.equal(staging.results[0].status, 'Integrated');
  assert.equal(staging.results[0].promotionTarget, 'staging');

  const master = computeBranchStatus(main.dir, { branch: 'master', subsystem: 'hr-management', shas: [src.C] });
  assert.equal(master.results[0].status, 'Eligible', 'independent target: master is not automatically integrated');
});

test('revert → Reverted, then re-promotion → Integrated (original record retained)', () => {
  const { src, main } = fixture();
  const integrateMsg = buildProvenanceMessage({
    subsystemId: 'hr-management',
    sourceRepo: 'ORG/hr-management',
    sourceCommits: [src.C],
    mode: 'individual',
    target: 'staging',
    subject: 'C feature',
  });
  const integrationCommit = commitMessageOnBranch(main.dir, 'staging', integrateMsg, { 'src/modules/hr/c.txt': 'C feature\n' });

  const revertMsg = buildRevertMessage({
    subsystemId: 'hr-management',
    sourceCommits: [src.C],
    revertsIntegrationSha: integrationCommit,
    subject: 'C feature',
  });
  commitMessageOnBranch(main.dir, 'staging', revertMsg, false);

  const reverted = computeBranchStatus(main.dir, { branch: 'staging', subsystem: 'hr-management', shas: [src.C] });
  assert.equal(reverted.results[0].status, 'Reverted');
  assert.equal(reverted.results[0].revertedBy !== null, true);

  // Re-promotion records a new integrate event.
  const reMsg = buildProvenanceMessage({
    subsystemId: 'hr-management',
    sourceRepo: 'ORG/hr-management',
    sourceCommits: [src.C],
    mode: 'individual',
    target: 'staging',
    subject: 'C feature (restored)',
  });
  commitMessageOnBranch(main.dir, 'staging', reMsg, { 'src/modules/hr/c.txt': 'C feature\n' });
  const restored = computeBranchStatus(main.dir, { branch: 'staging', subsystem: 'hr-management', shas: [src.C] });
  assert.equal(restored.results[0].status, 'Integrated');
});

test('inheritance: a branch that receives the integration commit by ancestry reports Integrated', () => {
  const { src, main } = fixture();
  const integrateMsg = buildProvenanceMessage({
    subsystemId: 'hr-management',
    sourceRepo: 'ORG/hr-management',
    sourceCommits: [src.C],
    mode: 'individual',
    target: 'staging',
    subject: 'C feature',
  });
  commitMessageOnBranch(main.dir, 'staging', integrateMsg, { 'src/modules/hr/c.txt': 'C feature\n' });

  const before = computeBranchStatus(main.dir, { branch: 'master', subsystem: 'hr-management', shas: [src.C] });
  assert.equal(before.results[0].status, 'Eligible');

  // Simulate main-system branch merge: master fast-forwards onto staging's history.
  advanceRef(main.dir, 'master', sha(main.dir, 'staging'));

  const after = computeBranchStatus(main.dir, { branch: 'master', subsystem: 'hr-management', shas: [src.C] });
  assert.equal(after.results[0].status, 'Integrated');
  assert.equal(after.results[0].promotionTarget, 'staging', 'original target recorded for audit');
});

test('a merged commit WITHOUT trailers does NOT count as integrated (squash-stripped metadata)', () => {
  const { src, main } = fixture();
  commitMessageOnBranch(main.dir, 'staging', 'integrate(hr-management): lost metadata', { 'src/modules/hr/c.txt': 'C feature\n' });
  const status = computeBranchStatus(main.dir, { branch: 'staging', subsystem: 'hr-management', shas: [src.C] });
  assert.equal(status.results[0].status, 'Eligible');
});

test('listIntegrationHistory + replayEvents return events and active records', () => {
  const { src, main } = fixture();
  const msg = buildProvenanceMessage({
    subsystemId: 'hr-management',
    sourceRepo: 'ORG/hr-management',
    sourceCommits: [src.C],
    mode: 'individual',
    target: 'staging',
    subject: 'C feature',
  });
  commitMessageOnBranch(main.dir, 'staging', msg, { 'src/modules/hr/c.txt': 'C feature\n' });
  const events = collectEvents(main.dir, 'staging');
  assert.equal(events.length, 1);
  assert.equal(events[0].type, 'integrate');
  const replayed = replayEvents(events);
  assert.equal(replayed.statuses.size, 1);

  const hist = listIntegrationHistory(main.dir, { branch: 'staging', subsystem: 'hr-management' });
  assert.equal(hist.items.length, 1);
  assert.equal(hist.active.length, 1);
});
