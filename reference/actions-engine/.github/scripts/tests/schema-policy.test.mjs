import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { validateSchema, assertSchema, SchemaError } from '../lib/json-schema.mjs';
import {
  loadConfig,
  loadSubsystem,
  matchGlob,
  isTargetAllowed,
  assertSafeBranchName,
  assertFullSha,
  PolicyError,
} from '../lib/policy.mjs';
import { makeTempDir, initRepo, commitAll, writeRegistry, configObject, subsystemObject } from './helpers/sandbox.mjs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, '..', '..', '..');
const SCHEMA_DIR = path.join(REPO_ROOT, '.integration', 'schema');

function readJson(p) {
  return JSON.parse(fs.readFileSync(p, 'utf8'));
}

test('json-schema: rejects additional properties (fail closed)', () => {
  const schema = { type: 'object', additionalProperties: false, properties: { a: { type: 'string' } } };
  assert.equal(validateSchema(schema, { a: 'x' }).valid, true);
  const res = validateSchema(schema, { a: 'x', b: 1 });
  assert.equal(res.valid, false);
  assert.match(res.errors.join('\n'), /additional property "b"/);
});

test('json-schema: type/enum/const/pattern enforced', () => {
  const schema = {
    type: 'object',
    required: ['n', 'k', 's'],
    properties: {
      n: { type: 'integer', minimum: 1 },
      k: { enum: ['x', 'y'] },
      s: { const: false },
    },
  };
  assert.equal(validateSchema(schema, { n: 2, k: 'x', s: false }).valid, true);
  assert.equal(validateSchema(schema, { n: 0, k: 'x', s: false }).valid, false);
  assert.equal(validateSchema(schema, { n: 2, k: 'z', s: false }).valid, false);
  assert.equal(validateSchema(schema, { n: 2, k: 'x', s: true }).valid, false);
  assert.equal(validateSchema(schema, { n: 2, k: 'x' }).valid, false);
});

test('json-schema: throws on an unsupported keyword (refuses to under-enforce)', () => {
  const schema = { type: 'object', unevaluatedProperties: false };
  assert.throws(() => validateSchema(schema, {}), SchemaError);
});

test('json-schema: local $ref resolution', () => {
  const schema = {
    $defs: { name: { type: 'string', minLength: 1 } },
    type: 'object',
    properties: { n: { $ref: '#/$defs/name' } },
  };
  assert.equal(validateSchema(schema, { n: 'ok' }).valid, true);
  assert.equal(validateSchema(schema, { n: '' }).valid, false);
});

test('real config.json validates against the shipped config schema', () => {
  const schema = readJson(path.join(SCHEMA_DIR, 'config.schema.json'));
  const config = readJson(path.join(REPO_ROOT, '.integration', 'config.json'));
  assertSchema(schema, config, 'config.json');
});

test('real hr-management.json validates against the shipped subsystem schema', () => {
  const schema = readJson(path.join(SCHEMA_DIR, 'subsystem.schema.json'));
  const subsystem = readJson(path.join(REPO_ROOT, '.integration', 'subsystems', 'hr-management.json'));
  assertSchema(schema, subsystem, 'hr-management.json');
  assert.equal(subsystem.initialBaselines.staging.status, 'requires-manual-verification');
  assert.match(subsystem.repository, /^ORG\//, 'placeholder ORG must remain explicit');
});

test('allowDirectPush:true is rejected (policy cannot be weakened to permit direct push)', () => {
  const dir = makeTempDir('cfg');
  initRepo(dir);
  const bad = configObject();
  bad.promotion.allowDirectPush = true;
  writeRegistry(dir, { config: bad });
  assert.throws(() => loadConfig(dir, null), PolicyError);
});

test('malformed JSON registry fails closed', () => {
  const dir = makeTempDir('cfg');
  initRepo(dir);
  fs.mkdirSync(path.join(dir, '.integration', 'subsystems'), { recursive: true });
  fs.writeFileSync(path.join(dir, '.integration', 'config.json'), '{ not json');
  assert.throws(() => loadConfig(dir, null), /not valid JSON/);
});

test('subsystem id must match filename/id', () => {
  const dir = makeTempDir('cfg');
  initRepo(dir);
  const sub = subsystemObject({ id: 'other-id' });
  writeRegistry(dir, { config: configObject(), subsystem: sub, subsystemId: 'hr-management' });
  assert.throws(() => loadSubsystem(dir, null, 'hr-management'), /declares id="other-id"/);
});

test('loadConfig reads authoritative policy from the registry branch ref', () => {
  const dir = makeTempDir('cfg');
  initRepo(dir);
  writeRegistry(dir, { config: configObject({ registryBranch: 'master' }) });
  commitAll(dir, 'add registry');
  const res = loadConfig(dir, 'master');
  assert.equal(res.source, 'ref:master');
  assert.equal(res.config.promotion.allowDirectPush, false);
});

test('matchGlob: safe glob semantics (not regex)', () => {
  assert.equal(matchGlob('release/*', 'release/1.0'), true);
  assert.equal(matchGlob('release/*', 'release/1/0'), false);
  assert.equal(matchGlob('release/*', 'development'), false);
  assert.equal(matchGlob('release/**', 'release/a/b'), true);
  assert.equal(matchGlob('release/v?.0', 'release/v1.0'), true);
  // A regex metacharacter in the pattern must be treated literally, not interpreted.
  assert.equal(matchGlob('a.b', 'axb'), false);
  assert.equal(matchGlob('a.b', 'a.b'), true);
});

test('isTargetAllowed: exact list + patterns', () => {
  const config = configObject();
  assert.equal(isTargetAllowed(config, 'staging').allowed, true);
  assert.equal(isTargetAllowed(config, 'release/2026-01').allowed, true);
  assert.equal(isTargetAllowed(config, 'feature/x').allowed, false);
});

test('assertSafeBranchName rejects unsafe branch names', () => {
  for (const bad of ['-x', '/x', 'x/', 'a..b', 'a b', 'a\\b', 'a:b', 'a^b', 'a?b', 'a*b', 'a@{b}']) {
    assert.throws(() => assertSafeBranchName(bad), PolicyError, `should reject ${bad}`);
  }
  assert.equal(assertSafeBranchName('release/2026-01'), 'release/2026-01');
});

test('assertFullSha requires a full lowercase hex SHA', () => {
  const valid = 'a'.repeat(40);
  assert.equal(assertFullSha(valid), valid);
  assert.throws(() => assertFullSha(valid.slice(0, 7)), PolicyError);
  assert.throws(() => assertFullSha(valid.toUpperCase()), PolicyError);
  assert.throws(() => assertFullSha(`${valid}^`), PolicyError);
});
