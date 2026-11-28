// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git::diff
// Responsibility:   Differences in Git format: hunks with stable identifiers, statistics, word differences, whitespace handling and moved blocks.
// Dependencies:     canonical, command, funcname, hunk, id, lines, moved, nurt_contracts, rename, select, source, words
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Differences in Git format: hunks with stable identifiers, statistics, word differences, whitespace handling and moved blocks.
//!
//! The line comparison and the hunk layout follow Git: the Myers algorithm
//! with the slider rules and the indentation heuristic, changes joined into
//! one hunk when they are at most twice the context apart, context lines
//! taken from the new text and the header text taken from the nearest
//! function line above the hunk.
//!
//! Hunks are identified as `path#number`, numbered in the canonical
//! difference (three context lines, no rename detection) of the working
//! tree against the index ([`Side::Worktree`]) or of the index against
//! `HEAD` ([`Side::Staged`]); [`hunks_for`] lists them and
//! [`apply_selected`] applies a selection of hunks and rows to a text.
//!
//! ```
//! use nurt_git::diff::{apply_selected, text_hunks, Direction, HunkId};
//!
//! let old = b"fn main() {\n    one();\n}\n";
//! let new = b"fn main() {\n    two();\n}\n";
//! let hunks = text_hunks("src/main.rs", old, new);
//! assert_eq!(hunks[0].id().to_string(), "src/main.rs#1");
//! let id: HunkId = format!("src/main.rs#1:{}", hunks[0].fingerprint()).parse()?;
//! assert_eq!(apply_selected(old, &hunks, &[id], Direction::Forward)?, new);
//! # Ok::<(), nurt_contracts::error::Error>(())
//! ```

mod canonical;
pub(crate) mod command;
#[cfg(test)]
pub(crate) mod fixture;
mod funcname;
mod hunk;
mod id;
mod lines;
mod moved;
mod rename;
mod select;
pub(crate) mod source;
mod words;

pub use canonical::{
    Driver, FileDiff, FileOptions, FilePair, Side, configured_lines, diff_contents, diff_pair,
    diff_texts, hunks_for, text_hunks,
};
pub use command::{
    DiffOutput, DiffRequest, GENERATED_PATTERNS, GitDiff, LOCK_FILES, MAX_CONTEXT, is_generated,
    is_lock_file, run_diff,
};
pub use funcname::{FUNCNAME_MAX, FuncMatcher, PatternError, builtin_patterns};
pub use hunk::{
    CANONICAL_CONTEXT, FINGERPRINT_LEN, Hunk, HunkLine, Layout, LineKind, MovedBlock, NO_NEWLINE,
    TextDiff,
};
pub use id::{HunkId, HunkProblem, MAX_ROW, RowRange, hunk_error};
pub use lines::{Algorithm, Change, LineOptions};
pub use moved::{MOVED_MIN_LINES, mark_moved, only_moved};
pub use rename::{
    CHUNK_MAX, DEFAULT_LIMIT, DEFAULT_THRESHOLD, Loader, RenameOptions, detect_renames,
};
pub use select::{Direction, apply_selected};
pub use source::{
    BINARY_PROBE, DiffAttr, Entry, Opened, Reader, looks_binary, mode_text, open_repository,
};
pub use words::{NEWLINE_MARK, word_lines};

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
