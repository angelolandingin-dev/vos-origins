#!/usr/bin/env node
// reconcile-dependencies.mjs — merge a selected source commit's DEPENDENCY DELTA into the
// target manifest, never overwriting the whole manifest, and fail closed on conflicts.
//
// §7 algorithm:
//   1. For each selected source commit, diff `package.json` between SHA^1 and SHA across
//      dependencies / devDependencies / peerDependencies / optionalDependencies.
//   2. Reconcile additions/updates/removals against the *target branch* manifest.
//   3. Fail closed on incompatible ranges, unknown ranges, or unreviewed removals.
//   4. Regenerate the target lockfile separately (never copy the source lockfile).

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { compareRanges } from './lib/semver.mjs';

export const DEP_SECTIONS = ['dependencies', 'devDependencies', 'peerDependencies', 'optionalDependencies'];
const MAIN_OWNED_MANIFEST_KEYS = ['scripts', 'workspaces', 'engines', 'packageManager', 'overrides', 'resolutions', 'peerDependenciesMeta'];
const IGNORED_MANIFEST_KEYS = ['name', 'version', 'description', 'author', 'license', 'private', 'type', 'main', 'module', 'exports', 'types', 'publishConfig', 'files', 'repository', 'bugs', 'homepage', 'keywords'];

const CODE_EXTENSIONS = new Set(['.js', '.jsx', '.ts', '.tsx', '.mjs', '.cjs', '.mts', '.cts', '.vue', '.svelte']);

const NODE_BUILTINS = new Set([
  'assert', 'async_hooks', 'buffer', 'child_process', 'cluster', 'console', 'constants',
  'crypto', 'dgram', 'diagnostics_channel', 'dns', 'domain', 'events', 'fs', 'http',
  'http2', 'https', 'inspector', 'module', 'net', 'os', 'path', 'perf_hooks', 'process',
  'punycode', 'querystring', 'readline', 'repl', 'stream', 'string_decoder', 'sys', 'test',
  'timers', 'tls', 'trace_events', 'tty', 'url', 'util', 'v8', 'vm', 'wasi',
  'worker_threads', 'zlib',
]);

function sectionMap(pkg) {
  const out = {};
  for (const section of DEP_SECTIONS) out[section] = pkg && typeof pkg[section] === 'object' && pkg[section] ? pkg[section] : {};
  return out;
}

/** Diff two package.json objects → dependency delta + manifest-key changes. */
export function computeDependencyDelta(oldPkg, newPkg) {
  const before = sectionMap(oldPkg || {});
  const after = sectionMap(newPkg || {});
  const added = [];
  const updated = [];
  const removed = [];
  const moved = [];

  const names = new Set([...DEP_SECTIONS.flatMap((s) => Object.keys(before[s])), ...DEP_SECTIONS.flatMap((s) => Object.keys(after[s]))]);
  for (const name of names) {
    const presences = DEP_SECTIONS.map((s) => ({ section: s, range: after[s][name], had: Object.prototype.hasOwnProperty.call(before[s], name) })).filter((p) => p.range !== undefined);
    const hads = DEP_SECTIONS.map((s) => ({ section: s, range: before[s][name], had: Object.prototype.hasOwnProperty.call(before[s], name) })).filter((p) => p.had);

    if (hads.length === 0 && presences.length > 0) {
      for (const p of presences) added.push({ section: p.section, name, range: p.range });
      continue;
    }
    if (presences.length === 0 && hads.length > 0) {
      for (const h of hads) removed.push({ section: h.section, name, range: h.range });
      continue;
    }
    if (presences.length > 0 && hads.length > 0) {
      const from = hads[0];
      const to = presences[0];
      if (from.section === to.section) {
        if (from.range !== to.range) updated.push({ section: from.section, name, from: from.range, to: to.range });
      } else {
        moved.push({ name, fromSection: from.section, toSection: to.section, from: from.range, to: to.range });
      }
    }
  }

  const changedTopLevel = [];
  const allKeys = new Set([...Object.keys(oldPkg || {}), ...Object.keys(newPkg || {})]);
  for (const key of allKeys) {
    if (DEP_SECTIONS.includes(key)) continue;
    if (JSON.stringify((oldPkg || {})[key]) !== JSON.stringify((newPkg || {})[key])) changedTopLevel.push(key);
  }

  return {
    added,
    updated,
    removed,
    moved,
    manifestChanges: {
      mainOwned: changedTopLevel.filter((k) => MAIN_OWNED_MANIFEST_KEYS.includes(k)),
      ignored: changedTopLevel.filter((k) => IGNORED_MANIFEST_KEYS.includes(k)),
      other: changedTopLevel.filter((k) => !MAIN_OWNED_MANIFEST_KEYS.includes(k) && !IGNORED_MANIFEST_KEYS.includes(k)),
    },
  };
}

