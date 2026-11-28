<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           crates/nurt-git-difftest/README.md -->
<!-- Responsibility:   Documents nurt-git-difftest. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# nurt-git-difftest

Differential tests of the `git.*` commands of Danaco Nurt against the `git`
program. The crate is not published; the `git` program is used by these
tests only.

For every scenario the runner prepares a repository, makes two byte-exact
copies, runs the operation with `git` on copy A and with the Nurt command on
copy B, and compares the normalized state of both copies. The Nurt operation
is then undone through the recording journal and copy B is compared with the
initial state. `git fsck --strict` checks both copies. Writes of the Nurt side
outside its journal transaction fail the scenario; with `transport = "http"`
or `"ssh"` the repository `upstream`, written by the test server, is not
audited.

Program execution of the Nurt side (repository hooks) goes through a process
service that starts the programs with the environment of the scenario; the
`git` side runs the same hooks natively.

## Running

```
cargo test -p nurt-git-difftest
cargo test -p nurt-git-difftest --test scenarios -- merge/ --skip merge/octopus
```

Scenarios are skipped, with a message and without a failure, when `git` is
not on the search path or is older than 2.43.

Arguments of the `scenarios` test after `--`:

| Argument | Effect |
|---|---|
| `<text>` | Runs the scenarios whose identifier contains the text; several may be given. |
| `--exact` | Filters match whole identifiers. |
| `--skip <text>` | Leaves out the scenarios whose identifier contains the text. |
| `--heavy` | Runs the scenarios on heavy corpus repositories (`--ignored` and `--include-ignored` have the same effect). |
| `--require-nurt` | Fails the scenarios whose command has no implementation in the command table. |
| `--keep-failed` | Keeps the case directories of failed scenarios and prints their location. |
| `--test-threads <n>` | Number of scenarios run in parallel; the default is the number of processors. |
| `--list` | Lists the selected scenarios without running them. |

The run ends with one summary line:
`scenarios: <n> total, <n> passed, <n> git side only, <n> skipped, <n> failed, <n> invalid`.
Each failure is listed with the differing keys and the values of both sides.

| Status | Meaning |
|---|---|
| passed | Both sides agree, undo restored the initial state, `fsck` passed. |
| git side only | The `git` side and its checks passed; the command has no implementation in the table. |
| skipped | The platform, the transport or a heavy corpus repository excludes the scenario. |
| failed | At least one check failed. |
| invalid | The scenario file does not follow the format below. |

## Scenario files

A scenario is the file `scenarios/<group>/<name>.toml`; its identifier is
`<group>/<name>`. The groups `common` and `conflicts` hold the scenarios of
the runner; the other groups are named after the command under test.

```toml
id = "merge/conflict-modify-delete"
description = "Merge where topic deletes a.txt and main modifies it."
corpus = "branchy-small"
setup = [
  ["switch", "-q", "topic"],
  ["rm", "-q", "a.txt"],
  ["commit", "-q", "-m", "topic: delete a"],
  ["switch", "-q", "main"],
]
files = { "b.txt" = "local change\n" }
clock = "2026-01-01T12:00:00+01:00"
platforms = ["linux", "macos", "windows"]

[git]
args = ["merge", "--no-edit", "topic"]
expect_exit = 1

[nurt]
command = "git.merge"
params = { branch = "topic" }
expect = "error:E_GIT_CONFLICT"

[compare]
state = ["index", "worktree", "refs", "reflog", "state_files", "objects:HEAD"]
undo = true
ignore = []
```

### Top-level keys

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | text | required | Identifier; equal to the path of the file. |
| `description` | text | required | What the scenario checks; justifies every `ignore` rule. |
| `corpus` | text | required | Corpus repository copied into the case directory (table below). |
| `setup` | list of steps | `[]` | Preparation steps, run in order before the copies are made. |
| `files` | table | `{}` | Writes applied after `setup`, keyed by path relative to the repository root. |
| `clock` | text | `2026-01-01T12:00:00+00:00` | Instant of both sides, RFC 3339 with seconds; the offset is a whole number of hours. |
| `platforms` | list of `linux`, `macos`, `windows` | all | Platforms on which the scenario runs. |
| `transport` | `file`, `http`, `ssh` | `file` | Transport between the repository and its remotes. |

### Setup steps

| Form | Effect |
|---|---|
| `["<arg>", …]` | Runs `git` with the arguments in the repository. |
| `{ git = ["<arg>", …], cwd = "<dir>" }` | Runs `git` in a directory relative to the repository root; `..` leads to the case directory, where helper repositories such as `../upstream` are created. |
| `{ write = "<path>", content = "<text>" }` | Writes a file; `hex = "<bytes>"` writes bytes, `symlink = "<target>"` creates a link, `executable = true` sets the executable bit. |
| `["@write", "<path>", "<text>"]` | Writes a text file. |
| `["@clock", "<date>"]` | Sets the author and committer date of the following `git` steps. |

