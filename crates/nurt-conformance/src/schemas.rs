// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-conformance::schemas
// Responsibility:   Published schemas and validation.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Published schemas and validation.
//!
//! The set holds the schemas of the formats published by `nurt-contracts`,
//! among them `nurt-rpc/1`, `nurt-response/1`, `nurt-event/1`, `nurt-check/1`
//! and `nurt-fs-snapshot/1`, and the schemas of the suite's own formats.
//! References between the published schemas are resolved by their `$id`.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use nurt_contracts::schema::ID_BASE;
use serde_json::{Value, json};

use crate::Error;
use crate::snapshot;

/// Name that selects validation against the JSON Schema 2020-12 meta-schema.
pub const METASCHEMA: &str = "metaschema";

/// Schema of conformance cases `nurt-conformance-case/1`.
pub const CASE: &str = include_str!("../../../conformance/schema/conformance-case-1.schema.json");

/// Schema of binding cases `nurt-binding-cases/1`.
pub const BINDING_CASES: &str =
    include_str!("../../../conformance/schema/binding-cases-1.schema.json");

/// Schema of conformance reports `nurt-conformance-report/1`.
pub const CONFORMANCE_REPORT: &str =
    include_str!("../../../conformance/schema/conformance-report-1.schema.json");

/// Formats of the suite and their schemas.
pub const SUITE_SCHEMAS: &[(&str, &str)] = &[
    ("conformance-case", CASE),
    ("binding-cases", BINDING_CASES),
    ("conformance-report", CONFORMANCE_REPORT),
];

/// Compiled validators of the published schemas.
pub struct SchemaSet {
    documents: BTreeMap<&'static str, Value>,
    validators: BTreeMap<&'static str, jsonschema::Validator>,
    results: BTreeMap<String, jsonschema::Validator>,
}

impl std::fmt::Debug for SchemaSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchemaSet")
            .field("names", &self.documents.keys().collect::<Vec<_>>())
            .finish()
    }
}

fn invalid(name: &str, error: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("schema {name}: {error}"))
}

fn messages(validator: &jsonschema::Validator, instance: &Value) -> Result<(), Vec<String>> {
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|error| format!("{}: {error}", error.instance_path()))
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

impl SchemaSet {
    /// Returns the set shared by the process, compiled on first use.
    ///
    /// # Errors
    ///
    /// Returns an error when a schema is not valid JSON or does not compile.
    pub fn shared() -> Result<&'static Self, Error> {
        static SHARED: OnceLock<Result<SchemaSet, String>> = OnceLock::new();
        SHARED
            .get_or_init(|| Self::new().map_err(|error| error.to_string()))
            .as_ref()
            .map_err(|message| Error::Invalid(message.clone()))
    }

    /// Parses and compiles every schema.
    ///
    /// # Errors
    ///
    /// Returns an error when a schema is not valid JSON or does not compile.
    pub fn new() -> Result<Self, Error> {
        let mut documents = BTreeMap::new();
        let mut parsed = Vec::new();
        for (name, text) in nurt_contracts::schema::ALL.iter().chain(SUITE_SCHEMAS) {
            let document: Value = serde_json::from_str(text).map_err(|e| invalid(name, e))?;
            parsed.push((*name, document));
        }
        let mut registry = jsonschema::Registry::new();
        for (name, document) in parsed.iter().take(nurt_contracts::schema::ALL.len()) {
            let uri = format!("{ID_BASE}nurt-{name}-1.schema.json");
            registry = registry
                .add(uri, document.clone())
                .map_err(|e| invalid(name, e))?;
        }
        let registry = registry.prepare().map_err(|e| invalid("registry", e))?;
        let options = jsonschema::options().with_registry(&registry);
        let mut validators = BTreeMap::new();
        for (name, document) in parsed {
            let validator = options.build(&document).map_err(|e| invalid(name, e))?;
            validators.insert(name, validator);
            documents.insert(name, document);
        }
        let rpc = documents
            .get("rpc")
            .ok_or_else(|| Error::Invalid("schema rpc is missing".to_owned()))?;
        let id = rpc.get("$id").and_then(Value::as_str).unwrap_or_default();
        let mut results = BTreeMap::new();
        let defined = rpc
            .pointer("/$defs/results/properties")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|properties| properties.keys());
        for method in defined {
            let reference = json!({"$ref": format!("{id}#/$defs/results/properties/{method}")});
            let validator = options
                .build(&reference)
                .map_err(|e| invalid(&format!("rpc result {method}"), e))?;
            results.insert(method.clone(), validator);
        }
        Ok(Self {
            documents,
            validators,
            results,
        })
    }

    /// Returns the names of the schemas, without [`METASCHEMA`].
    #[must_use]
    pub fn names(&self) -> Vec<&'static str> {
        self.documents.keys().copied().collect()
    }

    /// Returns the schema document with the given name.
    #[must_use]
    pub fn document(&self, name: &str) -> Option<&Value> {
        self.documents.get(name)
    }

    /// Validates `instance` against the schema with the given name, or
    /// against the JSON Schema 2020-12 meta-schema for [`METASCHEMA`].
    ///
    /// # Errors
    ///
    /// Returns the validation messages, or one message naming an unknown
    /// schema.
    pub fn validate(&self, name: &str, instance: &Value) -> Result<(), Vec<String>> {
        if name == METASCHEMA {
            return validate_metaschema(instance).map_err(|message| vec![message]);
        }
        let validator = self
            .validators
            .get(name)
            .ok_or_else(|| vec![format!("unknown schema {name:?}")])?;
        messages(validator, instance)
    }

    /// Validates a message of `nurt-rpc/1`: a request, a reply or an event
    /// notification.
    ///
    /// # Errors
    ///
    /// Returns the validation messages.
    pub fn validate_message(&self, message: &Value) -> Result<(), Vec<String>> {
        self.validate("rpc", message)
    }

    /// Validates the result of a method against the definition of `nurt-rpc/1`
    /// for that method. A method that the protocol does not define, such as
    /// the methods of the feature `testkit`, is not checked.
    ///
    /// # Errors
    ///
    /// Returns the validation messages.
    pub fn validate_result(&self, method: &str, result: &Value) -> Result<(), Vec<String>> {
        self.results
            .get(method)
            .map_or(Ok(()), |validator| messages(validator, result))
    }

    /// Validates a snapshot against `nurt-fs-snapshot/1`.
    ///
    /// The member `exhaustive`, which only the suite uses, is ignored, and
    /// `format` may be omitted, as in a snapshot embedded in a case.
    ///
    /// # Errors
    ///
    /// Returns the validation messages.
    pub fn validate_snapshot(&self, snapshot: &Value) -> Result<(), Vec<String>> {
        let mut document = snapshot.clone();
        if let Some(members) = document.as_object_mut() {
            members
                .entry("format")
                .or_insert_with(|| json!(snapshot::FORMAT));
        }
        self.validate("fs-snapshot", &document)
    }
}

