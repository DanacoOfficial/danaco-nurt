// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Response `nurt-response/1`, language errors and events.

use std::fmt;

use serde_json::Value;

use crate::error::{Error, ErrorKind};

/// Response to a request (`nurt-response/1`): the effect report
/// `nurt-report/1` and, for text formats, the rendered text.
///
/// A language error is part of a response, not a call error:
/// [`Response::is_ok`] is `false` and [`Response::error`] returns the error
/// with its stable `E_*` code, position and fix.
///
/// # Examples
///
/// ```
/// use nurt::Response;
///
/// let response = Response::from_json(
///     r#"{"nurt": "1.0", "report": {"nurt": "1.0", "status": "failed", "kind": "find",
///        "error": {"code": "E_SYNTAX", "message": "unclosed list",
///                  "fix": ["find \"/srv/app\" glob: [\"*.go\"]"]}}}"#,
/// )?;
/// assert!(!response.is_ok());
/// let error = response.error().expect("language error");
/// assert_eq!(error.code(), "E_SYNTAX");
/// assert_eq!(error.fix(), ["find \"/srv/app\" glob: [\"*.go\"]"]);
/// # Ok::<(), nurt::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    raw: Value,
}

impl Response {
    /// Response from a JSON value.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Protocol`] when the value is not an object with
    /// an object `report`.
    pub fn from_value(raw: Value) -> Result<Self, Error> {
        if raw.get("report").is_some_and(Value::is_object) {
            Ok(Self { raw })
        } else {
            Err(Error::new(
                ErrorKind::Protocol,
                "response has no `report` object",
            ))
        }
    }

    /// Response from JSON text.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::Protocol`] when the text is not JSON or not a
    /// response.
    pub fn from_json(text: &str) -> Result<Self, Error> {
        let raw = serde_json::from_str(text).map_err(|error| {
            Error::new(
                ErrorKind::Protocol,
                format!("response is not valid JSON: {error}"),
            )
        })?;
        Self::from_value(raw)
    }

    /// The effect report `nurt-report/1`.
    #[must_use]
    pub fn report(&self) -> &Value {
        &self.raw["report"]
    }

    /// Status of the report: `ok`, `planned`, `running`, `partial`,
    /// `rolled_back` or `failed`.
    #[must_use]
    pub fn status(&self) -> Option<&str> {
        self.report().get("status").and_then(Value::as_str)
    }

    /// Text rendered from the report, when the requested format is a text
    /// format.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.raw
            .get("rendered")
            .and_then(|rendered| rendered.get("text"))
            .and_then(Value::as_str)
    }

    /// Whether the report carries no language error.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        !self.report().get("error").is_some_and(Value::is_object)
    }

    /// Language error of the report.
    #[must_use]
    pub fn error(&self) -> Option<LanguageError> {
        self.report()
            .get("error")
            .filter(|error| error.is_object())
            .map(|error| LanguageError::from_value(error.clone()))
    }

    /// Cursor of the next page, when the result has more pages.
    #[must_use]
    pub fn next_cursor(&self) -> Option<&str> {
        self.report().get("next_cursor").and_then(Value::as_str)
    }

    /// Whether the result has a further page.
    #[must_use]
    pub fn has_more(&self) -> bool {
        self.next_cursor().is_some()
    }

    /// The whole response as JSON.
    #[must_use]
    pub fn raw(&self) -> &Value {
        &self.raw
    }

    /// The whole response as JSON, by value.
    #[must_use]
    pub fn into_raw(self) -> Value {
        self.raw
    }

    /// The response, or its language error.
    ///
    /// # Errors
    ///
    /// Returns the language error of the report, when there is one.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt::Response;
    ///
    /// let response = Response::from_json(
    ///     r#"{"nurt": "1.0", "report": {"nurt": "1.0", "status": "ok", "kind": "find"}}"#,
    /// )?;
    /// assert!(response.into_result().is_ok());
    /// # Ok::<(), nurt::Error>(())
    /// ```
    pub fn into_result(self) -> Result<Self, LanguageError> {
        match self.error() {
            Some(error) => Err(error),
            None => Ok(self),
        }
    }
}

