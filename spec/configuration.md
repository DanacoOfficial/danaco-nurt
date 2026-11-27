<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Configuration `nurt-config/1`

Danaco Nurt reads its configuration from TOML files, environment variables
and run-time settings at seven levels of precedence. The machine-readable form
of the format is the JSON Schema `spec/schemas/nurt-config-1.schema.json`,
generated from the Rust type `nurt_contracts::config::Config`. Example files are in `spec/golden/config/`;
`spec/golden/config/defaults.toml` lists every key with its default.

## 1. Files

A configuration file is a TOML document. Tables correspond to the sections
below, and every key is optional: an empty file is the built-in
configuration.

| System | System file | User file | State directory |
|---|---|---|---|
| Linux | `/etc/nurt/nurt.toml` | `$XDG_CONFIG_HOME/nurt/nurt.toml` (by default `~/.config/nurt/nurt.toml`) | `$XDG_STATE_HOME/nurt/` |
| macOS | `/Library/Application Support/Nurt/nurt.toml` | `~/Library/Application Support/Nurt/nurt.toml` | `~/Library/Application Support/Nurt/` |
| Windows | `%PROGRAMDATA%\Nurt\nurt.toml` | `%APPDATA%\Nurt\nurt.toml` | `%LOCALAPPDATA%\Nurt\` |

A project file is named `.nurt.toml` and lies in the project directory. The
variable `NURT_CONFIG` names one more file. An application that embeds the
library may pass a configuration in JSON when it opens the engine
(`nurt_engine_open`); it is applied at the level of the `NURT_CONFIG` file,
after that file.

## 2. Levels and precedence

| Level | Identifier | Source |
|---|---|---|
| 1 | `builtin` | built-in defaults |
| 2 | `system` | system file |
| 3 | `user` | user file |
| 4 | `project` | `.nurt.toml` of the project |
| 5 | `config_file` | file named by `NURT_CONFIG`; configuration of an embedding application |
| 6 | `environment` | `NURT_*` variables, read when the process starts |
| 7 | `runtime` | invocation parameters, request options, `journal.limits` |

A higher level overrides a lower one, key by key:

- tables merge: a key set at a higher level replaces only that key;
- scalars and arrays are replaced as a whole (an array is never appended);
- layers of the same level are applied in the order in which they are read.

The level that set each key is recorded. It is reported, for example, by
`journal.status` for the effective journal limits.

A key that the format does not define is ignored and reported to the
caller; a value that does not match the type of its key is an error that
names the key, the level and the file.

## 3. Project files and trust

A project file comes from a repository and is not trusted. It may set only:

| Keys | Condition |
|---|---|
| `scope.base`, `scope.project_roots` | always |
| `report.*`, `run.*` | always |
| `journal.*` | only for a trusted project |

A project is trusted when its directory is listed in `scope.trusted_projects`.
The list can be set in the system file, the user file, the `NURT_CONFIG` file
(or the configuration of an embedding application) and the variable
`NURT_TRUSTED_PROJECTS`. It cannot be set in a project file or at run time,
so that neither a repository nor a request can grant trust.

Keys that are not applied are reported with these notable codes:

| Key | Level | Notable code |
|---|---|---|
| `scope.trust*` | project, runtime | `trust_config_ignored` |
| `service.*` | project | `trust_config_ignored` |
| `journal.*` | project that is not trusted | `journal_config_ignored` |
| any other key outside the list above | project | `project_config_ignored` |

The rule is available to implementations as the pure function
`nurt_contracts::config::allowed_at(level, key)`.

## 4. Protection floors

`journal.floor.max_ops` and `journal.floor.max_age` are the lowest step and
age limits of the journal. A lower effective value, from any level, is raised
to the floor and the change is noted in `journal.status`; `unlimited` is
above every floor. Values that come from a project file are raised to at
least 10 steps and 5 minutes even when no floor is set. Floors are normally
set by an administrator (system file) or by an embedding application when it
opens the engine; a floor can only raise protection: when several levels set
the same floor, the highest value wins, whatever the precedence of the levels,
so that a user file cannot lower a floor set in the system file.

## 5. Values

| Type | Syntax | Examples |
|---|---|---|
| duration | digits followed by `ms`, `s`, `m`, `h`, `d` or `w`, also combined; no bare numbers | `"1500ms"`, `"30m"`, `"1h30m"` |
| size | number with `b`, `kb`, `mb`, `gb`, `tb` (powers of 1000) or `kib`, `mib`, `gib`, `tib` (powers of 1024); a fraction is allowed; an integer means bytes | `"256mib"`, `"1.5tb"`, `4096` |
| percentage | number with up to two decimal places followed by `%` | `"15%"`, `"12.5%"` |
| step limit | integer `1` to `100000`, or `"unlimited"`; `0` is not accepted | `30`, `"unlimited"` |
| age limit | duration from `10s` to `365d`, or `"unlimited"` | `"30m"` |
| size budget | size of at least `1mib`, percentage of the volume capacity (`1%` to `90%`), percentage of the free space including journal data (`1%free` to `100%free`), or `"unlimited"` | `"20gib"`, `"10%"`, `"50%free"` |
| byte limit | size or `"unlimited"` | `"unlimited"` |
| size or percentage | size, or percentage of the volume capacity | `"5gib"`, `"15%"` |

Units are written in lower case. When the configuration is written back (for
example by `nurt_engine_config_get`), durations and sizes use the largest
unit that represents the value exactly: `"120s"` is written as `"2m"`,
`"24h"` as `"1d"`.

## 6. Environment variables

| Variable | Key | Value |
|---|---|---|
| `NURT_CONFIG` | — | path of an additional configuration file |
| `NURT_SYNTAX` | `syntax.variant` | text |
| `NURT_SYNTAX_TOLERANT` | `syntax.tolerant` | boolean |
| `NURT_CORRECTION` | `correction.enabled` | boolean |
| `NURT_BASE` | `scope.base` | text |
| `NURT_PROJECT_ROOTS` | `scope.project_roots` | list |
| `NURT_TRUSTED_PROJECTS` | `scope.trusted_projects` | list |
| `NURT_JOURNAL` | `journal.enabled` | boolean |
| `NURT_JOURNAL_MAX_OPS` | `journal.max_ops` | text |
| `NURT_JOURNAL_MAX_AGE` | `journal.max_age` | text |
| `NURT_JOURNAL_MAX_SIZE` | `journal.max_size` | text |
| `NURT_JOURNAL_MAX_OP_SIZE` | `journal.max_op_size` | text |
| `NURT_JOURNAL_OVERSIZE` | `journal.oversize` | text |
| `NURT_JOURNAL_OVERWRITE` | `journal.overwrite` | text |
| `NURT_JOURNAL_NETWORK` | `journal.volumes.network` | boolean |
| `NURT_JOURNAL_REMOVABLE` | `journal.volumes.removable` | boolean |
| `NURT_SPACE_RESERVE` | `space.reserve` | text |
| `NURT_SPACE_SOFT` | `space.soft` | text |
| `NURT_SPACE_POLL_MS` | `space.poll_ms` | integer |
| `NURT_RUN_TIMEOUT` | `run.timeout` | text |
| `NURT_RUN_WAIT` | `run.wait` | text |
| `NURT_LANGUAGE` | `output.language` | text |
| `NURT_COLOR` | `output.color` | text |
| `NURT_RESULTS_DIR` | `output.results_dir` | text |
| `NURT_MAX_RECORDS` | `limits.max_records` | integer |
| `NURT_THREADS` | `limits.threads` | integer |
| `NURT_PROGRAMS_CATALOGS` | `programs.catalogs` | list |
| `NURT_HTTP_PROXY` | `http.proxy` | text |
| `NURT_EXT_DIR` | `ext.dir` | text |
| `NURT_STATE_DIR` | `state.dir` | text |
| `NURT_LOG` | `log.level` | text |
| `NURT_SESSION` | `session.id` | text |

Booleans are `1`, `true`, `yes`, `on`, `0`, `false`, `no` or `off` (in any
case). Lists are separated by `:` (`;` on Windows); empty items are skipped.
Text is converted by the type of the key, so `NURT_JOURNAL_MAX_OPS=unlimited`
and `NURT_JOURNAL_MAX_OPS=30` are both valid. The HTTP client also honours
the standard variables `HTTP_PROXY`, `HTTPS_PROXY` and `NO_PROXY`. Other
variables are ignored.

## 7. Keys

"None" in the default column means that the key is not set and the
behaviour in the meaning column applies.

### 7.1 `[syntax]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `syntax.variant` | `a`, `b`, `c` | `a` | notation variant of the text form |
| `syntax.tolerant` | boolean | `true` | also accept the notation of the other variants |

