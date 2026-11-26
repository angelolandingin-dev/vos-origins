export type SyntaxTokenKind =
  | 'plain'
  | 'keyword'
  | 'atom'
  | 'variable'
  | 'string'
  | 'comment'
  | 'tag'
  | 'attr'
  | 'entity'
  | 'number';

export interface SyntaxToken {
  kind: SyntaxTokenKind;
  text: string;
}

type LanguageFamily = 'js' | 'rust' | 'json' | 'markup' | 'css' | 'config' | 'sql' | 'plain';

const JS_ATOMS = new Set(['true', 'false', 'null', 'undefined', 'NaN', 'Infinity']);
const RUST_ATOMS = new Set(['true', 'false']);
const JSON_ATOMS = new Set(['true', 'false', 'null']);
const CONFIG_ATOMS = new Set(['true', 'false', 'null', 'yes', 'no', 'on', 'off']);

const JS_DEF_KEYWORDS = new Set([
  'const',
  'let',
  'var',
  'function',
  'class',
  'interface',
  'type',
  'enum',
  'import',
]);

const JS_KEYWORDS = new Set([
  'as',
  'async',
  'await',
  'break',
  'case',
  'catch',
  'class',
  'const',
  'continue',
  'debugger',
  'default',
  'delete',
  'do',
  'else',
  'enum',
  'export',
  'extends',
  'finally',
  'for',
  'from',
  'function',
  'if',
  'implements',
  'import',
  'in',
  'instanceof',
  'interface',
  'let',
  'new',
  'of',
  'package',
  'private',
  'protected',
  'public',
  'readonly',
  'return',
  'static',
  'super',
  'switch',
  'this',
  'throw',
  'try',
  'type',
  'typeof',
  'var',
  'void',
  'while',
  'with',
  'yield',
]);

const RUST_KEYWORDS = new Set([
  'as',
  'async',
  'await',
  'break',
  'const',
  'continue',
  'crate',
  'dyn',
  'else',
  'enum',
  'extern',
  'fn',
  'for',
  'if',
  'impl',
  'in',
  'let',
  'loop',
  'match',
  'mod',
  'move',
  'mut',
  'pub',
  'ref',
  'return',
  'self',
  'Self',
  'static',
  'struct',
  'super',
  'trait',
  'type',
  'unsafe',
  'use',
  'where',
  'while',
]);

const SQL_KEYWORDS = new Set([
  'select',
  'from',
  'where',
  'insert',
  'into',
  'values',
  'update',
  'set',
  'delete',
  'create',
  'table',
  'alter',
  'drop',
  'index',
  'join',
  'left',
  'right',
  'inner',
  'outer',
  'on',
  'group',
  'by',
  'order',
  'having',
  'limit',
  'as',
  'and',
  'or',
  'not',
  'null',
  'is',
  'in',
  'exists',
  'distinct',
  'true',
  'false',
  'begin',
  'commit',
  'rollback',
]);

const CSS_KEYWORDS = new Set([
  '!important',
  '@import',
  '@media',
  '@theme',
  '@custom-variant',
  '@layer',
]);

function familyFor(language: string): LanguageFamily {
  const normalized = language.trim().toLowerCase();
  if (
    normalized === 'typescript' ||
    normalized === 'tsx' ||
    normalized === 'ts' ||
    normalized === 'javascript' ||
    normalized === 'jsx' ||
    normalized === 'js' ||
    normalized.includes('typescript') ||
    normalized.includes('javascript')
  ) {
    return 'js';
  }
  if (normalized === 'rust' || normalized === 'rs') return 'rust';
  if (normalized === 'json' || normalized === 'jsonc') return 'json';
  if (normalized === 'html' || normalized === 'xml' || normalized === 'svg') return 'markup';
  if (normalized === 'css' || normalized === 'scss') return 'css';
  if (
    normalized === 'yaml' ||
    normalized === 'yml' ||
    normalized === 'toml' ||
    normalized === 'sh' ||
    normalized === 'bash' ||
    normalized === 'shell'
  ) {
    return 'config';
  }
  if (normalized === 'sql') return 'sql';
  return 'plain';
}

