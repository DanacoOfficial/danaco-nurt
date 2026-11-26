<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Effect report and request envelope

| Format | Schema | Rust types |
|---|---|---|
| `nurt-report/1`, the effect report of one program | `schemas/nurt-report-1.schema.json` | `nurt_contracts::report` |
| `nurt-request/1`, the request envelope | `schemas/nurt-request-1.schema.json` | `nurt_contracts::envelope::Request` |
| `nurt-response/1`, the response to a request | `$defs/response` of the request schema | `nurt_contracts::envelope::Response` |

All three are JSON documents in UTF-8; CBOR is an equivalent encoding. Each
carries a version `"nurt": "MAJOR.MINOR"`, currently `"1.0"`. Within major
version 1 the formats only gain optional fields and new values of the code
enumerations; a reader ignores report fields it does not know. The product
name never appears in the output of a program.

## 1. Principles of the report

1. The report shows the effect, not the fact that something ran: targets,
   changed and deleted files with their number and total size, sizes and
   hashes before and after, a sample of differences, a summary of bulk
   operations, and unusual effects.
2. The report carries the operation number `op` and nothing else about the
   journal. No field, flag or sentence of the report states whether, how or
   until when an operation can be undone; there is no journal state and no
   expiry. The state of journal entries is shown only by an explicit
   `history` query. The report of `undo` and `redo` is an ordinary effect
   report.
3. `irreversible` is present only when an effect is irreversible. Its absence
   carries no information about undo.
4. `notable` describes unusual effects. It never blocks anything; policy
   belongs to the caller.
5. Lists are omitted when empty and optional fields when absent, so that the
   report of an ordinary effect stays short.

## 2. Report fields

| Field | Type | Meaning |
|---|---|---|
| `nurt` | text, required | format version `1.<minor>` |
| `status` | code, required | outcome, section 3 |
| `kind` | text, required | the changing command, or the source command of a reading program (`replace`, `remove`, `run`, `find`); `request` for envelope errors detected before any program is known |
| `op` | integer from 1 | journal entry of the operation; absent for reading programs, previews and changing programs without targets |
| `ran` | text | canonical text of the program that ran; present only when corrections were applied |
| `ast_hash` | `sha256:<64 hex>` | hash of the syntax tree, see `syntax-tree.md` |
| `result_file` | path | file holding the full result (UTF-8) when the result was written to a file (`save_to` or `output.results_dir`) |
| `corrections` | list | corrections applied to the program text: `id`, `rule` (`L`, `T`, `V`, `B` or `P` and two digits), `class` (`C`, `S`, `F`, `P`), `at` (`line`, `column`, `length`), `from`, `to`, `kind`, `alternatives` |
| `targets` | object | `count` (required), `files`, `directories`, `bytes`, `listed` (number of entries of `changes` on this page) |
| `summary` | object | `files_created`, `files_modified`, `files_deleted`, `files_renamed`, `directories_created`, `directories_deleted`, `replacements`, `lines_added`, `lines_removed`, `bytes_before`, `bytes_after`, `bytes_freed` |
| `changes` | list | changed targets on this page, section 5 |
| `diff` | list | hunks of unified differences: `path`, `old_start`, `old_len`, `new_start`, `new_len`, `lines` (prefix `+`, `-` or space), `header` |
| `bulk` | object | summary of a bulk operation: `by_directory`, `by_extension` (largest first, with `count` and `bytes`), `sample_head`, `sample_tail` |
| `exec` | record `exec` | a program that ran |
| `job` | record `job` | a program that continues in the background; present when the status is `running` |
| `test` | record `test_run` | a test run |
| `analysis` | record `analysis` | an analysis verdict |
| `git` | record `git_effect` | the effect on a repository |
| `records` | object | result of a reading program: `type` (record type), `matched`, `header` (header record of a stream), `rows` |
| `notable` | list | unusual effects in descending order of severity, section 6 |
| `irreversible` | non-empty list | irreversible effects, section 7 |
| `error` | object | error of the language, section 8 |
| `next_cursor` | text | cursor of the next page; absent on the last page |
| `timing` | object | `parse_ms`, `read_ms`, `plan_ms`, `apply_ms` (verbosity `detailed`) |

Records (`exec`, `job`, `test`, `analysis`, `git`, `records.header` and
`records.rows`) appear in their JSON form defined by the value and record
formats; the report schema does not restrict their fields. Domain results that
do not fit the common fields are carried by a record in a named optional
field; a new domain adds a new optional field of the same kind.

