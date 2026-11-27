<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Command catalog `nurt-catalog/1`

The command catalog is the single source of command signatures. The binder,
the built-in help, the model description, the reference documentation, the
tool definitions and the conformance suite all read it. It consists of:

| File | Content |
|---|---|
| `catalog/_index.toml` | the index `nurt-catalog-index/1`: catalog files and the expected number of commands |
| `catalog/core.toml` | the core commands, always described to the model |
| `catalog/<group>.toml` | one file per optional group, described on request |
| `spec/schemas/nurt-catalog-1.schema.json` | the JSON schema of a catalog file, generated from the Rust types |

The Rust types are in the module `nurt_contracts::spec`: `CatalogFile`,
`CatalogIndex`, `StageSpec`, `ParamSpec`, `ParamType`, `DefaultValue`,
`IoSpec`, `FormSpec`, `ActionEffect`, `ExternalUse`.

## 1. Catalog file

```toml
format = "nurt-catalog/1"
group = "documents"
summary = "Structured documents and SQLite."

[[command]]
name = "set"
category = "change"
reversibility = "reversible"
effect = "local"
implementation = "builtin"
input = ["none", "entry"]
output = ["effect"]
summary = "Set values at key paths, preserving layout."

[[command.param]]
name = "path"
type = "path | list<path>"
position = 1
required = true
summary = "Document to change."
```

### 1.1 File keys

| Key | Type | Required | Meaning |
|---|---|---|---|
| `format` | text | yes | always `nurt-catalog/1` |
| `group` | identifier | yes | `core` or the group name; equal to the file name without `.toml` |
| `summary` | text | yes | one sentence ending with a full stop |
| `command` | array of tables | no | the commands of the group |

### 1.2 Command keys (`[[command]]`)

| Key | Type | Required | Meaning |
|---|---|---|---|
| `name` | text | yes | full command name: an identifier or two identifiers joined by a dot (`git.status`); unique in the whole catalog |
| `group` | identifier | no | normally omitted; when present, equal to the file group |
| `category` | enum | yes | `source`, `transform`, `check`, `change`, `run`, `system`, `journal`, `help` |
| `reversibility` | enum | yes | `read`, `reversible`, `reversible_if_preserved`, `irreversible` |
| `effect` | enum | yes | default effect: `read`, `local`, `external`, `process` |
| `implementation` | enum | yes | `builtin`, `user_program`, `language_server`, `package_manager`, `system_program` |
| `input` | list of kinds | yes | accepted input kinds (section 4) |
| `output` | list of kinds | yes | produced output kinds (section 4) |
| `summary` | text | yes | one sentence ending with a full stop |
| `reversibility_note` | text | no | conditions of reversibility; omitted when there are none |
| `effects` | list of tables | no | actions whose effect differs from the default (section 7) |
| `waits` | bool | no | the command always blocks the call by waiting (`sleep`); default `false` |
| `os` | list of enum | no | operating systems the command is limited to: `linux`, `macos`, `windows`; empty means all; on other systems binding fails with `E_OS_ONLY` |
| `external` | table | no | use of an external program or system component beyond `implementation` (section 8) |
| `fallback` | enum | no | implementation used when the primary one is unavailable, such as `builtin` syntax analysis for a `language_server` command without an installed server; the result states which one ran |
| `required_groups` | list of identifiers | no | exclusive groups of which exactly one parameter must be given (section 6) |
| `param` | array of tables | no | parameters (section 2) |
| `form` | array of tables | no | input-dependent forms of a polymorphic command (section 5) |

### 1.3 Parameter keys (`[[command.param]]`)