### 7.2 `[correction]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `correction.enabled` | boolean | `true` | correction of the written form of programs |
| `correction.max_read` | integer | `8` | correction budget of programs that only read |
| `correction.max_effect` | integer | `4` | correction budget of programs that change something |
| `correction.require_first_char` | boolean | `true` | fuzzy correction of names keeps the first character |
| `correction.candidates_ttl` | duration | `10m` | lifetime of candidates that can be accepted after an ambiguous program |

### 7.3 `[scope]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `scope.base` | path | none | absolute directory for relative paths |
| `scope.project_roots` | list of paths | `[]` | project directories; changes outside them are reported as `outside_project` |
| `scope.trusted_projects` | list of paths | `[]` | projects whose `.nurt.toml` may change `journal.*` (section 3) |

### 7.4 `[journal]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `journal.enabled` | boolean | `true` | record changes in the journal |
| `journal.max_ops` | step limit | `30` | number of recent changing operations that can be undone |
| `journal.max_age` | age limit | `30m` | how long previous content is kept |
| `journal.max_size` | size budget | `unlimited` | size budget per volume; the space reserve always applies |
| `journal.max_size_min` | size | `2gib` | lower clamp of percentage budgets |
| `journal.max_size_max` | size | `50gib` | upper clamp of percentage budgets |
| `journal.max_size_total` | byte limit | `unlimited` | budget summed over all volumes |
| `journal.max_op_size` | byte limit | `unlimited` | limit of a single operation |
| `journal.oversize` | `skip`, `keep` | `skip` | operation larger than the budget: done without keeping previous content (reported as irreversible), or kept above the budget |
| `journal.age_clock` | `wall`, `active` | `wall` | entries age in wall-clock time, or only while the computer runs |
| `journal.floor.max_ops` | step limit | none | lowest step limit (section 4) |
| `journal.floor.max_age` | age limit | none | lowest age limit (section 4) |
| `journal.pin_max_age` | duration | `24h` | default duration of `journal.pin` |
| `journal.overwrite` | `exchange`, `clone_or_copy` | `exchange` | keeping overwritten files |
| `journal.count_undo_as_op` | boolean | `true` | an undo counts as a step |
| `journal.history_keep` | integer | `1000` | expired entries whose metrics are kept for `history` |
| `journal.dir_name` | text | `.nurt-journal` | name of the journal directory on each volume |
| `journal.volumes.network` | boolean | `false` | keep a journal on network volumes |
| `journal.volumes.removable` | boolean | `false` | keep a journal on removable volumes |
| `journal.volume` | table by mount point | empty | settings of one volume: `enabled`, `reserve`, `soft`, `max_ops`, `max_age`, `max_size`; absent keys are inherited |

