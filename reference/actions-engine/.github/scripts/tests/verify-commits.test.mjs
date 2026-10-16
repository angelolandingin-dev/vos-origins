import { test } from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';

import { parseCommitInput, verifyCommits, VerifyError } from '../verify-source-commits.mjs';
import { createSourceChain, makeTempDir, gOut, initRepo, commitFiles } from './helpers/sandbox.mjs';

function chain(prefix) {
  return createSourceChain(path.join(makeTempDir('src'), prefix));
}

test('parseCommitInput: rejects anything that is not a JSON array of full SHAs', () => {
  const full = 'a'.repeat(40);
  assert.deepEqual(parseCommitInput(JSON.stringify([full])), [full]);
  assert.throws(() => parseCommitInput(''), VerifyError);
  assert.throws(() => parseCommitInput('not json'), VerifyError);
  assert.throws(() => parseCommitInput(JSON.stringify({ sha: full })), VerifyError);
  assert.throws(() => parseCommitInput(JSON.stringify([])), VerifyError);
  assert.throws(() => parseCommitInput(JSON.stringify([full.slice(0, 8)])), /full 40\/64-char lowercase hex SHA/);
  assert.throws(() => parseCommitInput(JSON.stringify([full, full])), VerifyError, 'duplicate');
});

test('verifyCommits: individual mode uses each commit first-parent delta', () => {
  const src = chain('ind');
  const res = verifyCommits({ repoDir: src.dir, sourceBaseRef: 'master', commits: [src.C], mode: 'individual' });
  assert.equal(res.ok, true);
  assert.equal(res.plan.length, 1);
  const paths = res.plan[0].entries.map((e) => e.path || e.newPath);
  assert.ok(paths.includes('src/modules/hr/c.txt'));
  assert.ok(paths.includes('package.json'));
  assert.equal(res.plan[0].baseSha, src.B, 'first-parent base must be B');
});

test('verifyCommits: multi-select preserves deliberate order and records all SHAs', () => {
  const src = chain('multi');
  const res = verifyCommits({ repoDir: src.dir, sourceBaseRef: 'master', commits: [src.B, src.C], mode: 'individual' });
  assert.deepEqual(res.order, [src.B, src.C]);
  assert.deepEqual(res.provenanceCommits, [src.B, src.C]);
});

test('verifyCommits: rejects a non-ancestry-consistent order', () => {
  const src = chain('order');
  assert.throws(
    () => verifyCommits({ repoDir: src.dir, sourceBaseRef: 'master', commits: [src.C, src.B], mode: 'individual' }),
    /not ancestry-consistent/,
  );
});

test('verifyCommits: rejects a SHA not reachable from the protected source branch', () => {
  const src = chain('unreach');
  // A commit on a side branch that is NOT an ancestor of master.
  gOut(src.dir, ['checkout', '-q', '-b', 'feature', src.A]);
  const side = commitFiles(src.dir, 'side: unreachable', { 'src/side.txt': 'side\n' });
  gOut(src.dir, ['checkout', '-q', 'master']);
  assert.throws(
    () => verifyCommits({ repoDir: src.dir, sourceBaseRef: 'master', commits: [side], mode: 'individual' }),
    /not reachable from the configured protected source branch/,
  );
});

test('verifyCommits: cumulative mode requires a verified ancestor base_sha', () => {
  const src = chain('cum');
  const ok = verifyCommits({ repoDir: src.dir, sourceBaseRef: 'master', commits: [src.C], mode: 'cumulative', baseSha: src.A });
  assert.equal(ok.ok, true);
  assert.deepEqual(ok.provenanceCommits, [src.C]);
  const rangePaths = ok.plan[0].entries.map((e) => e.path || e.newPath);
  assert.ok(rangePaths.includes('src/modules/hr/b.txt'), 'cumulative range includes B changes');
  assert.throws(() => verifyCommits({ repoDir: src.dir, sourceBaseRef: 'master', commits: [src.C], mode: 'cumulative' }), /requires base_sha/);
  assert.throws(
    () => verifyCommits({ repoDir: src.dir, sourceBaseRef: 'master', commits: [src.C], mode: 'cumulative', baseSha: src.D }),
    /not an ancestor/,
  );
});

test('verifyCommits: rejects an unknown mode', () => {
  const src = chain('mode');
  assert.throws(() => verifyCommits({ repoDir: src.dir, sourceBaseRef: 'master', commits: [src.C], mode: 'bogus' }), /Unsupported mode/);
});
