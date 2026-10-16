// Minimal, dependency-free JSON Schema validator.
//
// WHY: the integration engine must run in GitHub Actions with no install step and must
// FAIL CLOSED. Pulling in a third-party validator (ajv, etc.) adds supply-chain and
// availability risk. This validator supports a deliberately small, well-defined keyword
// subset and *throws* on any keyword it does not implement, so a schema that the engine
// cannot fully understand is rejected rather than silently under-enforced.
//
// Supported keywords:
//   $schema, $id, $comment, title, description, examples, default, deprecated,
//   type, enum, const, required, properties, additionalProperties, patternProperties,
//   propertyNames, items (schema or tuple), minItems, maxItems, uniqueItems, contains,
//   minLength, maxLength, pattern, minimum, maximum, exclusiveMinimum, exclusiveMaximum,
//   oneOf, anyOf, allOf, not, $ref (local "#/..." JSON Pointer only), $defs, definitions,
//   minProperties, maxProperties.
//
// Any other keyword throws SchemaError (fail closed).

export class SchemaError extends Error {
  constructor(message) {
    super(message);
    this.name = 'SchemaError';
  }
}

const METADATA_KEYWORDS = new Set([
  '$schema', '$id', '$comment', 'title', 'description', 'examples', 'default',
  'deprecated', 'readOnly', 'writeOnly',
]);

const SUPPORTED_KEYWORDS = new Set([
  'type', 'enum', 'const', 'required', 'properties', 'additionalProperties',
  'patternProperties', 'propertyNames', 'items', 'minItems', 'maxItems', 'uniqueItems',
  'contains', 'minLength', 'maxLength', 'pattern', 'minimum', 'maximum',
  'exclusiveMinimum', 'exclusiveMaximum', 'oneOf', 'anyOf', 'allOf', 'not',
  '$ref', '$defs', 'definitions', 'minProperties', 'maxProperties',
]);

const TYPES = new Set(['object', 'array', 'string', 'number', 'integer', 'boolean', 'null']);

function jsonType(value) {
  if (value === null) return 'null';
  if (Array.isArray(value)) return 'array';
  return typeof value; // object | string | number | boolean
}

function deepEqual(a, b) {
  if (a === b) return true;
  if (typeof a !== typeof b) return false;
  if (a === null || b === null) return a === b;
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
    return a.every((v, i) => deepEqual(v, b[i]));
  }
  if (typeof a === 'object') {
    const ka = Object.keys(a).sort();
    const kb = Object.keys(b).sort();
    if (ka.length !== kb.length || ka.some((k, i) => k !== kb[i])) return false;
    return ka.every((k) => deepEqual(a[k], b[k]));
  }
  return false;
}

function resolvePointer(root, pointer) {
  if (!pointer.startsWith('#')) {
    throw new SchemaError(`Only local $ref pointers ("#/...") are supported, got: ${pointer}`);
  }
  const rest = pointer.slice(1);
  if (rest === '') return root;
  if (!rest.startsWith('/')) {
    throw new SchemaError(`Malformed $ref pointer: ${pointer}`);
  }
  let node = root;
  for (const rawToken of rest.slice(1).split('/')) {
    const token = rawToken.replace(/~1/g, '/').replace(/~0/g, '~');
    if (node === null || typeof node !== 'object' || !(token in node)) {
      throw new SchemaError(`Unresolvable $ref pointer: ${pointer}`);
    }
    node = node[token];
  }
  return node;
}

