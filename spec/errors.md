<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Error codes

Every error of the language has a stable code written in upper case with the prefix `E_`, a message and, where known, the stage, the position in the program and ready replacements of the program text. Errors of the stages `parse` and `bind` have no effect on the world: they describe a mistake in the program and how to correct it, never a refusal. Codes are only added within a major version of the language; the meaning of a code never changes.

The list of codes below, the enumeration `ErrorCode` of the crate `nurt-contracts` and the definition `ErrorCode` of the schema `nurt-value-1.schema.json` are the same; a test of the crate compares them, and another test checks that every code mentioned in the specification and in the documentation is in the list.

## Error object

The error is the member `error` of the effect report. Members other than `code` and `message` are present only when they have a value.

| Member | Type | Meaning |
|---|---|---|
| `code` | text | Stable code from the table below. |
| `message` | text | Message in English, one or two sentences. |
| `at` | object | Position of the error: `stage`, the stage in which the error occurred (`0` to `7` or `5a`), and the place in the program, `line`, `column` and `length`, counted in Unicode scalar values from 1. Errors raised while a stage runs carry `stage` without a place in the program. |
| `location` | object | Position in a data file: `path`, `line`, `column`, `end_line`, `end_column`, `byte`. |
| `found` | text | The fragment that was found, for example the name of an unsupported construct. |
| `problem` | text | Short name of the specific problem, for example `not_merged`. |
| `fix` | list of text | Ready replacements of the program text, best first. |
| `help` | text | Help topic with the rules that apply. |
| `expected` | any | What was expected: names, types, counts or hashes; up to eight nearest names for unknown names. |
| `actual` | any | What was found instead. |
| `sample` | list of text | Sample of the records or paths concerned. |
| `candidates` | list of objects | Ranked ready programs: `rank`, `ref` (identifier for `accept`), `program`, `why`. |
| `next_steps` | list of text | Further calls that complete the task, for example the parts of a split program. |
| `effect_applied` | bool | The effect of the program is in the world despite the error, as after a failed assertion in stage `5a` or a merge with conflicts; absent when false. |
| `details` | object | Data specific to the code, for example `path`, `os_error`, `holders` of a file in use, `partial_read` of a failed stage, `released` items of a partial journal entry. |
| `cause` | object | Error that caused this one, in the same form. |

## Stages

| Stage | Name | Effect in the world |
|---|---|---|
| `0` | `parse` | none |
| `1` | `bind` | none |
| `2` | `read` | reads only |
| `3` | `check` | none; a failed assertion changes nothing |
| `4` | `plan` | none; a preview ends here |
| `5` | `apply` | the change, recorded in the journal |
| `5a` | `after` | none; stages on the records of the effect, the effect stays |
| `6` | `verify` | a failed verification rolls the effect back |
| `7` | `report` | none |

## Status of the C interface

Functions of the C interface that return a response document (`nurt_run_json`, `nurt_check`, `nurt_parse`, `nurt_correct`, `nurt_exec_wait`, `nurt_page`, `nurt_history`, `nurt_space`) return `NURT_OK` for every language error and carry the error in the document. Functions without a response document, such as `nurt_engine_open`, `nurt_print` and `nurt_ext_load`, return the status in the column `C status` and give the error object through `nurt_last_error`. `NURT_E_ABI` is reserved for a mismatch of interface versions; no language error maps to it. Codes with the status `NURT_OK` occur only in calls that return a response document.

## Codes

