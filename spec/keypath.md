<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           spec/keypath.md -->
<!-- Responsibility:   Documents Key paths. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Key paths

A key path addresses values inside a structured document: JSON, JSONC, YAML,
TOML, XML, INI, CSV, TSV, Java properties, environment files and property
lists. It is the value of the language type `key`. A text literal in a
parameter of type `key` is converted when the program is bound, so a malformed
key is reported before any command runs, with error code `E_KEY_SYNTAX`.

The Rust type is `nurt_contracts::keypath::KeyPath`. Its parser and canonical
printer depend only on the standard library.

```
read "package.json" key: "scripts.test"
set "deploy.yaml" key: "#1:spec.replicas" value: 3
set "package.json" key: "keywords[+]" value: "cli"
read "Cargo.toml" key: "dependencies.*.version"
read "users.json" key: "users[?age >= 18 and active].name"
```

## Grammar

```
keypath      = [ doc_selector ] ( json_path | "$" | native ) ;
doc_selector = "#" ( digit { digit } | "*" ) ":" ;

native       = first { step } ;
first        = name | "*" | "**" | bracket ;
step         = "." ( name | "*" | "**" ) | bracket ;
name         = name_char { name_char } ;
name_char    = any character except "." "[" "]" '"' "'" ;
bracket      = "[" ws ( index | "*" | "**" | "+" | quoted | "?" filter ) ws "]" ;
index        = [ "-" ] digit { digit } ;

json_path    = "$" { "." ( name | "*" ) | ".." ( name | "*" | bracket ) | bracket } ;

quoted       = '"' { char | escape } '"' | "'" { char | escape } "'" ;
escape       = "\" ( '"' | "'" | "\" | "/" | "n" | "r" | "t" | "b" | "f"
             | "u" hex hex hex hex | "u{" hex { hex } "}" ) ;

filter       = or_expr ;
or_expr      = and_expr { ( "or" | "||" ) and_expr } ;
and_expr     = not_expr { ( "and" | "&&" ) not_expr } ;
not_expr     = ( "not" | "!" ) not_expr | comparison ;
comparison   = operand [ ( "==" | "!=" | "<" | "<=" | ">" | ">=" ) operand ] ;
operand      = "(" or_expr ")" | literal | field ;
field        = "@" { field_step } | field_name { field_step } ;
field_step   = "." field_name | "[" ws quoted ws "]" ;
field_name   = ( letter | "_" ) { letter | digit | "_" | "-" | "$" } ;
literal      = quoted | number | "true" | "false" | "null" ;
number       = [ "-" ] digit { digit } [ "." digit { digit } ] [ ( "e" | "E" ) [ "+" | "-" ] digit { digit } ] ;
```

`ws` is optional white space. A path that starts with `$` followed by `.`, `[`
or the end of the text is JSONPath; any other `$` starts an ordinary name, so
`$schema` and `definitions.$ref` are names.

## Segments

| Written | Segment | Meaning |
|---|---|---|
| `name`, `["name"]`, `['name']` | key | member of a map |
| `[0]`, `[-1]` | index | element of a list; negative values count from the end |
| `[*]`, `.*`, `*` | wildcard | every member or element of one level |
| `.**`, `**`, `[**]`, `..` (JSONPath) | recursive | zero or more levels: `a.**` selects `a` and all of its descendants, `**.version` every `version` member at any depth |
| `[+]` | append | the position after the last element of a list; only the final segment |
| `[?expression]` | filter | elements of a list (or members of a map) for which the expression holds |
| `#1:`, `#*:` | document selector | document of a multi-document stream (YAML); no selector is the first document |
| `$` alone | root | the whole document |

Rules:

1. A name keeps every character as written, including inner spaces; nothing
   is trimmed.
2. A name made of digits is a key: `a.0` is the member `"0"` of a map, or the
   first element when the node is a list; `a[0]` is always an index. When a
   command creates nodes, `a.0` creates a map and `a[0]` a list.
3. In XML documents a name that starts with `@` addresses an attribute
   (`project.@version`) and `#text` the text content; a namespaced element is
   written `prefix:name` with the prefixes of the document, or
   `["{uri}name"]`. In every other format these are ordinary names, for
   example the JSON-LD member `@context`.
