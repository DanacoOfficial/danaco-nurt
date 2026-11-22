<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Syntax trees: `nurt-ast/1` and `nurt-bound/1`

Nurt executes and records a syntax tree, never program text. Program text in
any syntax variant is corrected and parsed into a **surface tree**
(`nurt-ast/1`). Binding the surface tree with the command catalog produces a
**bound tree** (`nurt-bound/1`). An instance on a remote device receives a
surface tree encoded as JSON or CBOR and binds it with its own catalog, so a
program never passes through another interpreter and no quoting layers are
involved. The journal and the effect report store the bound tree.

The normative definitions are the JSON Schemas
[`schemas/nurt-ast-1.schema.json`](schemas/nurt-ast-1.schema.json) and
[`schemas/nurt-bound-1.schema.json`](schemas/nurt-bound-1.schema.json). The
Rust types of the modules `nurt_contracts::ast` and `nurt_contracts::bound`
serialize to exactly these documents.

## 1. Identifiers and versions

| Item | Surface tree | Bound tree |
|---|---|---|
| Format name | `nurt-ast/1` | `nurt-bound/1` |
| Version member | `"nurt": "1.0"` | `"nurt": "1.0"` and `"bound": "1.0"` |
| Media type | `application/vnd.nurt.ast+json; v=1`, `application/vnd.nurt.ast+cbor; v=1` | carried inside reports and journal entries |
| File extension | `.nurt.json` (tree), `.nurt` (program text, UTF-8) | — |
| Schema | `nurt-ast-1.schema.json` | `nurt-bound-1.schema.json` |
| Canonical encoding | JSON according to RFC 8785 (JCS); deterministic CBOR (RFC 8949, section 4.2) | same |

Versions are written `MAJOR.MINOR`. Within one major version only optional
members and new node kinds are added; new commands and parameters are catalog
data and never change the schema. A reader accepts a document of its own major
version whose minor version is not greater than its own; a newer minor version
is rejected with `E_AST_VERSION` and the list of unknown node kinds. After a
change of the major version, readers accept the previous major version for at
least 24 months.

The two documents are distinct: the surface schema rejects the root members
`bound`, `catalog_rev` and `corr_rev`, the stage member `effect` and the
argument member `default`; the bound schema requires them.

## 2. Common conventions

* Every node object carries its kind in the member `"t"`. Argument objects and
  map entries are not nodes and have no `"t"`.
* Optional members are omitted when absent. An empty field path of a
  reference is omitted. Arrays that are part of a node (`args`, `stages`,
  `items`, `entries`) are always present, possibly empty.
* User data never appears as a member name: map entries are objects
  `{"k": key, "v": value}`.
* `meta` is a free-form object at the root (session, source text, comments).
  Comments of program text are not part of the tree.
* `span` is `[line, column, end_line, end_column]`. Lines and columns start at
  1, columns count Unicode scalar values and the end position is exclusive (the
  position directly after the last character). Spans are informative.

### Spans of the surface tree

| Node | Covers |
|---|---|
| `let` | from `let` to the end of the pipeline |
| `pipeline` | from the start of the head to the end of the last stage |
| `stage` | from the command name to the end of the last argument |
| argument | a named argument from its name to the end of its value; a positional argument equals its value |
| literal, identifier | the token, including quotes, the `r#` delimiters of raw text and the regular expression flags |
| `ref` | from `@` to the end of the last field |
| `list`, `map`, `call`, `sub` | from the opening bracket or name to the closing bracket |
| `bin` | from the start of the left operand to the end of the right operand |
| `un`, `spread` | from the operator to the end of the operand |
| `field` | from the start of the record expression to the end of the field name |
| `range` | from the first to the last bound written |
| parenthesized expression | the node of the inner expression; its span includes the parentheses |

## 3. Surface tree

Root members: `nurt` (required), `program` (required), `meta`.
`program.statements` holds at least one statement.

