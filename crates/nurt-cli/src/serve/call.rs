// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-cli::serve::call
// Responsibility:   Methods of the protocol, their parameters and the shapes of their results.
// Dependencies:     nurt_contracts, nurt_core, serde, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Methods of the protocol, their parameters and the shapes of their
//! results.
//!
//! A language error of an engine operation is part of the result: the
//! result is then a response `nurt-response/1` whose report has the status
//! `failed`, the error and, as `kind`, the name of the method.

use nurt_contracts::ast::AstDocument;
use nurt_contracts::envelope::{Language, Request, Response};
use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::kinds::SyntaxVariant;
use nurt_contracts::report::{Report, ReportStatus};
use nurt_contracts::schema;
use nurt_core::CatalogQuery;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::protocol::RpcError;

/// Opens the connection.
pub(crate) const INITIALIZE: &str = "initialize";
/// Cancels a request or a background job.
pub(crate) const CANCEL: &str = "cancel";
/// Ends the connection after the requests in flight.
pub(crate) const SHUTDOWN: &str = "shutdown";

/// Every method, for the message of an unknown method.
pub(crate) const METHODS: &[&str] = &[
    INITIALIZE, "run", "check", "parse", "print", "correct", "page", "describe", "help", "schema",
    "catalog", "history", "space", "info", CANCEL, SHUTDOWN,
];

/// Subject of `describe` and topic of `help` when none is given: the
/// language itself.
pub(crate) const DEFAULT_SUBJECT: &str = "core";

/// Parameters `{program, variant?}` of `parse` and `correct`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProgramParams {
    pub(crate) program: String,
    #[serde(default)]
    pub(crate) variant: Option<SyntaxVariant>,
}

/// Parameters `{ast, variant?}` of `print`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrintParams {
    pub(crate) ast: Value,
    #[serde(default)]
    pub(crate) variant: Option<SyntaxVariant>,
}

/// Parameters `{cursor, limit?}` of `page`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PageParams {
    pub(crate) cursor: String,
    #[serde(default)]
    pub(crate) limit: Option<u32>,
}

/// Parameters `{subject?, language?}` of `describe`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DescribeParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) language: Option<Language>,
}

/// Parameters `{topic?, language?}` of `help`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HelpParams {
    #[serde(default)]
    pub(crate) topic: Option<String>,
    #[serde(default)]
    pub(crate) language: Option<Language>,
}

/// Parameters `{name}` of `schema`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SchemaParams {
    pub(crate) name: String,
}

/// Parameters `{volume?}` of `space`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpaceParams {
    #[serde(default)]
    pub(crate) volume: Option<String>,
}

/// Call of an engine operation with its parameters.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Call {
    Run(Box<Request>),
    Check(Box<Request>),
    Parse(ProgramParams),
    Print(PrintParams),
    Correct(ProgramParams),
    Page(PageParams),
    Describe(DescribeParams),
    Help(HelpParams),
    Schema(SchemaParams),
    /// Selection of `catalog`: `null` or `{"group": name}` or
    /// `{"command": name}`, the value of `what`.
    Catalog(Value),
    /// Query of `history`: a query object, or `null` for none.
    History(Value),
    Space(SpaceParams),
    Info,
    /// A `run` or `check` whose syntax tree is not a valid `nurt-ast/1`
    /// tree; answered as a language error of the method, not as invalid
    /// parameters.
    Rejected {
        /// Name of the method.
        method: &'static str,
        /// The language error `E_AST_SCHEMA`.
        error: Box<Error>,
    },
}

impl Call {
    /// Reads the parameters of an engine method.
    pub(crate) fn parse(method: &str, params: Option<Value>) -> Result<Self, RpcError> {
        let call = match method {
            "run" => match envelope(method, params.clone()) {
                Ok(request) => Self::Run(Box::new(request)),
                Err(error) => return rejected("run", params.as_ref(), error),
            },
            "check" => match envelope(method, params.clone()) {
                Ok(request) => Self::Check(Box::new(request)),
                Err(error) => return rejected("check", params.as_ref(), error),
            },
            "parse" => Self::Parse(object(method, params)?),
            "print" => Self::Print(object(method, params)?),
            "correct" => Self::Correct(object(method, params)?),
            "page" => Self::Page(object(method, params)?),
            "describe" => Self::Describe(object(method, params)?),
            "help" => Self::Help(object(method, params)?),
            "schema" => Self::Schema(object(method, params)?),
            "catalog" => Self::Catalog(catalog_selection(params)?),
            "history" => Self::History(history_query(params)?),
            "space" => Self::Space(object(method, params)?),
            "info" => {
                optional_object(method, params)?;
                Self::Info
            }
            _ => return Err(RpcError::method_not_found(method, METHODS)),
        };
        Ok(call)
    }