function isIdentStart(ch: string): boolean {
  return (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') || ch === '_' || ch === '$';
}

function isIdentPart(ch: string): boolean {
  return isIdentStart(ch) || (ch >= '0' && ch <= '9') || ch === '-';
}

function isDigit(ch: string): boolean {
  return ch >= '0' && ch <= '9';
}

function isHexDigit(ch: string): boolean {
  return isDigit(ch) || (ch >= 'a' && ch <= 'f') || (ch >= 'A' && ch <= 'F') || ch === '_';
}

function pushToken(tokens: SyntaxToken[], kind: SyntaxTokenKind, text: string) {
  if (!text) return;
  const last = tokens[tokens.length - 1];
  if (kind === 'plain' && last?.kind === 'plain') {
    last.text += text;
  } else {
    tokens.push({ kind, text });
  }
}

function scanQuoted(text: string, start: number, quote: string): number {
  let i = start + 1;
  while (i < text.length) {
    const ch = text[i];
    if (ch === '\\') {
      i += 2;
      continue;
    }
    if (ch === quote) {
      return i + 1;
    }
    i += 1;
  }
  return text.length;
}

function atomSetFor(family: LanguageFamily): Set<string> | null {
  switch (family) {
    case 'js':
      return JS_ATOMS;
    case 'rust':
      return RUST_ATOMS;
    case 'json':
      return JSON_ATOMS;
    case 'config':
      return CONFIG_ATOMS;
    default:
      return null;
  }
}

function keywordSetFor(family: LanguageFamily): Set<string> | null {
  switch (family) {
    case 'js':
      return JS_KEYWORDS;
    case 'rust':
      return RUST_KEYWORDS;
    case 'sql':
      return SQL_KEYWORDS;
    case 'css':
      return CSS_KEYWORDS;
    default:
      return null;
  }
}

export function tokenizeDiffLine(text: string, language: string): SyntaxToken[] {
  const family = familyFor(language);
  if (family === 'plain' || text.length === 0) {
    return [{ kind: 'plain', text }];
  }

  const keywords = keywordSetFor(family);
  const atoms = atomSetFor(family);
  const tokens: SyntaxToken[] = [];
  let inTag = false;
  let inImportList = false;
  let prevChar = '';
  let prevWord = '';
  let i = 0;

  while (i < text.length) {
    const ch = text[i] ?? '';
    const next = text[i + 1] ?? '';

    if (ch === ' ' || ch === '\t') {
      let end = i + 1;
      while (end < text.length && (text[end] === ' ' || text[end] === '\t')) end += 1;
      pushToken(tokens, 'plain', text.slice(i, end));
      i = end;
      continue;
    }

    if (
      ((family === 'js' || family === 'rust' || family === 'json' || family === 'css') &&
        ch === '/' &&
        next === '/') ||
      (family === 'config' && ch === '#') ||
      (family === 'sql' && ch === '-' && next === '-')
    ) {
      pushToken(tokens, 'comment', text.slice(i));
      break;
    }

    if (
      (family === 'js' || family === 'rust' || family === 'json' || family === 'css') &&
      ch === '/' &&
      next === '*'
    ) {
      const end = text.indexOf('*/', i + 2);
      if (end === -1) {
        pushToken(tokens, 'comment', text.slice(i));
        break;
      }
      pushToken(tokens, 'comment', text.slice(i, end + 2));
      i = end + 2;
      continue;
    }

    if (family === 'markup' && text.startsWith('<!--', i)) {
      const end = text.indexOf('-->', i + 4);
      if (end === -1) {
        pushToken(tokens, 'comment', text.slice(i));
        break;
      }
      pushToken(tokens, 'comment', text.slice(i, end + 3));
      i = end + 3;
      continue;
    }

    if (ch === '"' || ch === "'" || (ch === '`' && family !== 'sql')) {
      const end = scanQuoted(text, i, ch);
      const literal = text.slice(i, end);
      if (family === 'json' && ch === '"') {
        let look = end;
        while (look < text.length && (text[look] === ' ' || text[look] === '\t')) {
          look += 1;
        }
        pushToken(tokens, text[look] === ':' ? 'attr' : 'string', literal);
      } else {
        pushToken(tokens, 'string', literal);
      }
      prevChar = ch;
      prevWord = '';
      i = end;
      continue;
    }

    const canStartJsxTag =
      family === 'markup' ||
      (family === 'js' &&
        (!prevChar || !isIdentPart(prevChar)) &&
        prevChar !== ')' &&
        prevChar !== ']');

    if (canStartJsxTag && ch === '<' && (isIdentStart(next) || next === '/')) {
      let tagStart = i + 1;
      if (next === '/') tagStart += 1;
      let tagEnd = tagStart;
      while (tagEnd < text.length) {
        const tc = text[tagEnd] ?? '';
        if (isIdentPart(tc) || tc === '.') tagEnd += 1;
        else break;
      }
      if (tagEnd > tagStart) {
        inTag = next !== '/';
        pushToken(tokens, 'plain', text.slice(i, tagStart));
        pushToken(tokens, 'tag', text.slice(tagStart, tagEnd));
        prevChar = text[tagEnd - 1] ?? '';
        prevWord = '';
        i = tagEnd;
        continue;
      }
    }

    if (inTag && ch === '>' && text[i - 1] !== '=') {
      inTag = false;
      pushToken(tokens, 'plain', ch);
      prevChar = ch;
      prevWord = '';
      i += 1;
      continue;
    }

    if (family === 'css' && (ch === '@' || ch === '!')) {
      let end = i + 1;
      while (end < text.length && isIdentPart(text[end] ?? '')) end += 1;
      const word = text.slice(i, end);
      pushToken(tokens, CSS_KEYWORDS.has(word) ? 'keyword' : 'plain', word);
      prevChar = word[word.length - 1] ?? '';
      prevWord = word;
      i = end;
      continue;
    }

    if (isDigit(ch) || (ch === '#' && family === 'css' && isHexDigit(next))) {
      let end = i + 1;
      if (ch === '0' && (next === 'x' || next === 'X' || next === 'b' || next === 'B')) {
        end += 1;
        while (end < text.length && isHexDigit(text[end] ?? '')) end += 1;
      } else {
        while (end < text.length) {
          const nc = text[end] ?? '';
          if (isHexDigit(nc) || nc === '.') end += 1;
          else break;
        }
      }
      pushToken(tokens, 'number', text.slice(i, end));
      prevChar = text[end - 1] ?? '';
      prevWord = '';
      i = end;
      continue;
    }

    if (isIdentStart(ch)) {
      let end = i + 1;
      while (end < text.length) {
        const wc = text[end] ?? '';
        // Only treat hyphen inside identifiers for CSS/config/markup attrs.
        if (wc === '-' && family !== 'css' && family !== 'config' && family !== 'markup') {
          break;
        }
        if (isIdentPart(wc)) end += 1;
        else break;
      }
      const word = text.slice(i, end);
      const lookup = family === 'sql' ? word.toLowerCase() : word;

      if (atoms?.has(lookup)) {
        pushToken(tokens, 'atom', word);
      } else if (keywords?.has(lookup)) {
        pushToken(tokens, 'keyword', word);
        if (family === 'js' && word === 'import') inImportList = true;
        if (family === 'js' && word === 'from') inImportList = false;
      } else {
        let look = end;
        while (look < text.length && (text[look] === ' ' || text[look] === '\t')) {
          look += 1;
        }
        const after = text[look] ?? '';
        if (
          after === '=' &&
          inTag &&
          (family === 'markup' || family === 'js') &&
          text[look + 1] !== '='
        ) {
          pushToken(tokens, 'attr', word);
        } else if (family === 'css' && after === ':' && text[look + 1] !== ':') {
          pushToken(tokens, 'atom', word);
        } else if (family === 'config' && after === ':' && text[look + 1] !== ':') {
          pushToken(tokens, 'attr', word);
        } else if (family === 'js') {
          const isPropertyAccess = prevChar === '.';
          const isObjectKey =
            after === ':' &&
            text[look + 1] !== ':' &&
            (prevChar === '{' || prevChar === ',' || prevChar === '');
          const isDefinition = inImportList || JS_DEF_KEYWORDS.has(prevWord);
          if (isPropertyAccess || isObjectKey || isDefinition) {
            pushToken(tokens, 'plain', word);
          } else {
            pushToken(tokens, 'variable', word);
          }
        } else if (family === 'rust') {
          const isPropertyAccess = prevChar === '.';
          const isDefinition =
            prevWord === 'fn' ||
            prevWord === 'let' ||
            prevWord === 'struct' ||
            prevWord === 'enum' ||
            prevWord === 'trait' ||
            prevWord === 'mod';
          if (isPropertyAccess || isDefinition) {
            pushToken(tokens, 'plain', word);
          } else {
            pushToken(tokens, 'variable', word);
          }
        } else if (after === '(') {
          pushToken(tokens, 'entity', word);
        } else {
          pushToken(tokens, 'plain', word);
        }
      }
      prevChar = word[word.length - 1] ?? '';
      prevWord = word;
      i = end;
      continue;
    }

    if (family === 'js' && ch === '}') inImportList = false;
    pushToken(tokens, 'plain', ch);
    prevChar = ch;
    prevWord = '';
    i += 1;
  }

  return tokens;
}

export interface DiffRange {
  start: number;
  end: number;
}

export interface HighlightedSyntaxToken extends SyntaxToken {
  emphasized: boolean;
}

interface TextSegment {
  text: string;
  start: number;
  end: number;
}

function segmentDiffLine(text: string): TextSegment[] {
  const segments: TextSegment[] = [];
  const pattern = /\s+|[A-Za-z0-9_$]+|./g;
  let match = pattern.exec(text);
  while (match !== null) {
    const value = match[0];
    segments.push({
      text: value,
      start: match.index,
      end: match.index + value.length,
    });
    match = pattern.exec(text);
  }
  return segments;
}

function mergeRanges(ranges: DiffRange[]): DiffRange[] {
  if (ranges.length <= 1) return ranges;
  const merged: DiffRange[] = [{ ...ranges[0]! }];
  for (let i = 1; i < ranges.length; i += 1) {
    const next = ranges[i]!;
    const prev = merged[merged.length - 1]!;
    if (next.start <= prev.end) {
      prev.end = Math.max(prev.end, next.end);
    } else {
      merged.push({ ...next });
    }
  }
  return merged;
}

function nonWsLength(text: string): number {
  let count = 0;
  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i];
    if (ch !== ' ' && ch !== '\t') count += 1;
  }
  return count;
}

