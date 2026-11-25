export type SyntaxTokenKind =
  'plain' | 'keyword' | 'string' | 'comment' | 'tag' | 'attr' | 'entity' | 'number';

export interface SyntaxToken {
  kind: SyntaxTokenKind;
  text: string;
}

type LanguageFamily = 'js' | 'rust' | 'json' | 'markup' | 'css' | 'config' | 'sql' | 'plain';

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
  'false',
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
  'null',
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
  'true',
  'try',
  'type',
  'typeof',
  'undefined',
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
  'false',
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
  'true',
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

const CONFIG_KEYWORDS = new Set(['true', 'false', 'null', 'yes', 'no', 'on', 'off']);
const JSON_KEYWORDS = new Set(['true', 'false', 'null']);
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

function keywordSetFor(family: LanguageFamily): Set<string> | null {
  switch (family) {
    case 'js':
      return JS_KEYWORDS;
    case 'rust':
      return RUST_KEYWORDS;
    case 'json':
      return JSON_KEYWORDS;
    case 'config':
      return CONFIG_KEYWORDS;
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
  const tokens: SyntaxToken[] = [];
  let i = 0;

  while (i < text.length) {
    const ch = text[i] ?? '';
    const next = text[i + 1] ?? '';

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
      i = end;
      continue;
    }

    if (
      (family === 'markup' || family === 'js') &&
      ch === '<' &&
      (isIdentStart(next) || next === '/')
    ) {
      let tagStart = i + 1;
      if (next === '/') tagStart += 1;
      let tagEnd = tagStart;
      while (tagEnd < text.length) {
        const tc = text[tagEnd] ?? '';
        if (isIdentPart(tc) || tc === '.') tagEnd += 1;
        else break;
      }
      if (tagEnd > tagStart) {
        pushToken(tokens, 'plain', text.slice(i, tagStart));
        pushToken(tokens, 'tag', text.slice(tagStart, tagEnd));
        i = tagEnd;
        continue;
      }
    }

    if (family === 'css' && (ch === '@' || ch === '!')) {
      let end = i + 1;
      while (end < text.length && isIdentPart(text[end] ?? '')) end += 1;
      const word = text.slice(i, end);
      pushToken(tokens, CSS_KEYWORDS.has(word) ? 'keyword' : 'plain', word);
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

      if (keywords?.has(lookup)) {
        pushToken(tokens, 'keyword', word);
      } else {
        let look = end;
        while (look < text.length && (text[look] === ' ' || text[look] === '\t')) {
          look += 1;
        }
        const after = text[look] ?? '';
        if (after === '=' && (family === 'markup' || family === 'js') && text[look + 1] !== '=') {
          pushToken(tokens, 'attr', word);
        } else if (after === '(') {
          pushToken(tokens, 'entity', word);
        } else if (
          (family === 'config' || family === 'css') &&
          after === ':' &&
          text[look + 1] !== ':'
        ) {
          pushToken(tokens, 'attr', word);
        } else {
          pushToken(tokens, 'plain', word);
        }
      }
      i = end;
      continue;
    }

    pushToken(tokens, 'plain', ch);
    i += 1;
  }

  return tokens;
}