    /// Name of the method, used as the report kind of a failure.
    pub(crate) const fn method(&self) -> &'static str {
        match self {
            Self::Run(_) => "run",
            Self::Check(_) => "check",
            Self::Parse(_) => "parse",
            Self::Print(_) => "print",
            Self::Correct(_) => "correct",
            Self::Page(_) => "page",
            Self::Describe(_) => "describe",
            Self::Help(_) => "help",
            Self::Schema(_) => "schema",
            Self::Catalog(_) => "catalog",
            Self::History(_) => "history",
            Self::Space(_) => "space",
            Self::Info => "info",
            Self::Rejected { method, .. } => method,
        }
    }
}

/// Outcome of a `run` or `check` whose parameters are not a request
/// envelope: a language error `E_AST_SCHEMA` when only the syntax tree is
/// at fault, otherwise the error of the invalid parameters.
fn rejected(
    method: &'static str,
    params: Option<&Value>,
    invalid: RpcError,
) -> Result<Call, RpcError> {
    match params.and_then(ast_schema_error) {
        Some(error) => Ok(Call::Rejected {
            method,
            error: Box::new(error),
        }),
        None => Err(invalid),
    }
}

/// The error of a request whose only fault is a `ast` member that is not a
/// syntax tree `nurt-ast/1`.
fn ast_schema_error(params: &Value) -> Option<Error> {
    let members = params.as_object()?;
    let ast = members.get("ast")?;
    let mut rest = members.clone();
    rest.remove("ast");
    serde_json::from_value::<Request>(Value::Object(rest)).ok()?;
    let fault = serde_json::from_value::<AstDocument>(ast.clone()).err()?;
    Some(Error::new(
        ErrorCode::AstSchema,
        format!("`ast` is not a syntax tree nurt-ast/1: {fault}"),
    ))
}

/// Parameters of a method that takes an object; absent parameters are an
/// empty object.
pub(crate) fn object<T: DeserializeOwned>(
    method: &str,
    params: Option<Value>,
) -> Result<T, RpcError> {
    let params = params.unwrap_or_else(|| Value::Object(Map::new()));
    serde_json::from_value(params).map_err(|error| {
        RpcError::invalid_params(format!("invalid parameters of `{method}`: {error}"))
    })
}

fn envelope(method: &str, params: Option<Value>) -> Result<Request, RpcError> {
    let params = params.ok_or_else(|| {
        RpcError::invalid_params(format!(
            "`{method}` needs a request envelope nurt-request/1"
        ))
    })?;
    serde_json::from_value(params).map_err(|error| {
        RpcError::invalid_params(format!(
            "the parameters of `{method}` are not a request envelope nurt-request/1: {error}"
        ))
    })
}

/// Parameters of a method whose parameters are optional; absent
/// parameters and an empty object are `null`.
fn optional_object(method: &str, params: Option<Value>) -> Result<Value, RpcError> {
    match params {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(Value::Object(object)) if object.is_empty() => Ok(Value::Null),
        Some(value @ Value::Object(_)) => Ok(value),
        Some(_) => Err(RpcError::invalid_params(format!(
            "the parameters of `{method}` must be an object"
        ))),
    }
}

/// Selection of one group or one command in `catalog`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CatalogSelection {
    /// The groups with the number of their commands.
    Summary,
    /// The commands of one group.
    Group(String),
    /// The specification of one command.
    Command(String),
}