function rangeChars(ranges: DiffRange[]): number {
  let sum = 0;
  for (const r of ranges) sum += r.end - r.start;
  return sum;
}

export function computeInlineDiff(
  oldText: string,
  newText: string,
): { oldRanges: DiffRange[]; newRanges: DiffRange[] } {
  if (oldText === newText) {
    return { oldRanges: [], newRanges: [] };
  }

  const oldSegs = segmentDiffLine(oldText);
  const newSegs = segmentDiffLine(newText);

  let prefix = 0;
  const minSegs = Math.min(oldSegs.length, newSegs.length);
  while (prefix < minSegs && oldSegs[prefix]?.text === newSegs[prefix]?.text) {
    prefix += 1;
  }

  let oldEnd = oldSegs.length;
  let newEnd = newSegs.length;
  while (
    oldEnd > prefix &&
    newEnd > prefix &&
    oldSegs[oldEnd - 1]?.text === newSegs[newEnd - 1]?.text
  ) {
    oldEnd -= 1;
    newEnd -= 1;
  }

  const oldMid = oldSegs.slice(prefix, oldEnd);
  const newMid = newSegs.slice(prefix, newEnd);

  let oldRanges: DiffRange[] = [];
  let newRanges: DiffRange[] = [];

  if (oldMid.length === 0) {
    if (newMid.length > 0) {
      newRanges = [{ start: newMid[0]!.start, end: newMid[newMid.length - 1]!.end }];
    }
  } else if (newMid.length === 0) {
    oldRanges = [{ start: oldMid[0]!.start, end: oldMid[oldMid.length - 1]!.end }];
  } else if (oldMid.length > 120 || newMid.length > 120) {
    oldRanges = [{ start: oldMid[0]!.start, end: oldMid[oldMid.length - 1]!.end }];
    newRanges = [{ start: newMid[0]!.start, end: newMid[newMid.length - 1]!.end }];
  } else {
    const m = oldMid.length;
    const n = newMid.length;
    const cols = n + 1;
    const dp = new Uint16Array((m + 1) * cols);

    for (let i = m - 1; i >= 0; i -= 1) {
      for (let j = n - 1; j >= 0; j -= 1) {
        if (oldMid[i]?.text === newMid[j]?.text) {
          dp[i * cols + j] = (dp[(i + 1) * cols + (j + 1)] ?? 0) + 1;
        } else {
          const down = dp[(i + 1) * cols + j] ?? 0;
          const right = dp[i * cols + (j + 1)] ?? 0;
          dp[i * cols + j] = down >= right ? down : right;
        }
      }
    }

    let i = 0;
    let j = 0;
    while (i < m && j < n) {
      if (oldMid[i]?.text === newMid[j]?.text) {
        i += 1;
        j += 1;
      } else if ((dp[(i + 1) * cols + j] ?? 0) >= (dp[i * cols + (j + 1)] ?? 0)) {
        const seg = oldMid[i]!;
        oldRanges.push({ start: seg.start, end: seg.end });
        i += 1;
      } else {
        const seg = newMid[j]!;
        newRanges.push({ start: seg.start, end: seg.end });
        j += 1;
      }
    }
    while (i < m) {
      const seg = oldMid[i]!;
      oldRanges.push({ start: seg.start, end: seg.end });
      i += 1;
    }
    while (j < n) {
      const seg = newMid[j]!;
      newRanges.push({ start: seg.start, end: seg.end });
      j += 1;
    }

    oldRanges = mergeRanges(oldRanges);
    newRanges = mergeRanges(newRanges);
  }

  // Avoid saturating completely rewritten lines with full-width bright emphasis.
  const oldBase = Math.max(1, nonWsLength(oldText));
  const newBase = Math.max(1, nonWsLength(newText));
  if (rangeChars(oldRanges) / oldBase > 0.75 && rangeChars(newRanges) / newBase > 0.75) {
    return { oldRanges: [], newRanges: [] };
  }

  return { oldRanges, newRanges };
}

