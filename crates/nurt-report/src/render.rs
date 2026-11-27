// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-report::render
// Responsibility:   Rendering of a report in the requested format and the response envelope.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Rendering of a report in the requested format and the response envelope.

use nurt_contracts::envelope::{OutputFormat, Rendered, Response};
use nurt_contracts::report::Report;

use crate::options::RenderOptions;
use crate::shape::{Shaped, shape};
use crate::{compact, markdown, text};

/// Renders a report in the format of the options.
///
/// The report is first reduced by [`shape`]. The format `text` follows the
/// layouts of the report specification, `markdown` writes headings, tables
/// and fenced blocks, `compact` writes one record per line as `key=value`
/// pairs without headings, and `json` writes the reduced report as one line of
/// JSON with a fixed order of fields. The terminal width and the colors of the
/// options apply to the format `text` only.
///
/// # Examples
///
/// ```
/// use nurt_contracts::report::{Report, ReportStatus};
/// use nurt_report::{RenderOptions, render};
///
/// let mut report = Report::new(ReportStatus::Ok, "remove");
/// report.op = Some(21);
/// assert_eq!(render(&report, &RenderOptions::default()), "op 21  remove  ok");
/// ```
#[must_use]
pub fn render(report: &Report, options: &RenderOptions) -> String {
    render_shaped(&shape(report, options), options)
}

fn render_shaped(shaped: &Shaped, options: &RenderOptions) -> String {
    match options.format {
        OutputFormat::Json => serde_json::to_string(&shaped.report).unwrap_or_default(),
        OutputFormat::Markdown => markdown::render(shaped),
        OutputFormat::Compact => compact::render(shaped, options),
        _ => text::render(shaped, options),
    }
}

/// Builds the response to a request from a report.
///
/// The response carries the report reduced by [`shape`], so that the report
/// and the rendered text describe the same result. The rendered text is
/// present for every format except `json`, for which the report itself is
/// the result. The text of a response never contains terminal colors and is
/// not shortened to a terminal width, whatever the options say. The cursor of
/// the next page stays in [`Report::next_cursor`].
///
/// # Examples
///
/// ```
/// use nurt_contracts::envelope::OutputFormat;
/// use nurt_contracts::report::{Report, ReportStatus};
/// use nurt_report::{RenderOptions, respond};
///
/// let mut report = Report::new(ReportStatus::Ok, "remove");
/// report.op = Some(21);
///
/// let response = respond(&report, &RenderOptions::default());
/// let rendered = response.rendered.as_ref().unwrap();
/// assert_eq!(rendered.format, OutputFormat::Text);
/// assert_eq!(rendered.text, "op 21  remove  ok");
/// assert!(response.validate().is_ok());
///
/// let options = RenderOptions {
///     format: OutputFormat::Json,
///     ..RenderOptions::default()
/// };
/// assert!(respond(&report, &options).rendered.is_none());
/// ```
#[must_use]
pub fn respond(report: &Report, options: &RenderOptions) -> Response {
    let shaped = shape(report, options);
    let plain = RenderOptions {
        color: false,
        width: None,
        ..options.clone()
    };
    let rendered = match options.format {
        OutputFormat::Json => None,
        format => Some(Rendered {
            format,
            text: render_shaped(&shaped, &plain),
        }),
    };
    let mut response = Response::new(shaped.report);
    response.rendered = rendered;
    response
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