Fields marked `x-nurt-volatile` in the schema (`op`, `result_file`,
`next_cursor`, `timing`) differ between runs of the same program and are
ignored when reports are compared.

## 3. Status

| Status | Meaning | Rules |
|---|---|---|
| `ok` | the program finished; a changing program applied its effect | may carry `error` for a check that failed after the effect was applied; the effect remains |
| `planned` | preview (`preview: true`): targets and effect computed, nothing changed | no `op` |
| `running` | a program started in the background is still running | `job` required |
| `partial` | the effect was applied to part of the targets, for example the undo of a bulk entry whose preserved content was partly released to free disk space | `op` required; `irreversible` or `error` lists the rest |
| `rolled_back` | the effect was applied and reverted because a later check failed | `error` required (`E_ROLLED_BACK` with `cause`) |
| `failed` | the program stopped with an error | `error` required |

A program stopped by its time limit, by a resource limit or by cancellation
ends with `failed` and the error `E_TIMEOUT`, `E_LIMIT` or `E_CANCELLED`.

## 4. Verbosity

| Level | Content |
|---|---|
| `quiet` | `status`, `op`, `targets`, `summary`, `notable`, `irreversible`; no changes and no differences |
| `normal` (default) | `summary`, changes up to `limit` (default 50), a sample of differences up to `report.diff_max_hunks` (20) hunks and `report.diff_max_lines` (200) lines, `bulk` for bulk operations |
| `detailed` | all changes and differences, paged through the cursor, and `timing` |

An operation is a bulk operation from `report.mass_threshold` (200) targets.
At level `normal` it shows the totals, the ten largest directories and
extensions, five targets from the start and two from the end, and `notable`.
The full list of targets is always available through the cursor; nothing is
hidden.

## 5. Changes

| `action` | Meaning |
|---|---|
| `create` | target created |
| `modify` | content changed |
| `delete` | target deleted |
| `rename` | target renamed or moved; `from` is required |
| `attributes` | mode, ownership or access control changed |
| `link` | link created or changed |
| `truncate` | target truncated |
| `state` | state of a target outside the file system changed; `path` is `<provider>:<name>` (for example `service:nginx`), `before` and `after` hold the states |

`flags` carries additional markers of a change, for example `crlf` or
`keys_created`.

## 6. Unusual effects (`notable`)

Each entry is `{code, severity, path?, count?, detail?}`; entries are sorted
by severity, `alert` first. Severities in increasing order: `info`, `notice`,
`warning`, `alert`. The table gives the default severity; thresholds are
configured under `report.notable.*`. Codes with source "report" are derived by
the report builder from the changes; all other codes are emitted by the
command that observed the condition.