impl CatalogSelection {
    /// Reads the member `what` of the parameters of `catalog`: `null` for
    /// the whole catalog, an object with `group` or with `command`.
    pub(crate) fn from_value(what: &Value) -> Result<Self, RpcError> {
        let invalid = |message: String| {
            RpcError::invalid_params(format!("invalid parameters of `catalog`: {message}"))
        };
        let object = match what {
            Value::Null => return Ok(Self::Summary),
            Value::Object(object) => object,
            _ => return Err(invalid("`what` must be an object or null".to_owned())),
        };
        let name = |key: &str| match object.get(key) {
            Some(Value::String(name)) if !name.is_empty() => Ok(Some(name.clone())),
            Some(_) => Err(invalid(format!("`{key}` must be a non-empty string"))),
            None => Ok(None),
        };
        if let Some(key) = object
            .keys()
            .find(|key| !matches!(key.as_str(), "group" | "command"))
        {
            return Err(invalid(format!(
                "unknown member `{key}` of `what`; expected `group` or `command`"
            )));
        }
        match (name("group")?, name("command")?) {
            (None, None) => Ok(Self::Summary),
            (Some(group), None) => Ok(Self::Group(group)),
            (None, Some(command)) => Ok(Self::Command(command)),
            (Some(_), Some(_)) => Err(invalid(
                "`what` takes `group` or `command`, not both".to_owned(),
            )),
        }
    }

    /// Query of the engine for this selection.
    pub(crate) fn query(&self) -> CatalogQuery<'_> {
        match self {
            Self::Summary => CatalogQuery::Summary,
            Self::Group(group) => CatalogQuery::Group(group),
            Self::Command(command) => CatalogQuery::Command(command),
        }
    }
}

/// Parameters `{what?}` of `catalog`; returns the validated `what`.
fn catalog_selection(params: Option<Value>) -> Result<Value, RpcError> {
    let Value::Object(mut object) = optional_object("catalog", params)? else {
        return Ok(Value::Null);
    };
    let what = object.remove("what").unwrap_or(Value::Null);
    if let Some(key) = object.keys().next() {
        return Err(RpcError::invalid_params(format!(
            "unknown parameter `{key}` of `catalog`; expected `what`"
        )));
    }
    CatalogSelection::from_value(&what)?;
    Ok(if what.as_object().is_some_and(Map::is_empty) {
        Value::Null
    } else {
        what
    })
}

/// Parameters `{query?}` of `history`; returns the query object or `null`.
///
/// The query is an object, the text of an object or `null`. An object
/// without the member `query` is itself the query.
fn history_query(params: Option<Value>) -> Result<Value, RpcError> {
    let invalid = |message: &str| {
        RpcError::invalid_params(format!("invalid parameters of `history`: {message}"))
    };
    let Value::Object(mut object) = optional_object("history", params)? else {
        return Ok(Value::Null);
    };
    let Some(query) = object.remove("query") else {
        return Ok(Value::Object(object));
    };
    if !object.is_empty() {
        return Err(invalid("`query` cannot be combined with other members"));
    }
    let query = match query {
        Value::String(text) if text.trim().is_empty() => Value::Null,
        Value::String(text) => serde_json::from_str(&text)
            .map_err(|error| invalid(&format!("`query` is not valid JSON text: {error}")))?,
        other => other,
    };
    match query {
        Value::Null => Ok(Value::Null),
        Value::Object(object) if object.is_empty() => Ok(Value::Null),
        Value::Object(object) => Ok(Value::Object(object)),
        _ => Err(invalid(
            "`query` must be an object, JSON text of an object or null",
        )),
    }
}

/// JSON form of a result.
pub(crate) fn to_json<T: Serialize + ?Sized>(value: &T) -> Result<Value, RpcError> {
    serde_json::to_value(value)
        .map_err(|error| RpcError::internal(format!("cannot serialise the result: {error}")))
}

/// Result of a method that failed with a language error.
pub(crate) fn failure(kind: &str, error: Error) -> Result<Value, RpcError> {
    let mut report = Report::new(ReportStatus::Failed, kind);
    report.error = Some(error);
    to_json(&Response::new(report))
}

/// Result of an engine operation: its value, or the failure response of its
/// language error.
pub(crate) fn answer(kind: &str, outcome: Result<Value, Error>) -> Result<Value, RpcError> {
    outcome.or_else(|error| failure(kind, error))
}

/// Object with the single member `name`.
pub(crate) fn member(name: &str, value: Value) -> Value {
    let mut object = Map::new();
    object.insert(name.to_owned(), value);
    Value::Object(object)
}

/// `first` with the members of `second` added, when both are objects.
pub(crate) fn merge(first: Value, second: Value) -> Value {
    match (first, second) {
        (Value::Object(mut first), Value::Object(second)) => {
            first.extend(second);
            Value::Object(first)
        }
        (first, _) => first,
    }
}

/// Result of `run`: the response, or `NURT_E_CANCELLED` with the response
/// when the request was cancelled.
pub(crate) fn run_result(response: Value) -> Result<Value, RpcError> {
    let code = response
        .pointer("/report/error/code")
        .and_then(Value::as_str);
    if code == Some(ErrorCode::Cancelled.as_str()) {
        Err(RpcError::cancelled(response))
    } else {
        Ok(response)
    }
}

