// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Agreement of `http.request` with its entry in `catalog/network.toml`.
//!
//! Every parameter of the catalog entry is accepted by the static check,
//! every enumerated value is recognized, the defaults equal the constants of
//! the implementation and the effects of the methods match the effect plan.

// Tests that write POSIX paths run on Unix only; their helpers are unused on Windows.
#![cfg_attr(windows, allow(dead_code, unused_imports))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a failed step ends the test"
)]

#[path = "support/exec_ctx.rs"]
mod exec_ctx;

use indexmap::IndexMap;
use nurt_contracts::error::Result;
use nurt_contracts::kinds::Reversibility;
use nurt_contracts::spec::{CatalogFile, StageSpec};
use nurt_contracts::stage::{CheckCtx, EffectPlan, EffectStage, StageImpl, StageInput};
use nurt_contracts::value::{self, Value};
use nurt_net::client::{DEFAULT_MAX_REDIRECTS, DEFAULT_TIMEOUT};
use nurt_net::http::request::{
    COMMAND, DEFAULT_QUERY_LIMIT, HttpRequest, MAX_FIND_CONTEXT, MAX_QUERY_LIMIT,
};

use exec_ctx::text;

const URL: &str = "https://example.test/page";

fn spec() -> StageSpec {
    let path = format!(
        "{}/../../catalog/network.toml",
        std::env::var("CARGO_MANIFEST_DIR")
            .as_deref()
            .unwrap_or(env!("CARGO_MANIFEST_DIR"))
    );
    let mut file: CatalogFile = toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    file.resolve_groups();
    file.command(COMMAND).unwrap().clone()
}

fn with_url<'a>(values: &[(&'a str, Value)]) -> Vec<(&'a str, Value)> {
    let mut all = vec![("url", text(URL))];
    all.extend_from_slice(values);
    all
}

fn check(values: &[(&str, Value)]) -> Result<()> {
    let spec = spec();
    let ctx = exec_ctx::ctx();
    let bound = exec_ctx::args(COMMAND, &with_url(values));
    HttpRequest
        .check(&bound, &CheckCtx::new(&spec, ctx.host(), ctx.config()))
        .map(|_| ())
}

fn plan(values: &[(&str, Value)]) -> Result<EffectPlan> {
    let ctx = exec_ctx::ctx();
    let bound = exec_ctx::args(COMMAND, &with_url(values));
    let stage = HttpRequest;
    let effect: &dyn EffectStage = stage.as_effect().unwrap();
    effect.plan(&bound, StageInput::none(), &ctx)
}

fn regex(pattern: &str) -> Value {
    Value::Regex(value::Regex {
        pattern: pattern.to_owned(),
        flags: value::RegexFlags::default(),
    })
}

