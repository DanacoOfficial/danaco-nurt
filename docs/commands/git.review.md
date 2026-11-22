---
command: git.review
group: git
summary: Check changes before a commit for conflict markers, whitespace errors, large or binary files, secrets, missing tests and subject format.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.review

## Synopsis

```text
git.review <repo> [name: value]...
```

## Description

Checks changes before a commit and reports one `finding` record for each
problem. The repository is not changed.

Without `base`, `scope` selects the changes:

- `staged`: the index against `HEAD` (against the empty tree before the
  first commit);
- `working`: the working tree against the index, files with conflicts
  against the `ours` stage, and the untracked files that are not ignored;
- `all`: the working tree against `HEAD`, with the untracked files that are
  not ignored.

With `base`, the files of `target` (default `HEAD`) are compared with the
files of `base` and `scope` has no effect. Deleted files, links and
submodules are not checked. At most 300 untracked files are read; an
untracked file larger than 512 KiB is treated as binary without reading its
content.

The checks look at the added lines of the difference with three lines of
context, the difference that `git.diff` shows and whose hunk identifiers
`git.stage` and `git.restore` accept:

| Check | Rule | Severity | Finding |
|---|---|---|---|
| `secrets` | identifier of the rule | of the rule | An added line matches a rule of the review rule service. The value is never reported. |
| `conflict_markers` | `conflict_markers` | error | An added line starts with `<<<<<<< ` or `>>>>>>> `; one finding per line. |
| `whitespace` | `whitespace` | warning | Added lines end with spaces or tabs; one finding per file with the count. Markdown files (`.md`, `.markdown`) are left out of this rule. |
| `whitespace` | `space_before_tab` | warning | The indentation of added lines has a space before a tab; one finding per file. |
| `whitespace` | `blank_at_eof` | warning | The change adds blank lines at the end of the file; the line is the first of them. |
| `debug_statements` | `debug_statements` | warning | An added line of a source file calls `console.log(`, `console.debug(`, `debugger`, `dbg!(`, `pdb.set_trace(` or `breakpoint()`. |
| `final_newline` | `final_newline` | warning | The change leaves the file without a final line feed. |
| `large_files` | `large_files` | warning | The new content is larger than 512 KiB; `value` and `threshold` give the sizes in bytes. |
| `binary` | `binary` | warning | The new content is binary (a NUL byte in the first 8000 bytes, or the `binary` attribute); line checks are not applied. |
| `tests` | `tests_missing`, `tests_unchanged` | warning | A source file changed and no changed test path contains its name; `tests_unchanged` names an existing test. |
| `subject` | see below | see below | The subject line of `message` breaks a rule. |

Source files are files with the extensions `c`, `cc`, `cjs`, `cpp`, `cs`,
`cxx`, `go`, `h`, `hpp`, `java`, `js`, `jsx`, `kt`, `kts`, `mjs`, `php`,
`py`, `rb`, `rs`, `scala`, `swift`, `ts`, `tsx` and `vue`. A test path lies
in a directory `test`, `tests`, `spec`, `specs`, `__tests__` or `testing`, or
its name has the form `test_*`, `*_test.*`, `*.test.*`, `*.spec.*`,
`*_spec.*`, `*Test.*` or `*Tests.*`. The name of a source file is its stem;
for the stems `index`, `main`, `mod`, `lib`, `__init__`, `utils` and `common`
it is the name of the directory.

The rules of `subject` apply to the first non-blank line of `message`:

| Rule | Severity | Applies | Finding |
|---|---|---|---|
| `subject_empty` | error | always | The message has no subject line. |
| `conventional_header` | error | `conventional: true` | The subject is not `type(scope)!: description`. |
| `missing_scope` | warning | `conventional: true` | The header has no scope. |
| `subject_case` | warning | `conventional: true` | The description starts with an upper case letter. |
| `subject_length` | warning | always | The subject has more than `subject_max` characters. |
| `subject_period` | warning | always | The subject ends with a full stop. |
| `subject_mood` | warning | always | The first word of the description is a past or third person form such as `added` or `fixes`. |
| `blank_line` | warning | always | The second line of the message is not blank. |

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `repo` | `path` | required | Repository or path inside it. |
| `scope` | `enum(all\|staged\|working)` | `all` | Staged changes, working tree changes, or both. |
| `paths` | `list<path>` | none | Only changes in these files or directories. |
| `base` | `text` | none | Base revision of a range to review instead of uncommitted changes. |
| `target` | `text` | none | End revision of the range; needs `base`. |
| `message` | `text` | none | Planned commit message to check. |
| `checks` | `list<enum(conflict_markers\|whitespace\|large_files\|binary\|secrets\|subject\|tests\|debug_statements\|final_newline)>` | none | Checks to run: `conflict_markers`, `whitespace`, `large_files`, `binary`, `secrets`, `subject`, `tests`, `debug_statements`, `final_newline`. |
| `subject_max` | `int` | `72` | Longest allowed subject line in characters, at least 20. |
| `conventional` | `bool` | `false` | Require a Conventional Commits subject line. |
| `exclude` | `list<glob>` | `[]` | Paths to leave out; a pattern without `/` matches the file name in any directory. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `finding` |

## Effect

| Property | Value |
|---|---|
| Category | `source` |
| Effect | `read` |
| Reversibility | `read` |
| Implementation | `builtin` |

## Examples

Review the staged changes and the planned message before a commit.

```nurt example
git.review "/home/user/repo" scope: staged message: "feat(parser): add numbers" conventional: true
```

```text expected
finding path: src/parse.rs kind: conflict_markers severity: error rule: conflict_markers line: 42 hunk: "src/parse.rs#2:9c41e0"
finding path: src/parse.rs kind: whitespace severity: warning rule: whitespace line: 17 count: 3
```

Check only the white space of the working tree changes below `src`.

```nurt example
git.review "/home/user/repo" scope: working paths: ["src"] checks: [whitespace, final_newline]
```

Review the commits of a branch against `main`, without generated files.

```nurt example
git.review "/home/user/repo" base: "main" target: "topic" exclude: ["*.lock", "dist/**"]
```

Check a commit message with a shorter subject limit.

```nurt example
git.review "/home/user/repo" checks: [subject] message: "Fixed the parser." subject_max: 50
```

```text expected
finding path: /home/user/repo target: message kind: subject severity: warning rule: subject_period line: 1
finding path: /home/user/repo target: message kind: subject severity: warning rule: subject_mood line: 1
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_MISSING_PARAM` | `target` without `base` (problem `missing_base`). | Give `base` |
| `E_INVALID_VALUE` | Unknown scope or check, or `subject_max` below 20 (problem `out_of_range`). | Correct the value |
| `E_GIT_NOT_REPOSITORY` | `repo` is not in a repository, or a path is outside it. | Give a path inside the repository |
| `E_GIT_REVISION` | `base` or `target` names no commit. | Use a branch, a tag or a commit |
| `E_GIT_UNSUPPORTED` | `scope` `all` or `working` in a bare repository (problem `bare_repository`). | Use `scope: staged` or a range with `base` |
| `E_CANCELLED` | The call was cancelled. | Repeat the call |

## Related

`git.commit`, `git.diff`, `git.stage`, `git.restore`, `git.status`.
