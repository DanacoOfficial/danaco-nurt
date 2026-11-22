---
command: find
group: core
summary: Find entries under a directory by name pattern and attributes.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# find

## Synopsis

```text
find <path> [<glob>] [name: value]...
```

## Description

`find` walks the directory `path` and produces one `entry` record for every
entry that matches `glob`, `type`, `file_type` and `fuzzy`, down to `depth`
levels. Unlike `list`, the root itself is a candidate at depth 0: `find
"/a" depth: 0 type: any` yields exactly one record when `/a` exists and fails
with `E_NOT_FOUND` otherwise. A root that is a file is a candidate in the
same way.

`glob` patterns are matched against the path relative to `path`, with `/`
separators on every system. `*`, `?` and `[...]` do not match `/`; `**`
matches any number of directories, so `*.rs` matches the top level only and
`**/*.rs` matches at any depth. The root is matched by its own name. An
entry matching any pattern of the list is selected. Patterns are case
sensitive unless `case_sensitive: false`.

`glob` and `exclude` compare patterns and paths in Unicode normalization
form C (NFC). A name stored decomposed, as on macOS volumes, matches a
pattern written composed, and a decomposed pattern matches a composed name.

Entries are excluded before they are matched, and an excluded directory is
not entered, so `!pattern` cannot restore an entry below an excluded
directory. The rules apply in this order: `hidden: false`, `exclude_preset`,
`exclude` and, with `gitignore: true`, the ignore files.

| `exclude_preset` | Excluded directories |
|---|---|
| `vcs` | `.git`, `.hg`, `.svn`; also a `.git` file |
| `deps` | `node_modules`, `vendor`, `.venv`, `venv` |
| `build` | `target`, `dist`, `build`, `out` |
| `cache` | `.cache`, `__pycache__`, `.pytest_cache` |

`exclude` uses the gitignore syntax: a pattern without `/` matches a name at
any depth, a leading `/` anchors it at `path`, a trailing `/` matches
directories only, and the last matching pattern decides, where `!pattern`
restores. With `gitignore: true`, `.ignore` files apply in every directory,
and `.gitignore`, `.git/info/exclude` and the global exclude file apply in a
git repository; the files of the ancestors of `path` apply as well, and a
deeper file takes precedence.

`file_type` selects the files whose name matches a pattern of one of the
types. The types are those of `search` and carry the names of
`rg --type-list`. A pattern matches the whole name, `*` stands for any
sequence of characters, and letter case is significant: `*.c` does not match
`main.C`. Directories and links that are not followed are never selected by
`file_type`.

