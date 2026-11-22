// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! JSON form of records and arguments passed to extensions.
//!
//! A record is the object `{"kind": "<record kind>", "fields": {...}}` with
//! the field values in plain JSON (texts, numbers, booleans, lists and
//! objects); the arguments of a command are one object from parameter name
//! to plain JSON value. Parameters without a value are left out.
//!
//! ```
//! use nurt_contracts::value::{Record, RecordKind, Value};
//! use nurt_ext::wire::{record_from_json, record_to_json};
//!
//! let record = Record::new(RecordKind::Item).with("name", Value::from("a"));
//! let json = record_to_json(&record);
//! assert_eq!(json, r#"{"kind":"item","fields":{"name":"a"}}"#);
//! assert_eq!(record_from_json(&json).unwrap(), record);
//! ```

use std::str::FromStr;

use nurt_contracts::error::Result;
use nurt_contracts::stage::{BoundArgs, ExecCtx};
use nurt_contracts::value::{Fields, Record, RecordKind, Value};
use serde_json::{Map, Value as Json};

/// Member holding the record kind.
pub const KIND_MEMBER: &str = "kind";
/// Member holding the record fields.
pub const FIELDS_MEMBER: &str = "fields";

/// JSON form of a record.
#[must_use]
pub fn record_to_json(record: &Record) -> String {
    let mut object = Map::new();
    object.insert(KIND_MEMBER.to_owned(), Json::from(record.kind.as_str()));
    object.insert(FIELDS_MEMBER.to_owned(), record.to_plain_json());
    Json::Object(object).to_string()
}

/// Record from its JSON form.
///
/// # Errors
///
/// Fails with a description of the problem when the text is not JSON, is
/// not an object with a known `kind`, has members other than `kind` and
/// `fields`, or `fields` is not an object.
pub fn record_from_json(text: &str) -> std::result::Result<Record, String> {
    let json: Json =
        serde_json::from_str(text).map_err(|error| format!("invalid JSON: {error}"))?;
    let Json::Object(mut object) = json else {
        return Err("a record must be a JSON object".to_owned());
    };
    let kind = match object.remove(KIND_MEMBER) {
        Some(Json::String(kind)) => {
            RecordKind::from_str(&kind).map_err(|_| format!("`{kind}` is not a record kind"))?
        }
        _ => return Err(format!("a record needs the text member `{KIND_MEMBER}`")),
    };
    let fields = match object.remove(FIELDS_MEMBER) {
        None => Map::new(),
        Some(Json::Object(fields)) => fields,
        Some(_) => return Err(format!("the member `{FIELDS_MEMBER}` must be an object")),
    };
    if let Some(name) = object.keys().next() {
        return Err(format!("unknown member `{name}` of a record"));
    }
    let fields: Fields = fields
        .into_iter()
        .map(|(name, value)| (name, Value::from_plain_json(value)))
        .collect();
    Ok(Record { kind, fields })
}

/// JSON form of the arguments of a stage.
///
/// Per-record expressions are evaluated once, without a record.
///
/// # Errors
///
/// Fails with `E_TYPE` for a pipeline argument and with the error of an
/// expression evaluation.
pub fn args_to_json(args: &BoundArgs, ctx: &ExecCtx) -> Result<String> {
    let mut object = Map::new();
    for name in args.names() {
        if let Some(value) = args.eval(name, None, ctx)? {
            object.insert(name.to_owned(), value.to_plain_json());
        }
    }
    Ok(Json::Object(object).to_string())
}

#[cfg(test)]
mod tests {
    use nurt_contracts::bound::{BoundHead, BoundPipeline, BoundRef};
    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::stage::{ArgValue, BoundArgs};
    use nurt_contracts::value::{Record, RecordKind, Value};
    use nurt_testkit::ctx::TestContext;

    use super::{args_to_json, record_from_json, record_to_json};

    #[test]
    fn records_survive_the_json_form() {
        let record = Record::new(RecordKind::Finding)
            .with("line", Value::from(3_i64))
            .with("ok", Value::from(true))
            .with("tags", Value::from(vec![Value::from("x")]));
        assert_eq!(record_from_json(&record_to_json(&record)).unwrap(), record);
        let bare = record_from_json(r#"{"kind":"item"}"#).unwrap();
        assert!(bare.is_empty());
    }

    #[test]
    fn malformed_records_are_described() {
        let cases = [
            ("not json", "invalid JSON"),
            ("[1]", "JSON object"),
            (r#"{"fields":{}}"#, "`kind`"),
            (r#"{"kind":"nothing"}"#, "not a record kind"),
            (r#"{"kind":"item","fields":1}"#, "must be an object"),
            (r#"{"kind":"item","extra":1}"#, "unknown member `extra`"),
        ];
        for (text, expected) in cases {
            let problem = record_from_json(text).unwrap_err();
            assert!(problem.contains(expected), "{text}: {problem}");
        }
    }

    #[test]
    fn arguments_become_one_object() {
        let context = TestContext::new();
        let ctx = context.ctx();
        let args = BoundArgs::new("acme.lint")
            .with("limit", ArgValue::Value(Value::from(5_i64)), false)
            .with("strict", ArgValue::Value(Value::from(true)), true)
            .with("path", ArgValue::Absent, false);
        assert_eq!(
            args_to_json(&args, &ctx).unwrap(),
            r#"{"limit":5,"strict":true}"#
        );
        let reference = BoundPipeline {
            head: BoundHead::Ref(BoundRef {
                name: "files".to_owned(),
                fields: Vec::new(),
            }),
            stages: Vec::new(),
            span: None,
        };
        let pipeline =
            BoundArgs::new("acme.lint").with("from", ArgValue::Pipeline(reference), false);
        assert_eq!(
            args_to_json(&pipeline, &ctx).unwrap_err().code(),
            ErrorCode::Type
        );
    }
}
