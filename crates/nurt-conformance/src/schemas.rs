// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Published schemas and validation.
//!
//! The set holds the schemas of the language formats published by
//! `nurt-contracts` and the schemas of the suite's own formats.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::Error;

/// Name that selects validation against the JSON Schema 2020-12 meta-schema.
pub const METASCHEMA: &str = "metaschema";

/// Schema of conformance cases `nurt-conformance-case/1`.
pub const CASE: &str = include_str!("../../../conformance/schema/conformance-case-1.schema.json");

/// Schema of file system snapshots `nurt-fs-snapshot/1`.
pub const FS_SNAPSHOT: &str = include_str!("../../../conformance/schema/fs-snapshot-1.schema.json");

/// Schema of binding cases `nurt-binding-cases/1`.
pub const BINDING_CASES: &str =
    include_str!("../../../conformance/schema/binding-cases-1.schema.json");

/// Schema of conformance reports `nurt-conformance-report/1`.
pub const CONFORMANCE_REPORT: &str =
    include_str!("../../../conformance/schema/conformance-report-1.schema.json");

/// Formats of the suite and their schemas.
pub const SUITE_SCHEMAS: &[(&str, &str)] = &[
    ("conformance-case", CASE),
    ("fs-snapshot", FS_SNAPSHOT),
    ("binding-cases", BINDING_CASES),
    ("conformance-report", CONFORMANCE_REPORT),
];

/// Compiled validators of the published schemas.
pub struct SchemaSet {
    documents: BTreeMap<&'static str, Value>,
    validators: BTreeMap<&'static str, jsonschema::Validator>,
}

impl std::fmt::Debug for SchemaSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchemaSet")
            .field("names", &self.documents.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl SchemaSet {
    /// Parses and compiles every schema.
    ///
    /// # Errors
    ///
    /// Returns an error when a schema is not valid JSON or does not compile.
    pub fn new() -> Result<Self, Error> {
        let mut documents = BTreeMap::new();
        let mut validators = BTreeMap::new();
        let all = nurt_contracts::schema::ALL.iter().chain(SUITE_SCHEMAS);
        for (name, text) in all {
            let document: Value = serde_json::from_str(text)
                .map_err(|e| Error::Invalid(format!("schema {name}: {e}")))?;
            let validator = jsonschema::validator_for(&document)
                .map_err(|e| Error::Invalid(format!("schema {name}: {e}")))?;
            documents.insert(*name, document);
            validators.insert(*name, validator);
        }
        Ok(Self {
            documents,
            validators,
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
    use serde_json::json;

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
    fn validation_reports_messages_and_unknown_names() {
        let set = SchemaSet::new().unwrap();
        let snapshot = json!({"entries": [{"path": "a", "type": "file", "content_utf8": "x"}]});
        assert_eq!(set.validate("fs-snapshot", &snapshot), Ok(()));
        let invalid = json!({"entries": [{"path": "/a", "type": "file"}]});
        assert!(
            !set.validate("fs-snapshot", &invalid)
                .unwrap_err()
                .is_empty()
        );
        assert!(set.validate("none", &json!({})).unwrap_err()[0].contains("unknown schema"));
        assert!(set.validate(METASCHEMA, &json!({"type": 12})).is_err());
        assert!(format!("{set:?}").contains("fs-snapshot"));
    }
}
