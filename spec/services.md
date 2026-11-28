<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           spec/services.md -->
<!-- Responsibility:   Documents Shared services. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Shared services

Command crates depend only on `nurt-contracts`. When a command of one group
needs a capability implemented by another group, it calls a service trait
from `nurt_contracts::services`. The engine registers the implementations
once, in a `Services` registry carried by the execution context. An engine
configured for the running process (`EngineConfig::system`) registers the
services of every group of the build that the embedding application did not
supply itself: documents, source code, the process host, the file listing
(when the engine has a file system), repository queries, the text codec and
the file holders. The review rule provider has no implementation of its own.

A missing service is never an error by itself. It means the group that
provides it is not loaded; the calling command applies the fallback listed
below and, where the result is affected, reports it in the effect report.

## Services

| Trait | Provided by | Used by | Without the service |
|---|---|---|---|
| `DocService` | documents group | `read key:`, `write format:`, `validate` on documents, test and analysis commands that read configuration files | `read key:` and `write format:` fail with `E_EXTERNAL_MISSING`; `validate` is skipped with a note |
| `CodeService` | code group | `read symbol:`, `read view: outline \| symbols`, `search mode: syntax`, `search scope:`, `search enclosing:`, `validate` on source code, reference checks of `remove`, `rename` and `replace`, `info details: symbols`, `git.diff symbol:`, `git.log symbol:`, `analyze.check checks: complexity, duplication` | `view:` falls back to the full content with a note; `scope:` other than `any`, `symbol:` and `mode: syntax` fail with `E_EXTERNAL_MISSING`; reference checks are textual; git commands use the function context rules of git |
| `ProcessHost` | run group | language server client, git hooks, service and package managers | the language server client reports `source: syntax`; hooks and managers fail with `E_EXTERNAL_MISSING` |
| `FileListService` | file commands | repository-wide code commands (`code.symbols`, `code.references`, `code.profile`) | the code group walks the tree with the same exclusions on its own |
| `GitQueryService` | git group | `code.profile` hot spots, `code.diagnostics changed:`, `info details: git`, `analyze.* changed_only` | the dependent fields are empty; `changed: true` fails with `E_EXTERNAL_MISSING` |
| `TextCodecService` | text group | document and code commands that decode or encode files | UTF-8 and UTF-16 with a byte order mark only |
| `FileHolderService` | system group | the file layer (`E_IN_USE` lists holders), `open_files` | `E_IN_USE` without holders |
| `ReviewRuleProvider` | engine or extension | `git.review checks: [secrets]` | built-in rules only |

## Common rules

1. Every trait is object safe, `Send` and `Sync`, and is used as
   `&dyn Trait`. Handles of running processes (`ServiceChild`) are `Send`
   only: they have one user at a time.
2. Methods are synchronous. Long work is bounded by the caller: the process
   host takes the cancellation token of the stage; the other services work on
   bytes that are already in memory.
3. The document and code services never read files. The caller reads the file
   through the file layer and passes the bytes, so the same methods check
   content that is not yet written (`validate`) and run on virtual file
   systems in tests.
4. Errors use the contract error type. A service never returns an error for an
   ordinary negative answer: a key that selects nothing is an empty list, a
   program that exits with a non-zero code is an outcome.
5. Positions are given twice: as byte ranges (zero-based, end exclusive) for
   editing, and as one-based lines and columns counted in Unicode scalar
   values for records.

## Document service

| Method | Purpose |
|---|---|
| `formats()` | formats with their file name extensions and whether keys can be changed with layout preserved |
| `detect_format(path, bytes)` | format from the file name and content; `None` for unstructured content |
| `read_keys(bytes, format, key, depth)` | values selected by a key path, as `DocItem` (canonical concrete key, value, region, comment, document index, YAML anchor, alias and tag) |
| `check_syntax(bytes, format)` | syntax issues; an empty list means the content parses |
| `serialize_records(records, format, options)` | records written as a document (`write format: json \| yaml \| toml \| csv \| tsv`) |

Format identifiers are lowercase (`json`, `jsonc`, `yaml`, `toml`, `xml`,
`ini`, `csv`, `tsv`, `properties`, `env`, `plist`); extensions may add more.
Changing documents is not part of the service: `set`, and `remove` or
`replace` on `item` records, belong to the documents group and are dispatched
by record type.

## Code service

| Method | Purpose |
|---|---|
| `languages()` | languages with a grammar |
| `language_of(path, bytes)` | language from the file name and content |
| `outline(bytes, language, depth)` | symbols in source order with nesting depth |
| `find_symbol(bytes, language, name)` | definitions of a name; `Container.name` narrows the container; several results mean the name is ambiguous |
| `enclosing(bytes, language, offset)` | innermost symbol containing a byte |
| `classify_spans(bytes, language)` | comment and string regions; the rest is code |
| `check_syntax(bytes, language)` | syntax errors |
| `compile_pattern(pattern, language)` | structural pattern with metavariables `$X` and `$$$X`, as an opaque `SyntaxPattern` |
| `syntax_search(bytes, pattern)` | matches with captures |
| `render_template(match, template)` | replacement text with captures substituted |
| `references(bytes, language, name)` | occurrences of an identifier with role and lexical class |
| `function_metrics(bytes, language)` | lines, parameters, cyclomatic complexity and nesting per function |
| `tokens(bytes, language)` | tokens for duplicate detection |

A language without a grammar fails with `E_UNSUPPORTED_LANGUAGE`; a pattern
that does not parse in the language fails with `E_PATTERN_SYNTAX`. Searches
across a repository combine `FileListService` with per-file calls.

## Process host

| Method | Purpose |
|---|---|
| `resolve_program(program, cwd)` | program file as `run` would find it |
| `run_program(spec, cancel)` | run to completion with captured output, time limit, cancellation and optional detection of changed files |
| `spawn_service(spec)` | long-running helper with piped standard streams in its own process group or job object |

Programs follow the rules of `run`: arguments as a list without a shell,
lookup in `PATH` (and `PATHEXT` on Windows), batch files started through the
command interpreter with every argument quoted, no terminal, and the whole
process tree stopped on timeout, cancellation or `kill_tree`.

## File listing

`walk(spec)` lists entries under a root with the parameters and defaults of
`find`: globs (default `**`), entry kind (default files), depth, hidden
entries (default included), symbolic links (default not followed), ignore
files (default off), exclusions in gitignore syntax and named exclusion groups
`vcs`, `deps`, `build`, `cache`. `WalkSpec::source_tree` is the request used by
repository-wide code analysis: ignore files on and `vcs`, `deps`, `cache`
excluded.

## Registry

`Services` is built with `with_doc`, `with_code`, `with_process_host`,
`with_file_list`, `with_git_query`, `with_text_codec`, `with_file_holders` and
`with_review_rules`. The accessors of the same names (`doc()`, `code()`, …)
return `Option<&dyn Trait>`; `available()` lists the registered services by
their stable names (`doc`, `code`, `process_host`, `file_list`, `git_query`,
`text_codec`, `file_holders`, `review_rules`).

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
