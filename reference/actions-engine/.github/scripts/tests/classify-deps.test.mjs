import { test } from 'node:test';
import assert from 'node:assert/strict';

import { classifyPath, inspectPathSafety, DEFAULT_POLICY } from '../handle-special-files.mjs';
import { computeDependencyDelta, reconcileDependencies, checkImportPrerequisites, extractImportSpecifiers, toPackageName } from '../reconcile-dependencies.mjs';
import { compareRanges } from '../lib/semver.mjs';

const policy = DEFAULT_POLICY;

test('classifyPath: policies per §6.4', () => {
  assert.equal(classifyPath('src/modules/hr/index.ts', { policy }).status, 'apply');
  assert.equal(classifyPath('public/logo.png', { policy }).status, 'apply');
  assert.equal(classifyPath('package.json', { policy }).status, 'dependency-merge');
  assert.equal(classifyPath('package-lock.json', { policy }).status, 'regenerate');
  assert.equal(classifyPath('pnpm-lock.yaml', { policy }).status, 'regenerate');
  assert.equal(classifyPath('.github/workflows/ci.yml', { policy }).status, 'main-owned');
  assert.equal(classifyPath('.github/CODEOWNERS', { policy }).status, 'main-owned');
  assert.equal(classifyPath('.env.local', { policy }).status, 'secret-excluded');
  assert.equal(classifyPath('config/credentials.json', { policy }).status, 'secret-excluded');
  assert.equal(classifyPath('certs/server.pem', { policy }).status, 'secret-excluded');
  assert.equal(classifyPath('tsconfig.json', { policy }).status, 'explicit-review');
  assert.equal(classifyPath('next.config.ts', { policy }).status, 'explicit-review');
  assert.equal(classifyPath('packages/app/package.json', { policy }).status, 'explicit-review');
});

test('classifyPath: unsafe paths and special modes are rejected', () => {
  assert.equal(classifyPath('../evil.txt', { policy }).status, 'rejected');
  assert.equal(classifyPath('/etc/passwd', { policy }).status, 'rejected');
  assert.equal(classifyPath('C:/windows/system32', { policy }).status, 'rejected');
  assert.equal(classifyPath('a/../../b', { policy }).status, 'rejected');
  assert.equal(classifyPath('link', { policy, mode: '120000' }).status, 'rejected');
  assert.equal(classifyPath('submodule', { policy, mode: '160000' }).status, 'rejected');
});

test('inspectPathSafety normalizes backslashes', () => {
  assert.deepEqual(inspectPathSafety('src\\modules\\hr\\a.ts'), { safe: true, path: 'src/modules/hr/a.ts' });
});

test('computeDependencyDelta: add / update / move / remove', () => {
  const oldPkg = { dependencies: { a: '^1.0.0', b: '^2.0.0' }, devDependencies: { t: '^1.0.0' } };
  const newPkg = { dependencies: { a: '^1.5.0', c: '^3.0.0' }, devDependencies: { b: '^2.0.0' }, peerDependencies: { p: '^1.0.0' } };
  const d = computeDependencyDelta(oldPkg, newPkg);
  assert.deepEqual(d.added, [{ section: 'dependencies', name: 'c', range: '^3.0.0' }, { section: 'peerDependencies', name: 'p', range: '^1.0.0' }]);
  assert.deepEqual(d.updated, [{ section: 'dependencies', name: 'a', from: '^1.0.0', to: '^1.5.0' }]);
  assert.deepEqual(d.moved, [{ name: 'b', fromSection: 'dependencies', toSection: 'devDependencies', from: '^2.0.0', to: '^2.0.0' }]);
  assert.deepEqual(d.removed, [{ section: 'devDependencies', name: 't', range: '^1.0.0' }]);
});

test('computeDependencyDelta: manifest key changes are classified for review', () => {
  const d = computeDependencyDelta({ scripts: { a: 'x' } }, { scripts: { a: 'y' } });
  assert.deepEqual(d.manifestChanges.mainOwned, ['scripts']);
});

test('reconcileDependencies: add is applied, unrelated packages retained', () => {
  const target = { name: 'main', dependencies: { next: '^16.2.1' }, devDependencies: { typescript: '^5' } };
  const delta = computeDependencyDelta({ dependencies: { next: '^16.2.1' } }, { dependencies: { next: '^16.2.1', exceljs: '^4.4.0' } });
  const res = reconcileDependencies({ deltas: [delta], targetManifest: target });
  assert.equal(res.blockers.length, 0);
  assert.equal(res.manifest.dependencies.exceljs, '^4.4.0');
  assert.equal(res.manifest.dependencies.next, '^16.2.1');
  assert.equal(res.manifest.devDependencies.typescript, '^5');
});