/**
 * Reconcile one or more dependency deltas (already in applied order) into the target manifest.
 * @returns {{ manifest, added, updated, removed, moved, review, blockers, noop }}
 */
export function reconcileDependencies({ deltas, targetManifest, allowRemovals = false }) {
  const manifest = JSON.parse(JSON.stringify(targetManifest));
  for (const section of DEP_SECTIONS) if (!manifest[section]) manifest[section] = {};

  const added = [];
  const updated = [];
  const removed = [];
  const moved = [];
  const review = [];
  const blockers = [];
  const noop = [];

  const findAnywhere = (name) => {
    for (const section of DEP_SECTIONS) {
      if (Object.prototype.hasOwnProperty.call(manifest[section], name)) return { section, range: manifest[section][name] };
    }
    return null;
  };

  // Aggregate per-commit deltas in order. Later commits win for the same (section,name).
  const seen = new Map();
  const orderedEvents = [];
  for (const delta of deltas) {
    for (const a of delta.added) orderedEvents.push({ type: 'add', section: a.section, name: a.name, range: a.range });
    for (const u of delta.updated) orderedEvents.push({ type: 'update', section: u.section, name: u.name, from: u.from, range: u.to });
    for (const m of delta.moved) orderedEvents.push({ type: 'move', name: m.name, fromSection: m.fromSection, toSection: m.toSection, from: m.from, range: m.to });
    for (const r of delta.removed) orderedEvents.push({ type: 'remove', section: r.section, name: r.name, range: r.range });
    for (const key of delta.manifestChanges.mainOwned) {
      blockers.push({
        code: 'MANIFEST_KEY_REQUIRES_REVIEW',
        message: `Selected commit(s) change a reviewed package.json key: "${key}". This requires explicit human review and is not applied automatically.`,
        details: { key },
      });
    }
    for (const key of delta.manifestChanges.other) {
      blockers.push({
        code: 'MANIFEST_SHAPE_REQUIRES_REVIEW',
        message: `Selected commit(s) change non-dependency package.json field "${key}", which is not applied automatically; explicit review required.`,
        details: { key },
      });
    }
    if (delta.manifestChanges.ignored.length) {
      noop.push({ note: `main-owned manifest fields ignored: ${delta.manifestChanges.ignored.join(', ')}` });
    }
  }

  const eventKey = (e) => `${e.section || e.toSection || ''}:${e.name}`;
  for (const event of orderedEvents) seen.set(eventKey(event), event);
  const effectiveEvents = [...seen.values()];

  for (const event of effectiveEvents) {
    if (event.type === 'remove') {
      const current = findAnywhere(event.name);
      if (!current) {
        noop.push({ name: event.name, note: 'removal requested but not present in target' });
        continue;
      }
      if (!allowRemovals) {
        blockers.push({
          code: 'DEPENDENCY_REMOVAL_REQUIRES_REVIEW',
          message: `Selected commit(s) remove "${event.name}", but the main system and other active modules may still require it. Removal requires explicit review.`,
          details: { name: event.name, targetRange: current.range },
        });
        continue;
      }
      for (const section of DEP_SECTIONS) delete manifest[section][event.name];
      removed.push({ name: event.name, previousRange: current.range });
      continue;
    }

    if (event.type === 'move') {
      const current = findAnywhere(event.name);
      if (current && current.range !== event.range) {
        const cmp = compareRanges(current.range, event.range);
        if (cmp.kind === 'incompatible') {
          blockers.push({
            code: 'DEPENDENCY_INCOMPATIBLE_RANGE',
            message: `Cannot relocate "${event.name}" from ${current.section} to ${event.toSection}: target range ${current.range} is incompatible with source range ${event.range}.`,
            details: { name: event.name, targetRange: current.range, sourceRange: event.range },
          });
          continue;
        }
      }
      for (const section of DEP_SECTIONS) delete manifest[section][event.name];
      manifest[event.toSection][event.name] = event.range;
      moved.push({ name: event.name, fromSection: event.fromSection, toSection: event.toSection, range: event.range });
      continue;
    }

    // add / update
    const sourceRange = event.range;
    const current = findAnywhere(event.name);
    const targetSection = event.section;

    if (!current) {
      manifest[targetSection][event.name] = sourceRange;
      added.push({ section: targetSection, name: event.name, range: sourceRange });
      continue;
    }

    const cmp = compareRanges(current.range, sourceRange);
    if (cmp.kind === 'identical') {
      noop.push({ name: event.name, note: 'target already satisfies an identical range' });
      continue;
    }
    if (cmp.kind === 'incompatible') {
      blockers.push({
        code: 'DEPENDENCY_INCOMPATIBLE_RANGE',
        message: `Incompatible dependency range for "${event.name}": target has ${current.range}, selected source requires ${sourceRange}. Fail closed — human decision required.`,
        details: { name: event.name, targetRange: current.range, sourceRange, section: targetSection },
      });
      continue;
    }
    if (cmp.kind === 'unknown') {
      blockers.push({
        code: 'DEPENDENCY_RANGE_UNKNOWN',
        message: `Cannot prove compatibility for "${event.name}" (target ${current.range} vs source ${sourceRange}: ${cmp.reason || 'unknown'}). Fail closed.`,
        details: { name: event.name, targetRange: current.range, sourceRange },
      });
      continue;
    }
    // compatible, different
    if (current.section !== targetSection) {
      delete manifest[current.section][event.name];
      moved.push({ name: event.name, fromSection: current.section, toSection: targetSection, range: sourceRange });
    }
    manifest[targetSection][event.name] = sourceRange;
    updated.push({ section: targetSection, name: event.name, from: current.range, to: sourceRange });
    review.push({
      name: event.name,
      section: targetSection,
      from: current.range,
      to: sourceRange,
      note: 'compatible but different range; reconciled — requires reviewer awareness',
    });
  }

  return { manifest, added, updated, removed, moved, review, blockers, noop };
}

