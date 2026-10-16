// Pragmatic semver range-compatibility checker.
//
// Used by dependency reconciliation to decide whether a source requirement can be
// reconciled into the target manifest. It is intentionally conservative: anything it
// cannot reason about is reported as "unknown", and the engine fails closed on unknown
// or incompatible results rather than force-adopting a range.
//
// Supported: exact, ^, ~, >=, >, <=, <, ranges with spaces (AND), `||` (OR),
// wildcards (*, x, X), partial versions (1, 1.2), and npm alias specs (npm:name@range).
// Prerelease tags are ignored for interval math (documented simplification).

const WILDCARD = /^[xX*]$/;

function parseVersion(part) {
  const cleaned = part.replace(/^[v=]+/, '');
  const [coreRaw] = cleaned.split('+');
  const [core] = coreRaw.split('-');
  const bits = core.split('.');
  const nums = [];
  for (let i = 0; i < 3; i += 1) {
    const token = bits[i];
    if (token === undefined || WILDCARD.test(token)) return { nums, wildcardFrom: i };
    if (!/^\d+$/.test(token)) return null;
    nums.push(Number(token));
  }
  return { nums, wildcardFrom: null };
}

function pad(nums) {
  const out = [nums[0] ?? 0, nums[1] ?? 0, nums[2] ?? 0];
  return out;
}

function cmp(a, b) {
  for (let i = 0; i < 3; i += 1) {
    if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
  }
  return 0;
}

const NEG_INF = null;
const POS_INF = null;

function interval(lo, loInclusive, hi, hiInclusive) {
  return { lo, loInclusive, hi, hiInclusive };
}

function intersectIntervals(a, b) {
  let lo;
  let loInclusive;
  if (a.lo === NEG_INF) {
    lo = b.lo;
    loInclusive = b.loInclusive;
  } else if (b.lo === NEG_INF) {
    lo = a.lo;
    loInclusive = a.loInclusive;
  } else {
    const c = cmp(a.lo, b.lo);
    if (c > 0) {
      lo = a.lo;
      loInclusive = a.loInclusive;
    } else if (c < 0) {
      lo = b.lo;
      loInclusive = b.loInclusive;
    } else {
      lo = a.lo;
      loInclusive = a.loInclusive && b.loInclusive;
    }
  }

  let hi;
  let hiInclusive;
  if (a.hi === POS_INF) {
    hi = b.hi;
    hiInclusive = b.hiInclusive;
  } else if (b.hi === POS_INF) {
    hi = a.hi;
    hiInclusive = a.hiInclusive;
  } else {
    const c = cmp(a.hi, b.hi);
    if (c < 0) {
      hi = a.hi;
      hiInclusive = a.hiInclusive;
    } else if (c > 0) {
      hi = b.hi;
      hiInclusive = b.hiInclusive;
    } else {
      hi = a.hi;
      hiInclusive = a.hiInclusive && b.hiInclusive;
    }
  }

  if (lo !== NEG_INF && hi !== POS_INF) {
    const c = cmp(lo, hi);
    if (c > 0) return null;
    if (c === 0 && !(loInclusive && hiInclusive)) return null;
  }
  return interval(lo, loInclusive, hi, hiInclusive);
}

function comparatorInterval(op, version) {
  const parsed = parseVersion(version);
  if (!parsed) return { ok: false };

  // Comparison operators (`>=`, `>`, `<=`, `<`) treat a partial version as the padded
  // lower/upper bound; wildcards/`x` alone are still unbounded.
  if (['>=', '>', '<=', '<'].includes(op)) {
    if (parsed.wildcardFrom !== null && parsed.nums.length === 0) {
      return { ok: true, iv: interval(NEG_INF, false, POS_INF, false) };
    }
    const v = pad(parsed.nums);
    if (op === '>=') return { ok: true, iv: interval(v, true, POS_INF, false) };
    if (op === '>') return { ok: true, iv: interval(v, false, POS_INF, false) };
    if (op === '<=') return { ok: true, iv: interval(NEG_INF, false, v, true) };
    return { ok: true, iv: interval(NEG_INF, false, v, false) };
  }

  if (parsed.wildcardFrom !== null) {
    const nums = parsed.nums;
    const major = nums[0] ?? 0;
    if (parsed.wildcardFrom === 0) return { ok: true, iv: interval(NEG_INF, false, POS_INF, false) };
    if (parsed.wildcardFrom === 1) return { ok: true, iv: interval([major, 0, 0], true, [major + 1, 0, 0], false) };
    return { ok: true, iv: interval([major, nums[1], 0], true, [major, nums[1] + 1, 0], false) };
  }
  const v = pad(parsed.nums);
  switch (op) {
    case '^': {
      let hi;
      if (v[0] !== 0) hi = [v[0] + 1, 0, 0];
      else if (v[1] !== 0) hi = [0, v[1] + 1, 0];
      else hi = [0, 0, v[2] + 1];
      return { ok: true, iv: interval(v, true, hi, false) };
    }
    case '~': {
      const parsedLen = parsed.nums.length;
      const hi = parsedLen >= 2 ? [v[0], v[1] + 1, 0] : [v[0] + 1, 0, 0];
      return { ok: true, iv: interval(v, true, hi, false) };
    }
    case '=':
    case '':
    default: {
      // Bare version: partial counts act as ranges.
      const len = parsed.nums.length;
      if (len === 3) return { ok: true, iv: interval(v, true, v, true) };
      if (len === 2) return { ok: true, iv: interval(v, true, [v[0], v[1] + 1, 0], false) };
      return { ok: true, iv: interval(v, true, [v[0] + 1, 0, 0], false) };
    }
  }
}

