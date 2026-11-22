// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Presentation of responses for people: colours and the text of a report.

use nurt_contracts::config::ColorMode;
use nurt_contracts::envelope::{OutputFormat, Response};
use nurt_contracts::report::{Report, ReportStatus};

const RESET: &str = "\u{1b}[0m";
const BOLD: &str = "\u{1b}[1m";
const RED: &str = "\u{1b}[31m";
const GREEN: &str = "\u{1b}[32m";

/// Terminal styles of one output stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    enabled: bool,
}

impl Style {
    /// Styles according to `output.color`: `always` and `never` decide
    /// alone; `auto` colours a terminal unless `NO_COLOR` is set.
    #[must_use]
    pub fn resolve(mode: ColorMode, terminal: bool, no_color: bool) -> Self {
        let enabled = match mode {
            ColorMode::Always => true,
            ColorMode::Never => false,
            ColorMode::Auto => terminal && !no_color,
        };
        Self { enabled }
    }

    /// Styles that add nothing to the text.
    #[must_use]
    pub fn plain() -> Self {
        Self { enabled: false }
    }

    /// Whether the styles add colour to the text.
    #[must_use]
    pub fn is_enabled(self) -> bool {
        self.enabled
    }

    fn paint(self, code: &str, text: &str) -> String {
        if self.enabled {
            format!("{code}{text}{RESET}")
        } else {
            text.to_owned()
        }
    }

    /// Text of an error.
    #[must_use]
    pub fn error(self, text: &str) -> String {
        self.paint(RED, text)
    }

    /// Text of a successful outcome.
    #[must_use]
    pub fn success(self, text: &str) -> String {
        self.paint(GREEN, text)
    }

    /// Emphasised text.
    #[must_use]
    pub fn emphasis(self, text: &str) -> String {
        self.paint(BOLD, text)
    }
}

/// Name of a report status as written in the report.
#[must_use]
pub fn status_name(status: ReportStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Text of a response in `format`.
///
/// `json` is the response document. The other formats print the text
/// rendered by the engine and, when the engine gave none, a short summary of
/// the report.
#[must_use]
pub fn render(response: &Response, format: OutputFormat, style: Style) -> String {
    if format == OutputFormat::Json {
        return serde_json::to_string_pretty(response).unwrap_or_default();
    }
    match &response.rendered {
        Some(rendered) => rendered.text.trim_end().to_owned(),
        None => summary(&response.report, format, style),
    }
}

fn summary(report: &Report, format: OutputFormat, style: Style) -> String {
    let name = status_name(report.status);
    let status = if report.error.is_some() || report.status == ReportStatus::Failed {
        style.error(&name)
    } else {
        style.success(&name)
    };
    let mut lines = vec![format!("{status} {}", style.emphasis(&report.kind))];
    if let Some(error) = &report.error {
        lines.push(style.error(&error.to_string()));
        lines.extend(error.fix.iter().map(|fix| format!("fix: {fix}")));
        lines.extend(error.help.iter().map(|help| format!("help: {help}")));
    }
    let separator = if format == OutputFormat::Compact {
        " | "
    } else {
        "\n"
    };
    lines.join(separator)
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::{Error, ErrorCode};

    use super::*;

    fn failed() -> Response {
        let mut report = Report::new(ReportStatus::Failed, "remove");
        report.error = Some(Error::new(
            ErrorCode::UnknownCommand,
            "unknown command `remvoe`",
        ));
        Response::new(report)
    }

    #[test]
    fn auto_colour_needs_a_terminal_without_no_color() {
        assert!(Style::resolve(ColorMode::Auto, true, false).enabled);
        assert!(!Style::resolve(ColorMode::Auto, false, false).enabled);
        assert!(!Style::resolve(ColorMode::Auto, true, true).enabled);
        assert!(Style::resolve(ColorMode::Always, false, true).enabled);
        assert!(!Style::resolve(ColorMode::Never, true, false).enabled);
    }

    #[test]
    fn plain_style_adds_no_escape_sequences() {
        assert_eq!(Style::plain().error("failed"), "failed");
        assert!(
            Style::resolve(ColorMode::Always, false, false)
                .error("failed")
                .starts_with(RED)
        );
    }

    #[test]
    fn summary_names_status_kind_and_error() {
        let text = render(&failed(), OutputFormat::Text, Style::plain());
        assert_eq!(
            text,
            "failed remove\nE_UNKNOWN_COMMAND: unknown command `remvoe`"
        );
        let compact = render(&failed(), OutputFormat::Compact, Style::plain());
        assert_eq!(
            compact,
            "failed remove | E_UNKNOWN_COMMAND: unknown command `remvoe`"
        );
    }

    #[test]
    fn json_is_the_response_document() {
        let text = render(&failed(), OutputFormat::Json, Style::plain());
        let parsed: Response = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, failed());
    }

    #[test]
    fn rendered_text_is_printed_as_given() {
        let mut response = failed();
        response.rendered = Some(nurt_contracts::envelope::Rendered {
            format: OutputFormat::Text,
            text: "done\n".to_owned(),
        });
        assert_eq!(
            render(&response, OutputFormat::Text, Style::plain()),
            "done"
        );
    }
}
