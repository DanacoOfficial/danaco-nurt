// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Keys of YAML mappings: merge keys `<<` and explicit keys `? `.
//!
//! A merge key inserts the entries of other mappings into the mapping that
//! holds it (YAML 1.1 merge type): `<<: *base` merges one mapping, and
//! `<<: [*a, *b]` merges several, the earlier ones taking precedence. Keys
//! written in the mapping itself take precedence over every merged key, and
//! merged mappings may hold merge keys of their own.
//!
//! Reading a merged key yields the value in the source mapping. Setting a
//! merged key writes an explicit key into the mapping instead of changing
//! the shared source ([`MERGE_OVERRIDE_FLAG`]). Removing a key that exists
//! only through a merge key is refused with `yaml.merge_inherited_remove`,
//! because it would require a change of the shared source.
//!
//! An explicit key (`? key`) is read like any key and the value under it is
//! changed in place; a change of the key itself is refused with
//! `yaml.complex_key`.

use std::collections::HashSet;

use indexmap::IndexMap;
use nurt_contracts::error::Result;

use super::anchor::Anchors;
use super::model::{MapEntry, NodeId, NodeKind, YamlModel};
use crate::constructs::{self, YAML_COMPLEX_KEY, YAML_MERGE_INHERITED_REMOVE};
use crate::edit::ByteEdit;

/// Flag of a change that wrote an explicit key over a merged key.
pub const MERGE_OVERRIDE_FLAG: &str = "merge_override";

/// Longest plain key in bytes; a longer key, or one with a line break, can
/// only be written as an explicit key (YAML 1.2, section 7.4.2).
pub const MAX_IMPLICIT_KEY_BYTES: usize = 1024;

/// Deepest chain of merge keys followed through merged mappings.
pub const MAX_MERGE_DEPTH: usize = 64;

/// Key of a mapping that comes from a merged mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inherited {
    /// Name of the key.
    pub name: String,
    /// Value node in the source mapping.
    pub value: NodeId,
    /// Mapping that holds the entry.
    pub source: NodeId,
}

/// Name of a key node as used in key paths.
///
/// A scalar key is its value, an alias key the name of its target, and a
/// collection key its flow form (`[a, b]`, `{x: 1}`).
pub fn key_name(model: &YamlModel, key: NodeId) -> Option<String> {
    key_name_with(model, model.anchors(), key)
}

pub(crate) fn key_name_with(model: &YamlModel, anchors: &Anchors, key: NodeId) -> Option<String> {
    let target = anchors.resolve(key)?;
    let mut text = String::new();
    write_flow(model, anchors, target, &mut text, 0)?;
    Some(text)
}

fn write_flow(
    model: &YamlModel,
    anchors: &Anchors,
    node: NodeId,
    out: &mut String,
    depth: usize,
) -> Option<()> {
    if depth > MAX_MERGE_DEPTH {
        return None;
    }
    let node = anchors.resolve(node)?;
    match &model.node(node).kind {
        NodeKind::Scalar(scalar) => out.push_str(&scalar.value),
        NodeKind::Sequence(sequence) => {
            out.push('[');
            for (index, item) in sequence.items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_flow(model, anchors, item.node, out, depth + 1)?;
            }
            out.push(']');
        }
        NodeKind::Mapping(mapping) => {
            out.push('{');
            for (index, entry) in mapping.entries.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_flow(model, anchors, entry.key, out, depth + 1)?;
                out.push_str(": ");
                write_flow(model, anchors, entry.value, out, depth + 1)?;
            }
            out.push('}');
        }
        NodeKind::Alias(_) => return None,
    }
    Some(())
}

/// Mappings merged into `map`, highest precedence first.
///
/// The value of each merge key is followed through aliases; a sequence
/// contributes its mappings in order. Values that are not mappings are
/// skipped.
pub fn merge_sources(model: &YamlModel, map: NodeId) -> Vec<NodeId> {
    sources_with(model, model.anchors(), map)
}

fn sources_with(model: &YamlModel, anchors: &Anchors, map: NodeId) -> Vec<NodeId> {
    let Some(mapping) = model.node(map).as_mapping() else {
        return Vec::new();
    };
    let mut sources = Vec::new();
    for entry in mapping.entries.iter().filter(|entry| entry.merge) {
        let Some(value) = anchors.resolve(entry.value) else {
            continue;
        };
        match &model.node(value).kind {
            NodeKind::Mapping(_) => sources.push(value),
            NodeKind::Sequence(sequence) => sources.extend(
                sequence
                    .items
                    .iter()
                    .filter_map(|item| anchors.resolve(item.node))
                    .filter(|&node| model.node(node).as_mapping().is_some()),
            ),
            NodeKind::Scalar(_) | NodeKind::Alias(_) => {}
        }
    }
    sources
}

