// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Options that control shaping and rendering of a report.

use nurt_contracts::envelope::{DiffMode, OutputFormat, RequestOptions, Verbosity};

/// Default number of hunks shown at verbosity `normal`.
pub const DEFAULT_DIFF_MAX_HUNKS: usize = 20;

/// Default number of difference lines shown at verbosity `normal`.
pub const DEFAULT_DIFF_MAX_LINES: usize = 200;

/// Controls the shaping and the rendering of a report.
///
/// The defaults match an absent option of a request: format `text`,
/// verbosity `normal`, a page of 50 changes, unified differences, no limit on
/// the length of text fields, no terminal width and no colors.
///
/// # Examples
///
/// ```
/// use nurt_contracts::envelope::{OutputFormat, RequestOptions};
/// use nurt_report::RenderOptions;
///
/// let request = RequestOptions {
///     format: Some(OutputFormat::Compact),
///     limit: Some(10),
///     ..RequestOptions::default()
/// };
/// let options = RenderOptions::from_request(&request);
/// assert_eq!(options.format, OutputFormat::Compact);
/// assert_eq!(options.limit, 10);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderOptions {
    /// Format of the rendered result.
    pub format: OutputFormat,
    /// Level of detail of the report.
    pub verbosity: Verbosity,
    /// Largest number of changes on one page.
    pub limit: usize,
    /// Form of differences.
    pub diff: DiffMode,
    /// Largest number of hunks shown at verbosity `normal`.
    pub diff_max_hunks: usize,
    /// Largest number of difference lines shown at verbosity `normal`.
    pub diff_max_lines: usize,
    /// Largest number of characters of a text field of a record; `0` means
    /// no limit.
    pub max_chars: usize,
    /// Fields of the records to keep, in this order; `None` keeps all.
    pub fields: Option<Vec<String>>,
    /// Terminal width for the text format; longer lines are shortened.
    pub width: Option<usize>,
    /// Whether the text format uses ANSI colors; only for a terminal.
    pub color: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self::from_request(&RequestOptions::default())
    }
}

impl RenderOptions {
    /// Takes the options that concern the result from the call options of a
    /// request.
    ///
    /// The terminal width and the colors are settings of the command line
    /// and are never part of a request: they stay unset.
    #[must_use]
    pub fn from_request(options: &RequestOptions) -> Self {
        Self {
            format: options.format(),
            verbosity: options.verbosity(),
            limit: usize::try_from(options.limit()).unwrap_or(usize::MAX),
            diff: options.diff(),
            diff_max_hunks: DEFAULT_DIFF_MAX_HUNKS,
            diff_max_lines: DEFAULT_DIFF_MAX_LINES,
            max_chars: usize::try_from(options.max_chars()).unwrap_or(usize::MAX),
            fields: options.fields.clone(),
            width: None,
            color: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_absent_request_options() {
        let options = RenderOptions::default();
        assert_eq!(options.format, OutputFormat::Text);
        assert_eq!(options.verbosity, Verbosity::Normal);
        assert_eq!(options.limit, 50);
        assert_eq!(options.diff, DiffMode::Unified);
        assert_eq!(options.diff_max_hunks, DEFAULT_DIFF_MAX_HUNKS);
        assert_eq!(options.diff_max_lines, DEFAULT_DIFF_MAX_LINES);
        assert_eq!(options.max_chars, 0);
        assert!(options.fields.is_none());
        assert!(!options.color);
    }

    #[test]
    fn takes_given_request_options() {
        let request = RequestOptions {
            verbosity: Some(Verbosity::Quiet),
            max_chars: Some(80),
            fields: Some(vec!["path".to_owned()]),
            ..RequestOptions::default()
        };
        let options = RenderOptions::from_request(&request);
        assert_eq!(options.verbosity, Verbosity::Quiet);
        assert_eq!(options.max_chars, 80);
        assert_eq!(options.fields, Some(vec!["path".to_owned()]));
    }
}