/// Arguments that give `name` a value of its catalog type, together with
/// the arguments it applies to.
fn sample(name: &str) -> Vec<(&'static str, Value)> {
    match name {
        "url" => Vec::new(),
        "method" => vec![("method", text("post"))],
        "headers" => vec![(
            "headers",
            Value::Map(IndexMap::from([("Accept".to_owned(), text("text/html"))])),
        )],
        "body" => vec![("method", text("post")), ("body", text("payload"))],
        "json" => vec![("method", text("post")), ("json", Value::Int(1))],
        "save" => vec![("save", Value::Path("/srv/page.html".to_owned()))],
        "as" => vec![("as", text("markdown"))],
        "extract" => vec![("extract", text("links"))],
        "table" => vec![("extract", text("tables")), ("table", Value::Int(2))],
        "main_only" => vec![("main_only", Value::Bool(false))],
        "links" => vec![("links", text("list"))],
        "images" => vec![("images", text("drop"))],
        "section" => vec![("as", text("markdown")), ("section", text("Usage"))],
        "outline" => vec![("outline", text("never"))],
        "query" => vec![("as", text("text")), ("query", text("install"))],
        "query_limit" => vec![
            ("as", text("text")),
            ("query", text("install")),
            ("query_limit", Value::Int(MAX_QUERY_LIMIT)),
        ],
        "find" => vec![("as", text("text")), ("find", regex("install"))],
        "find_context" => vec![("find_context", Value::Int(MAX_FIND_CONTEXT))],
        "links_filter" => vec![("extract", text("links")), ("links_filter", regex("docs"))],
        "same_host" => vec![("extract", text("links")), ("same_host", Value::Bool(true))],
        "charset" => vec![("charset", text("iso-8859-2"))],
        "offset" => vec![("offset", Value::Int(100))],
        "cache" => vec![("cache", text("use"))],
        "unchanged_since" => vec![("unchanged_since", text("e3b0c44298fc"))],
        "timeout" => vec![(
            "timeout",
            Value::Duration(value::Duration::from_millis(5000)),
        )],
        "follow_redirects" => vec![("follow_redirects", Value::Bool(false))],
        "max_redirects" => vec![("max_redirects", Value::Int(3))],
        "verify_tls" => vec![("verify_tls", Value::Bool(false))],
        "retries" => vec![("retries", Value::Int(2))],
        other => panic!("the catalog parameter `{other}` has no sample value"),
    }
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn every_catalog_parameter_passes_the_check() {
    for param in &spec().params {
        if let Err(error) = check(&sample(&param.name)) {
            panic!("`{}` is rejected: {}", param.name, error.data().message);
        }
    }
}

#[test]
fn every_enumerated_value_passes_the_check() {
    for param in spec()
        .params
        .iter()
        .filter(|param| !param.values.is_empty())
    {
        for value in &param.values {
            if let Err(error) = check(&[(param.name.as_str(), text(value))]) {
                panic!(
                    "`{}: {value}` is rejected: {}",
                    param.name,
                    error.data().message
                );
            }
        }
    }
}

#[test]
fn unknown_enumerated_value_is_rejected() {
    let error = check(&[("cache", text("forever"))]).unwrap_err();
    assert_eq!(error.code(), nurt_contracts::error::ErrorCode::UnknownValue);
}

#[test]
fn catalog_defaults_match_the_implementation() {
    let spec = spec();
    let expected: Vec<(&str, String)> = vec![
        ("method", "get".to_owned()),
        ("as", "raw".to_owned()),
        ("extract", "content".to_owned()),
        ("table", "0".to_owned()),
        ("main_only", "true".to_owned()),
        ("links", "inline".to_owned()),
        ("images", "alt".to_owned()),
        ("outline", "auto".to_owned()),
        ("query_limit", DEFAULT_QUERY_LIMIT.to_string()),
        ("find_context", "1".to_owned()),
        ("same_host", "false".to_owned()),
        ("charset", "\"auto\"".to_owned()),
        ("offset", "0".to_owned()),
        ("cache", "bypass".to_owned()),
        ("timeout", format!("{}s", DEFAULT_TIMEOUT.as_secs())),
        ("follow_redirects", "true".to_owned()),
        ("max_redirects", DEFAULT_MAX_REDIRECTS.to_string()),
        ("verify_tls", "true".to_owned()),
        ("retries", "0".to_owned()),
    ];
    for (name, default) in expected {
        let param = spec.param(name).unwrap();
        assert_eq!(
            param.default.as_ref().map(|value| value.as_str()),
            Some(default.as_str()),
            "default of `{name}`"
        );
    }
}

#[test]
fn catalog_bounds_match_the_implementation() {
    let spec = spec();
    let query_limit = &spec.param("query_limit").unwrap().summary;
    assert!(query_limit.contains(&format!("from 1 to {MAX_QUERY_LIMIT}")));
    let find_context = &spec.param("find_context").unwrap().summary;
    assert!(find_context.contains(&format!("up to {MAX_FIND_CONTEXT}")));
    assert!(check(&sample("query_limit")).is_ok());
    let past = check(&[
        ("as", text("text")),
        ("query", text("install")),
        ("query_limit", Value::Int(MAX_QUERY_LIMIT + 1)),
    ]);
    assert!(past.is_err());
    assert!(check(&[("find_context", Value::Int(MAX_FIND_CONTEXT + 1))]).is_err());
}

#[test]
fn method_effects_match_the_plan() {
    let spec = spec();
    let methods = &spec.param("method").unwrap().values;
    for method in methods {
        let planned = plan(&[("method", text(method))]).unwrap();
        let effect = spec.effects.iter().find(|effect| effect.action == *method);
        let irreversible =
            effect.is_some_and(|effect| effect.reversibility == Reversibility::Irreversible);
        assert_eq!(
            !planned.irreversible.is_empty(),
            irreversible,
            "irreversibility of `{method}`"
        );
    }
}

#[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
#[test]
fn save_effect_matches_the_plan() {
    let spec = spec();
    let effect = spec
        .effects
        .iter()
        .find(|effect| effect.action == "save")
        .unwrap();
    assert_eq!(effect.reversibility, Reversibility::Reversible);
    let planned = plan(&sample("save")).unwrap();
    assert_eq!(planned.changes.len(), 1);
    assert!(planned.irreversible.is_empty());
}
