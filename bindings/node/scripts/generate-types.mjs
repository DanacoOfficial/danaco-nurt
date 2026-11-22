// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// Writes TypeScript types derived from the JSON schemas of the specification
// to src/generated/. Supports the subset of JSON Schema 2020-12 used by the
// schemas: type, properties, required, additionalProperties, items, enum,
// const, $ref to $defs, oneOf, anyOf and allOf. Validation keywords without
// a TypeScript equivalent (pattern, minimum, minItems, ...) are ignored, and
// alternatives that only list required members are treated as validation.

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const packageDirectory = join(dirname(fileURLToPath(import.meta.url)), "..");
const schemaDirectory = join(packageDirectory, "..", "..", "spec", "schemas");
const outputDirectory = join(packageDirectory, "src", "generated");

/** Schemas turned into modules; `prefix` is prepended to the names of $defs. */
export const SCHEMAS = [
  { file: "nurt-ast-1.schema.json", module: "nurt-ast-1", root: "Ast", prefix: "Ast" },
  { file: "nurt-config-1.schema.json", module: "nurt-config-1", root: "Config", prefix: "" },
  { file: "nurt-report-1.schema.json", module: "nurt-report-1", root: "Report", prefix: "Report" },
  { file: "nurt-request-1.schema.json", module: "nurt-request-1", root: "Request", prefix: "Request" },
];

const TYPE_KEYWORDS = [
  "$ref",
  "const",
  "enum",
  "type",
  "properties",
  "additionalProperties",
  "items",
  "oneOf",
  "anyOf",
  "allOf",
];
const IDENTIFIER = /^[A-Za-z_$][A-Za-z0-9_$]*$/;
const DEFINITION_REFERENCE = /^#\/\$defs\/(.+)$/;

/**
 * Renders the TypeScript module of one schema.
 *
 * @param {unknown} schema parsed JSON schema
 * @param {{ root: string, prefix: string, source: string }} options name of
 *   the root type, prefix of the names of $defs and file name of the schema
 * @returns {string} module text
 */
export function renderModule(schema, options) {
  if (!isPlainObject(schema)) {
    throw new Error(`${options.source}: the schema is not an object`);
  }
  const definitions = isPlainObject(schema.$defs) ? schema.$defs : {};
  const names = new Map();
  const used = new Set([options.root]);
  for (const key of Object.keys(definitions)) {
    const name = `${options.prefix}${pascalCase(key)}`;
    if (used.has(name)) {
      throw new Error(`${options.source}: the type name ${name} of $defs/${key} is already used`);
    }
    used.add(name);
    names.set(key, name);
  }
  const context = { names, source: options.source };
  const declarations = [declaration(options.root, schema, context)];
  for (const [key, definition] of Object.entries(definitions)) {
    declarations.push(declaration(names.get(key), definition, context));
  }
  const banner = `// Types derived from spec/schemas/${options.source}. Do not edit; run scripts/generate-types.mjs.\n`;
  return `${banner}\n${declarations.join("\n\n")}\n`;
}

function declaration(name, schema, context) {
  return `${docComment(schema, "")}export type ${name} = ${typeExpression(schema, context, "")};`;
}

function typeExpression(schema, context, indent) {
  if (schema === true) {
    return "unknown";
  }
  if (schema === false) {
    return "never";
  }
  if (!isPlainObject(schema)) {
    throw new Error(`${context.source}: a subschema is not an object or a boolean`);
  }
  const parts = [];
  const base = baseExpression(schema, context, indent);
  if (base !== undefined) {
    parts.push(base);
  }
  for (const keyword of ["oneOf", "anyOf"]) {
    if (!Array.isArray(schema[keyword])) {
      continue;
    }
    const alternatives = schema[keyword].filter(carriesType).map((item) => typeExpression(item, context, indent));
    if (alternatives.length > 0) {
      parts.push(alternatives.length === 1 ? alternatives[0] : `(${alternatives.join(" | ")})`);
    }
  }
  if (Array.isArray(schema.allOf)) {
    for (const item of schema.allOf.filter(carriesType)) {
      parts.push(`(${typeExpression(item, context, indent)})`);
    }
  }
  if (parts.length === 0) {
    return "unknown";
  }
  if (parts.length === 1) {
    return unwrap(parts[0]);
  }
  return parts.join(" & ");
}

function baseExpression(schema, context, indent) {
  if (typeof schema.$ref === "string") {
    return reference(schema.$ref, context);
  }
  if (Object.hasOwn(schema, "const")) {
    return literal(schema.const, context);
  }
  if (Array.isArray(schema.enum)) {
    return schema.enum.map((value) => literal(value, context)).join(" | ");
  }
  if (schema.type !== undefined) {
    const types = Array.isArray(schema.type) ? schema.type : [schema.type];
    return types.map((type) => primitive(type, schema, context, indent)).join(" | ");
  }
  if (schema.properties !== undefined || schema.required !== undefined || schema.additionalProperties !== undefined) {
    return objectExpression(schema, context, indent);
  }
  if (schema.items !== undefined) {
    return arrayExpression(schema, context, indent);
  }
  return undefined;
}