Paths of setup steps are relative to the repository root and may start with
one `..`. Steps run with the clock of the scenario until an `@clock` step.

### `files`

A value is the text of the file, or a table with exactly one of `content`,
`hex`, `symlink` and `delete = true`, and the optional `executable`. Paths
stay inside the working tree; `.git` and its NTFS and HFS variants are
rejected.

### `[git]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `args` | list of text | | Arguments of one `git` command, without the program name. |
| `steps` | list of lists of text | | Several commands; all but the last must exit with 0. |
| `expect_exit` | int | `0` | Exit code of the last command. |
| `cwd` | text | repository root | Working directory relative to the repository root. |

Exactly one of `args` and `steps` is given.

### `[nurt]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `command` | text | required | Command of the `git` group, for example `git.merge`. |
| `params` | table | `{}` | Parameters, typed by the signature in `catalog/git.toml`; relative paths are resolved against the repository root; `repo` is added when absent. |
| `expect` | `ok` or `error:<CODE>` | `ok` | Expected result, with a stable error code. |
| `record_count` | int | | Exact number of output records. |
| `[[nurt.records]]` | tables | | Expected output records: `kind` names the record kind, the other keys are fields compared in plain JSON form; `$ROOT` in a text is the case directory of the Nurt side. Each table matches a different record. |

Parameters missing from the catalog are bound with the type of their TOML
value and reported in a note.

### `[compare]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `state` | list of selectors | every section except `fsck` | Sections compared between the two copies. |
| `undo` | bool | `true` | Undoes the Nurt operation and compares copy B with the initial state. |
| `ignore` | list of rules | `[]` | Keys left out of the comparison. |
| `strict` | bool | `false` | Compares dates, identities, signatures and raw identifiers of new objects as well. |
| `output` | `"status"` or table | none | Compares the output records of the command with the output of a `git` command (below). |

A selector is a section name, optionally followed by `@<path>` naming
another repository of the case relative to the repository root, for example
`refs@.git/remote.git` or `fsck@../upstream`. A rule is `"<selector>"` for a
whole section or `"<selector> <key>"` for one key; a key ending with `*` is a
prefix.

| Section | Content |
|---|---|
| `index` | Index version; per entry and stage the mode, identifier and stage, with `skip-worktree` and `intent-to-add`; resolve-undo data. File status data is not compared. |
| `worktree` | Every entry of the working tree outside `.git`: kind, executable bit, content (text up to 1024 bytes, otherwise length and SHA-256), link target. |
| `status` | Records of `git status --porcelain=v2 --branch --untracked-files=all --ignored`. |
| `refs` | Every reference with its target, `HEAD` and the `HEAD` of every linked worktree. |
| `reflog` | Every reflog; outside strict mode each entry keeps the old and new identifier and the message. |
| `state_files` | `MERGE_HEAD`, `MERGE_MSG`, `MERGE_MODE`, `MERGE_AUTOSTASH`, `MERGE_RR`, `ORIG_HEAD`, `AUTO_MERGE`, `FETCH_HEAD`, `CHERRY_PICK_HEAD`, `REVERT_HEAD`, `REBASE_HEAD`, `SQUASH_MSG`, `COMMIT_EDITMSG`, `info/sparse-checkout`, `shallow`, the directories `rebase-merge`, `rebase-apply`, `sequencer` and the worktree registrations; outside strict mode without `GIT_AUTHOR_DATE` of `author-script`. |
| `config` | Every key of `config` and `config.worktree`, as parsed by `git config --list`. |
| `objects` | Commits and annotated tags created by the operation, labelled `new#<n>` and `newtag#<n>` in a walk from the references; outside strict mode without dates and signatures. `objects:<rev>` keeps those reachable from a revision. |
| `fsck` | Result of `git fsck --strict`; always checked for the repository of the scenario. |

Values never contain the location of the copy: the case directory is
replaced by `$ROOT` and the address of the case directory on a test server by
`$SERVER`. Identifiers of new objects appear as their labels in every
section.

### `output`

`output` checks the records returned by the Nurt command against the text
output of a `git` command run on copy A after the operation. Both outputs are
reduced to lines, which are compared in order.

`output = "status"` runs `git status --porcelain=v2 -z --untracked-files=all`
and formats the `git_status` records as `XY <path>`, with ` <- <source>` for
renames. The table form selects another command:

| Key | Type | Default | Meaning |
|---|---|---|---|
| `format` | `status` or `lines` | `lines` | Formatting of both sides. |
| `git` | list of text | required for `lines` | Arguments of the `git` command, without the program name. |
| `record` | text | required for `lines` | Template of a line; `{field}` is a field of the plain JSON form of the record, `{{` and `}}` are braces. |
| `kind` | record kind | all kinds (`git_status` for `status`) | Only records of this kind are formatted. |
| `sort` | bool | `false` | Compares the lines as sorted lists. |