/// Entry written in `map` itself for the key `name`; the last one when the
/// key repeats. Merge keys are not entries of this kind.
pub fn local_entry<'m>(model: &'m YamlModel, map: NodeId, name: &str) -> Option<&'m MapEntry> {
    local_entry_with(model, model.anchors(), map, name)
}

fn local_entry_with<'m>(
    model: &'m YamlModel,
    anchors: &Anchors,
    map: NodeId,
    name: &str,
) -> Option<&'m MapEntry> {
    model
        .node(map)
        .as_mapping()?
        .entries
        .iter()
        .rev()
        .filter(|entry| !entry.merge)
        .find(|entry| key_name_with(model, anchors, entry.key).as_deref() == Some(name))
}

/// Keys of `map` that come from merged mappings and are not written in
/// `map` itself, in order of precedence of their sources.
pub fn inherited_members(model: &YamlModel, map: NodeId) -> Vec<Inherited> {
    let mut members = IndexMap::new();
    let mut visiting = HashSet::new();
    collect(model, model.anchors(), map, 0, &mut visiting, &mut members);
    members
        .into_values()
        .filter(|member: &Inherited| member.source != map)
        .collect()
}

/// Effective entries of `map` with the first-wins rule: local entries, then
/// each merged mapping in order of precedence.
fn collect(
    model: &YamlModel,
    anchors: &Anchors,
    map: NodeId,
    depth: usize,
    visiting: &mut HashSet<NodeId>,
    members: &mut IndexMap<String, Inherited>,
) {
    if depth > MAX_MERGE_DEPTH || !visiting.insert(map) {
        return;
    }
    if let Some(mapping) = model.node(map).as_mapping() {
        for entry in mapping.entries.iter().filter(|entry| !entry.merge) {
            if let Some(name) = key_name_with(model, anchors, entry.key) {
                members.entry(name.clone()).or_insert(Inherited {
                    name,
                    value: entry.value,
                    source: map,
                });
            }
        }
    }
    for source in sources_with(model, anchors, map) {
        collect(model, anchors, source, depth + 1, visiting, members);
    }
    visiting.remove(&map);
}

/// Value of the key `name` of `map` that comes from a merged mapping;
/// `None` when the key is written in `map` or is not merged.
pub fn inherited_value(model: &YamlModel, map: NodeId, name: &str) -> Option<NodeId> {
    inherited_members(model, map)
        .into_iter()
        .find(|member| member.name == name)
        .map(|member| member.value)
}

/// Returns `true` when setting the key `name` of `map` writes an explicit
/// key over a merged one, which the change reports with
/// [`MERGE_OVERRIDE_FLAG`].
pub fn is_override(model: &YamlModel, map: NodeId, name: &str) -> bool {
    local_entry(model, map, name).is_none() && inherited_value(model, map, name).is_some()
}

/// Checks the removal of the key `name` from `map`.
///
/// # Errors
///
/// Returns `E_UNSUPPORTED_CONSTRUCT` `yaml.merge_inherited_remove` when the
/// key exists only through a merge key.
pub fn check_remove(model: &YamlModel, map: NodeId, name: &str) -> Result<()> {
    if is_override(model, map, name) {
        return Err(constructs::unsupported(
            YAML_MERGE_INHERITED_REMOVE,
            format!(
                "key `{name}` comes from a mapping merged with `<<` and is shared with other mappings; remove it from the merged mapping or set it to null"
            ),
        ));
    }
    Ok(())
}

