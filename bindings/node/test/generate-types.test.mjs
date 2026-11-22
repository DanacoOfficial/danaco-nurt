import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, it } from "node:test";
import { renderModule, SCHEMAS } from "../scripts/generate-types.mjs";
import { packageDirectory } from "./support.mjs";

const schemaDirectory = join(packageDirectory, "..", "..", "spec", "schemas");

function readSchema(file) {
  return JSON.parse(readFileSync(join(schemaDirectory, file), "utf8"));
}

function render(schema, root = "Root", prefix = "Root") {
  return renderModule(schema, { root, prefix, source: "test.schema.json" });
}

describe("renderModule", () => {
  it("renders required and optional members with their descriptions", () => {
    const text = render({
      type: "object",
      required: ["name"],
      additionalProperties: false,
      properties: {
        name: { type: "string", description: "Name of the item." },
        size: { type: "integer" },
        "kebab-key": { type: "boolean" },
      },
    });
    assert.match(text, /export type Root = \{\n {2}\/\*\* Name of the item\. \*\/\n {2}name: string;\n {2}size\?: number;\n {2}"kebab-key"\?: boolean;\n\};/);
  });

  it("renders enumerations, constants, nullable types and arrays", () => {
    const text = render({
      type: "object",
      additionalProperties: false,
      properties: {
        mode: { enum: ["a", "b"] },
        kind: { const: "let" },
        limit: { type: ["integer", "null"] },
        names: { type: "array", items: { type: "string" } },
        pairs: { type: "array", items: { type: "array", items: { type: "number" } } },
      },
    });
    assert.match(text, /mode\?: "a" \| "b";/);
    assert.match(text, /kind\?: "let";/);
    assert.match(text, /limit\?: number \| null;/);
    assert.match(text, /names\?: string\[\];/);
    assert.match(text, /pairs\?: Array<number\[\]>;/);
  });

  it("names definitions with the prefix and resolves references", () => {
    const text = render({
      $ref: "#/$defs/item_list",
      $defs: { item_list: { type: "array", items: { $ref: "#/$defs/item" } }, item: { type: "string" } },
    });
    assert.match(text, /export type Root = RootItemList;/);
    assert.match(text, /export type RootItemList = RootItem\[\];/);
    assert.match(text, /export type RootItem = string;/);
  });

  it("renders alternatives as a union and ignores alternatives that only list required members", () => {
    const union = render({ anyOf: [{ $ref: "#/$defs/a" }, { type: "null" }], $defs: { a: { type: "string" } } });
    assert.match(union, /export type Root = RootA \| null;/);
    const exclusive = render({
      type: "object",
      properties: { program: { type: "string" }, cursor: { type: "string" } },
      oneOf: [{ required: ["program"] }, { required: ["cursor"] }],
    });
    assert.doesNotMatch(exclusive, /&/);
    assert.match(exclusive, /program\?: string;/);
  });

  it("renders an open object with an index signature of a compatible type", () => {
    assert.match(render({ type: "object", additionalProperties: { type: "string" } }), /\[key: string\]: string;/);
    const mixed = render({ type: "object", properties: { a: { type: "number" } }, additionalProperties: { type: "string" } });
    assert.match(mixed, /\[key: string\]: unknown;/);
    assert.match(render({ type: "object", required: ["t"] }), /t: unknown;\n {2}\[key: string\]: unknown;/);
  });

  it("rejects references outside the schema", () => {
    assert.throws(() => render({ $ref: "other.schema.json#/$defs/x" }), /unsupported reference/);
    assert.throws(() => render({ $ref: "#/$defs/missing" }), /missing definition/);
  });

  it("renders every schema of the specification", () => {
    for (const entry of SCHEMAS) {
      const text = renderModule(readSchema(entry.file), { root: entry.root, prefix: entry.prefix, source: entry.file });
      assert.match(text, new RegExp(`export type ${entry.root} = `));
    }
  });
});

describe("schema members used by the client", () => {
  it("are present in nurt-request/1", () => {
    const schema = readSchema("nurt-request-1.schema.json");
    for (const name of ["nurt", "program", "variant", "ast", "cursor", "accept", "options"]) {
      assert.ok(Object.hasOwn(schema.properties, name), name);
    }
    for (const name of ["preview", "base", "limit", "format", "timeout", "session", "language"]) {
      assert.ok(Object.hasOwn(schema.$defs.options.properties, name), name);
    }
    assert.deepEqual(schema.$defs.response.required, ["nurt", "report"]);
    assert.ok(Object.hasOwn(schema.$defs.rendered.properties, "text"));
    assert.deepEqual(schema.properties.variant.enum, ["a", "b", "c"]);
  });

  it("are present in nurt-report/1", () => {
    const schema = readSchema("nurt-report-1.schema.json");
    for (const name of ["status", "error", "next_cursor"]) {
      assert.ok(Object.hasOwn(schema.properties, name), name);
    }
    for (const name of ["code", "message", "at", "fix", "help"]) {
      assert.ok(Object.hasOwn(schema.$defs.error.properties, name), name);
    }
  });
});