function primitive(type, schema, context, indent) {
  switch (type) {
    case "string":
      return "string";
    case "integer":
    case "number":
      return "number";
    case "boolean":
      return "boolean";
    case "null":
      return "null";
    case "array":
      return arrayExpression(schema, context, indent);
    case "object":
      return objectExpression(schema, context, indent);
    default:
      throw new Error(`${context.source}: unsupported type ${String(type)}`);
  }
}

function arrayExpression(schema, context, indent) {
  if (schema.prefixItems !== undefined) {
    throw new Error(`${context.source}: prefixItems is not supported`);
  }
  if (schema.items === undefined) {
    return "unknown[]";
  }
  const element = typeExpression(schema.items, context, indent);
  return IDENTIFIER.test(element) ? `${element}[]` : `Array<${element}>`;
}

function objectExpression(schema, context, indent) {
  const properties = isPlainObject(schema.properties) ? schema.properties : {};
  const required = new Set(Array.isArray(schema.required) ? schema.required : []);
  const inner = `${indent}  `;
  const lines = [];
  for (const [key, value] of Object.entries(properties)) {
    const optional = required.has(key) ? "" : "?";
    lines.push(`${docComment(value, inner)}${inner}${propertyKey(key)}${optional}: ${typeExpression(value, context, inner)};`);
  }
  const undeclared = [...required].filter((key) => !Object.hasOwn(properties, key));
  for (const key of undeclared) {
    lines.push(`${inner}${propertyKey(key)}: unknown;`);
  }
  const additional = schema.additionalProperties;
  if (additional !== false) {
    const typed = additional !== undefined && additional !== true && lines.length === 0;
    lines.push(`${inner}[key: string]: ${typed ? typeExpression(additional, context, inner) : "unknown"};`);
  }
  if (lines.length === 0) {
    return "Record<string, never>";
  }
  return `{\n${lines.join("\n")}\n${indent}}`;
}

function reference(target, context) {
  const match = DEFINITION_REFERENCE.exec(target);
  if (match === null) {
    throw new Error(`${context.source}: unsupported reference ${target}`);
  }
  const key = (match[1] ?? "").replaceAll("~1", "/").replaceAll("~0", "~");
  const name = context.names.get(key);
  if (name === undefined) {
    throw new Error(`${context.source}: reference to a missing definition ${target}`);
  }
  return name;
}

function literal(value, context) {
  if (typeof value === "string" || typeof value === "number" || typeof value === "boolean" || value === null) {
    return JSON.stringify(value);
  }
  throw new Error(`${context.source}: unsupported constant ${JSON.stringify(value)}`);
}

function carriesType(schema) {
  return typeof schema === "boolean" || (isPlainObject(schema) && TYPE_KEYWORDS.some((keyword) => Object.hasOwn(schema, keyword)));
}

function unwrap(expression) {
  return expression.startsWith("(") && expression.endsWith(")") && balanced(expression.slice(1, -1))
    ? expression.slice(1, -1)
    : expression;
}

function balanced(text) {
  let depth = 0;
  for (const character of text) {
    if (character === "(") {
      depth += 1;
    } else if (character === ")") {
      depth -= 1;
      if (depth < 0) {
        return false;
      }
    }
  }
  return depth === 0;
}

function propertyKey(key) {
  return IDENTIFIER.test(key) ? key : JSON.stringify(key);
}

function pascalCase(key) {
  return key
    .split(/[^A-Za-z0-9]+/)
    .filter((part) => part !== "")
    .map((part) => part[0].toUpperCase() + part.slice(1))
    .join("");
}

function docComment(schema, indent) {
  if (!isPlainObject(schema) || typeof schema.description !== "string" || schema.description.trim() === "") {
    return "";
  }
  const lines = schema.description
    .trim()
    .replaceAll("*/", "*\\/")
    .split("\n")
    .map((line) => line.trimEnd());
  if (lines.length === 1) {
    return `${indent}/** ${lines[0]} */\n`;
  }
  const body = lines.map((line) => (line === "" ? `${indent} *` : `${indent} * ${line}`)).join("\n");
  return `${indent}/**\n${body}\n${indent} */\n`;
}

/** @returns {value is Record<string, any>} */
function isPlainObject(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function main() {
  mkdirSync(outputDirectory, { recursive: true });
  for (const entry of SCHEMAS) {
    const schema = JSON.parse(readFileSync(join(schemaDirectory, entry.file), "utf8"));
    const text = renderModule(schema, { root: entry.root, prefix: entry.prefix, source: entry.file });
    writeFileSync(join(outputDirectory, `${entry.module}.ts`), text);
  }
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}