### 7.5 `[space]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `space.reserve` | size or percentage | `15%` | free space that the journal never takes |
| `space.reserve_min` | size | `5gib` | lower clamp of a percentage reserve |
| `space.reserve_max` | size | `50gib` | upper clamp of a percentage reserve |
| `space.soft` | size or percentage | none: reserve plus the larger of 5 % and 2 GiB, no more than reserve plus 20 GiB | threshold at which the service starts releasing journal data |
| `space.margin` | size or percentage | none: the smaller of 1 % and 1 GiB | margin above the reserve |
| `space.poll_ms` | integer | `250` | period of free space measurement, in milliseconds |
| `space.poll_fast_ms` | integer | `50` | measurement period near a threshold, in milliseconds |
| `space.heartbeat_ms` | integer | `500` | heartbeat period of the service, in milliseconds |
| `space.heartbeat_max_age_ms` | integer | `2000` | age after which a heartbeat is stale, in milliseconds |
| `space.sync_budget_ms` | integer | `1500` | budget of synchronous release before a write, in milliseconds |
| `space.settle_ms` | integer | `1000` | wait for free space to grow after a release, in milliseconds |
| `space.quota_aware` | boolean | `true` | take quotas of the account into account |

### 7.6 `[run]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `run.timeout` | duration | `120s` | time limit of a program |
| `run.wait` | duration | `3s` | time to wait in the foreground before a program continues in the background |
| `run.idle_timeout` | duration | `60s` | time without output and processor activity after which a program is reported as stalled |
| `run.kill_grace` | duration | `5s` | time between the termination request and killing the process group |
| `run.env_defaults` | table of text | `PYTHONUTF8="1"`, `PYTHONIOENCODING="utf-8"`, `NO_COLOR="1"`, `TERM="dumb"`, `GIT_TERMINAL_PROMPT="0"`, `PAGER="cat"`, `GIT_PAGER="cat"`, `DEBIAN_FRONTEND="noninteractive"` | variables added to the environment of every program |
| `run.track_changes` | `auto`, `scan`, `events`, `none` | `auto` | detection of file changes made by a program |
| `run.track_ignore` | list of names | `[".git"]` | names excluded from change detection |
| `run.preimage` | `auto`, `off` | `auto` | keep previous content of declared and watched files before a program runs |
| `run.preimage_budget` | duration | `1500ms` | time budget of keeping previous content |
| `run.preimage_max_entries` | integer | `20000` | largest number of files kept |
| `run.preimage_copy_max` | size | `256mib` | largest amount copied when cloning is not available |
| `run.preimage_exclude` | list of names | `node_modules`, `.git`, `target`, `build`, `dist`, `.venv`, `__pycache__` | names never kept |
| `run.irreversible_patterns` | list of patterns | `[]` | argument patterns added to the built-in table of programs with effects outside the computer; tokens are separated by spaces, `*` matches any token, `a\|b` matches alternatives; the built-in table cannot be removed |
| `run.parallel` | integer | `1` | default number of batches run at the same time |
| `run.spool_max` | byte limit | `1gib` | largest output kept per program: its first part and, up to a quarter of the limit (at most 64 MiB), its last part; the bytes between them are omitted, marked with a line `[... N bytes omitted ...]` in the spool file and reported with the note `spool_limit`; the counters and the hash cover all output; `unlimited` keeps everything and can be set at any level, including the run-time level for one run |
| `run.supervisor` | `auto`, `none` | `auto` | supervision of programs that continue in the background after the command line exits; `none` allows background programs only inside a long-lived process |
| `run.waits_threshold` | duration | `3s` | waiting time above which a program is listed among the waits of a checked program |

