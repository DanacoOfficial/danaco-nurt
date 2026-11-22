// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Field names of the typed record kinds.
//!
//! The names are read from the typed record structures of the contract: the
//! deserializer of a structure announces its field names before it reads a
//! value, so a deserializer that records the announcement and stops yields
//! the fields of every record kind without a second list to keep in step.
//! A kind whose structure does not announce its fields has no list, and its
//! fields are not checked.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::OnceLock;

use nurt_contracts::record::TypedRecord;
use nurt_contracts::spec::IoSpec;
use serde::Deserialize;
use serde::de::value::Error as ProbeError;
use serde::de::{self, DeserializeSeed, Deserializer, IntoDeserializer, MapAccess, Visitor};

use crate::binder::Flow;

/// Member that names the kind in the JSON form of a typed record.
const TAG: &str = "kind";

/// Member that holds the fields in the JSON form of a typed record.
const CONTENT: &str = "record";

/// Message of the error that ends a probe.
const PROBED: &str = "field names recorded";

/// Field names announced by one structure.
type Names = &'static [&'static str];

/// Returns the field names of a record kind, or `None` for a kind without a
/// typed structure.
pub(crate) fn record_fields(kind: &str) -> Option<Names> {
    static FIELDS: OnceLock<HashMap<&'static str, Names>> = OnceLock::new();
    FIELDS
        .get_or_init(|| {
            TypedRecord::KINDS
                .iter()
                .filter_map(|kind| {
                    let name = kind.as_str();
                    probe(name).map(|fields| (name, fields))
                })
                .collect()
        })
        .get(kind)
        .copied()
}

/// Returns the fields of the records of a flow: the union of the fields of
/// its kinds, or `None` when one of the kinds has no known fields.
pub(crate) fn flow_fields(flow: &Flow) -> Option<Vec<&'static str>> {
    let Flow::Records(kinds) = flow else {
        return None;
    };
    let mut fields: Vec<&'static str> = Vec::new();
    for kind in kinds {
        let IoSpec::Record(name) = kind else {
            return None;
        };
        for field in record_fields(name)? {
            if !fields.contains(field) {
                fields.push(field);
            }
        }
    }
    (!fields.is_empty()).then_some(fields)
}

fn probe(kind: &'static str) -> Option<Names> {
    let found = Cell::new(None);
    let outer = Outer {
        kind,
        found: &found,
    };
    // The probe always ends with an error once the names are recorded.
    let _ = TypedRecord::deserialize(outer);
    found.get()
}

/// Deserializer of the tagged form `{"kind": …, "record": …}`.
struct Outer<'f> {
    kind: &'static str,
    found: &'f Cell<Option<Names>>,
}

impl<'de> Deserializer<'de> for Outer<'_> {
    type Error = ProbeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ProbeError> {
        visitor.visit_map(Members {
            kind: self.kind,
            found: self.found,
            step: 0,
        })
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

/// Members of the tagged form: the kind first, then the record.
struct Members<'f> {
    kind: &'static str,
    found: &'f Cell<Option<Names>>,
    step: u8,
}

impl<'de> MapAccess<'de> for Members<'_> {
    type Error = ProbeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, ProbeError> {
        let key = match self.step {
            0 => TAG,
            1 => CONTENT,
            _ => return Ok(None),
        };
        seed.deserialize(key.into_deserializer()).map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, ProbeError> {
        self.step += 1;
        if self.step == 1 {
            seed.deserialize(self.kind.into_deserializer())
        } else {
            seed.deserialize(Probe { found: self.found })
        }
    }
}

/// Deserializer that records the field names of the structure it is asked
/// for and stops.
struct Probe<'f> {
    found: &'f Cell<Option<Names>>,
}

impl<'de> Deserializer<'de> for Probe<'_> {
    type Error = ProbeError;

    fn deserialize_any<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, ProbeError> {
        Err(de::Error::custom(PROBED))
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, ProbeError> {
        self.found.set(Some(fields));
        Err(de::Error::custom(PROBED))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map enum identifier ignored_any
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_fields_of_an_entry() {
        let fields = record_fields("entry").unwrap();
        for name in ["path", "name", "extension", "type", "size", "modified"] {
            assert!(fields.contains(&name), "{name}");
        }
    }

    #[test]
    fn reads_the_fields_of_processes_and_jobs() {
        assert!(record_fields("process").unwrap().contains(&"memory"));
        assert!(record_fields("job").unwrap().contains(&"exit_code"));
    }

    #[test]
    fn has_no_fields_for_an_unknown_kind() {
        assert!(record_fields("nothing").is_none());
    }

    #[test]
    fn joins_the_fields_of_a_flow() {
        let flow = Flow::Records(vec![
            IoSpec::Record("line".to_owned()),
            IoSpec::Record("entry".to_owned()),
        ]);
        let fields = flow_fields(&flow).unwrap();
        assert!(fields.contains(&"text"));
        assert!(fields.contains(&"size"));
        assert!(flow_fields(&Flow::any()).is_none());
        assert!(flow_fields(&Flow::Start).is_none());
    }
}