function parseComparatorSet(set) {
  const tokens = set.trim().split(/\s+/).filter(Boolean);
  if (tokens.length === 0) return { ok: true, iv: interval(NEG_INF, false, POS_INF, false) };
  let acc = interval(NEG_INF, false, POS_INF, false);
  for (const token of tokens) {
    let op = '';
    let version = token;
    const m = token.match(/^(>=|<=|>|<|\^|~|=)/);
    if (m) {
      op = m[1];
      version = token.slice(op.length);
    }
    if (WILDCARD.test(version)) {
      // `*` alone — no constraint. `>=*` etc. are unusual; treat as unbounded.
      if (['>=', '<=', '>', '<'].includes(op)) continue;
      continue;
    }
    const res = comparatorInterval(op, version);
    if (!res.ok) return { ok: false };
    const next = intersectIntervals(acc, res.iv);
    if (!next) return { ok: true, empty: true, iv: null };
    acc = next;
  }
  return { ok: true, iv: acc };
}

function normalizeSpec(raw) {
  if (typeof raw !== 'string') return { kind: 'invalid' };
  const spec = raw.trim();
  if (spec === '') return { kind: 'any' };
  // npm aliases: npm:name@range
  if (spec.startsWith('npm:')) {
    const at = spec.lastIndexOf('@');
    if (at > 4) return normalizeSpec(spec.slice(at + 1));
    return { kind: 'alias', raw: spec };
  }
  if (/^(link:|file:|portal:|workspace:|https?:|git\+|git:|github:)/.test(spec)) {
    return { kind: 'nonsemver', raw: spec };
  }
  if (/^[a-zA-Z][a-zA-Z0-9-]*$/.test(spec) && !WILDCARD.test(spec)) {
    // dist-tags: latest, next, beta ...
    return { kind: 'tag', raw: spec };
  }
  const sets = spec.split('||');
  const intervals = [];
  for (const set of sets) {
    const parsed = parseComparatorSet(set);
    if (!parsed.ok) return { kind: 'unparseable', raw: spec };
    if (parsed.empty) continue;
    intervals.push(parsed.iv);
  }
  if (intervals.length === 0) return { kind: 'empty', raw: spec };
  return { kind: 'range', intervals, raw: spec };
}

/**
 * Compare two dependency range specs.
 * Returns one of: 'identical' (equal semantics), 'compatible' (intersection non-empty),
 * 'incompatible' (disjoint), 'unknown' (cannot prove either way → caller must fail closed).
 */
export function compareRanges(a, b) {
  if (a === b) return { kind: 'identical' };
  const na = normalizeSpec(a);
  const nb = normalizeSpec(b);
  if (na.kind === 'invalid' || nb.kind === 'invalid') return { kind: 'unknown', reason: 'invalid spec' };
  if (na.kind === 'any' || nb.kind === 'any') return { kind: 'compatible', reason: 'wildcard' };
  if (na.kind === 'unparseable' || nb.kind === 'unparseable') return { kind: 'unknown', reason: 'unparseable range' };
  if (na.kind === 'nonsemver' || nb.kind === 'nonsemver' || na.kind === 'alias' || nb.kind === 'alias') {
    return na.raw === nb.raw ? { kind: 'identical' } : { kind: 'unknown', reason: 'non-semver spec differs' };
  }
  if (na.kind === 'tag' || nb.kind === 'tag') {
    return na.raw === nb.raw ? { kind: 'identical' } : { kind: 'unknown', reason: 'dist-tag differs' };
  }
  if (na.kind === 'empty' || nb.kind === 'empty') return { kind: 'incompatible', reason: 'empty range' };
  for (const ia of na.intervals) {
    for (const ib of nb.intervals) {
      if (intersectIntervals(ia, ib)) return { kind: 'compatible' };
    }
  }
  return { kind: 'incompatible' };
}

export function isCompatible(a, b) {
  const res = compareRanges(a, b);
  return res.kind === 'identical' || res.kind === 'compatible';
}