### 7.7 `[output]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `output.language` | `en`, `pl` | `en` | language of messages and help |
| `output.color` | `auto`, `always`, `never` | `auto` | colours for people |
| `output.head` | integer | `20` | first lines of program output shown in a response |
| `output.tail` | integer | `40` | last lines of program output shown in a response |
| `output.max_bytes` | integer | `4096` | bytes of text of one program output stream shown in a response; `0` is no limit |
| `output.spool_ttl` | duration | `1h` | lifetime of files with complete program output |
| `output.results_dir` | path | none: `results` in the state directory | directory of complete results saved to files |

### 7.8 `[report]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `report.mass_threshold` | integer | `200` | targets from which an operation is reported as a bulk operation |
| `report.diff_max_hunks` | integer | `20` | hunks in the sample of differences |
| `report.diff_max_lines` | integer | `200` | lines in the sample of differences |
| `report.notable.mass_files` | integer | `100` | deleted files from which `mass_delete` is reported |
| `report.notable.mass_bytes` | size | `1gib` | deleted bytes from which `mass_delete` is reported |
| `report.notable.mass_ratio` | number | `0.8` | share of a directory deleted from which `mass_delete` is reported |
| `report.notable.shrink_ratio` | number | `0.5` | share of its size by which a file of at least 1 KiB shrinks from which `size_shrunk` is reported |
| `report.notable.per_file_repl` | integer | `100` | replacements in one file from which `many_per_file` is reported |
| `report.notable.targets_high` | integer | `500` | targets from which `target_count_high` is reported |