/// Error of the language reported in a response: a stable `E_*` code, the
/// stage, the position in the program and ready fixes.
#[derive(Debug, Clone, PartialEq)]
pub struct LanguageError {
    raw: Value,
}

impl LanguageError {
    fn from_value(raw: Value) -> Self {
        Self { raw }
    }

    fn text_field(&self, name: &str) -> Option<&str> {
        self.raw.get(name).and_then(Value::as_str)
    }

    /// Stable code, for example `E_SYNTAX`.
    #[must_use]
    pub fn code(&self) -> &str {
        self.text_field("code").unwrap_or_default()
    }

    /// Message in the language of the request.
    #[must_use]
    pub fn message(&self) -> &str {
        self.text_field("message").unwrap_or_default()
    }

    /// Stage in which the error occurred, for example `0` for parsing. The
    /// server writes it as `at.stage`; a `stage` member of the error itself
    /// is read as well.
    #[must_use]
    pub fn stage(&self) -> Option<&str> {
        self.raw
            .get("at")
            .and_then(|at| at.get("stage"))
            .and_then(Value::as_str)
            .or_else(|| self.text_field("stage"))
    }

    /// Position in the program: `line`, `column` and `length`.
    #[must_use]
    pub fn at(&self) -> Option<&Value> {
        self.raw.get("at")
    }

    /// Ready replacements of the program text, best first.
    #[must_use]
    pub fn fix(&self) -> Vec<&str> {
        self.raw
            .get("fix")
            .and_then(Value::as_array)
            .map(|fixes| fixes.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }

    /// Help topic with the rules that apply.
    #[must_use]
    pub fn help(&self) -> Option<&str> {
        self.text_field("help")
    }

    /// The whole error object as JSON.
    #[must_use]
    pub fn raw(&self) -> &Value {
        &self.raw
    }
}

impl fmt::Display for LanguageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())
    }
}

impl std::error::Error for LanguageError {}

/// Event of the server (`nurt/event`): progress and output of an execution,
/// background jobs, journal changes, free space, schedules and log messages.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    raw: Value,
}

impl Event {
    /// Event from a JSON value.
    #[must_use]
    pub fn from_value(raw: Value) -> Self {
        Self { raw }
    }

    /// Type of the event, for example `exec.finished` or `job.progress`.
    #[must_use]
    pub fn kind(&self) -> &str {
        self.raw
            .get("t")
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    /// Sequence number of the event within its execution.
    #[must_use]
    pub fn seq(&self) -> Option<u64> {
        self.raw.get("seq").and_then(Value::as_u64)
    }

    /// Identifier of the background job the event belongs to; `None` for
    /// events that are not job events.
    #[must_use]
    pub fn job_id(&self) -> Option<&str> {
        if self.kind().starts_with("job.") {
            self.raw.get("id").and_then(Value::as_str)
        } else {
            None
        }
    }

    /// The whole event as JSON.
    #[must_use]
    pub fn raw(&self) -> &Value {
        &self.raw
    }

    /// The whole event as JSON, by value.
    #[must_use]
    pub fn into_raw(self) -> Value {
        self.raw
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn response(report: Value) -> Response {
        Response::from_value(json!({"nurt": "1.0", "report": report})).unwrap()
    }

    #[test]
    fn rejects_value_without_report_object() {
        for value in [json!({}), json!({"report": "ok"}), json!([1]), json!(null)] {
            let error = Response::from_value(value).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Protocol);
        }
    }

    #[test]
    fn rejects_text_that_is_not_json() {
        let error = Response::from_json("{\"report\":").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Protocol);
        assert!(error.message().starts_with("response is not valid JSON"));
    }

    #[test]
    fn successful_report_has_no_error() {
        let response = response(json!({"nurt": "1.0", "status": "ok", "kind": "find"}));
        assert!(response.is_ok());
        assert!(response.error().is_none());
        assert_eq!(response.status(), Some("ok"));
        assert!(response.clone().into_result().is_ok());
    }

    #[test]
    fn null_error_counts_as_no_error() {
        let response = response(json!({"status": "ok", "error": null}));
        assert!(response.is_ok());
        assert!(response.error().is_none());
    }