4. Fields of a filter are read relative to the element being tested; `@`
   alone is the element itself (`tags[?@ == "beta"]`). A filter that cannot be
   evaluated on an element (missing field, values of different types under
   `<`) is false for that element.
5. JSONPath slices (`[1:3]`), unions (`[0,1]`), functions (`length(@)`) and
   absolute references inside filters (`$.x`) are not supported and are
   reported as errors.

## Canonical form

Every key path has one canonical spelling, the shortest unambiguous one. The
`key` field of `item` records and the bound syntax tree use it. For every key
path `k`, parsing the canonical text of `k` gives `k` back.

| Element | Canonical spelling |
|---|---|
| key | bare name after `.` (no dot for the first segment) when the name is non-empty, has no `.`, `[`, `]`, `"`, `'` or control character, does not start or end with white space and is not `*` or `**`; otherwise `["…"]` |
| first key | as above, and also quoted when it is `$` or looks like a document selector (`#1:x`) |
| index | `[n]` without leading zeros |
| wildcard | `[*]` |
| recursive | `**` first, `.**` elsewhere |
| append | `[+]` |
| document selector | `#n:` or `#*:`; `#0:` is omitted |
| root | `$` |
| quoted text | double quotes; `\"`, `\\`, `\n`, `\r`, `\t` and `\u{…}` for other control characters |
| filter | operators `==`, `!=`, `<`, `<=`, `>`, `>=`, `and`, `or`, `not`, separated by single spaces; parentheses only where precedence requires them (and always around a comparison under `not`) |
| filter field | bare names joined by `.`; a name that is not a valid `field_name` or is one of `true`, `false`, `null`, `and`, `or`, `not` in brackets (`@["a b"]`); the element itself `@` |
| filter number | integers as written; other numbers in the shortest form that reads back as the same value, always with a fraction or an exponent (`1000.0`, `1e300`) |

Examples:

| Written | Canonical |
|---|---|
| `scripts.test` | `scripts.test` |
| `$.a.b` | `a.b` |
| `$['a b']` | `a b` |
| `a.*` | `a[*]` |
| `$..name` | `**.name` |
| `a[ 007 ]` | `a[7]` |
| `#0:a` | `a` |
| `["a.b"].c` | `["a.b"].c` |
| `a[?@.name=='x'&&size>2]` | `a[?name == "x" and size > 2]` |
| `a[?!(x == 1)]` | `a[?not (x == 1)]` |

## Errors

A malformed key path is a binding error `E_KEY_SYNTAX`. The error carries the
byte offset and the one-based column (in characters) inside the key text, a
kind, and a corrected key when one correction is evident.

| Kind | Example | Correction |
|---|---|---|
| `empty` | `""` | — |
| `leading_dot` | `.a` | `a` |
| `trailing_dot` | `a.` | `a` |
| `empty_name` | `a..b` | `a.**.b` |
| `dot_before_bracket` | `a.[0]` | `a[0]` |
| `empty_bracket` | `a[]` | `a[*]` |
| `unquoted_name` | `a[b]` | `a["b"]` |
| `character_requires_quoting` | `it's` | `["it's"]` |
| `unexpected_character` | `a[0]b` | `a[0].b` |
| `unexpected_end` | `a[` | — |
| `unterminated_string` | `a["x` | — |
| `invalid_escape` | `a["\q"]` | — |
| `index_out_of_range` | `a[99999999999999999999]` | — |
| `invalid_number` | `a[?x == 1e400]` | — |
| `empty_filter` | `a[?]` | — |
| `invalid_filter` | `a[?x = 1]` | `a[?x == 1]` |
| `misplaced_append` | `a[+].b` | — |
| `missing_path` | `#1:` | `#1:$` |
| `unsupported` | `a[1:3]` | — |

Filters nest at most 64 levels of parentheses and negations.

## Relation to the syntax trees

In the surface syntax tree a key is a `text` node, or a structured `key` node
whose segments are names, indices, wildcards (`*`, `**`, `+`) and filters
written as expressions. `KeyPath::from_key_segs` converts the structured form;
filters may use fields, identifiers, literals, comparisons and `and`, `or`,
`not`. In the bound syntax tree a key is a `key` node holding the canonical
text.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