/// Checks that no edit touches an explicit key (`? key`) or its indicator.
///
/// # Errors
///
/// Returns `E_UNSUPPORTED_CONSTRUCT` `yaml.complex_key` for the first edit
/// that changes bytes of an explicit key.
pub fn guard_key_edits(model: &YamlModel, edits: &[ByteEdit]) -> Result<()> {
    for id in model.node_ids() {
        let Some(mapping) = model.node(id).as_mapping() else {
            continue;
        };
        for entry in &mapping.entries {
            let Some(question) = &entry.question else {
                continue;
            };
            let key = model.node(entry.key).full_span();
            let start = question.start;
            let end = key.end.max(question.end);
            if edits.iter().any(|edit| touches(edit, start, end)) {
                return Err(constructs::unsupported(
                    YAML_COMPLEX_KEY,
                    format!(
                        "the explicit key `{}` cannot be changed; change the value under it or edit the key text",
                        model.text(&(start..end)).trim()
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn touches(edit: &ByteEdit, start: usize, end: usize) -> bool {
    let span = edit.span;
    if span.is_empty() {
        span.start > start && span.start < end
    } else {
        span.start < end && start < span.end
    }
}

/// Checks that a new key can be written as an implicit key.
///
/// # Errors
///
/// Returns `E_UNSUPPORTED_CONSTRUCT` `yaml.complex_key` for a key with a
/// line break or longer than [`MAX_IMPLICIT_KEY_BYTES`].
pub fn check_new_key(name: &str) -> Result<()> {
    if name.contains(['\n', '\r']) || name.len() > MAX_IMPLICIT_KEY_BYTES {
        return Err(constructs::unsupported(
            YAML_COMPLEX_KEY,
            format!(
                "a key with a line break or longer than {MAX_IMPLICIT_KEY_BYTES} bytes needs the explicit form `? key`; use a shorter single-line key"
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;
    use crate::model::Span;

    fn parse(text: &str) -> YamlModel {
        YamlModel::parse(text).expect("valid YAML")
    }

    fn root(model: &YamlModel) -> NodeId {
        model.documents()[0].root
    }

    fn value_node(model: &YamlModel, key: &str) -> NodeId {
        model.map_get(root(model), key).expect("key exists")
    }

    #[test]
    fn finds_no_merged_keys_in_a_node_that_is_not_a_mapping() {
        let model = parse("a: 1\nb: [x]\n");
        for node in [value_node(&model, "a"), value_node(&model, "b")] {
            assert!(merge_sources(&model, node).is_empty());
            assert!(inherited_members(&model, node).is_empty());
            assert!(local_entry(&model, node, "x").is_none());
            assert_eq!(inherited_value(&model, node, "x"), None);
        }
    }

    #[test]
    fn skips_merge_values_that_are_not_mappings() {
        let model =
            parse("base: &base {a: 1}\nmap:\n  <<: [*base, plain]\n  b: 2\nother:\n  <<: text\n");
        let map = value_node(&model, "map");
        assert_eq!(merge_sources(&model, map), vec![value_node(&model, "base")]);
        assert!(merge_sources(&model, value_node(&model, "other")).is_empty());
        assert!(is_override(&model, map, "a"));
        assert!(!is_override(&model, map, "b"));
        assert!(check_remove(&model, map, "b").is_ok());
        let error = check_remove(&model, map, "a").expect_err("inherited key");
        assert_eq!(error.found.as_deref(), Some(YAML_MERGE_INHERITED_REMOVE));
    }

    #[test]
    fn stops_following_merge_chains_past_the_depth_limit() {
        let levels = MAX_MERGE_DEPTH + 2;
        let mut text = String::from("m0: &m0 {k0: 0}\n");
        for level in 1..=levels {
            writeln!(
                text,
                "m{level}: &m{level} {{<<: *m{}, k{level}: {level}}}",
                level - 1
            )
            .expect("write to a string");
        }
        let model = parse(&text);
        let top = value_node(&model, &format!("m{levels}"));
        let names: Vec<String> = inherited_members(&model, top)
            .into_iter()
            .map(|member| member.name)
            .collect();
        assert!(names.contains(&format!("k{}", levels - MAX_MERGE_DEPTH)));
        assert!(!names.contains(&format!("k{}", levels - MAX_MERGE_DEPTH - 1)));
    }

    #[test]
    fn leaves_keys_nested_past_the_depth_limit_unnamed() {
        let depth = MAX_MERGE_DEPTH + 2;
        let key = format!("{}x{}", "[".repeat(depth), "]".repeat(depth));
        let model = parse(&format!("{key}: 1\nshort: 2\n"));
        let mapping = model.node(root(&model)).as_mapping().expect("mapping");
        assert_eq!(key_name(&model, mapping.entries[0].key), None);
        assert_eq!(
            key_name(&model, mapping.entries[1].key).as_deref(),
            Some("short")
        );
    }

    #[test]
    fn allows_edits_at_the_edges_of_an_explicit_key_only() {
        let model = parse("? a\n: 1\nb: 2\n");
        let allowed = [
            ByteEdit::insert(0, "c: 3\n"),
            ByteEdit::insert(3, " "),
            ByteEdit::replace(Span::new(6, 7), "5"),
            ByteEdit::delete(Span::new(8, 13)),
        ];
        for edit in allowed {
            assert!(guard_key_edits(&model, &[edit]).is_ok());
        }
        let refused = [
            ByteEdit::insert(2, "z"),
            ByteEdit::delete(Span::new(2, 3)),
            ByteEdit::replace(Span::new(0, 1), "-"),
        ];
        for edit in refused {
            let error = guard_key_edits(&model, &[edit]).expect_err("explicit key");
            assert_eq!(error.found.as_deref(), Some(YAML_COMPLEX_KEY));
        }
    }

    #[test]
    fn refuses_new_keys_that_need_the_explicit_form() {
        assert!(check_new_key("name").is_ok());
        assert!(check_new_key("two\nlines").is_err());
        assert!(check_new_key(&"k".repeat(MAX_IMPLICIT_KEY_BYTES + 1)).is_err());
    }
}