### 7.9 `[text]`, `[limits]`, `[fs]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `text.legacy_encodings` | list | `windows-1250`, `ibm852`, `iso-8859-2`, `windows-1252` | legacy encodings considered, in order, when a file is not valid UTF-8 |
| `text.new_file_line_ending` | `lf`, `crlf` | `lf` | line ending of new files |
| `limits.max_records` | integer | `5000000` | largest number of records held in memory by one stage; more is an error (`E_LIMIT`), never a silent cut |
| `limits.max_file_bytes_text` | size | `1gib` | largest file read as text |
| `limits.threads` | integer | none: number of processor cores | worker threads |
| `fs.sharing_retry_ms` | integer | `2000` | time of retries after a sharing violation (Windows), in milliseconds |

### 7.9a `[loop]`, `[function]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `loop.max_iterations` | integer | `10000` | largest number of iterations of a `repeat` or `while` without `limit`; reaching it ends `repeat` with `E_LIMIT` and `while` normally, and the report says so |
| `loop.max_time` | duration | `10m` | longest time of a `repeat` or `while` without `limit`, with the same outcome |
| `function.max_depth` | integer | `64` | deepest nesting of calls of functions defined with `define`; deeper recursion ends with `E_LIMIT` |

### 7.10 `[code]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `code.servers` | table by language | `python`: `pyright-langserver --stdio`; `typescript`, `javascript`: `typescript-language-server --stdio`; `go`: `gopls`; `rust`: `rust-analyzer`; `c`, `cpp`: `clangd` | language servers; each entry has `command` (program and arguments, found in `PATH`) and optional `init` (initialization options) |
| `code.servers_allow` | `all`, `off` or list of languages | `all` | servers that may be started |
| `code.server_idle` | duration | `10m` | time after which an unused server stops |
| `code.server_retry` | duration | `5m` | time before a failed server is started again |
| `code.max_servers` | integer | `4` | servers running at the same time |
| `code.timeout` | duration | `20s` | time limit of a server request |
| `code.timeout_max` | duration | `2m` | largest time limit a command may request |
| `code.index_max_files` | integer | `20000` | largest number of files indexed for syntax analysis |
| `code.file_bytes_max` | size | `1mib` | largest file analysed |
| `code.diagnostics_quiet` | duration | `1500ms` | time without new diagnostics after which diagnostics are complete (at least `50ms`) |

### 7.11 `[documents]`, `[sql]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `documents.max_bytes` | size | `256mib` | largest structured document read |
| `documents.yaml_version` | `1.1`, `1.2` | `1.2` | YAML version of documents without a directive |
| `documents.xml_external_entities` | boolean | `false` | resolve external entities of XML documents |
| `sql.busy_timeout` | duration | `5s` | wait for a locked SQLite database |