| Type | Name patterns |
|---|---|
| `asm` | `*.asm`, `*.s`, `*.S` |
| `c` | `*.c`, `*.h`, `*.H`, `*.c.in`, `*.h.in`, `*.cats` |
| `clojure` | `*.clj`, `*.cljc`, `*.cljs`, `*.cljx`, `*.edn` |
| `cmake` | `*.cmake`, `CMakeLists.txt` |
| `config` | `*.cfg`, `*.conf`, `*.config`, `*.ini` |
| `cpp` | `*.C`, `*.H`, `*.cc`, `*.cpp`, `*.cxx`, `*.c++`, `*.h`, `*.hh`, `*.hpp`, `*.hxx`, `*.h++`, `*.inl` |
| `cs` | `*.cs`, `*.csx` |
| `css` | `*.css`, `*.scss` |
| `csv` | `*.csv` |
| `dart` | `*.dart` |
| `docker` | `*Dockerfile*`, `*dockerfile*` |
| `elixir` | `*.ex`, `*.eex`, `*.exs`, `*.heex`, `*.leex`, `*.livemd` |
| `erlang` | `*.erl`, `*.hrl` |
| `go` | `*.go` |
| `gradle` | `*.gradle`, `*.gradle.kts` |
| `groovy` | `*.groovy`, `*.gradle` |
| `haskell` | `*.hs`, `*.lhs`, `*.cpphs`, `*.c2hs`, `*.hsc` |
| `html` | `*.htm`, `*.html`, `*.ejs` |
| `java` | `*.java`, `*.jsp`, `*.jspx`, `*.properties` |
| `js` | `*.js`, `*.jsx`, `*.vue`, `*.cjs`, `*.mjs` |
| `json` | `*.json`, `*.jsonc`, `composer.lock`, `*.sarif` |
| `jsonl` | `*.jsonl`, `*.ndjson` |
| `julia` | `*.jl` |
| `kotlin` | `*.kt`, `*.kts` |
| `less` | `*.less` |
| `lua` | `*.lua` |
| `make` | `GNUmakefile`, `Gnumakefile`, `gnumakefile`, `Makefile`, `makefile`, `Makefile.am`, `makefile.am`, `Makefile.in`, `makefile.in`, `*.mk`, `*.mak` |
| `markdown`, `md` | `*.markdown`, `*.md`, `*.mdown`, `*.mdwn`, `*.mkd`, `*.mkdn`, `*.mdx` |
| `nix` | `*.nix` |
| `ocaml` | `*.ml`, `*.mli`, `*.mll`, `*.mly` |
| `perl` | `*.perl`, `*.pl`, `*.PL`, `*.plh`, `*.plx`, `*.pm`, `*.t` |
| `php` | `*.php`, `*.php3`, `*.php4`, `*.php5`, `*.php7`, `*.php8`, `*.pht`, `*.phtml` |
| `protobuf` | `*.proto` |
| `ps` | `*.cdxml`, `*.ps1`, `*.ps1xml`, `*.psd1`, `*.psm1` |
| `py` | `*.py`, `*.pyi`, `*.pyw` |
| `r` | `*.R`, `*.r`, `*.Rmd`, `*.Rnw` |
| `rst` | `*.rst` |
| `ruby` | `Gemfile`, `*.gemspec`, `.irbrc`, `Rakefile`, `*.rake`, `*.rb`, `config.ru` |
| `rust` | `*.rs` |
| `sass` | `*.sass`, `*.scss` |
| `scala` | `*.scala`, `*.sbt` |
| `sh` | `*.bash`, `.bashrc`, `.bash_profile`, `.bash_login`, `.bash_logout`, `*.sh`, `.profile`, `*.zsh`, `.zshrc`, `.zshenv`, `.zprofile`, `*.ksh`, `*.csh`, `*.tcsh` |
| `sql` | `*.sql`, `*.psql` |
| `svelte` | `*.svelte` |
| `svg` | `*.svg` |
| `swift` | `*.swift` |
| `tex` | `*.tex`, `*.ltx`, `*.cls`, `*.sty`, `*.bib`, `*.dtx`, `*.ins` |
| `tf` | `*.tf`, `*.tfvars`, `*.tf.json` |
| `toml` | `*.toml`, `Cargo.lock` |
| `ts` | `*.ts`, `*.tsx`, `*.cts`, `*.mts` |
| `tsv` | `*.tsv` |
| `txt` | `*.txt` |
| `vue` | `*.vue` |
| `xml` | `*.xml`, `*.xml.dist`, `*.dtd`, `*.xsl`, `*.xslt`, `*.xsd`, `*.xjb`, `*.rng`, `*.sch`, `*.xhtml` |
| `yaml` | `*.yaml`, `*.yml` |
| `zig` | `*.zig` |

`fuzzy` keeps the entries whose name contains the characters of the text in
the same order and sets `rank`; a higher rank denotes a closer match.
Matches at the start of the name, after `_`, `-`, `.`, `/` or a space, at a
change from lower to upper case and in consecutive characters rank higher.
The text matches without regard to letter case unless it contains an upper
case letter. `sort: rank` puts the closest matches first and needs `fuzzy`.

With `follow_links: true`, links to directories are entered and `type`
classifies a link by its target. A link that leads back to a directory
being walked is a record but is not entered; the notable entry
`link_loop_skipped` gives the number of such links.

