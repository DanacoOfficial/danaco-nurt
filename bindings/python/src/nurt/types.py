# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           bindings/python/src/nurt/types.py
# Responsibility:   Script types.py.
# Dependencies:     none
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Types of the JSON formats ``nurt-request/1``, ``nurt-response/1`` and ``nurt-report/1``.

The module is produced from ``spec/schemas`` with ``tools/gen_types.py``;
edit the schemas and run the tool instead of editing the module.
"""

from __future__ import annotations

from typing import Any, Dict, List, Literal, Tuple, TypedDict


ReportStatus = Literal["ok", "planned", "running", "partial", "rolled_back", "failed"]


_PositionRequired = TypedDict(
    "_PositionRequired",
    {
        "line": int,
        "column": int,
    },
)
_PositionOptional = TypedDict(
    "_PositionOptional",
    {
        "length": int,
    },
    total=False,
)


class Position(_PositionRequired, _PositionOptional):
    """Object ``/$defs/position`` of ``nurt-report/1``."""


_CorrectionRequired = TypedDict(
    "_CorrectionRequired",
    {
        "id": str,
        "rule": str,
        "class": Literal["C", "S", "F", "P"],
        "from": str,
        "to": str,
    },
)
_CorrectionOptional = TypedDict(
    "_CorrectionOptional",
    {
        "at": Position,
        "kind": str,
        "alternatives": List[str],
    },
    total=False,
)


class Correction(_CorrectionRequired, _CorrectionOptional):
    """Object ``/$defs/correction`` of ``nurt-report/1``."""


_TargetsRequired = TypedDict(
    "_TargetsRequired",
    {
        "count": int,
    },
)
_TargetsOptional = TypedDict(
    "_TargetsOptional",
    {
        "files": int,
        "directories": int,
        "bytes": int,
        "listed": int,
    },
    total=False,
)


class Targets(_TargetsRequired, _TargetsOptional):
    """Object ``/$defs/targets`` of ``nurt-report/1``."""


_SummaryOptional = TypedDict(
    "_SummaryOptional",
    {
        "files_created": int,
        "files_modified": int,
        "files_deleted": int,
        "files_renamed": int,
        "directories_created": int,
        "directories_deleted": int,
        "replacements": int,
        "lines_added": int,
        "lines_removed": int,
        "bytes_before": int,
        "bytes_after": int,
        "bytes_freed": int,
    },
    total=False,
)


class Summary(_SummaryOptional):
    """Object ``/$defs/summary`` of ``nurt-report/1``."""


LineEnding = Literal["lf", "crlf", "cr", "mixed", "none"]


IrreversibleReason = Literal[
    "external_effect",
    "remote_ref_updated",
    "network",
    "database",
    "process_stopped",
    "service_changed",
    "package_changed",
    "previous_content_not_kept",
    "no_space_for_copy",
    "journal_inactive",
    "network_volume",
    "removable_volume",
    "purged",
    "special_file",
    "too_large_for_journal",
    "released_for_space",
    "job_output_removed",
    "process_state_changed",
    "journal_entries_expired",
    "journal_error",
]


_HunkMovedItemRequired = TypedDict(
    "_HunkMovedItemRequired",
    {
        "from_path": str,
        "from_line": int,
        "to_path": str,
        "to_line": int,
        "lines": int,
    },
)


class HunkMovedItem(_HunkMovedItemRequired):
    """Object ``/$defs/hunk/properties/moved/items`` of ``nurt-report/1``."""


_HunkRequired = TypedDict(
    "_HunkRequired",
    {
        "path": str,
        "old_start": int,
        "old_len": int,
        "new_start": int,
        "new_len": int,
        "lines": List[str],
    },
)
_HunkOptional = TypedDict(
    "_HunkOptional",
    {
        "id": str,
        "header": str,
        "old_path": str,
        "status": Literal["modified", "added", "deleted", "renamed", "binary", "copied"],
        "binary": bool,
        "old_mode": str,
        "new_mode": str,
        "ignore_whitespace": bool,
        "fingerprint": str,
        "staged": bool,
        "untracked": bool,
        "unit": Literal["lines", "words"],
        "moved": List[HunkMovedItem],
    },
    total=False,
)


class Hunk(_HunkRequired, _HunkOptional):
    """Object ``/$defs/hunk`` of ``nurt-report/1``."""


_ChangeRequired = TypedDict(
    "_ChangeRequired",
    {
        "path": str,
        "action": Literal["create", "modify", "delete", "rename", "attributes", "link", "truncate", "state"],
    },
)
_ChangeOptional = TypedDict(
    "_ChangeOptional",
    {
        "from": str,
        "to": str,
        "size_before": int,
        "size_after": int,
        "hash_before": str,
        "hash_after": str,
        "added": int,
        "removed": int,
        "replacements": int,
        "rows_changed": int,
        "flags": List[str],
        "encoding_before": str,
        "encoding_after": str,
        "line_ending_before": LineEnding,
        "line_ending_after": LineEnding,
        "bom_before": bool,
        "bom_after": bool,
        "link_target": str,
        "store": str,
        "irreversible": IrreversibleReason,
        "diff": List[Hunk],
        "before": str,
        "after": str,
    },
    total=False,
)


class Change(_ChangeRequired, _ChangeOptional):
    """Object ``/$defs/change`` of ``nurt-report/1``."""


_BulkByDirectoryItemRequired = TypedDict(
    "_BulkByDirectoryItemRequired",
    {
        "directory": str,
        "count": int,
    },
)
_BulkByDirectoryItemOptional = TypedDict(
    "_BulkByDirectoryItemOptional",
    {
        "bytes": int,
    },
    total=False,
)


class BulkByDirectoryItem(_BulkByDirectoryItemRequired, _BulkByDirectoryItemOptional):
    """Object ``/$defs/bulk/properties/by_directory/items`` of ``nurt-report/1``."""


_BulkByExtensionItemRequired = TypedDict(
    "_BulkByExtensionItemRequired",
    {
        "extension": str,
        "count": int,
    },
)
_BulkByExtensionItemOptional = TypedDict(
    "_BulkByExtensionItemOptional",
    {
        "bytes": int,
    },
    total=False,
)


class BulkByExtensionItem(_BulkByExtensionItemRequired, _BulkByExtensionItemOptional):
    """Object ``/$defs/bulk/properties/by_extension/items`` of ``nurt-report/1``."""


_BulkOptional = TypedDict(
    "_BulkOptional",
    {
        "by_directory": List[BulkByDirectoryItem],
        "by_extension": List[BulkByExtensionItem],
        "sample_head": List[str],
        "sample_tail": List[str],
    },
    total=False,
)


class Bulk(_BulkOptional):
    """Object ``/$defs/bulk`` of ``nurt-report/1``."""


_RecordRequired = TypedDict(
    "_RecordRequired",
    {
        "kind": str,
    },
)
_RecordOptional = TypedDict(
    "_RecordOptional",
    {
        "fields": Dict[str, Dict[str, Any]],
    },
    total=False,
)


class Record(_RecordRequired, _RecordOptional):
    """Record in the form of nurt-value/1: the record kind and the fields in order, each field a
    tagged value {"t": type, "v": payload}.
    """


_RecordsRequired = TypedDict(
    "_RecordsRequired",
    {
        "type": str,
    },
)
_RecordsOptional = TypedDict(
    "_RecordsOptional",
    {
        "matched": int,
        "header": Record,
        "rows": List[Record],
    },
    total=False,
)


class Records(_RecordsRequired, _RecordsOptional):
    """Object ``/$defs/records`` of ``nurt-report/1``."""


NotableCode = Literal[
    "outside_project",
    "system_path",
    "home_config",
    "mass_delete",
    "delete_tree",
    "overwrote_existing",
    "file_emptied",
    "size_shrunk",
    "many_per_file",
    "target_count_high",
    "nothing_matched",
    "unchanged",
    "binary_target",
    "binary_skipped",
    "special_file",
    "symlink_followed",
    "hardlink_broken",
    "hardlink_set_preserved",
    "attributes_changed",
    "encoding_preserved",
    "line_ending_preserved",
    "line_ending_mixed",
    "encoding_changed",
    "line_ending_changed",
    "bom_changed",
    "truncated_lines",
    "regex_backtracking",
    "decompressed",
    "cross_device",
    "to_is_dir",
    "case_collision",
    "case_alias",
    "nfd_name",
    "portability",
    "external_volume",
    "archive_path_escape",
    "permission_denied_skipped",
    "ignored",
    "alias_shared",
    "keys_created",
    "duplicate_key",
    "semantic_fallback",
    "references_left",
    "rename_incomplete",
    "detection_scope",
    "exit_nonzero",
    "post_stages_deferred",
    "rollback_partial",
    "output_binary",
    "spool_limit",
    "stdin_closed_early",
    "job_lost",
    "tool_missing",
    "tests_none",
    "trust_config_ignored",
    "project_config_ignored",
    "journal_config_ignored",
    "journal_oversize",
    "source_unavailable",
    "settings_broadcast",
    "hooks_skipped",
    "history_rewritten",
    "force_push",
    "detached_head",
    "branch_deleted_unmerged",
    "fast_forward",
    "conflicts",
    "untracked_removed",
    "local_changes_carried",
    "filter_not_applied",
    "credential_helper_ignored",
    "ssh_command_ignored",
    "fsmonitor_ignored",
    "submodules_not_updated",
    "sparse_checkout",
    "shallow_repository",
    "line_endings_converted",
    "upstream_set",
    "stash_kept",
    "signed",
    "amend_published",
    "commit_detached_head",
    "operation_in_progress",
    "excluded",
    "review_skipped",
    "checkpoint_without_index",
    "link_loop_skipped",
    "tls_verification_disabled",
    "http_option_ignored",
    "pack_without_deltas",
    "push_status_unknown",
    "host_key_unverified",
    "match_exec_ignored",
    "signing_program_ignored",
    "credentials_in_url",
    "limit_reached",
    "interactive_program",
    "opaque_script",
]


Severity = Literal["info", "notice", "warning", "alert"]


_NotableRequired = TypedDict(
    "_NotableRequired",
    {
        "code": NotableCode,
        "severity": Severity,
    },
)
_NotableOptional = TypedDict(
    "_NotableOptional",
    {
        "path": str,
        "count": int,
        "detail": str,
        "continuation": str,
    },
    total=False,
)


class Notable(_NotableRequired, _NotableOptional):
    """Object ``/$defs/notable`` of ``nurt-report/1``."""


_IrreversibleRequired = TypedDict(
    "_IrreversibleRequired",
    {
        "reason": IrreversibleReason,
    },
)
_IrreversibleOptional = TypedDict(
    "_IrreversibleOptional",
    {
        "path": str,
        "count": int,
        "detail": str,
    },
    total=False,
)


class Irreversible(_IrreversibleRequired, _IrreversibleOptional):
    """Object ``/$defs/irreversible`` of ``nurt-report/1``."""


ErrorCode = Literal[
    "E_LEXICAL",
    "E_ESCAPE",
    "E_SYNTAX",
    "E_AMBIGUOUS",
    "E_UNCORRECTABLE",
    "E_INTERNAL_CORRECTION",
    "E_EXPIRED_CANDIDATE",
    "E_AST_VERSION",
    "E_AST_SCHEMA",
    "E_UNKNOWN_COMMAND",
    "E_UNKNOWN_PARAM",
    "E_UNKNOWN_NAME",
    "E_UNKNOWN_FIELD",
    "E_UNKNOWN_VALUE",
    "E_MISSING_PARAM",
    "E_PARAM_CONFLICT",
    "E_TYPE",
    "E_ARITY",
    "E_INVALID_VALUE",
    "E_PATH_RELATIVE",
    "E_PATH_FOREIGN",
    "E_ONE_EFFECT",
    "E_OS_ONLY",
    "E_UNSUPPORTED",
    "E_REGEX",
    "E_KEY_SYNTAX",
    "E_PATTERN_SYNTAX",
    "E_QUERY",
    "E_SQL",
    "E_ARG",
    "E_CONTINUATION_EXPIRED",
    "E_EXTERNAL_MISSING",
    "E_EXTERNAL_FAILED",
    "E_PROGRAM_NOT_FOUND",
    "E_SPAWN",
    "E_ASSERT",
    "E_BINARY",
    "E_DOC_PARSE",
    "E_FORMAT",
    "E_ENCRYPTED",
    "E_UNSUPPORTED_CONSTRUCT",
    "E_KEY_NOT_FOUND",
    "E_KEY_CONFLICT",
    "E_UNSUPPORTED_LANGUAGE",
    "E_SYMBOL_NOT_FOUND",
    "E_SYMBOL_AMBIGUOUS",
    "E_RENAME_CONFLICT",
    "E_NO_MATCH",
    "E_PATCH_REJECTED",
    "E_OUT_OF_RANGE",
    "E_STALE",
    "E_ENCODING",
    "E_IO",
    "E_PERMISSION",
    "E_EXISTS",
    "E_NOT_FOUND",
    "E_IN_USE",
    "E_PRIVILEGE",
    "E_NO_SPACE",
    "E_TIMEOUT",
    "E_CANCELLED",
    "E_LIMIT",
    "E_STAGE_FAILED",
    "E_ROLLED_BACK",
    "E_ROLLBACK_FAILED",
    "E_JOURNAL_NOOP",
    "E_JOURNAL_EXPIRED",
    "E_JOURNAL_CONFLICT",
    "E_JOURNAL_PARTIAL",
    "E_JOURNAL_CORRUPT",
    "E_UNKNOWN_JOB",
    "E_JOB_STATE",
    "E_TEST_NO_FRAMEWORK",
    "E_UNKNOWN_RUN",
    "E_UNKNOWN_ANALYSIS",
    "E_OUTPUT_PARSE",
    "E_NETWORK",
    "E_HASH_MISMATCH",
    "E_SERVICE",
    "E_SERVICE_NOT_RUNNING",
    "E_CATALOG",
    "E_CONFIG",
    "E_GIT_NOT_REPOSITORY",
    "E_GIT_REVISION",
    "E_GIT_REF",
    "E_GIT_DIRTY",
    "E_GIT_CONFLICT",
    "E_GIT_IN_PROGRESS",
    "E_GIT_LOCKED",
    "E_GIT_NON_FAST_FORWARD",
    "E_GIT_NOTHING_TO_COMMIT",
    "E_GIT_HOOK",
    "E_GIT_SIGN",
    "E_GIT_AUTH",
    "E_GIT_HOST_KEY",
    "E_GIT_PUSH_REJECTED",
    "E_GIT_PROTOCOL",
    "E_GIT_UNSUPPORTED",
    "E_GIT_HUNK",
    "E_GIT_CHECKPOINT",
    "E_GIT_WORKTREE",
    "E_GIT_CONFIG",
    "E_INTERNAL",
]


ErrorStage = Literal["0", "1", "2", "3", "4", "5", "5a", "6", "7"]
"""Execution stage in which the error occurred."""


_ErrorAtOptional = TypedDict(
    "_ErrorAtOptional",
    {
        "line": int,
        "column": int,
        "length": int,
        "stage": ErrorStage,
    },
    total=False,
)


class ErrorAt(_ErrorAtOptional):
    """Position of the error: the execution stage and the place in the program text."""


_ErrorLocationRequired = TypedDict(
    "_ErrorLocationRequired",
    {
        "path": str,
    },
)
_ErrorLocationOptional = TypedDict(
    "_ErrorLocationOptional",
    {
        "line": int,
        "column": int,
        "end_line": int,
        "end_column": int,
        "byte": int,
    },
    total=False,
)


class ErrorLocation(_ErrorLocationRequired, _ErrorLocationOptional):
    """Object ``/$defs/error/properties/location`` of ``nurt-report/1``."""


_ErrorCandidatesItemRequired = TypedDict(
    "_ErrorCandidatesItemRequired",
    {
        "rank": int,
        "program": str,
    },
)
_ErrorCandidatesItemOptional = TypedDict(
    "_ErrorCandidatesItemOptional",
    {
        "ref": str,
        "why": str,
    },
    total=False,
)


class ErrorCandidatesItem(_ErrorCandidatesItemRequired, _ErrorCandidatesItemOptional):
    """Object ``/$defs/error/properties/candidates/items`` of ``nurt-report/1``."""


_ErrorRequired = TypedDict(
    "_ErrorRequired",
    {
        "code": ErrorCode,
        "message": str,
    },
)
_ErrorOptional = TypedDict(
    "_ErrorOptional",
    {
        "at": ErrorAt,
        "location": ErrorLocation,
        "found": str,
        "problem": str,
        "fix": List[str],
        "help": str,
        "expected": Any,
        "actual": Any,
        "sample": List[str],
        "candidates": List[ErrorCandidatesItem],
        "next_steps": List[str],
        "effect_applied": bool,
        "details": Dict[str, Any],
        "cause": "Error",
    },
    total=False,
)


class Error(_ErrorRequired, _ErrorOptional):
    """Error of the language, as defined by the error contract."""


_ContinuationRequired = TypedDict(
    "_ContinuationRequired",
    {
        "id": str,
        "unit": Literal["lines", "records", "bytes"],
        "shown": int,
        "remaining": int,
    },
)
_ContinuationOptional = TypedDict(
    "_ContinuationOptional",
    {
        "cut_lines": int,
    },
    total=False,
)


class Continuation(_ContinuationRequired, _ContinuationOptional):
    """What a shortened response leaves out; `next` reads the next part."""


_TimingOptional = TypedDict(
    "_TimingOptional",
    {
        "parse_ms": int,
        "read_ms": int,
        "plan_ms": int,
        "apply_ms": int,
    },
    total=False,
)


class Timing(_TimingOptional):
    """Object ``/$defs/timing`` of ``nurt-report/1``."""


_PartRequired = TypedDict(
    "_PartRequired",
    {
        "at": str,
        "kind": Literal["branch", "iteration", "step", "loop"],
        "ok": bool,
    },
)
_PartOptional = TypedDict(
    "_PartOptional",
    {
        "condition": bool,
        "branch": int,
        "command": str,
        "targets": int,
        "rolled_back": bool,
        "irreversible": bool,
        "error": str,
        "iterations": int,
        "op": int,
        "note": str,
    },
    total=False,
)


class Part(_PartRequired, _PartOptional):
    """Object ``/$defs/part`` of ``nurt-report/1``."""


_ReportRequired = TypedDict(
    "_ReportRequired",
    {
        "nurt": str,
        "status": ReportStatus,
        "kind": str,
    },
)
_ReportOptional = TypedDict(
    "_ReportOptional",
    {
        "op": int,
        "elevated": Any,
        "backend": Literal["self", "broker", "sudo", "doas", "run0"],
        "as_user": str,
        "elevation_reason": Literal["permission_denied", "elevated_entry"],
        "ran": str,
        "ast_hash": str,
        "result_file": str,
        "corrections": List[Correction],
        "targets": Targets,
        "summary": Summary,
        "changes": List[Change],
        "diff": List[Hunk],
        "bulk": Bulk,
        "exec": Record,
        "job": Record,
        "test": Record,
        "analysis": Record,
        "git": Record,
        "records": Records,
        "notable": List[Notable],
        "irreversible": List[Irreversible],
        "error": Error,
        "next_cursor": str,
        "continuation": Continuation,
        "timing": Timing,
        "parts": List[Part],
        "parts_total": int,
    },
    total=False,
)


class Report(_ReportRequired, _ReportOptional):
    """Effect report of one Danaco Nurt program. The report describes the effect, never the
    possibility or the time window of undo; `irreversible` is present only when an effect is
    irreversible.
    """


Format = Literal["text", "json", "markdown", "compact"]


_RequestOptionsOptional = TypedDict(
    "_RequestOptionsOptional",
    {
        "preview": bool,
        "base": str,
        "limit": int,
        "fields": List[str],
        "format": Format,
        "verbosity": Literal["quiet", "normal", "detailed"],
        "diff": Literal["none", "stat", "unified"],
        "diff_context": int,
        "max_chars": int,
        "save_to": str,
        "timeout": str,
        "session": str,
        "language": Literal["en", "pl"],
        "script": bool,
        "privilege": Literal["account", "elevated", "auto"],
        "as_user": str,
    },
    total=False,
)


class RequestOptions(_RequestOptionsOptional):
    """Call options shared by every command; an absent option takes its value from the
    configuration or the built-in default.
    """


_RequestRequired = TypedDict(
    "_RequestRequired",
    {
        "nurt": str,
    },
)
_RequestOptional = TypedDict(
    "_RequestOptional",
    {
        "program": str,
        "variant": Literal["a", "b", "c"],
        "ast": Dict[str, Any],
        "cursor": str,
        "accept": str,
        "options": RequestOptions,
        "params": Dict[str, Any],
    },
    total=False,
)


class Request(_RequestRequired, _RequestOptional):
    """Request envelope of Danaco Nurt. Exactly one of program, ast, cursor and accept is given;
    variant applies only to program. The response to a request is described by $defs/response
    (nurt-response/1).
    """


_RenderedRequired = TypedDict(
    "_RenderedRequired",
    {
        "format": Literal["text", "markdown", "compact"],
        "text": str,
    },
)


class Rendered(_RenderedRequired):
    """Object ``/$defs/rendered`` of ``nurt-request/1``."""


_ResponseDataRequired = TypedDict(
    "_ResponseDataRequired",
    {
        "nurt": str,
        "report": Report,
    },
)
_ResponseDataOptional = TypedDict(
    "_ResponseDataOptional",
    {
        "reports": List[Dict[str, Any]],
        "rendered": Rendered,
    },
    total=False,
)


class ResponseData(_ResponseDataRequired, _ResponseDataOptional):
    """Response to a request: the effect report nurt-report/1 and, for text formats, the
    rendered text.
    """


REQUEST_OPTIONS: Tuple[str, ...] = (
    "preview",
    "base",
    "limit",
    "fields",
    "format",
    "verbosity",
    "diff",
    "diff_context",
    "max_chars",
    "save_to",
    "timeout",
    "session",
    "language",
    "script",
    "privilege",
    "as_user",
)
"""Names of the options of ``nurt-request/1``."""

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