/// Result of `schema`: the published JSON Schema `name`.
pub(crate) fn schema(name: &str) -> Result<Value, RpcError> {
    if let Some(text) = schema::by_name(name) {
        return serde_json::from_str(text).map_err(|error| {
            RpcError::internal(format!("the schema `{name}` is not JSON: {error}"))
        });
    }
    let known: Vec<&str> = schema::ALL.iter().map(|(name, _)| *name).collect();
    let error = Error::new(
        ErrorCode::UnknownValue,
        format!("unknown schema `{name}`; known: {}", known.join(", ")),
    )
    .with_found(name)
    .with_detail("accepted", to_json(&known)?);
    failure("schema", error)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::serve::protocol::{INVALID_PARAMS, METHOD_NOT_FOUND};

    #[test]
    fn parameters_are_read_per_method() {
        let run = Call::parse("run", Some(json!({"nurt": "1.0", "program": "count"}))).unwrap();
        assert_eq!(run, Call::Run(Box::new(Request::program("count"))));
        assert_eq!(run.method(), "run");
        let parse = Call::parse("parse", Some(json!({"program": "x", "variant": "b"}))).unwrap();
        assert_eq!(
            parse,
            Call::Parse(ProgramParams {
                program: "x".to_owned(),
                variant: Some(SyntaxVariant::B),
            })
        );
        assert_eq!(
            Call::parse("help", None).unwrap(),
            Call::Help(HelpParams::default())
        );
        assert_eq!(
            Call::parse("catalog", Some(json!({}))).unwrap(),
            Call::Catalog(Value::Null)
        );
        assert_eq!(
            Call::parse("history", Some(json!({"limit": 2}))).unwrap(),
            Call::History(json!({"limit": 2}))
        );
        assert_eq!(Call::parse("info", None).unwrap(), Call::Info);
        for method in METHODS {
            let parsed = Call::parse(method, None);
            let rejected = parsed
                .as_ref()
                .is_err_and(|error| error.code == METHOD_NOT_FOUND);
            assert_eq!(
                rejected,
                [INITIALIZE, CANCEL, SHUTDOWN].contains(method),
                "{method}"
            );
        }
    }

    #[test]
    fn method_names_match_the_list() {
        let calls = [
            Call::Run(Box::new(Request::program("x"))),
            Call::Check(Box::new(Request::program("x"))),
            Call::Parse(ProgramParams {
                program: String::new(),
                variant: None,
            }),
            Call::Print(PrintParams {
                ast: Value::Null,
                variant: None,
            }),
            Call::Correct(ProgramParams {
                program: String::new(),
                variant: None,
            }),
            Call::Page(PageParams {
                cursor: String::new(),
                limit: None,
            }),
            Call::Describe(DescribeParams::default()),
            Call::Help(HelpParams::default()),
            Call::Schema(SchemaParams {
                name: String::new(),
            }),
            Call::Catalog(Value::Null),
            Call::History(Value::Null),
            Call::Space(SpaceParams::default()),
            Call::Info,
        ];
        let names: Vec<&str> = calls.iter().map(Call::method).collect();
        let engine_methods: Vec<&str> = METHODS
            .iter()
            .copied()
            .filter(|method| ![INITIALIZE, CANCEL, SHUTDOWN].contains(method))
            .collect();
        assert_eq!(names, engine_methods);
    }

    #[test]
    fn invalid_parameters_are_rejected() {
        let missing = Call::parse("run", None).unwrap_err();
        assert_eq!(missing.code, INVALID_PARAMS);
        let wrong = Call::parse("check", Some(json!({"program": 1}))).unwrap_err();
        assert!(wrong.message.contains("nurt-request/1"));
        let unknown = Call::parse("page", Some(json!({"cursor": "c", "size": 1}))).unwrap_err();
        assert!(unknown.message.contains("`page`"));
        let array = Call::parse("history", Some(json!([1]))).unwrap_err();
        assert_eq!(array.code, INVALID_PARAMS);
        let method = Call::parse("undo", None).unwrap_err();
        assert_eq!(method.code, METHOD_NOT_FOUND);
    }

    #[test]
    fn results_have_their_documented_shapes() {
        let failed = failure("help", Error::new(ErrorCode::Unsupported, "no help")).unwrap();
        assert_eq!(failed["report"]["status"], "failed");
        assert_eq!(failed["report"]["kind"], "help");
        assert_eq!(failed["report"]["error"]["code"], "E_UNSUPPORTED");
        let response: Response = serde_json::from_value(failed.clone()).unwrap();
        response.validate().unwrap();
        assert_eq!(answer("help", Ok(json!(1))).unwrap(), json!(1));
        assert_eq!(
            merge(json!({"a": 1}), json!({"b": 2})),
            json!({"a": 1, "b": 2})
        );
        assert_eq!(merge(json!("a"), json!({"b": 2})), json!("a"));
        assert_eq!(member("text", json!("x")), json!({"text": "x"}));
        assert_eq!(run_result(failed.clone()).unwrap(), failed);
        let cancelled = failure("run", Error::new(ErrorCode::Cancelled, "stop")).unwrap();
        assert_eq!(run_result(cancelled).unwrap_err().code, -32002);
    }

    #[test]
    fn schemas_are_published_by_name() {
        let report = schema("report").unwrap();
        assert!(report.is_object());
        assert!(report.get("report").is_none());
        let unknown = schema("nothing").unwrap();
        assert_eq!(unknown["report"]["error"]["code"], "E_UNKNOWN_VALUE");
        assert_eq!(unknown["report"]["kind"], "schema");
    }

    #[test]
    fn catalog_takes_the_selection_in_what() {
        let parse = |params| Call::parse("catalog", params);
        assert_eq!(
            parse(Some(json!({"what": null}))).unwrap(),
            Call::Catalog(Value::Null)
        );
        assert_eq!(
            parse(Some(json!({"what": {"group": "documents"}}))).unwrap(),
            Call::Catalog(json!({"group": "documents"}))
        );
        assert_eq!(
            parse(Some(json!({"what": {"command": "find"}}))).unwrap(),
            Call::Catalog(json!({"command": "find"}))
        );
        for params in [
            json!({"what": "find"}),
            json!({"what": {"group": 1}}),
            json!({"what": {"group": "a", "command": "b"}}),
            json!({"what": {"name": "a"}}),
            json!({"group": "a"}),
        ] {
            let error = parse(Some(params.clone())).unwrap_err();
            assert_eq!(error.code, INVALID_PARAMS, "{params}");
        }
    }

    #[test]
    fn catalog_selection_maps_to_the_engine_query() {
        let selection = |what: Value| CatalogSelection::from_value(&what).unwrap();
        assert_eq!(selection(Value::Null).query(), CatalogQuery::Summary);
        assert_eq!(
            selection(json!({"group": "g"})).query(),
            CatalogQuery::Group("g")
        );
        assert_eq!(
            selection(json!({"command": "c"})).query(),
            CatalogQuery::Command("c")
        );
    }

    #[test]
    fn history_takes_the_query_in_query_or_as_the_object() {
        let parse = |params| Call::parse("history", Some(params));
        let expected = Call::History(json!({"limit": 2}));
        assert_eq!(parse(json!({"limit": 2})).unwrap(), expected);
        assert_eq!(parse(json!({"query": {"limit": 2}})).unwrap(), expected);
        assert_eq!(parse(json!({"query": "{\"limit\": 2}"})).unwrap(), expected);
        for empty in [
            json!({"query": null}),
            json!({"query": ""}),
            json!({"query": {}}),
        ] {
            assert_eq!(parse(empty).unwrap(), Call::History(Value::Null));
        }
        for params in [
            json!({"query": 3}),
            json!({"query": "[1]"}),
            json!({"query": "not json"}),
            json!({"query": {}, "limit": 1}),
        ] {
            let error = parse(params.clone()).unwrap_err();
            assert_eq!(error.code, INVALID_PARAMS, "{params}");
        }
    }

    #[test]
    fn a_tree_that_is_not_nurt_ast_is_a_language_error_of_the_method() {
        let params = json!({"nurt": "1.0", "ast": {"nurt": "1.0", "program": 5}});
        let Ok(Call::Rejected { method, error }) = Call::parse("run", Some(params)) else {
            panic!("the tree is not rejected as a language error");
        };
        assert_eq!(method, "run");
        assert_eq!(error.code, ErrorCode::AstSchema);
    }

    #[test]
    fn an_empty_selection_of_the_catalog_is_the_whole_catalog() {
        assert_eq!(
            Call::parse("catalog", Some(json!({"what": {}}))).unwrap(),
            Call::Catalog(Value::Null)
        );
        assert_eq!(
            Call::parse("catalog", None).unwrap(),
            Call::Catalog(Value::Null)
        );
        assert_eq!(
            Call::parse("catalog", Some(Value::Null)).unwrap(),
            Call::Catalog(Value::Null)
        );
    }

    #[test]
    fn info_takes_no_parameters_but_tolerates_an_empty_object() {
        for params in [
            None,
            Some(Value::Null),
            Some(json!({})),
            Some(json!({"x": 1})),
        ] {
            assert_eq!(Call::parse("info", params).unwrap(), Call::Info);
        }
        for params in [json!([1]), json!(3), json!("x")] {
            let error = Call::parse("info", Some(params.clone())).unwrap_err();
            assert_eq!(error.code, INVALID_PARAMS, "{params}");
        }
    }

    #[test]
    fn describe_and_help_take_a_language_and_reject_unknown_members() {
        let describe = Call::parse(
            "describe",
            Some(json!({"subject": "\u{142}\u{f3}d\u{17a}", "language": "pl"})),
        )
        .unwrap();
        assert_eq!(
            describe,
            Call::Describe(DescribeParams {
                subject: Some("\u{142}\u{f3}d\u{17a}".to_owned()),
                language: Some(Language::Pl),
            })
        );
        for (method, params) in [
            ("describe", json!({"topic": "x"})),
            ("help", json!({"topic": "x", "depth": 1})),
            ("help", json!({"language": "de"})),
            ("schema", json!({"name": "report", "format": "json"})),
            ("schema", json!({})),
            ("space", json!({"path": "/"})),
            ("page", json!({"cursor": "c", "limit": -1})),
            ("print", json!({"variant": "a"})),
        ] {
            let error = Call::parse(method, Some(params.clone())).unwrap_err();
            assert_eq!(error.code, INVALID_PARAMS, "{method} {params}");
        }
    }

    #[test]
    fn every_published_schema_is_served_without_the_report_of_a_failure() {
        for (name, _) in nurt_contracts::schema::ALL {
            let served = schema(name).unwrap();
            assert!(served.is_object(), "{name}");
            assert!(served.get("report").is_none(), "{name}");
        }
    }

    #[test]
    fn an_unknown_schema_lists_the_known_names() {
        let unknown = schema("").unwrap();
        assert_eq!(unknown["report"]["status"], "failed");
        let message = unknown["report"]["error"]["message"].as_str().unwrap();
        assert!(
            message.starts_with("unknown schema ``; known: "),
            "{message}"
        );
        for (name, _) in nurt_contracts::schema::ALL {
            assert!(message.contains(name), "{name}");
        }
    }

    #[test]
    fn a_response_without_an_error_is_not_a_cancellation() {
        assert_eq!(run_result(json!({})).unwrap(), json!({}));
        assert_eq!(
            run_result(json!({"report": {"error": {"code": 5}}})).unwrap(),
            json!({"report": {"error": {"code": 5}}})
        );
        let failed = failure("run", Error::new(ErrorCode::Timeout, "slow")).unwrap();
        assert_eq!(run_result(failed.clone()).unwrap(), failed);
    }

    #[test]
    fn a_bad_tree_in_a_check_is_a_language_error_of_the_check() {
        let params = json!({"nurt": "1.0", "ast": {"nurt": "1.0", "program": 5}});
        assert!(matches!(
            Call::parse("check", Some(params)),
            Ok(Call::Rejected {
                method: "check",
                ..
            })
        ));
        assert_eq!(
            Call::Rejected {
                method: "check",
                error: Box::new(Error::new(ErrorCode::AstSchema, "x"))
            }
            .method(),
            "check"
        );
    }

    #[test]
    fn a_request_that_is_not_an_object_is_invalid_params() {
        for params in [json!([1]), json!("x"), json!(7)] {
            let error = Call::parse("run", Some(params.clone())).unwrap_err();
            assert_eq!(error.code, INVALID_PARAMS, "{params}");
            let error = Call::parse("check", Some(params.clone())).unwrap_err();
            assert_eq!(error.code, INVALID_PARAMS, "{params}");
        }
    }

    #[test]
    fn other_invalid_parameters_stay_invalid_parameters() {
        let params = json!({"nurt": "1.0", "program": 5});
        assert!(Call::parse("check", Some(params)).is_err());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