| `t` | Members | Text form (word notation) |
|---|---|---|
| `let` | `name`, `value: pipeline` | `let recent = find …` |
| `pipeline` | `head: stage \| ref`, `stages: [stage]` | `find … \|> filter … \|> first 20` |
| `stage` | `name`, `args: [argument]` | `filter size > 1mb` |
| argument | `name?`, `value` | `glob: "**/*.go"`, `20` |
| `text` | `v` | `"abc"`, `r"abc"`, `r#"…"#` |
| `ident` | `v` | `desc`, `size` |
| `regex` | `v`, `flags` (`imsx`, in this order) | `/oldName\(/i` |
| `int`, `num` | `v` (`int` is non-negative) | `40`, `3.5` |
| `size` | `v` (bytes) | `10mb` = 10 000 000, `1kib` = 1 024 |
| `dur` | `v` (milliseconds) | `30s`, `1h30m` |
| `bool`, `null` | `v` / — | `true`, `null` |
| `list` | `items` | `["a", "b"]` |
| `map` | `entries: [{k, v}]` | `{replace: "a", with: "b"}` |
| `ref` | `name`, `fields?` | `@recent`, `@recent.path` |
| `spread` | `x` | `...@recent.path` (list elements only) |
| `field` | `name`, `of?` | `file.size` |
| `call` | `fn`, `args` | `ago(1h)` |
| `un` | `op` (`not`, `neg`), `x` | `not hidden`, `-1` |
| `bin` | `op`, `l`, `r` | `size > 1mb` |
| `range` | `lo?`, `hi?` | `10..12`, `40..`, `..9` |
| `sub` | `p: pipeline` | `(run […] \|> assert exit_code: 0)` |
| `key` | `segs` | structured key: names, indexes, `{"wild": "*" \| "**" \| "+"}`, `{"filter": expr}` |

Rules of the surface tree:

1. The tree does not depend on the catalog. Positional arguments keep their
   order and have no name; no default is added.
2. The parser produces `text` for every quoted string and `ident` for every
   bare identifier. Paths, globs, instants and keys are `text` nodes; the
   binder types them by their place in the signature. The structured `key`
   node is an alternative to a key written as text for producers that build
   trees directly.
3. Text values are stored after escape processing. In a regular expression
   `\/` is stored as `/`; every other escape is stored unchanged. Flags are
   drawn from `imsx` and written in this order.
4. Integer literals are non-negative: `-1` is `un` with `op: "neg"` applied to
   `int` 1. Every integer of a tree lies within ±(2^53 - 1), the range that
   canonical JSON represents exactly; a larger literal is a lexical error.
5. `spread` appears only as an element of a list.
6. An identifier directly followed by `:` always starts a named argument, even
   when it is an operator word (`replace "a" "b" in: ["/srv/a.txt"]`).
7. The binary operators, in increasing binding strength: `or`; `and`; `not`
   (unary); `== != < <= > >= ~ !~ in` (non-associative); `+ -`; `* /`;
   unary `-`; field access `.` and calls.

Example: `ports state: listen |> filter port == 8080`

```json
{"nurt": "1.0", "program": {"statements": [{"t": "pipeline",
  "head": {"t": "stage", "name": "ports", "args": [
    {"name": "state", "value": {"t": "ident", "v": "listen", "span": [1, 14, 1, 20]}, "span": [1, 7, 1, 20]}],
    "span": [1, 1, 1, 20]},
  "stages": [{"t": "stage", "name": "filter", "args": [
    {"value": {"t": "bin", "op": "==",
      "l": {"t": "ident", "v": "port", "span": [1, 31, 1, 35]},
      "r": {"t": "int", "v": 8080, "span": [1, 39, 1, 43]}, "span": [1, 31, 1, 43]},
     "span": [1, 31, 1, 43]}], "span": [1, 24, 1, 43]}],
  "span": [1, 1, 1, 43]}]}}
```

## 4. Bound tree

Root members: `nurt`, `bound`, `catalog_rev`, `corr_rev`, `program` (all
required) and `meta`. `catalog_rev` identifies the command catalog and
`corr_rev` the correction tables used to produce the tree; both are opaque
non-empty strings.

Statements, `let` and `pipeline` have the same shape as in the surface tree.
A stage has the members:

| Member | Meaning |
|---|---|
| `name` | canonical command name from the catalog: one identifier or a group and an identifier joined by `.` |
| `effect` | effect kind from the catalog: `read`, `local`, `external`, `process` |
| `phase` | execution phase (section 5); required |
| `waits` | `true` when the stage blocks the call by waiting; omitted when `false` |
| `args` | every parameter of the signature, in signature order |
| `span` | source range of the surface stage |

An argument has `name`, `value`, `default` (`true` when the value comes from
the catalog default; omitted when `false`) and `span` (absent for defaults).