export function applyDiffHighlights(
  tokens: SyntaxToken[],
  ranges: DiffRange[],
): HighlightedSyntaxToken[] {
  if (ranges.length === 0) {
    return tokens.map((token) => ({ ...token, emphasized: false }));
  }

  const result: HighlightedSyntaxToken[] = [];
  let offset = 0;
  let rangeIndex = 0;

  for (const token of tokens) {
    let cursor = 0;
    const tokenLen = token.text.length;
    while (cursor < tokenLen) {
      const absStart = offset + cursor;
      const absEnd = offset + tokenLen;

      while (rangeIndex < ranges.length && ranges[rangeIndex]!.end <= absStart) {
        rangeIndex += 1;
      }

      const active = ranges[rangeIndex];
      if (!active || active.start >= absEnd) {
        result.push({
          kind: token.kind,
          text: token.text.slice(cursor),
          emphasized: false,
        });
        cursor = tokenLen;
      } else if (absStart < active.start) {
        const splitAt = active.start - offset;
        result.push({
          kind: token.kind,
          text: token.text.slice(cursor, splitAt),
          emphasized: false,
        });
        cursor = splitAt;
      } else {
        const splitAt = Math.min(tokenLen, active.end - offset);
        result.push({
          kind: token.kind,
          text: token.text.slice(cursor, splitAt),
          emphasized: true,
        });
        cursor = splitAt;
      }
    }
    offset += tokenLen;
  }

  return result;
}