test('reconcileDependencies: identical range is a no-op', () => {
  const delta = computeDependencyDelta({ dependencies: {} }, { dependencies: { a: '^1.0.0' } });
  const res = reconcileDependencies({ deltas: [delta], targetManifest: { dependencies: { a: '^1.0.0' } } });
  assert.equal(res.added.length, 0);
  assert.equal(res.updated.length, 0);
  assert.equal(res.noop.length, 1);
});

test('reconcileDependencies: compatible-but-different range is adopted AND flagged for review', () => {
  const delta = computeDependencyDelta({ dependencies: { a: '^1.0.0' } }, { dependencies: { a: '^1.5.0' } });
  const res = reconcileDependencies({ deltas: [delta], targetManifest: { dependencies: { a: '^1.2.0' } } });
  assert.equal(res.blockers.length, 0);
  assert.equal(res.manifest.dependencies.a, '^1.5.0');
  assert.equal(res.review.length, 1);
  assert.equal(res.updated.length, 1);
});

test('reconcileDependencies: incompatible range fails closed (no force-install)', () => {
  const delta = computeDependencyDelta({ dependencies: { a: '^3.0.0' } }, { dependencies: { a: '^4.4.0' } });
  const res = reconcileDependencies({ deltas: [delta], targetManifest: { dependencies: { a: '^3.0.0' } } });
  assert.equal(res.blockers.length, 1);
  assert.equal(res.blockers[0].code, 'DEPENDENCY_INCOMPATIBLE_RANGE');
  assert.equal(res.manifest.dependencies.a, '^3.0.0', 'manifest must be untouched on conflict');
});

test('reconcileDependencies: removal requires explicit review by default', () => {
  const delta = computeDependencyDelta({ dependencies: { a: '^1.0.0' } }, { dependencies: {} });
  const blocked = reconcileDependencies({ deltas: [delta], targetManifest: { dependencies: { a: '^1.0.0' } } });
  assert.equal(blocked.blockers[0].code, 'DEPENDENCY_REMOVAL_REQUIRES_REVIEW');
  assert.equal(blocked.manifest.dependencies.a, '^1.0.0');
  const allowed = reconcileDependencies({ deltas: [delta], targetManifest: { dependencies: { a: '^1.0.0' } }, allowRemovals: true });
  assert.equal(allowed.removed.length, 1);
  assert.equal(allowed.manifest.dependencies.a, undefined);
});

test('reconcileDependencies: script/workspace/engine changes are blocked (§7)', () => {
  const delta = computeDependencyDelta({ scripts: { build: 'a' } }, { scripts: { build: 'b' } });
  const res = reconcileDependencies({ deltas: [delta], targetManifest: {} });
  assert.equal(res.blockers[0].code, 'MANIFEST_KEY_REQUIRES_REVIEW');
});

test('compareRanges: semver compatibility decisions', () => {
  assert.equal(compareRanges('^1.0.0', '^1.0.0').kind, 'identical');
  assert.equal(compareRanges('^1.0.0', '^1.5.0').kind, 'compatible');
  assert.equal(compareRanges('^1.0.0', '^2.0.0').kind, 'incompatible');
  assert.equal(compareRanges('^0.2.0', '^0.2.5').kind, 'compatible');
  assert.equal(compareRanges('^0.2.0', '^0.3.0').kind, 'incompatible');
  assert.equal(compareRanges('>=2 <3', '^2.5.0').kind, 'compatible');
  assert.equal(compareRanges('workspace:*', '^1.0.0').kind, 'unknown');
  assert.equal(compareRanges('latest', 'next').kind, 'unknown');
  assert.equal(compareRanges('1.2.3', '1.2.3').kind, 'identical');
  assert.equal(compareRanges('*', '^9.9.9').kind, 'compatible');
});

test('import prerequisite detection: package name extraction', () => {
  assert.equal(toPackageName('./local'), null);
  assert.equal(toPackageName('node:fs'), null);
  assert.equal(toPackageName('@/components/x'), null);
  assert.equal(toPackageName('lodash/fp'), 'lodash');
  assert.equal(toPackageName('@scope/pkg/sub'), '@scope/pkg');
  assert.deepEqual(extractImportSpecifiers("import a from 'x';\nconst b = require('y');\nimport('z');"), ['x', 'y', 'z']);
});

test('checkImportPrerequisites flags a dependency added by an unselected commit', () => {
  const files = [{ path: 'src/use.ts', content: "import leftPad from 'left-pad';\nexport default leftPad;\n" }];
  const missing = checkImportPrerequisites({ files, targetManifest: { dependencies: { next: '^16.2.1' } }, deltaAddedNames: [] });
  assert.equal(missing.length, 1);
  assert.equal(missing[0].name, 'left-pad');
  const ok = checkImportPrerequisites({ files, targetManifest: { dependencies: {} }, deltaAddedNames: ['left-pad'] });
  assert.equal(ok.length, 0);
});