### 7.12 `[programs]`, `[http]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `programs.catalogs` | list of paths or tables `{ path, priority, enabled }` | `[]` | external catalogs of programs in the format `nurt-program-catalog/1`; without them the commands describe installed programs |
| `programs.index_dir` | path | none: cache directory of the user | directory of the program index |
| `programs.help_timeout` | duration | `3s` | time limit of reading the help of a program on request |
| `programs.sources_disabled` | list of `path`, `package`, `app`, `man`, `catalog` | `[]` | sources of program information that are not used |
| `http.proxy` | URL | none: system setting | proxy of HTTP requests and Git fetches |

### 7.13 `[git]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `git.trash_refs` | boolean | `false` | keep deleted branches under `refs/nurt/trash/` |
| `git.hooks.timeout` | duration | `10m` | time limit of a repository hook |
| `git.ssh.known_hosts` | `strict`, `accept_new` | `strict` | verification of SSH host keys |
| `git.ssh.identity_files` | list of paths | `[]`: standard identity files | private key files |
| `git.signing.key_file` | path | none | OpenPGP secret key file, imported into a temporary GnuPG home directory for signing commits when the repository configuration does not name a key held by `gpg-agent` |
| `git.signing.openpgp_source` | `gpg_agent`, `key_file` | none: gpg-agent, then the key file | the only holder of the secret key used for OpenPGP signatures; GnuPG (`gpg.openpgp.program` or `gpg.program`) makes the signature in both cases |
| `git.checkpoint.keep` | integer | `50` | checkpoints kept per repository |
| `git.status.limit` | integer | `200` | entries listed by `git.status` before paging |
| `git.network.timeout` | duration | `5m` | time limit of a network operation |

The configuration of a Git repository (for example `user.signingKey`,
`commit.gpgSign`, `http.proxy`, `core.*`) applies as in Git; the keys above
cover only what Git does not define.

### 7.14 `[verify]`, `[ext]`, `[service]`, `[state]`, `[log]`, `[session]`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `verify.history_max` | integer | `50` | test runs kept for comparison |
| `ext.dir` | path | none: `ext` in the configuration directory | directory of extensions |
| `ext.native` | boolean | `false` | allow native extension libraries besides WebAssembly |
| `service.mode` | `user`, `system` | `user` | installation mode of the background service; `user` runs for the account (on Linux with lingering enabled), `system` is recommended for servers |
| `state.dir` | path | none: state directory of the operating system | state directory |
| `log.level` | `error`, `warn`, `info`, `debug`, `trace` | `warn` | level of diagnostic logs |
| `session.id` | text | none | session identifier recorded in the journal |
| `session.ttl` | duration | `24h` | time without calls after which session settings (for example limits set by `journal.limits` for a session) expire |

## 8. Interface for implementations

The module `nurt_contracts::config` provides:

| Item | Purpose |
|---|---|
| `Config` and its sections | typed configuration with built-in defaults (`Default`) and TOML or JSON deserialization |
| `ConfigLevel` | the seven levels, in increasing precedence |
| `allowed_at(level, key)` | the rule of section 3 as a pure function |
| `ConfigLayer`, `resolve(layers, project_trusted)` | merging of levels with admission, notes and the level of each key |
| `trusted_projects(layers)` | trusted projects from the levels that may declare trust |
| `env_layer(vars, separator)` | the environment level built from process variables |
| `known_keys()`, `ENV_VARS` | every key and every variable of the format |
| `PROJECT_FLOOR_MAX_OPS`, `PROJECT_FLOOR_MAX_AGE` | floors of values from project files |

The conformance tests in `crates/nurt-contracts/tests/config_*.rs` check the
defaults against `spec/golden/config/defaults.toml`, the generated schema
against the approved file, every golden file against the schema, the
precedence of the seven levels for the three retention limits, the trust
rules and the environment level.