Value nodes carry no span. Kinds: `null`; `bool`, `int`, `num`; `text`,
`path`, `glob`, `key`, `enum`, `instant` with a text `v`; `size` and `dur`
with a non-negative integer `v`; `regex` with `v` and `flags` from `imsx`;
`list`; `map`; `range`; `ref`; `sub`; and the expression kinds `field`,
`call`, `un`, `bin`, `spread`. Integers lie within ±(2^53 - 1); constant
negation is folded, so `-1` binds to `int` -1.

Binding rules:

1. Every parameter of the signature appears once, in signature order,
   whether it was written positionally, by name or not at all.
2. A parameter that was not written receives its catalog default with
   `default: true`. A parameter without a default, a parameter whose default
   is unbounded, a parameter of an exclusive group that was not written (as
   in `assert`) and a parameter of the source form when the command is bound
   as a stage receive `null` with `default: true`.
3. Literals are cast to the type of their place: text to `path`, `glob`,
   `instant`, `key`; identifiers to `enum`; a single path to a one-element list
   where the place is a list of paths; an integer `n` to the range `n..n` where
   the place is a range.
4. In expressions evaluated on records (`filter`, `select`, `sort`,
   per-record parameters), a bare identifier is the field of the current record
   and binds to `field`.
5. Values derived from other parameters are written explicitly: the `mode` of
   a pattern resolves to `literal` or `regex`, and the default `watch` list of
   `run` is the list holding `cwd`.
6. Short source forms of change commands are expanded to their single
   pipeline: `remove "/a"` binds to `find "/a" depth: 0 type: any |> remove`,
   so both programs have the same bound tree.
7. The pipeline checked by `verify` is the `sub` value of its parameter
   `check`.
8. `effect` always comes from the catalog, never from the program. Stages of
   the phases `source`, `transform`, `check`, `after` and `verify` have the
   effect `read`.

## 5. Phases

| `phase` | Stage | Effect in the world |
|---|---|---|
| `source` | the head stage that produces records | read only |
| `transform` | record transformations before the effect | none |
| `check` | `assert` before the effect; failure changes nothing | none |
| `mutate` | the single change stage | change, journaled |
| `run` | the single program-running stage | program effects, tracked |
| `after` | `assert`, `filter`, `select`, `first`, `last`, `skip`, `count`, `sort`, `group` on the records produced by the effect | none; a failed `assert` reports `E_ASSERT` without rolling the effect back |
| `verify` | `verify (…)`; failure rolls the effect back | rollback |

A pipeline has at most one stage of phase `mutate` or `run`. Stages of phase
`after` stand only after it, and `verify`, when present, is the last stage.
Any other stage after the effect is rejected with `E_ONE_EFFECT`.

## 6. `ast_hash`

```
ast_hash = "sha256:" + lowercase_hex(SHA-256(UTF-8(JCS(hash_form(document)))))
```

The hash form of a document is the document without the member `meta` of the
root object and without the member `span` of every object
(`nurt_contracts::ast::strip_for_hash`). It is taken from the JSON produced by
serializing the decoded, typed tree, so omitted optional members and members
written with their default value (`"waits": false`, `"default": false`) never
change the hash. Programs that differ only in layout, comments or syntax
variant have the same `ast_hash`. The report and the journal use `ast_hash` of
the bound tree, which includes every default; a later change of a default
value never changes the hash or the meaning of a recorded program.

## 7. Conformance files

| Path | Content |
|---|---|
| `golden/ast/<name>.nurt` | program text, word notation |
| `golden/ast/<name>.json` | surface tree of the program, with spans; trees without program text use structured nodes and metadata |
| `golden/ast/<name>.hash` | `ast_hash` of the surface tree |
| `golden/bound/<name>.json` | bound tree of the same program against the reference catalog |
| `golden/bound/<name>.hash` | `ast_hash` of the bound tree |
| `golden/<format>/schema.generated.json` | schema generated from the Rust types |

Golden bound trees use the placeholder revisions `cat-1` and `corr-1`.
Conformance tests check that every tree is valid under the published schema
and under the generated schema, deserializes into the Rust types and
serializes back to the same JSON value, survives a CBOR round trip unchanged
and has the recorded `ast_hash`. Setting `NURT_BLESS=1` regenerates the
generated schemas and the recorded hashes after a reviewed change.