| Code | Default | Source | Condition |
|---|---|---|---|
| `outside_project` | alert | report | target outside every directory of `scope.project_roots` (when set) |
| `system_path` | alert | report | target in a system directory |
| `home_config` | warning | report | configuration in the home directory (`~/.ssh`, `~/.config`, shell profiles) |
| `mass_delete` | warning, alert at ten times the threshold | report | deleted files from `mass_files` (100), bytes from `mass_bytes` (1 GiB) or share from `mass_ratio` (80%) |
| `delete_tree` | notice | report | recursive deletion of a non-empty directory |
| `overwrote_existing` | warning | report | existing files replaced by a write, move or copy |
| `file_emptied` | warning | report | a file with content became empty |
| `size_shrunk` | warning | report | size shrank by `shrink_ratio` (50%, files from 1 KiB) or more |
| `many_per_file` | notice | report | `per_file_repl` (100) or more replacements in one file |
| `target_count_high` | notice | report | `targets_high` (500) or more targets |
| `nothing_matched` | notice | report | a changing stage received no targets |
| `unchanged` | info | command | a write produced identical content |
| `binary_target` | warning | command | a binary file was changed |
| `binary_skipped` | notice | command | binary files were skipped (`count`) |
| `special_file` | warning | command | a device, pipe, socket or pseudo file was written |
| `symlink_followed` | warning | command | a change went through a symbolic link to a target outside its directory |
| `hardlink_broken` | notice | command | a file with several hard links was replaced, separating the links |
| `hardlink_set_preserved` | info | command | a file with several hard links was written in place, keeping the links |
| `attributes_changed` | notice | command | mode, ownership or access control changed |
| `encoding_preserved`, `line_ending_preserved` | info | command | the file kept an encoding other than UTF-8 or line endings other than `lf` |
| `line_ending_mixed` | notice | command | mixed line endings |
| `encoding_changed`, `line_ending_changed`, `bom_changed` | notice | command | the encoding, the line endings or the byte order mark was converted |
| `truncated_lines` | notice | command | long lines were shortened in the output |
| `regex_backtracking` | warning | command | a regular expression hit the backtracking limit |
| `decompressed` | info | command | compressed content was decompressed transparently |
| `cross_device` | notice | command | a move between volumes, performed as copy and delete |
| `to_is_dir` | notice | command | the destination of a move or copy is an existing directory |
| `case_collision`, `case_alias`, `nfd_name` | notice | command | names differing in letter case collide; a name matched in another letter case or normalization form |
| `portability` | notice | command | a name not portable between systems |
| `external_volume` | notice | command | a target on a network or removable volume |
| `archive_path_escape` | warning | command | archive entries outside the destination were skipped |
| `permission_denied_skipped` | warning | command | files without read permission were skipped |
| `link_loop_skipped` | notice | command | a symbolic link leading back to one of its parent directories was not followed |
| `ignored` | info | command | changes in paths excluded from tracking (`run.track_ignore`) |
| `alias_shared` | warning | command | a node referenced by aliases was changed |
| `keys_created` | info | command | intermediate keys were created |
| `duplicate_key` | warning | command | a repeated key; the last occurrence was changed |
| `semantic_fallback` | notice | command | a code operation fell back to syntax analysis |
| `references_left` | notice | command | textual references to the old name remain |
| `rename_incomplete` | warning | command | a rename did not reach every reference |
| `detection_scope` | info | command | after `run`: changes outside `watch` are not detected |
| `exit_nonzero` | notice | command | a program exited with a code other than zero; `count` holds the code |
| `post_stages_deferred` | info | command | stages after the effect were skipped because the program moved to the background |
| `rollback_partial` | warning | command | a rollback restored only part of the changes |
| `output_binary` | notice | command | a program wrote binary output |
| `spool_limit` | warning | command | program output exceeded the spool limit |
| `stdin_closed_early` | info | command | a program closed its input early |
| `job_lost` | warning | command | a background job lost its supervisor |
| `tool_missing` | notice | command | a requested analysis tool is not installed |
| `tests_none` | notice | command | no tests were found or run |
| `trust_config_ignored` | warning | command | trust settings in a project configuration file were ignored; a project cannot make itself trusted |
| `project_config_ignored` | notice | command | keys of a project configuration file outside the keys allowed for projects were ignored |
| `journal_config_ignored` | notice | command | journal settings of an untrusted project were ignored |
| `journal_oversize` | warning | command | the operation exceeds the journal size limit; shown by the preview before the change |
| `source_unavailable` | warning | command | a data source of the operating system was unavailable; results are incomplete |
| `settings_broadcast` | notice | command | a settings change was broadcast to running programs |
| `hooks_skipped` | notice | command | repository hooks were not run |
| `history_rewritten` | warning | command | repository history was rewritten |
| `force_push` | alert | command | a remote branch was overwritten |
| `detached_head` | notice | command | detached head state |
| `branch_deleted_unmerged` | warning | command | an unmerged branch was deleted |
| `fast_forward` | info | command | a branch was updated by fast-forward |
| `conflicts` | warning | command | the operation stopped with conflicts (`count`) |
| `untracked_removed` | warning | command | untracked files were removed |
| `local_changes_carried` | info | command | uncommitted changes were carried to another branch |
| `filter_not_applied` | warning | command | a content filter of the repository was not applied |
| `credential_helper_ignored`, `ssh_command_ignored`, `fsmonitor_ignored`, `signing_program_ignored` | notice | command | a program named by the repository configuration was not run |
| `submodules_not_updated` | notice | command | submodules were not updated |
| `sparse_checkout`, `shallow_repository` | info | command | the repository uses sparse checkout or shallow history |
| `line_endings_converted` | info | command | line endings converted by repository settings |
| `upstream_set` | info | command | an upstream branch was set |
| `stash_kept` | notice | command | a stash entry was kept after a conflict |
| `signed` | info | command | a commit or tag was signed |
| `amend_published` | alert | command | the amended commit is reachable from a remote-tracking branch |
| `commit_detached_head` | warning | command | a commit was created on a detached `HEAD` |
| `operation_in_progress` | warning | command | a commit was created while a rebase, cherry-pick, revert or bisection is in progress |
| `excluded` | info | command | files matched by `exclude` were skipped (`count`) |
| `review_skipped` | notice | command | files were not reviewed: untracked files over the size limit, binary files or files over the count limit |
| `checkpoint_without_index` | notice | command | a checkpoint was created without the index because the index has conflicts |
| `tls_verification_disabled` | warning | command | verification of TLS certificates is disabled by the repository configuration |
| `http_option_ignored` | notice | command | an unsupported HTTP setting of the repository configuration was ignored |
| `pack_without_deltas` | info | command | a push sent objects without new delta compression |
| `push_status_unknown` | warning | command | the remote did not report the outcome of a reference update |
| `host_key_unverified` | warning | command | the host key of a remote was accepted without verification |
| `match_exec_ignored` | notice | command | a `Match exec` condition of the SSH configuration was not run |
| `credentials_in_url` | warning | command | credentials written in the URL of a remote were used |
| `limit_reached` | info | command | the output stopped at the number of entries given by `limit` (`count` holds the number of entries left out) |
| `interactive_program` | notice | command | a program classified as interactive was run (editor, pager, terminal monitor, interpreter without a script); it can wait for terminal input |
| `opaque_script` | notice | command | a program ran a script given in its arguments (`bash -c`, `python -c`); the effects of the script are not classified |

