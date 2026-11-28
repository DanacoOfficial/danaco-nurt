// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-bind::fields
// Responsibility:   Fields of the typed record kinds, with their types.
// Dependencies:     nurt_contracts, nurt_types
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Fields of the typed record kinds, with their types.
//!
//! Names and types are read from the typed record structures of the
//! contract (`TypedRecord::field_types`), so there is no second list to keep
//! in step. A kind whose structure is not typed has no list, and its fields
//! are not checked.

use std::collections::HashMap;

use nurt_contracts::record::{FieldTypes as KindFields, TypedRecord};
use nurt_contracts::spec::IoSpec;
use nurt_contracts::value::RecordKind;
use nurt_types::ValueType;

use crate::binder::Flow;

/// Types of the fields of the records of a flow, by field name; a field whose
/// type is not one language type, or differs between the kinds of the flow,
/// is absent.
pub(crate) type FieldTypes = HashMap<&'static str, ValueType>;

/// Returns the fields of a record kind with the names of their types, or
/// `None` for a kind without a typed structure.
fn record_fields(kind: &str) -> Option<KindFields> {
    kind.parse::<RecordKind>()
        .ok()
        .and_then(TypedRecord::field_types)
}

/// Returns the fields of every kind of a flow, or `None` when the flow is not
/// a stream of records of typed kinds.
fn flow_kinds(flow: &Flow) -> Option<Vec<KindFields>> {
    let Flow::Records(kinds) = flow else {
        return None;
    };
    kinds
        .iter()
        .map(|kind| match kind {
            IoSpec::Record(name) => record_fields(name),
            _ => None,
        })
        .collect()
}

/// Returns the fields of the records of a flow: the union of the fields of
/// its kinds, or `None` when one of the kinds has no known fields.
pub(crate) fn flow_fields(flow: &Flow) -> Option<Vec<&'static str>> {
    let mut fields: Vec<&'static str> = Vec::new();
    for &(name, _) in flow_kinds(flow)?.into_iter().flatten() {
        if !fields.contains(&name) {
            fields.push(name);
        }
    }
    (!fields.is_empty()).then_some(fields)
}

/// Returns the types of the fields of the records of a flow; empty when the
/// kinds of the flow are not known.
pub(crate) fn flow_field_types(flow: &Flow) -> FieldTypes {
    let mut types: HashMap<&'static str, Option<ValueType>> = HashMap::new();
    for &(name, type_name) in flow_kinds(flow).into_iter().flatten().flatten() {
        let ty = type_name.parse::<ValueType>().ok();
        types
            .entry(name)
            .and_modify(|known| {
                if *known != ty {
                    *known = None;
                }
            })
            .or_insert(ty);
    }
    types
        .into_iter()
        .filter_map(|(name, ty)| ty.map(|ty| (name, ty)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records(kinds: &[&str]) -> Flow {
        Flow::Records(
            kinds
                .iter()
                .map(|kind| IoSpec::Record((*kind).to_owned()))
                .collect(),
        )
    }

    #[test]
    fn reads_the_fields_of_an_entry() {
        let fields = record_fields("entry").unwrap();
        for name in ["path", "name", "extension", "type", "size", "modified"] {
            assert!(fields.iter().any(|(field, _)| *field == name), "{name}");
        }
    }

    #[test]
    fn reads_the_fields_of_processes_and_jobs() {
        let has = |kind: &str, name: &str| {
            record_fields(kind)
                .unwrap()
                .iter()
                .any(|(field, _)| *field == name)
        };
        assert!(has("process", "memory"));
        assert!(has("job", "exit_code"));
    }

    #[test]
    fn has_no_fields_for_an_unknown_kind() {
        assert!(record_fields("nothing").is_none());
    }

    #[test]
    fn joins_the_fields_of_a_flow() {
        let fields = flow_fields(&records(&["line", "entry"])).unwrap();
        assert!(fields.contains(&"text"));
        assert!(fields.contains(&"size"));
        assert!(flow_fields(&Flow::any()).is_none());
        assert!(flow_fields(&Flow::Start).is_none());
    }

    #[test]
    fn reads_the_types_of_the_fields_of_a_flow() {
        let types = flow_field_types(&records(&["entry"]));
        assert_eq!(types["size"], ValueType::Size);
        assert_eq!(types["modified"], ValueType::Instant);
        assert_eq!(types["path"], ValueType::Path);
        assert_eq!(types["name"], ValueType::Text);
    }

    #[test]
    fn omits_the_fields_whose_type_differs_between_kinds() {
        let entry = flow_field_types(&records(&["entry"]));
        let process = flow_field_types(&records(&["process"]));
        let joined = flow_field_types(&records(&["entry", "process"]));
        for (name, ty) in &entry {
            match process.get(name) {
                Some(other) if other != ty => assert!(!joined.contains_key(name), "{name}"),
                _ => assert_eq!(joined.get(name), Some(ty), "{name}"),
            }
        }
    }

    #[test]
    fn has_no_types_without_known_kinds() {
        assert!(flow_field_types(&Flow::any()).is_empty());
        assert!(flow_field_types(&Flow::Start).is_empty());
        assert!(flow_field_types(&records(&["entry", "nothing"])).is_empty());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