Records carry `depth` (0 for the root) and `relative` (`.` for the root).
The orders of `view` are those of `list`: `flat` sorts all records by `sort`
with ties by path, `tree` and `sizes` are depth first with the entries of one
directory sorted by `sort`. `sort: path` compares paths component by
component, so a directory comes right before its contents. With
`view: sizes`, every directory record carries `total_size` and `file_count`
for all files below it that pass the exclusions, whether or not they match
`glob`, `type` or `depth`.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | required | Directory to search. |
| `glob` | `glob \| list<glob>` | `"**"` | Name patterns relative to the directory. |
| `type` | `enum(file\|directory\|link\|any)` | `file` | Kind of entries to find. |
| `depth` | `int` | no limit | Maximum number of directory levels to descend. |
| `hidden` | `bool` | `true` | Include hidden entries. |
| `follow_links` | `bool` | `false` | Descend into linked directories. |
| `gitignore` | `bool` | `false` | Apply the rules of .gitignore files. |
| `exclude` | `list<glob>` | `[]` | Patterns to exclude, in gitignore syntax; a leading ! restores a match. |
| `exclude_preset` | `list<enum(vcs\|deps\|build\|cache)>` | `[]` | Predefined exclusions: version control, dependencies, build output, caches. |
| `file_type` | `list<text>` | `[]` | File types by extension, such as go or ts. |
| `fuzzy` | `text` | none | Rank entries by similarity of their name to this text. |
| `case_sensitive` | `bool` | `true` | Match name patterns with case sensitivity. |
| `sort` | `enum(path\|modified\|size\|rank)` | `path` | Sort key; rank orders by fuzzy similarity. |
| `order` | `enum(asc\|desc)` | `asc` | Sort order. |
| `view` | `enum(flat\|tree\|sizes)` | `flat` | Flat list, tree, or tree with directory size totals. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `entry` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

Notable entries: `permission_denied_skipped`, `source_unavailable`,
`nfd_name`, `portability` and `case_alias` as in `list`, and
`link_loop_skipped` for link loops. Each entry gives the number of entries
concerned and a sample of up to five paths.

## Examples

Find the Rust sources of a project at any depth:

```nurt example
find "/home/user/app" glob: "**/*.rs"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/Cargo.toml", "type": "file", "content_utf8": "\n" },
  { "path": "/home/user/app/src/main.rs", "type": "file", "content_utf8": "\n" },
  { "path": "/home/user/app/src/parser/lexer.rs", "type": "file", "content_utf8": "\n" },
  { "path": "/home/user/app/web/fooBar.ts", "type": "file", "content_utf8": "\n" },
  { "path": "/home/user/app/web/fabric.ts", "type": "file", "content_utf8": "\n" },
  { "path": "/home/user/app/web/node_modules/left-pad/index.ts", "type": "file", "content_utf8": "\n" }
] }
```

Find the TypeScript files outside version control and dependency
directories:

```nurt example
find "/home/user/app" exclude_preset: [vcs, deps] file_type: ["ts"]
```

Rank the files by the similarity of their names to `fb`:

```nurt example
find "/home/user/app" fuzzy: "fb" sort: rank
```

Produce the record of one path, as the source of a changing command:

```nurt example
find "/home/user/app/Cargo.toml" depth: 0 type: any
```

Find a file by its composed name on a volume that stores the name
decomposed; the record carries the NFC `name` and the stored bytes in
`raw_name`:

```nurt example
find "/home/user/notes" glob: "**/café.md"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/notes/2026/cafe\u0301.md", "type": "file", "content_utf8": "\n" },
  { "path": "/home/user/notes/2026/cafe.md", "type": "file", "content_utf8": "\n" }
] }
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_NOT_FOUND` | `path` does not exist | Check the path. |
| `E_PERMISSION` | `path` cannot be read | Search a directory the account may read. |
| `E_PATH_RELATIVE` | `path` is relative and no base directory is set | Give an absolute path or set the `base` option. |
| `E_INVALID_VALUE` | a pattern of `glob` or `exclude` is not valid, or `depth` is negative | Correct the pattern shown in `found`. |
| `E_UNKNOWN_VALUE` | an unknown file type, preset or enumeration value | Use one of the values listed in `expected`. |
| `E_MISSING_PARAM` | `sort: rank` without `fuzzy` | Give `fuzzy`, or sort by another key. |
| `E_LIMIT` | the result exceeds the record limit | Narrow it with `glob`, `depth` or `exclude`. |

## Related

`list` shows the entries of one directory; `search` finds text in files;
`info` describes one entry.