/// Validates a schema document against the JSON Schema 2020-12 meta-schema.
///
/// # Errors
///
/// Returns the first violation.
pub fn validate_metaschema(schema: &Value) -> Result<(), String> {
    jsonschema::draft202012::meta::validate(schema).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_schema_is_a_valid_2020_12_schema() {
        let set = SchemaSet::new().unwrap();
        assert_eq!(
            set.names().len(),
            SUITE_SCHEMAS.len() + nurt_contracts::schema::ALL.len()
        );
        assert!(set.document("tool").is_some());
        for name in set.names() {
            let document = set.document(name).unwrap();
            assert_eq!(validate_metaschema(document), Ok(()), "{name}");
            assert_eq!(set.validate(METASCHEMA, document), Ok(()), "{name}");
        }
    }

    #[test]
    fn snapshots_follow_the_published_schema() {
        let set = SchemaSet::shared().unwrap();
        let embedded = json!({"exhaustive": false, "entries": [
            {"path": "a/b", "type": "file", "content_utf8": "x", "mode": "0644"},
            {"path": "l", "type": "symlink", "target": "a"},
        ]});
        assert_eq!(set.validate_snapshot(&embedded), Ok(()));
        for invalid in [
            json!({"entries": [{"path": "/a", "type": "file"}]}),
            json!({"entries": [{"path": "a", "type": "file", "mode": "644"}]}),
            json!({"entries": [{"path": "a", "type": "symlink"}]}),
            json!({"entries": [{"path": "a", "type": "dir", "content_utf8": ""}]}),
        ] {
            assert!(set.validate_snapshot(&invalid).is_err(), "{invalid}");
        }
        assert!(set.validate("fs-snapshot", &embedded).is_err());
    }

    #[test]
    fn messages_and_results_follow_the_protocol_schema() {
        let set = SchemaSet::shared().unwrap();
        let request =
            json!({"jsonrpc": "2.0", "id": 1, "method": "schema", "params": {"name": "ast"}});
        assert_eq!(set.validate_message(&request), Ok(()));
        let renamed =
            json!({"jsonrpc": "2.0", "id": 1, "method": "schema", "params": {"which": "ast"}});
        assert!(set.validate_message(&renamed).is_err());
        let reply = json!({"jsonrpc": "2.0", "id": 1, "result": {}});
        assert_eq!(set.validate_message(&reply), Ok(()));
        assert!(
            set.validate_message(&json!({"jsonrpc": "2.0", "result": {}}))
                .is_err()
        );
        assert_eq!(
            set.validate_result("cancel", &json!({"cancelled": true})),
            Ok(())
        );
        assert!(set.validate_result("cancel", &json!({})).is_err());
        assert!(set.validate_result("print", &json!({})).is_err());
        assert_eq!(
            set.validate_result("testkit.clock.advance", &json!(1)),
            Ok(())
        );
    }

    #[test]
    fn validation_reports_messages_and_unknown_names() {
        let set = SchemaSet::new().unwrap();
        assert!(set.validate("none", &json!({})).unwrap_err()[0].contains("unknown schema"));
        assert!(set.validate(METASCHEMA, &json!({"type": 12})).is_err());
        assert!(format!("{set:?}").contains("fs-snapshot"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