| Code | Stages | C status | Meaning |
|---|---|---|---|
| `E_LEXICAL` | 0 | `NURT_E_ARG` | A token is not valid: an unknown character, an unterminated text or a malformed number, size or duration. |
| `E_ESCAPE` | 0 | `NURT_E_ARG` | An escape sequence in a text is not valid; the help shows the accepted escapes and the raw text form. |
| `E_SYNTAX` | 0 | `NURT_E_ARG` | The program does not follow the grammar. |
| `E_AMBIGUOUS` | 0, 1 | `NURT_E_ARG` | A correction is possible but touches a target of a change; `candidates` lists ranked ready programs. |
| `E_UNCORRECTABLE` | 0, 1 | `NURT_E_ARG` | The program cannot be corrected by the correction rules; `fix` and `help` show the way forward. |
| `E_INTERNAL_CORRECTION` | 0, 1 | `NURT_E_INTERNAL` | The correction violated an invariant; nothing is executed. |
| `E_EXPIRED_CANDIDATE` | 0, 1 | `NURT_E_ARG` | `accept` names a correction candidate that expired or is unknown; the program must be sent again. |
| `E_AST_VERSION` | 0 | `NURT_E_ARG` | The version of a syntax tree is newer than the instance supports; the unknown nodes are listed. |
| `E_AST_SCHEMA` | 0 | `NURT_E_ARG` | A syntax tree does not conform to its schema. |
| `E_UNKNOWN_COMMAND` | 1 | `NURT_E_ARG` | Unknown command; `expected` lists up to eight nearest names. |
| `E_UNKNOWN_PARAM` | 1 | `NURT_E_ARG` | Unknown parameter of a command; `expected` lists up to eight nearest names. |
| `E_UNKNOWN_NAME` | 1 | `NURT_E_ARG` | Unknown name `@name` of a `let` result. |
| `E_UNKNOWN_FIELD` | 1, 2 | `NURT_E_ARG` | Unknown field of a record; `expected` lists the fields. |
| `E_UNKNOWN_VALUE` | 1 | `NURT_E_ARG` | Unknown value of an enumeration; `expected` lists the values. |
| `E_MISSING_PARAM` | 1 | `NURT_E_ARG` | A required parameter is missing. |
| `E_PARAM_CONFLICT` | 1 | `NURT_E_ARG` | Parameters that exclude each other were given together; `expected` lists the admissible combinations. |
| `E_TYPE` | 1, 2 | `NURT_E_ARG` | A value or a record stream has a type the place does not accept; `expected` lists the accepted types. |
| `E_ARITY` | 1 | `NURT_E_ARG` | Wrong number of arguments. |
| `E_INVALID_VALUE` | 1 | `NURT_E_ARG` | A value has the right type but is not valid for the place, for example a cron expression, a time zone or a too short interval. |
| `E_CONFIG` | 0, 1 | `NURT_E_ARG` | The configuration is not valid: an unknown key, a value of the wrong type or out of range, or a file that does not parse; `location` points into the file. |
| `E_PATH_RELATIVE` | 1 | `NURT_E_ARG` | A relative path was given without the call parameter `base`. |
| `E_PATH_FOREIGN` | 1 | `NURT_E_ARG` | A path is written in a form foreign to the system, for example a drive letter on Linux or a name Windows does not allow. |
| `E_ONE_EFFECT` | 1 | `NURT_E_ARG` | More than one changing or running stage, or a stage after the effect that is not allowed there; `next_steps` splits the program. |
| `E_OS_ONLY` | 1 | `NURT_E_ARG` | The command exists only on another operating system. |
| `E_UNSUPPORTED` | 1, 5 | `NURT_E_ARG` | A parameter or a mode is not available on this system, for example a memory limit without kernel support. |
| `E_REGEX` | 1 | `NURT_E_ARG` | A regular expression or a replacement template is not valid. |
| `E_KEY_SYNTAX` | 1 | `NURT_E_ARG` | A key path into a document is not valid. |
| `E_PATTERN_SYNTAX` | 1 | `NURT_E_ARG` | A syntax pattern does not parse in the target language. |
| `E_QUERY` | 1, 2, 5 | `NURT_E_ARG` | A document query does not compile or fails to run. |
| `E_SQL` | 1, 2, 5 | `NURT_E_ARG` | An SQL statement cannot be prepared or executed, or changes data with `write: false`. |
| `E_ARG` | 1, 5 | `NURT_E_ARG` | An argument of a call cannot be passed without changing its meaning, for example a NUL character in a process argument. |
| `E_CONTINUATION_EXPIRED` | 2 | `NURT_E_ARG` | A continuation read by `next` expired or was removed by the limits `continuation.max_age`, `continuation.max_count` or `continuation.max_size`, or none is kept; `next_steps` holds the call that produced it. |
| `E_EXTERNAL_MISSING` | 1, 5 | `NURT_OK` | A program required by the command is not installed; `fix` shows how to install it. |
| `E_EXTERNAL_FAILED` | 2, 5 | `NURT_OK` | An external program used by the command, such as a language server, failed or broke its protocol. |
| `E_PROGRAM_NOT_FOUND` | 1, 5 | `NURT_OK` | A program to run was not found in the search path; `candidates` lists similar names. |
| `E_SPAWN` | 5 | `NURT_OK` | A program could not be started: no permission to execute, an unknown format, a missing interpreter or working directory. |
| `E_ASSERT` | 3, 4, 5a | `NURT_OK` | An assertion does not hold. Before the effect nothing changed; after the effect the effect stays and the report is complete. |
| `E_BINARY` | 2, 5 | `NURT_OK` | Binary content where text is required. |
| `E_DOC_PARSE` | 2, 5 | `NURT_E_ARG` | A file is not a valid document of its format; `location` shows where. |
| `E_FORMAT` | 2, 4, 5 | `NURT_OK` | A format is damaged or not supported, for example an archive or a notebook. |
| `E_ENCRYPTED` | 2 | `NURT_OK` | An archive or a document is encrypted. |
| `E_UNSUPPORTED_CONSTRUCT` | 4, 5 | `NURT_OK` | A construct of a document cannot be changed without losing its layout; the construct is named in `found`; nothing changed. |
| `E_KEY_NOT_FOUND` | 2, 5 | `NURT_OK` | A key path does not exist in the document. |
| `E_KEY_CONFLICT` | 5 | `NURT_OK` | A key path passes through a value of another kind, for example an index into a map. |
| `E_UNSUPPORTED_LANGUAGE` | 2, 5 | `NURT_OK` | No grammar is available for the language of a file. |
| `E_SYMBOL_NOT_FOUND` | 2, 5 | `NURT_OK` | A symbol has no definition. |
| `E_SYMBOL_AMBIGUOUS` | 2, 5 | `NURT_OK` | A symbol has several definitions; `candidates` lists ready programs. |
| `E_RENAME_CONFLICT` | 4, 5 | `NURT_OK` | A new name collides with an existing symbol in its scope. |
| `E_NO_MATCH` | 4 | `NURT_OK` | A replacement or an edit found no match while the call required one. |
| `E_PATCH_REJECTED` | 4 | `NURT_OK` | A hunk of a patch does not apply. |
| `E_OUT_OF_RANGE` | 2, 4 | `NURT_OK` | A line number, a page or a position lies outside the content, for example a line after the end of the file. |
| `E_STALE` | 5 | `NURT_OK` | A file changed after it was read; the whole operation was rolled back. |
| `E_ENCODING` | 5 | `NURT_OK` | A character cannot be represented in the encoding of the file. |
| `E_IO` | 2, 5 | `NURT_E_IO` | Input or output error of the file system. |
| `E_PERMISSION` | 2, 5 | `NURT_E_IO` | Access denied by permissions. |
| `E_EXISTS` | 2, 5 | `NURT_E_IO` | The target already exists. |
| `E_NOT_FOUND` | 2, 5 | `NURT_E_IO` | The target does not exist. |
| `E_IN_USE` | 2, 5 | `NURT_E_IO` | The target is in use; the processes holding it are listed. |
| `E_PRIVILEGE` | 2, 5 | `NURT_E_IO` | The operation needs privileges that are not available: no elevation backend ran it, or the system denied the elevation to the account. Details: `backend` (the backends tried and the result of each: `not_installed`, `password_required`, `denied`, `not_administrator`, `cannot_switch_user`, `failed`), `reason` (`disabled_by_configuration` when `privilege.allowed` is `false`); `next_steps` names the way forward. |
| `E_NO_SPACE` | 2, 5 | `NURT_E_IO` | Not enough space on the volume. |
| `E_TIMEOUT` | 2, 5 | `NURT_E_TIMEOUT` | A time limit was reached. |
| `E_CANCELLED` | 2, 5 | `NURT_E_CANCELLED` | The execution was cancelled. |
| `E_LIMIT` | 2, 5 | `NURT_OK` | A resource limit was reached. |
| `E_STAGE_FAILED` | 2, 5, 6 | `NURT_OK` | A stage failed; `partial_read` in `details` may carry what was read. |
| `E_ROLLED_BACK` | 2, 5, 6 | `NURT_OK` | The verification failed and the effect was rolled back. |
| `E_ROLLBACK_FAILED` | 2, 5, 6 | `NURT_OK` | A rollback could not restore everything; the files are listed with instructions. |
| `E_JOURNAL_NOOP` | 4 | `NURT_OK` | No such operation in the journal, or nothing to undo or redo. |
| `E_JOURNAL_EXPIRED` | 4 | `NURT_OK` | An entry of the journal has expired; nothing changed. |
| `E_JOURNAL_CONFLICT` | 4 | `NURT_OK` | The current state differs from the state after the operation; the differences and the choices are listed; nothing changed. |
| `E_JOURNAL_PARTIAL` | 4 | `NURT_OK` | The entry is partial: some items were released; the call asked for the whole entry; nothing changed. |
| `E_JOURNAL_CORRUPT` | 4, 5 | `NURT_E_IO` | An entry of the journal failed its integrity check; its data is kept for repair. |
| `E_UNKNOWN_JOB` | 2 | `NURT_OK` | An unknown background job. |
| `E_JOB_STATE` | 5 | `NURT_OK` | The action is not allowed in the current state of the job, for example resuming a job that is not suspended. |
| `E_TEST_NO_FRAMEWORK` | 2 | `NURT_OK` | The test framework of the project was not recognised. |
| `E_UNKNOWN_RUN` | 2 | `NURT_OK` | An unknown test run. |
| `E_UNKNOWN_ANALYSIS` | 2 | `NURT_OK` | An unknown analysis. |
| `E_OUTPUT_PARSE` | 5 | `NURT_OK` | The output of a test framework or an analyser could not be read; the raw execution result is included. |
| `E_NETWORK` | 2, 5 | `NURT_OK` | A network error: name resolution, connection, TLS, proxy, redirects or protocol. |
| `E_HASH_MISMATCH` | 5 | `NURT_OK` | A downloaded file does not have the expected hash; it was not kept. |
| `E_SERVICE` | 5 | `NURT_OK` | The service manager reported an error. |
| `E_SERVICE_NOT_RUNNING` | 2, 5 | `NURT_OK` | The background service of the instance is not running. |
| `E_CATALOG` | 2 | `NURT_OK` | A program catalog configured explicitly cannot be read. |
| `E_GIT_NOT_REPOSITORY` | 2 | `NURT_OK` | The path is not inside a repository; `candidates` lists nearby repositories. |
| `E_GIT_REVISION` | 2 | `NURT_OK` | A revision is unknown, ambiguous or malformed. |
| `E_GIT_REF` | 2, 5 | `NURT_OK` | A reference name is invalid, exists, is missing, is not merged or is checked out elsewhere. |
| `E_GIT_DIRTY` | 4 | `NURT_OK` | Local changes would be overwritten; nothing changed. |
| `E_GIT_CONFLICT` | 5 | `NURT_OK` | A merge ended with conflicts; the effect is applied and the conflicts are listed. |
| `E_GIT_IN_PROGRESS` | 4 | `NURT_OK` | A merge or rebase is in progress, or none is when one is required. |
| `E_GIT_LOCKED` | 4, 5 | `NURT_OK` | A lock file of the repository exists. |
| `E_GIT_NON_FAST_FORWARD` | 4 | `NURT_OK` | The histories diverged where only a fast-forward is allowed. |
| `E_GIT_NOTHING_TO_COMMIT` | 4 | `NURT_OK` | There is nothing to commit. |
| `E_GIT_HOOK` | 5 | `NURT_OK` | A hook ended with a non-zero exit code; no commit was created. |
| `E_GIT_SIGN` | 5 | `NURT_OK` | A commit or a tag could not be signed. |
| `E_GIT_AUTH` | 5 | `NURT_OK` | Authentication is required or was rejected by the remote. |
| `E_GIT_HOST_KEY` | 5 | `NURT_OK` | The host key of the remote is unknown or changed. |
| `E_GIT_PUSH_REJECTED` | 5 | `NURT_OK` | The remote rejected an update of at least one reference. |
| `E_GIT_PROTOCOL` | 5 | `NURT_OK` | The remote broke the protocol or lacks a required capability. |
| `E_GIT_UNSUPPORTED` | 2, 4 | `NURT_OK` | A feature of the repository is not supported. |
| `E_GIT_HUNK` | 4, 5 | `NURT_OK` | A hunk identifier is malformed or unknown, the hunk changed since it was listed, or the selected lines cannot be applied. |
| `E_GIT_CHECKPOINT` | 4, 6 | `NURT_OK` | A checkpoint has no saved index, or its content does not match after a restore; the paths are listed. |
| `E_GIT_WORKTREE` | 4 | `NURT_OK` | A worktree is unknown, is the main worktree, is locked or contains the working directory. |
| `E_GIT_CONFIG` | 2, 4, 5 | `NURT_OK` | The configuration of the repository is not valid: a value cannot be parsed or a required key is missing. |
| `E_INTERNAL` | 0, 1, 2, 3, 4, 5, 5a, 6, 7 | `NURT_E_INTERNAL` | An internal error; the report is complete up to this point. |

## Errors of the correction

A misspelt name of a command that changes the file system or a remote state, or runs a program (`remvoe "/srv/a.txt"`), is never corrected on its own. The answer is `E_AMBIGUOUS` in stage 1; nothing runs and the first of `candidates` is the program with the name written right, ready to be sent again or accepted with `accept`. A misspelt name of a command that only reads is corrected and the program runs, as are the names written in another letter case or by a synonym.

A text with an escape that the language does not have (`"\d+"`, `"C:\proj\data"`) is read according to its place. In a place of a regular expression it is that expression (`search "\d+" "/srv"` runs as `search /\d+/ "/srv"`), and a text written as a Windows path (`"C:\temp\new"`) in a place of a path is that path with its backslashes as separators. These readings are probable: in a program that reads they are applied and shown, in a program that changes something the answer is `E_AMBIGUOUS` with the corrected program. In every other place an unknown escape is `E_ESCAPE` in stage 0, and `fix` lists the whole program with the text written as raw text (`r"\d"`) and, for a Windows path, with `/`.