| Key | Type | Required | Meaning |
|---|---|---|---|
| `name` | identifier | yes | parameter name, unique in the command |
| `type` | type expression | yes | section 3 |
| `position` | integer ≥ 1 | no | position among positional arguments of the primary form; the positions of a command form the sequence `1..n` |
| `required` | bool | no | required in the primary form; default `false`; a required parameter has no `default`; with `default_from` it is missing only when its source has no value either |
| `repeated` | bool | no | a positional parameter that collects several arguments into a list; its type accepts a list; at most one per command |
| `per_record` | bool | no | the value may be an expression evaluated on each input record (`to: "/dst/" + name`) |
| `default` | text | no | default value as a literal of the language in variant A: `"1"`, `"true"`, `'"**"'`, `"asc"`, `"120s"`, `"10mb"`, `"[]"`; absent means no value (`null`) |
| `unbounded_default` | bool | no | the default is "no limit"; excludes `default` |
| `default_from` | identifier | no | the default is the value of another parameter of the command or of the call parameter `base`, converted to the type of this parameter (a path becomes a one-element list): `run cwd` defaults to `base`, `run watch` to `cwd`; excludes `default` and `unbounded_default` |
| `default_note` | text | no | description of a computed default ("the latest operation") |
| `values` | list of identifiers | conditional | the enumeration values of the type, repeated for readability; required and equal to the values of the type, in order, when the type contains `enum(…)` |
| `summary` | text | yes | short description |
| `exclusive_group` | identifier | no | parameters with the same group name are mutually exclusive (section 6) |
| `waits` | bool | no | a value of this parameter greater than its default, or the value `forever`, marks the stage `waits: true` in the bound tree (`run wait:` above `3s`, `jobs wait:` above `0s`) |
| `secret` | bool | no | the value is confidential (credentials in an address or a header); it is masked in the journal, the report and the printed bound tree |

A bare-word default (`asc`, `auto`) is one of `values`; the word `all` is also
accepted for parameters whose type contains `range` (the whole range).

## 2. Call parameters are not command parameters

The following parameters belong to the call envelope (`nurt-request/1`,
`options`) and to the `--name` options of the command line, not to any
command. They apply to every command and are never listed in catalog files.

| Parameter | Type and default | Meaning |
|---|---|---|
| `preview` | `bool = false` | compute targets and effect only, without changing anything |
| `base` | `path` | directory against which relative paths are resolved; not a current directory |
| `limit` | `int = 50` (at most 5000) | records or changes per page of the result |
| `cursor` | `text` | cursor of the next page; reading a page again does not run a changing program again |
| `fields` | `list<text>` | fields of the result, like a final `select` |
| `format` | `enum(text\|json\|markdown\|compact) = text` | form of the result |
| `verbosity` | `enum(quiet\|normal\|detailed) = normal` | detail level of the effect report |
| `diff` | `enum(none\|stat\|unified) = unified` | form of differences in the report of a change |
| `diff_context` | `int = 3` | context lines of differences |
| `max_chars` | `int = 0` (no limit) | limit of a single text field of the result; truncated fields carry `truncated: true` |
| `save_to` | `path` | write the full result to a file (UTF-8) instead of returning it |
| `timeout` | `duration = 120s` | time limit of the whole program (`run timeout:` limits one program) |
| `session` | `text` | session identifier recorded in the journal |
| `language` | `enum(en\|pl) = en` | language of help and hints; codes, fields and keywords are always English |
| `script` | `bool = false` | script mode: several pipelines, each a separate operation |
| `privilege` | `enum(account\|elevated\|auto) = account` | rights of the operation: the account that runs Nurt; the administrator (`root` on Linux and macOS, `LocalSystem` on Windows); or `auto`, which runs with the account and repeats the whole stage once with `elevated` when a target fails with `E_PERMISSION` or `E_PRIVILEGE` before any change is kept. The default comes from the configuration key `privilege.default`. Valid on every changing command, on `run` and on reading commands. No command is marked "never elevated" |
| `as_user` | `text` | account that runs the operation (equivalent of `sudo -u NAME`; Linux and macOS); implies elevation for the switch; ends with `E_UNSUPPORTED` on Windows |

A command may have a parameter with the name of a call parameter when its
meaning is narrower and stated in the catalog: `run timeout:` limits one
program, while the call parameter `timeout` limits the whole program;
`write format:` is the serialization of written records, while the call
parameter `format` is the form of the result. The two never collide, because
call parameters are not part of the program text.