function stringify(value) {
  if (typeof value === 'string') return JSON.stringify(value);
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

/**
 * Validate `value` against `schema`. Returns { valid, errors }.
 * Throws SchemaError if the schema itself cannot be fully interpreted (fail closed).
 */
export function validateSchema(schema, value, options = {}) {
  const root = options.root || schema;
  const errors = [];
  const state = { root, errors, depth: 0, maxDepth: options.maxDepth ?? 128 };
  checkNode(schema, value, '', state);
  return { valid: errors.length === 0, errors };
}

/** Convenience: throw with a readable message if invalid. */
export function assertSchema(schema, value, label = 'document') {
  const { valid, errors } = validateSchema(schema, value);
  if (!valid) {
    throw new SchemaError(
      `${label} failed schema validation:\n  - ${errors.slice(0, 50).join('\n  - ')}` +
        (errors.length > 50 ? `\n  ... and ${errors.length - 50} more` : ''),
    );
  }
}

function checkNode(schema, value, path, state) {
  if (state.depth++ > state.maxDepth) {
    throw new SchemaError('Schema validation exceeded maximum depth (possible cyclic $ref).');
  }
  try {
    if (schema === true || schema === undefined) return;
    if (schema === false) {
      state.errors.push(`${path || '(root)'}: value is not allowed (schema is false)`);
      return;
    }
    if (schema === null || typeof schema !== 'object' || Array.isArray(schema)) {
      throw new SchemaError(`Schema node must be an object or boolean at ${path || '(root)'}`);
    }

    for (const key of Object.keys(schema)) {
      if (!METADATA_KEYWORDS.has(key) && !SUPPORTED_KEYWORDS.has(key)) {
        throw new SchemaError(
          `Unsupported JSON Schema keyword "${key}" at ${path || '(root)'}: refusing to under-enforce.`,
        );
      }
    }

    if ('$ref' in schema) {
      const target = resolvePointer(state.root, schema.$ref);
      checkNode(target, value, path, state);
    }

    if ('type' in schema) checkType(schema.type, value, path, state);
    if ('const' in schema && !deepEqual(schema.const, value)) {
      state.errors.push(`${path || '(root)'}: expected const ${stringify(schema.const)}, got ${stringify(value)}`);
    }
    if ('enum' in schema) {
      if (!Array.isArray(schema.enum)) throw new SchemaError(`"enum" must be an array at ${path || '(root)'}`);
      if (!schema.enum.some((candidate) => deepEqual(candidate, value))) {
        state.errors.push(`${path || '(root)'}: value ${stringify(value)} not in enum ${stringify(schema.enum)}`);
      }
    }
    if ('allOf' in schema) for (const sub of schema.allOf) checkNode(sub, value, path, state);
    if ('anyOf' in schema) {
      const matched = schema.anyOf.some((sub) => validateSub(sub, value, state));
      if (!matched) state.errors.push(`${path || '(root)'}: does not match anyOf`);
    }
    if ('oneOf' in schema) {
      const count = schema.oneOf.filter((sub) => validateSub(sub, value, state)).length;
      if (count !== 1) state.errors.push(`${path || '(root)'}: matched ${count} oneOf branches, expected exactly 1`);
    }
    if ('not' in schema) {
      if (validateSub(schema.not, value, state)) state.errors.push(`${path || '(root)'}: matches "not" schema`);
    }

    const t = jsonType(value);
    if ((t === 'object' || t === 'array') && '$ref' in schema) {
      // still validate string/number constraints below only when relevant
    }

    if (t === 'object') checkObject(schema, value, path, state);
    if (t === 'array') checkArray(schema, value, path, state);
    if (t === 'string') checkString(schema, value, path, state);
    if (t === 'number' || t === 'integer') checkNumber(schema, value, path, state);
  } finally {
    state.depth--;
  }
}

function validateSub(subschema, value, state) {
  const before = state.errors.length;
  checkNode(subschema, value, '', state);
  const ok = state.errors.length === before;
  if (!ok) state.errors.splice(before); // discard sub-branch errors
  return ok;
}

function checkType(typeSpec, value, path, state) {
  const allowed = Array.isArray(typeSpec) ? typeSpec : [typeSpec];
  for (const t of allowed) {
    if (!TYPES.has(t)) throw new SchemaError(`Unknown JSON type "${t}" in schema at ${path || '(root)'}`);
  }
  const actual = jsonType(value);
  const ok = allowed.some((t) => {
    if (t === 'integer') return Number.isInteger(value);
    if (t === 'number') return typeof value === 'number' && Number.isFinite(value);
    return t === actual;
  });
  if (!ok) {
    state.errors.push(`${path || '(root)'}: expected type ${allowed.join('|')}, got ${actual}`);
  }
}

function checkObject(schema, value, path, state) {
  const props = schema.properties || {};
  const extraSchema = 'additionalProperties' in schema ? schema.additionalProperties : true;
  const patternProps = schema.patternProperties || {};
  const patterns = Object.entries(patternProps).map(([p, s]) => {
    try {
      return [new RegExp(p, 'u'), s, p];
    } catch (err) {
      throw new SchemaError(`Invalid patternProperties regex "${p}" at ${path || '(root)'}: ${err.message}`);
    }
  });

  if (Array.isArray(schema.required)) {
    for (const key of schema.required) {
      if (!Object.prototype.hasOwnProperty.call(value, key)) {
        state.errors.push(`${path || '(root)'}: missing required property "${key}"`);
      }
    }
  }

  const keys = Object.keys(value);
  if (typeof schema.minProperties === 'number' && keys.length < schema.minProperties) {
    state.errors.push(`${path || '(root)'}: expected >= ${schema.minProperties} properties, got ${keys.length}`);
  }
  if (typeof schema.maxProperties === 'number' && keys.length > schema.maxProperties) {
    state.errors.push(`${path || '(root)'}: expected <= ${schema.maxProperties} properties, got ${keys.length}`);
  }

  if ('propertyNames' in schema) {
    for (const key of keys) checkNode(schema.propertyNames, key, `${path}.<name:${key}>`, state);
  }

  for (const key of keys) {
    const childPath = `${path}/${key}`;
    let matched = false;
    if (Object.prototype.hasOwnProperty.call(props, key)) {
      matched = true;
      checkNode(props[key], value[key], childPath, state);
    }
    for (const [re, sub, src] of patterns) {
      if (re.test(key)) {
        matched = true;
        checkNode(sub, value[key], `${childPath}<pattern:${src}>`, state);
      }
    }
    if (!matched) {
      if (extraSchema === false) {
        state.errors.push(`${childPath}: additional property "${key}" is not allowed`);
      } else if (extraSchema !== true) {
        checkNode(extraSchema, value[key], childPath, state);
      }
    }
  }
}

function checkArray(schema, value, path, state) {
  if (typeof schema.minItems === 'number' && value.length < schema.minItems) {
    state.errors.push(`${path || '(root)'}: expected >= ${schema.minItems} items, got ${value.length}`);
  }
  if (typeof schema.maxItems === 'number' && value.length > schema.maxItems) {
    state.errors.push(`${path || '(root)'}: expected <= ${schema.maxItems} items, got ${value.length}`);
  }
  if (schema.uniqueItems === true) {
    for (let i = 0; i < value.length; i += 1) {
      for (let j = i + 1; j < value.length; j += 1) {
        if (deepEqual(value[i], value[j])) {
          state.errors.push(`${path || '(root)'}: items ${i} and ${j} are not unique`);
        }
      }
    }
  }
  if ('items' in schema) {
    if (Array.isArray(schema.items)) {
      value.forEach((item, i) => {
        if (i < schema.items.length) checkNode(schema.items[i], item, `${path}/${i}`, state);
      });
    } else {
      value.forEach((item, i) => checkNode(schema.items, item, `${path}/${i}`, state));
    }
  }
  if ('contains' in schema) {
    const ok = value.some((item) => validateSub(schema.contains, item, state));
    if (!ok) state.errors.push(`${path || '(root)'}: no array item matches "contains"`);
  }
}

function checkString(schema, value, path, state) {
  if (typeof schema.minLength === 'number' && value.length < schema.minLength) {
    state.errors.push(`${path || '(root)'}: string shorter than minLength ${schema.minLength}`);
  }
  if (typeof schema.maxLength === 'number' && value.length > schema.maxLength) {
    state.errors.push(`${path || '(root)'}: string longer than maxLength ${schema.maxLength}`);
  }
  if ('pattern' in schema) {
    let re;
    try {
      re = new RegExp(schema.pattern, 'u');
    } catch (err) {
      throw new SchemaError(`Invalid "pattern" regex at ${path || '(root)'}: ${err.message}`);
    }
    if (!re.test(value)) {
      state.errors.push(`${path || '(root)'}: string does not match pattern ${schema.pattern}`);
    }
  }
}

function checkNumber(schema, value, path, state) {
  const num = value;
  const { minimum, maximum, exclusiveMinimum, exclusiveMaximum } = schema;
  if (typeof minimum === 'number' && num < minimum) state.errors.push(`${path || '(root)'}: ${num} < minimum ${minimum}`);
  if (typeof maximum === 'number' && num > maximum) state.errors.push(`${path || '(root)'}: ${num} > maximum ${maximum}`);
  if (typeof exclusiveMinimum === 'number' && num <= exclusiveMinimum) {
    state.errors.push(`${path || '(root)'}: ${num} <= exclusiveMinimum ${exclusiveMinimum}`);
  }
  if (typeof exclusiveMaximum === 'number' && num >= exclusiveMaximum) {
    state.errors.push(`${path || '(root)'}: ${num} >= exclusiveMaximum ${exclusiveMaximum}`);
  }
}