// --- Import prerequisite heuristic -------------------------------------------------

export function extractImportSpecifiers(content) {
  if (typeof content !== 'string') return [];
  const specs = new Set();
  const patterns = [
    /\bfrom\s*['"]([^'"]+)['"]/g,
    /\brequire\(\s*['"]([^'"]+)['"]\s*\)/g,
    /\bimport\(\s*['"]([^'"]+)['"]\s*\)/g,
    /^\s*import\s*['"]([^'"]+)['"]/gm,
  ];
  for (const re of patterns) {
    let m;
    while ((m = re.exec(content)) !== null) specs.add(m[1]);
  }
  return [...specs];
}

export function toPackageName(specifier) {
  if (!specifier) return null;
  if (specifier.startsWith('.') || specifier.startsWith('/') || specifier.startsWith('#') || specifier.startsWith('node:') || specifier.startsWith('@/') || specifier.startsWith('~')) {
    return null;
  }
  if (specifier.startsWith('bun:')) return null;
  const parts = specifier.split('/');
  if (specifier.startsWith('@')) return parts.length >= 2 ? `${parts[0]}/${parts[1]}` : null;
  return parts[0] || null;
}

/**
 * Heuristic prerequisite check (§5.2/§7): does a selected commit import a package that is
 * neither already in the target manifest nor added by the selected delta itself? If so, the
 * integration likely depends on an UNSELECTED commit that introduced the dependency.
 */
export function checkImportPrerequisites({ files, targetManifest, deltaAddedNames, extraAvailable = [] }) {
  const available = new Set(extraAvailable);
  for (const section of DEP_SECTIONS) {
    for (const name of Object.keys(sectionMap(targetManifest)[section])) available.add(name);
  }
  for (const name of deltaAddedNames || []) available.add(name);

  const missing = new Map();
  for (const file of files) {
    const ext = path.extname(file.path).toLowerCase();
    if (!CODE_EXTENSIONS.has(ext)) continue;
    for (const spec of extractImportSpecifiers(file.content)) {
      const name = toPackageName(spec);
      if (!name) continue;
      const bare = name.startsWith('node:') ? name.slice(5) : name;
      if (NODE_BUILTINS.has(bare)) continue;
      if (available.has(name)) continue;
      const list = missing.get(name) || [];
      list.push({ path: file.path, specifier: spec });
      missing.set(name, list);
    }
  }
  return [...missing.entries()].map(([name, refs]) => ({ name, refs }));
}

// --- CLI ------------------------------------------------------------------------

function main() {
  const raw = fs.readFileSync(0, 'utf8').trim();
  const req = JSON.parse(raw);
  const targetManifest = req.targetManifestPath ? JSON.parse(fs.readFileSync(req.targetManifestPath, 'utf8')) : req.targetManifest;
  const deltas = (req.pairs || []).map((p) => computeDependencyDelta(p.oldPkg, p.newPkg));
  const result = reconcileDependencies({ deltas, targetManifest, allowRemovals: !!req.allowRemovals });
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

const invokedDirectly = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedDirectly) {
  try {
    main();
  } catch (err) {
    process.stderr.write(`${err.name}: ${err.message}\n`);
    process.exitCode = 1;
  }
}