The two call parameters `privilege` and `as_user` are the exception: they may
also be written on a stage, as named arguments, on every command that does not
declare a parameter of the same name (`write "/etc/hosts" "..." privilege:
elevated`, `package.install ["jq"] privilege: elevated`, `run [...] cwd: "/"
as_user: "deploy"`). The binder takes them out of the signature, checks the
value (`privilege` is `account`, `elevated` or `auto`, written as a word or a
text; `as_user` is a non-empty word or text; each at most once per stage) and
appends them to the arguments of the stage (`privilege` as an `enum` value, `as_user` as `text`) after the
parameters of the signature, with `default: false`. A value written on a stage
replaces the option of the request and the configuration key
`privilege.default` for the whole call.

Rule of `privilege: auto`: the call first runs with the rights of the account. When it ends with `E_PERMISSION` or `E_PRIVILEGE` and has left no operation (`op` absent: nothing was kept), the whole call is repeated once with `elevated`; the report states `elevated: true` and `elevation_reason: permission_denied`. A call that already changed something is never repeated, and a second denial is returned as it is. A stage reads the effective value from
the options of its context: `ctx.options().privilege()` and
`ctx.options().effective_privilege()`; the executor has already run the call
with the rights that value names, so a stage never needs to elevate itself.

### 2.1 Shared command parameters

Parameters with the same name have the same meaning and type in every command
that has them. The default may differ per command and is always written in
the catalog.

| Parameter | Type and usual default |
|---|---|
| `case_sensitive` | `bool = true` |
| `whole_word` | `bool = false` |
| `whitespace` | `enum(exact\|trailing\|indent\|normalize)`; `exact` for searching, `normalize` for `replace` and `edit` |
| `encoding` | `text = "auto"` |
| `line_ending` | `enum(lf\|crlf\|cr\|keep) = keep` |
| `hidden` | `bool = true` |
| `gitignore` | `bool = false` |
| `on_missing` | `enum(skip\|error) = skip` |
| `on_conflict` | `enum(overwrite\|skip\|error)`, default per command |
| `expect` | `range` |
| `if_match` | `text` (etag), or `text \| map<text>` (path to etag) for commands with several target files |
| `validate` | `enum(off\|warn\|reject) = reject` |
| `depth` | `int` |
| `type` | `enum(file\|directory\|link\|any)` for entry kinds |
| `order` | `enum(asc\|desc) = asc` |
| `file_type` | `list<text> = []`, file types by extension (`go`, `ts`); applies in every output mode |
| `hash` | `enum(none\|sha256\|sha512\|sha1\|md5\|blake3\|crc32)`; the algorithm list is the same wherever a hash algorithm is chosen |

## 3. Type expressions

The `type` key holds a type expression. The parser is part of the contract
(`ParamType::parse`); an invalid expression makes the catalog file invalid.

```text
type        = alternative { "|" alternative } ;
alternative = scalar
            | "list" [ "<" type ">" ]
            | "map"  [ "<" type ">" ]
            | "stream" "<" identifier ">"
            | "enum" "(" identifier { "|" identifier } ")" ;
scalar      = "text" | "int" | "number" | "bool" | "path" | "glob" | "regex"
            | "size" | "duration" | "percent" | "instant" | "key" | "bytes" | "range"
            | "expression" | "pipeline" | "field" | "named_expression" | "any" ;
identifier  = lowercase letter { lowercase letter | digit | "_" } ;
```

Spaces and tabs between tokens are ignored. Inside `list<…>` and `map<…>` the
full grammar applies, so `list<text | regex>` is a list whose elements are
text or regular expressions.

| Type | Meaning |
|---|---|
| `text`, `int`, `number`, `bool` | basic values |
| `path`, `glob`, `regex`, `key`, `instant`, `size`, `duration` | values with literal forms of the language; a quoted text is converted implicitly to `path`, `glob`, `instant`, `key`, `size` and `duration` in a place of that type |
| `percent` | share of a capacity, written as a quoted text: `"10%"`; a command may also accept `"10%free"`, a share of the free space, and says so in the parameter summary; there is no separate literal, the text is converted in a place of this type like `size` and `duration` |
| `bytes` | raw bytes (base64 in JSON) |
| `range` | `5`, `1..5`, `40..`, `..9`, `all`; lines count from 1, bounds inclusive |
| `expression` | expression evaluated on each input record (`filter`, `assert all:`) |
| `pipeline` | nested pipeline in parentheses (`verify (…)`, `write from: (…)`) |
| `field` | name of a record field (`sort size`) |
| `named_expression` | pair `name: expression` computing a new field (`select total: size * 2`) |
| `any` | any value |
| `enum(a\|b)` | one of the bare words |
| `list<T>`, `map<T>` | list; map from text keys; bare `list` and `map` mean `list<any>` and `map<any>` |
| `stream<R>` | result of a named pipeline `@name` with records of kind `R` (`any` for any kind) |
| `A \| B` | any of the alternatives |