```toml
[compare]
output = { format = "lines", git = ["log", "--format=%s"], record = "{subject}", kind = "commit" }
```

A difference is reported with the lines present on one side only, up to 20
lines. `$ROOT` in the output of either side stands for the case directory.

## Transports

The placeholder `$SERVER` in setup steps, `git` arguments and Nurt parameters
is the address of the case directory of the side: a `file://` address, or
`http://127.0.0.1:<port>/<side>` with `transport = "http"`. A repository
`../upstream` is reached as `$SERVER/upstream`. Addresses written during the
preparation are moved to each copy in its configuration and `FETCH_HEAD`
files.

The HTTP test server runs `git http-backend` for every request and accepts
fetches and pushes.

With `transport = "ssh"` the placeholder `$SERVER` is
`ssh://nurt@127.0.0.1:<port>/<side>`. The SSH test server is built on `russh`;
it authenticates the user `nurt` by public key and runs `git-upload-pack`,
`git-receive-pack` and `git-upload-archive` for the case directory, passing
`GIT_PROTOCOL` from the request. The preparation of each side creates the
directory `home/.ssh` with an Ed25519 key pair (`id_ed25519`, `id_ed25519.pub`)
derived from a fixed seed and a `known_hosts` file with the entry
`[127.0.0.1]:<port>` for the host key of the server. The `git` side connects
through `GIT_SSH_COMMAND` with `GIT_SSH_VARIANT=ssh`, a command that uses the
same key and `known_hosts`; the SSH protocol itself is exercised only by the
Nurt side. The server refuses unknown keys and any other command.

## Corpus

Repositories are built by recipes in `src/corpus.rs`, once per run, with a
clock that advances by one minute per commit, so object identifiers are the
same on every machine. A case directory holds the repository `repo` and the
helper repositories of its recipe beside it.

| Repository | Content | Platforms |
|---|---|---|
| `empty` | No commits, branch `main`. | all |
| `linear-small` | Five commits in nested directories, lightweight tag `v1`. | all |
| `branchy-small` | Branches `main`, `topic`, `feature` diverging from `main`, annotated tag `v1.0`. | all |
| `criss-cross` | Branches `left` and `right` merged into each other; two merge bases. | all |
| `renames` | Renamed directory, renamed and edited file, branch `before-move`. | all |
| `binary` | Binary files with every byte value, changed in a second commit. | all |
| `crlf` | `text=auto`, `eol=crlf` and `eol=lf` attributes, files checked out after conversion. | all |
| `symlinks` | Links to a file, to a directory, across directories, a dangling link. | Linux, macOS |
| `exec-bits` | Scripts gaining and losing the executable bit. | Linux, macOS |
| `unicode-names` | NFC and NFD names side by side, spaces, quotes, non-Latin scripts. | Linux |
| `case-names` | Names differing only in letter case. | Linux |
| `packed-refs` | Branches and a peeled annotated tag in `packed-refs`, one loose branch. | all |
| `packed-objects` | Objects in one pack file after `git gc`. | all |
| `ignored-files` | Ignored files and directory, a negated pattern, untracked files. | all |
| `dirty-worktree` | Staged, unstaged, partly staged, deleted and untracked changes. | all |
| `stash-present` | Two stash entries, the newer one with an untracked file. | all |
| `submodule` | Submodule `lib` cloned from the sibling repository `library`. | all |
| `shallow` | Depth 2 fetch from the sibling repository `source`, remote `origin`. | all |
| `sparse` | Cone-mode sparse checkout of `app`. | all |
| `merge-in-progress` | Merge of `theirs` stopped by a conflict in `shared.txt`. | all |
| `many-branches` | Branches `topic/01` to `topic/12`, `topic/01` merged with a merge commit. | all |
| `heavy-history` | 10 000 commits; heavy. | all |
| `heavy-files` | One commit with 100 000 files; heavy. | all |

## Environment of both sides

Both sides run with an empty home directory and global configuration,
without system configuration and attributes, with `LC_ALL=C`, `TZ=UTC`, the
identity `Nurt Test <test@nurt.invalid>` and author and committer dates of
the scenario clock. Only `PATH` (and on Windows the variables the system
needs) is inherited from the test process. The Nurt side receives the same
variables through its host, the clock of the scenario as a fixed clock, and
the offset of the clock as the time zone `Etc/GMT∓<hours>`.

## Checks of the product crates

The test `no_process` fails when the source of `nurt-git`, `nurt-git-ops` or
`nurt-git-transport` outside the hook modules and unit tests names
`std::process::Command`, and when these crates depend on `git2`,
`libgit2-sys`, `openssl-sys`, `curl` or `curl-sys`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