## 7. Irreversible effects (`irreversible`)

Each entry is `{reason, path?, count?, detail?}`.

| Reason | When |
|---|---|
| `external_effect` | a program with an effect outside the local file system was run (publishing, deployment, remote shells) |
| `remote_ref_updated` | references of a remote repository were updated or fetched |
| `network` | a network request that changes remote state was sent |
| `database` | a database was changed without a preserved copy |
| `process_stopped` | a process or job was stopped |
| `service_changed` | a system service was started, restarted, enabled, disabled or stopped |
| `package_changed` | system packages were installed, removed or upgraded |
| `previous_content_not_kept` | a program changed a file whose previous content was not preserved; `detail` gives the cause, for example the time budget or a file system without block sharing |
| `no_space_for_copy` | no space for a copy of the previous content; the change was made directly |
| `journal_inactive` | the journal is disabled or its space service is not running |
| `network_volume`, `removable_volume` | the target is on such a volume, which has no journal by default |
| `purged` | journal entries were purged |
| `special_file` | a device, pipe, socket or pseudo file was written |
| `too_large_for_journal` | the operation is larger than the limit for one journal entry |
| `released_for_space` | preserved content of part of a bulk operation was released to free disk space |
| `job_output_removed` | the kept output of a background job was removed together with the job |
| `process_state_changed` | the state of a running process was changed: suspended, resumed, signalled or given another priority |
| `journal_entries_expired` | journal entries of earlier operations expired because of this operation, for example when the journal limits were lowered |
| `journal_error` | the journal failed while recording the operation; the change was made without preserved previous content |

## 8. Errors

`error` is `{code, message, at?, location?, found?, problem?, fix?, help?,
expected?, actual?, sample?, candidates?, next_steps?, effect_applied?,
details?, cause?}`. `at` is `{line?, column?, length?, stage?}`: `stage` is the
execution stage as text (`"0"` to `"7"`, `"5a"`), and `line`, `column` and
`length` give the place in the program text when the error has one. The list of
codes is part of the schema (`$defs/error_code`); every code defined by the
error contract is listed there, and a test keeps the two in step. The error
structure itself is specified together with the error codes; the report
schema allows additional error fields.

Errors before any effect leave the world unchanged and come with a ready fix.
A check that fails after the effect was applied, for example `assert
exit_code: 0` after `run`, is reported with status `ok` and the error; the
effect remains.

## 9. Request envelope (`nurt-request/1`)

```json
{
  "nurt": "1.0",
  "program": "find \"/srv/app/src\" glob: \"**/*.go\"",
  "variant": "a",
  "options": { "limit": 20, "verbosity": "normal" }
}
```

Exactly one input is given:

| Input | Meaning |
|---|---|
| `program`, with optional `variant` | program text in notation `a`, `b` or `c`; without `variant` the configured variant applies |
| `ast` | surface syntax tree `nurt-ast/1` |
| `cursor` | cursor of a further page of an earlier result; reading a page never runs a changing program again |
| `accept` | identifier of a candidate from an earlier `E_AMBIGUOUS` error |