The canonical text of a type (used in serialization, help and JSON) writes
alternatives separated by `" | "`, enumeration values separated by `|`
without spaces, and bare `list` and `map` as `list<any>` and `map<any>`.
Parsing the canonical text gives the same type.

Errors are typed (`TypeParseError`): `empty`, `unexpected_end`,
`unexpected_char`, `unknown_type`, `invalid_enum_value`,
`duplicate_enum_value`, `duplicate_alternative`, `too_deep` (nesting deeper
than 16 levels), each with the byte offset of the problem.

## 4. Input and output kinds

`input` and `output` list kinds (`IoSpec`):

| Kind | Meaning |
|---|---|
| `none` | input only: the command starts a pipeline (source form) |
| `any` | records of any kind |
| `same` | output only: the records of the input, unchanged in kind (`filter`, `sort`) |
| scalar keyword (`int`, `text`) | a single value (`count` produces `int`, `help` produces `text`) |
| record kind (`entry`, `match`, `effect`, …) | records of that kind |

The kind `effect` is the effect report of a changing or running stage seen as
a record; the stages after an effect (`assert`, `filter`, `select`, `first`,
`last`, `skip`, `count`, `sort`, `verify`) consume it.

## 5. Forms of polymorphic commands

Some commands accept different parameters depending on their input: the
source form of `replace` takes `pattern`, `with` and `in`, while the stage
form after `search` takes only `with`. Such commands list forms:

```toml
[[command.form]]
name = "source"
input = ["none"]
positional = ["pattern", "with"]
required = ["pattern", "with", "in"]
summary = "Replace a pattern in the given files."

[[command.form]]
name = "matches"
input = ["match", "node", "item"]
positional = ["with"]
required = ["with"]
params = ["with", "occurrence", "template", "expect", "if_match", "validate"]
summary = "Replace the text of the input matches."
```

| Key | Required | Meaning |
|---|---|---|
| `name` | yes | form name, unique in the command |
| `input` | yes | input kinds of the form; every input kind of the command belongs to exactly one form |
| `output` | no | output kinds of the form; empty means the output of the command |
| `positional` | no | positional parameters of the form, in order |
| `required` | no | required parameters of the form; a name of an exclusive group means "exactly one of the group" |
| `params` | no | parameters accepted by the form; empty means all |
| `expand` | no | in a source form: the parameter that names the entries the command works on; the binder expands the form (section 5.2) |
| `summary` | yes | one sentence |

The binder selects the form whose `input` contains the kind of the input
records (`none` for a source). The first form is the primary form: the
`position` and `required` keys of the parameters and the command key
`required_groups` describe it, and the catalog check verifies that they agree.
A command without forms has a single form described by its parameters.

### 5.1 Repeated positional parameters

A repeated parameter (`repeated = true`) collects positional arguments into a
list. When positional parameters follow it (`copy FROM… TO`), the later
positions take the last arguments and the repeated parameter takes the rest,
as in `copy "/a" "/b" "/dst"` (`from: ["/a", "/b"]`, `to: "/dst"`). With a
single positional argument the repeated parameter takes it.

### 5.2 Expanded source forms

A source form with the key `expand` is not bound as written. The binder
expands it to an `info` head whose `path` is the value of the named
parameter, followed by the command bound in its form for `entry` input:

```toml
[[command.form]]
name = "source"
input = ["none"]
positional = ["path"]
required = ["path"]
expand = "path"
summary = "Remove the given files and directories."
```

`remove "/a" "/b"` binds to `info ["/a", "/b"] |> remove`, so the two programs
have the same bound tree. `info` accepts a list of paths, does not descend
into directories and reports a missing path as an entry with `exists: false`,
which the command handles according to its `on_missing` parameter. The core
catalog expands the source forms of `copy` and `move` (`from`) and of
`remove`, `rename` and `attributes` (`path`). The named parameter accepts a
path, the form takes no input, and the form for `entry` input does not accept
the parameter. Source forms without `expand` (`replace`, `insert`) are bound
as written.