    #[test]
    fn language_error_exposes_its_fields() {
        let response = response(json!({
            "status": "failed",
            "error": {
                "code": "E_SYNTAX",
                "message": "unclosed list",
                "stage": "0",
                "at": {"line": 1, "column": 27, "length": 1},
                "fix": ["a", "b"],
                "help": "lists"
            }
        }));
        assert!(!response.is_ok());
        let error = response.clone().into_result().unwrap_err();
        assert_eq!(error.code(), "E_SYNTAX");
        assert_eq!(error.message(), "unclosed list");
        assert_eq!(error.stage(), Some("0"));
        assert_eq!(
            error.at(),
            Some(&json!({"line": 1, "column": 27, "length": 1}))
        );
        assert_eq!(error.fix(), ["a", "b"]);
        assert_eq!(error.help(), Some("lists"));
        assert_eq!(error.to_string(), "E_SYNTAX: unclosed list");
        assert_eq!(error.raw()["code"], "E_SYNTAX");
    }

    #[test]
    fn stage_is_read_from_the_position_of_the_error() {
        let error = response(json!({
            "status": "failed",
            "error": {
                "code": "E_STALE",
                "message": "changed",
                "at": {"stage": "5"}
            }
        }))
        .error()
        .unwrap();
        assert_eq!(error.stage(), Some("5"));
        let positioned = response(json!({
            "error": {
                "code": "E_SYNTAX",
                "message": "m",
                "at": {"line": 1, "column": 2, "length": 1, "stage": "0"},
                "stage": "9"
            }
        }))
        .error()
        .unwrap();
        assert_eq!(positioned.stage(), Some("0"));
        let without_stage = response(json!({
            "error": {"code": "E_SYNTAX", "message": "m", "at": {"line": 1, "column": 2}}
        }))
        .error()
        .unwrap();
        assert_eq!(without_stage.stage(), None);
    }

    #[test]
    fn error_without_text_fields_reads_as_empty() {
        let error = response(json!({"error": {"code": 5, "message": []}}))
            .error()
            .unwrap();
        assert_eq!(error.code(), "");
        assert_eq!(error.message(), "");
        assert_eq!(error.to_string(), ": ");
        assert!(response(json!({"error": "text"})).is_ok());
        assert!(response(json!({"error": "text"})).error().is_none());
    }

    #[test]
    fn language_error_without_optional_fields_is_empty() {
        let error = response(json!({"error": {"code": "E_ARG", "message": "m"}}))
            .error()
            .unwrap();
        assert!(error.fix().is_empty());
        assert_eq!(error.help(), None);
        assert_eq!(error.stage(), None);
        assert_eq!(error.at(), None);
    }

    #[test]
    fn rendered_text_and_cursor_are_read_from_the_envelope() {
        let response = Response::from_value(json!({
            "nurt": "1.0",
            "report": {"status": "ok", "next_cursor": "c_91fe"},
            "rendered": {"format": "text", "text": "3 files"}
        }))
        .unwrap();
        assert_eq!(response.text(), Some("3 files"));
        assert_eq!(response.next_cursor(), Some("c_91fe"));
        assert!(response.has_more());
        assert_eq!(response.raw()["rendered"]["format"], "text");
        assert_eq!(response.into_raw()["nurt"], "1.0");
    }

    #[test]
    fn response_without_rendered_text_has_no_text() {
        let response = response(json!({"status": "ok"}));
        assert_eq!(response.text(), None);
        assert!(!response.has_more());
    }

    #[test]
    fn event_exposes_type_sequence_and_job() {
        let event = Event::from_value(json!({
            "t": "job.progress", "seq": 4, "id": "j-2", "done": 3, "total": 9
        }));
        assert_eq!(event.kind(), "job.progress");
        assert_eq!(event.seq(), Some(4));
        assert_eq!(event.job_id(), Some("j-2"));
        let other = Event::from_value(json!({"t": "progress", "id": "x"}));
        assert_eq!(other.job_id(), None);
        assert_eq!(event.raw()["total"], 9);
        assert_eq!(event.into_raw()["done"], 3);
    }

    #[test]
    fn event_without_fields_has_empty_type() {
        let event = Event::from_value(json!({}));
        assert_eq!(event.kind(), "");
        assert_eq!(event.seq(), None);
        assert_eq!(event.job_id(), None);
    }
}