Options (`options`) are shared by every command. An absent option takes its
value from the configuration, and without a configured value from the default
below. Options of the call override the configuration.

| Option | Type and default | Meaning |
|---|---|---|
| `preview` | bool, `false` | compute targets and effect only; change nothing |
| `base` | absolute path | directory against which relative paths are resolved; not a current directory |
| `limit` | 1 to 5000, `50` | records or changes per page |
| `fields` | list of text | fields of the result to keep |
| `format` | `text`, `json`, `markdown` or `compact`, `text` | form of the result |
| `verbosity` | `quiet`, `normal` or `detailed`, `normal` | level of detail of the report |
| `diff` | `none`, `stat` or `unified`, `unified` | form of differences |
| `diff_context` | int, `3` | context lines of differences |
| `max_chars` | int, `0` | largest number of characters of one text field; `0` means no limit; shortened fields are marked `truncated: true` |
| `save_to` | absolute path | file that receives the full result; the report gives `result_file` |
| `timeout` | duration, `120s` | time limit of the whole program, for example `250ms`, `30s`, `5m`, `1h30m` or `7d` |
| `session` | text | session identifier recorded in the journal |
| `language` | `en` or `pl`, `en` | language of messages and help; codes, fields and keywords are always English |
| `script` | bool, `false` | several pipelines, each its own operation |

Unknown fields are rejected in the envelope and in `options`: a misspelt
option is never ignored. Paths are absolute in canonical notation: `/srv/app`,
`C:/proj`, `//server/share` or a provider path such as `registry:/HKCU`.

An envelope that is not valid JSON or does not match the schema is a call
error of the interface (`NURT_E_ARG` of the C interface). An envelope that
matches the schema but breaks a rule of the envelope, for example two inputs
at once, is answered with a report of status `failed` and the error `E_ARG`.

## 10. Response (`nurt-response/1`)

```json
{
  "nurt": "1.0",
  "report": { "nurt": "1.0", "status": "ok", "kind": "remove", "op": 21 },
  "rendered": { "format": "text", "text": "op 21  remove  ok\n" }
}
```

The report is always present. `rendered` holds the text in the requested
format for `text`, `markdown` and `compact`; it is absent for `json`, where
the report is the result. A further page exists when `report.next_cursor` is
present. `nurt_run_json`, the `run` and `page` methods of `nurt serve` and the
`nurt` tool of the integrations return this document.

### 10.1 Script with several pipelines

A script (option `script`) whose execution ran more than one operation keeps
the result of every pipeline. The response then holds `reports`, an array
with one report per executed pipeline, in program order:

```json
{
  "nurt": "1.0",
  "report": { "nurt": "1.0", "status": "ok", "kind": "remove", "op": 21 },
  "reports": [
    { "nurt": "1.0", "status": "ok", "kind": "find",
      "records": { "type": "row", "rows": [ { "value": 3 } ] } },
    { "nurt": "1.0", "status": "ok", "kind": "remove", "op": 21,
      "changes": [] }
  ]
}
```

- Each report of `reports` carries the `kind` of its own pipeline (the last
  effect command of the pipeline, else its source command), the status and the
  error of its own operation, and its own changes, notable and irreversible
  effects, `op` and records.
- `report` is the report of the whole program. Its `kind` is the last effect
  command of the program, else the source command of the first pipeline. It
  carries the changes, notable and irreversible effects and the error of every
  operation, but no records, because the records belong to the report of their
  pipeline.
- A script stops at the first operation that ends with an error; `reports`
  then ends with the report of that operation.
- `rendered` holds the text of `report` followed by the text of every report
  of `reports`, each starting on a new line.
- The reports of `reports` are not paged and carry no `next_cursor`; the
  paging and the result file (`save_to`) apply to `report`.
- A program with a single operation has no `reports`; its `report` carries the
  records as before.

## 11. Conformance

The directory `golden/report/` holds reports for every status, for every group
of error codes, for a bulk operation and for reports with unusual and
irreversible effects; `golden/request/` holds envelopes for every input and
`golden/request/response/` holds responses. Every golden file validates
against the published schema and against the schema generated from the Rust
types, and survives JSON and CBOR round trips unchanged. The generated schemas
are kept as approved snapshots in `golden/report/schema/` and
`golden/request/schema/`.