## 6. Exclusive and required groups

Parameters that share `exclusive_group` exclude each other: at most one of
them may be given. A group listed in the command key `required_groups` (or in
the `required` list of a form) requires exactly one. A group has at least two
members, and its name is not the name of a parameter.

```toml
required_groups = ["condition"]
[[command.param]]
name = "count"
type = "range"
exclusive_group = "condition"
summary = "Expected number of records."
```

## 7. Effects of actions

`effects` lists actions whose effect or reversibility differs from the
command default:

```toml
effects = [
  { action = "push", effect = "external", reversibility = "irreversible" },
]
```

`action` is an identifier naming one of:

| Meaning | Example |
|---|---|
| a value of the `action` parameter of the command | `git.stash` `action: drop` |
| a value of another enumeration parameter | `http.request` `method: post` (`action = "post"`) |
| the name of a parameter whose use changes the effect | `http.request` `save:` (`action = "save"`) |
| the input record kind, for commands whose effect depends on it | `remove` on `job` (`action = "job"`) |

`note` states the condition when it is not obvious from the identifier.

## 8. External use

`external` marks the use of a program or system component outside the
executable, as required for every such operation:

```toml
external = { kind = "system_program", programs = ["launchctl"], note = "Services on macOS." }
```

| Key | Required | Meaning |
|---|---|---|
| `kind` | yes | `user_program`, `language_server`, `package_manager` or `system_program` |
| `programs` | no | program names |
| `when` | no | parameter whose use triggers the external use; absent when unconditional |
| `note` | no | remark shown in the description and in the report |

## 9. Index

```toml
format = "nurt-catalog-index/1"
total = 148

[[file]]
name = "core"
file = "core.toml"
summary = "Core commands for files, text, records, programs, the journal and help."
commands = 33
```

The first entry is `core`; every entry names `<name>.toml`; the sum of
`commands` equals `total`.

### 9.1 Catalog revision

The catalog revision identifies the exact catalog used for binding and is
recorded as `catalog_rev` in every bound tree:

```
catalog_rev = "sha256:" + lowercase_hex(SHA-256(UTF-8(JCS(files))))
```

`files` is the JSON array of the catalog files in the order of the index, each
serialized from `CatalogFile` (the form validated by
`nurt-catalog-1.schema.json`), and JCS is the canonical JSON of RFC 8785. Any
change of a command, a parameter or a description therefore changes the
revision. The command registry of the engine reports the same value as
`catalog_rev` in the result of a catalog query and in `nurt_info`. The groups
of registered extensions follow the files of the index as files of the same
form, so a registry with extensions reports a different revision from the one
recorded in a bound tree, which names the built-in catalog used for binding.
The revision of the correction tables, `corr_rev`, has the same
form.

## 10. Checks

`CatalogFile::check`, `CatalogIndex::check` and `check_catalog` return the list
of problems; the test suite requires an empty list for every file in
`catalog/`:

1. every file parses into `CatalogFile` (unknown keys are rejected), has the
   format `nurt-catalog/1` and a group equal to its file name;
2. command names are valid and unique in the whole catalog;
3. every type expression parses; every `default` is non-empty; a required
   parameter has no default; `values` repeat the enumeration values of the
   type; a bare-word default is one of the values; `default_from` names
   another parameter of the command or `base` and excludes `default`;
4. the positions of each command form the sequence `1..n`; a repeated
   parameter is positional and accepts a list;
5. exclusive groups have at least two members; required groups exist;
6. forms are consistent with the parameters and cover every input kind once;
   `expand` stands only in a source form, names a parameter that accepts a
   path, and that parameter is not accepted by the form for `entry` input;
7. the index lists every file with its number of commands, and when every
   listed file is present the number of commands equals `total`.

## 11. Compatibility

Within `nurt-catalog/1` the catalog only grows: new commands, new optional
parameters whose default keeps the previous behaviour, new enumeration values
and new optional keys. Removing or renaming a command or a parameter, changing
a type incompatibly, or changing a default is a change of the major version.
