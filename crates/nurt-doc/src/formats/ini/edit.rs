// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Planning and verification of changes to INI documents.

use std::collections::BTreeMap;
use std::ops::Range;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::keypath::{KeyPath, KeySegment};
use nurt_contracts::value::Value;

use super::parse::{Parsed, parse};
use super::profile::{IniProfile, Repeat, Rules};
use super::support::errors::{key_conflict, unsupported};
use super::support::text::{Splice, apply_splices, dominant_newline, line_break_before_append};
use super::support::tree::{Map, Node, normalize_index, resolve};
use super::value::encode;
use super::{BOM_LEN, IniDocument};

/// Construct name: a value the profile cannot write on one line.
const UNREPRESENTABLE: &str = "ini.unrepresentable_value";
/// Construct name: a list or map below the level the profile supports.
const NESTED: &str = "ini.nested_value";
/// Construct name: a scalar assigned to a section.
const SECTION_VALUE: &str = "ini.section_value";
/// Construct name: a key outside any section where the profile forbids it.
const GLOBAL_KEY: &str = "ini.global_key";
/// Construct name: a key name the profile cannot write.
const KEY_NAME: &str = "ini.key_name";
/// Construct name: a section name the profile cannot write.
const SECTION_NAME: &str = "ini.section_name";
/// Construct name: the changed document does not read back as expected.
const VERIFY: &str = "ini.verify";

/// Entry as compared by the verification: section path, key name as
/// compared by the profile, value.
type Signature = (Vec<String>, String, Option<String>);

/// Changes to apply and the entries expected in the changed regions.
#[derive(Debug, Default)]
pub(crate) struct Plan {
    pub(crate) splices: Vec<Splice>,
    pub(crate) count: usize,
    expected: Vec<Signature>,
}

struct Planner<'a> {
    doc: &'a IniDocument,
    rules: Rules,
    newline: &'static str,
    plan: Plan,
    inserts: BTreeMap<usize, String>,
}

/// Plans `set` of `value` at `key`.
pub(crate) fn plan_set(
    doc: &IniDocument,
    key: &KeyPath,
    value: &Value,
    create: bool,
) -> Result<Plan, Error> {
    let mut planner = Planner::new(doc);
    if let Some(parent) = append_parent(doc, key)? {
        planner.append(&parent, value, create)?;
        return Ok(planner.finish());
    }
    let value_of = |node: &Node| doc.node_value(node);
    match resolve(&doc.tree, key, &value_of) {
        Ok(hits) => {
            for hit in &hits {
                planner.replace(hit.node, &section_path_of(&hit.path, doc), value)?;
            }
        }
        Err(error) if error.code() == ErrorCode::KeyNotFound && create && key.is_concrete() => {
            planner.create(&key_names(key)?, value)?;
        }
        Err(error) => return Err(error),
    }
    Ok(planner.finish())
}

/// Plans removal of the values selected by `key`.
pub(crate) fn plan_remove(doc: &IniDocument, key: &KeyPath) -> Result<Plan, Error> {
    if key.is_root() {
        return Err(key_conflict(
            "the document root cannot be removed; address a section or a key",
            "document root",
            &[],
        ));
    }
    let value_of = |node: &Node| doc.node_value(node);
    let hits = resolve(&doc.tree, key, &value_of)?;
    let mut planner = Planner::new(doc);
    let mut sections = Vec::new();
    let mut entries = Vec::new();
    for hit in &hits {
        collect_ids(hit.node, &mut sections, &mut entries);
    }
    sections.sort_unstable();
    sections.dedup();
    for &section in &sections {
        planner.remove_section(section);
    }
    entries.sort_unstable();
    entries.dedup();
    for id in entries {
        if !sections.contains(&doc.parsed.entries[id].section) {
            planner.remove_entry(id);
        }
    }
    Ok(planner.finish())
}

fn collect_ids(node: &Node, sections: &mut Vec<usize>, entries: &mut Vec<usize>) {
    match node {
        Node::Leaf(id) => entries.push(*id),
        Node::Map(map) => {
            sections.extend(&map.ids);
            for (_, child) in &map.members {
                collect_ids(child, sections, entries);
            }
        }
        Node::List(elements) | Node::Repeated(elements) => {
            for element in elements {
                collect_ids(element, sections, entries);
            }
        }
    }
}

/// Parent of a key that ends with `[+]`, or with an index equal to the
/// length of an existing list.
fn append_parent(doc: &IniDocument, key: &KeyPath) -> Result<Option<KeyPath>, Error> {
    let Some(last) = key.segments().last() else {
        return Ok(None);
    };
    let parent = KeyPath::new(key.document(), key.segments()[..key.segments().len() - 1].to_vec())
        .map_err(|error| key_conflict(&error.to_string(), "key path", &[]))?;
    match last {
        KeySegment::Append => Ok(Some(parent)),
        KeySegment::Index(index) if *index >= 0 => {
            let value_of = |node: &Node| doc.node_value(node);
            let length = match resolve(&doc.tree, &parent, &value_of) {
                Ok(hits) => match hits.first().map(|hit| hit.node) {
                    Some(Node::List(elements)) => Some(elements.len()),
                    _ => None,
                },
                Err(_) => None,
            };
            Ok(length
                .filter(|&len| normalize_index(*index, len).is_none() && usize::try_from(*index).ok() == Some(len))
                .map(|_| parent))
        }
        _ => Ok(None),
    }
}

fn key_names(key: &KeyPath) -> Result<Vec<String>, Error> {
    key.segments()
        .iter()
        .map(|segment| match segment {
            KeySegment::Key(name) => Ok(name.clone()),
            _ => Err(key_conflict(
                "a new key is addressed by names only; remove indices and wildcards",
                "segment",
                &[],
            )),
        })
        .collect()
}

/// Section path of the node reached by `path`: the map names, without the
/// final key or index.
fn section_path_of(path: &[KeySegment], doc: &IniDocument) -> Vec<String> {
    let mut names = Vec::new();
    let mut node = &doc.tree;
    for segment in path {
        let KeySegment::Key(name) = segment else {
            break;
        };
        let Node::Map(map) = node else {
            break;
        };
        let Some(position) = map.position(name) else {
            break;
        };
        let (member, child) = &map.members[position];
        if !matches!(child, Node::Map(_)) {
            break;
        }
        names.push(member.clone());
        node = child;
    }
    names
}

/// Text written for a scalar value; `None` for lists, maps and records.
fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::Text(text)
        | Value::BigInt(text)
        | Value::Decimal(text)
        | Value::Key(text)
        | Value::Enum(text)
        | Value::Glob(text) => Some(text.clone()),
        Value::Int(int) => Some(int.to_string()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Null => Some(String::new()),
        Value::Date(date) => Some(date.text.clone()),
        Value::Path(path) => Some(path.as_str().to_owned()),
        _ => None,
    }
}

fn texts(value: &Value) -> Result<Vec<String>, Error> {
    let items: Vec<&Value> = match value {
        Value::List(items) => items.iter().collect(),
        other => vec![other],
    };
    items
        .into_iter()
        .map(|item| {
            scalar_text(item).ok_or_else(|| {
                unsupported(NESTED, "list elements of INI keys are text; write scalars only")
            })
        })
        .collect()
}

impl<'a> Planner<'a> {
    fn new(doc: &'a IniDocument) -> Self {
        Self {
            doc,
            rules: doc.profile.rules(),
            newline: dominant_newline(&doc.text),
            plan: Plan::default(),
            inserts: BTreeMap::new(),
        }
    }

    fn finish(mut self) -> Plan {
        for (offset, text) in std::mem::take(&mut self.inserts) {
            self.plan.splices.push(Splice::insert(offset, text));
        }
        self.plan
    }

    fn parsed(&self) -> &'a Parsed {
        &self.doc.parsed
    }

    fn text(&self) -> &'a str {
        &self.doc.text
    }

    fn folded(&self, name: &str) -> String {
        if self.rules.fold_key_names {
            name.to_ascii_lowercase()
        } else {
            name.to_owned()
        }
    }

    fn is_list_profile(&self) -> bool {
        self.rules.repeat == Repeat::List
    }

    fn insert(&mut self, offset: usize, text: &str) {
        self.inserts.entry(offset).or_default().push_str(text);
    }

    fn replace(&mut self, node: &Node, section: &[String], value: &Value) -> Result<(), Error> {
        match (node, value) {
            (Node::Map(map), Value::Map(fields)) => self.replace_section(map, section, fields),
            (Node::Map(_), _) => Err(unsupported(
                SECTION_VALUE,
                "a section holds keys; set a map value or address a key of the section",
            )),
            (_, Value::Map(_) | Value::Record(_)) => Err(unsupported(
                NESTED,
                "INI keys hold text; maps are possible only as sections",
            )),
            (Node::Leaf(id), Value::List(_)) => {
                let entry = &self.parsed().entries[*id];
                if self.is_list_profile() || entry.list_suffix {
                    self.replace_occurrences(&[*id], &texts(value)?)
                } else {
                    Err(unsupported(NESTED, "this INI profile has no lists; write one value"))
                }
            }
            (Node::Leaf(id), _) => {
                let text = texts(value)?.concat();
                self.set_entry_value(*id, &text)
            }
            (Node::List(elements) | Node::Repeated(elements), _) => {
                let ids: Vec<usize> = elements
                    .iter()
                    .filter_map(|n| match n {
                        Node::Leaf(id) => Some(*id),
                        _ => None,
                    })
                    .collect();
                self.replace_occurrences(&ids, &texts(value)?)
            }
        }
    }

    fn set_entry_value(&mut self, id: usize, value: &str) -> Result<(), Error> {
        let entry = &self.parsed().entries[id];
        self.plan.count += 1;
        if entry.value.as_deref() == Some(value) {
            return Ok(());
        }
        let section = self.parsed().sections[entry.section].path.clone();
        self.plan
            .expected
            .push((section, self.folded(&entry.name), Some(value.to_owned())));
        let old_raw = entry
            .value
            .as_ref()
            .map(|_| &self.text()[entry.value_range.clone()]);
        let encoded = encode(self.doc.profile, value, old_raw).ok_or_else(|| {
            unsupported(
                UNREPRESENTABLE,
                "the value has line breaks, edge whitespace or a final backslash that this INI profile cannot write",
            )
        })?;
        if entry.value.is_some() {
            self.plan
                .splices
                .push(Splice::new(entry.value_range.clone(), encoded));
        } else {
            let separator = self.separator_near(Some(entry.section));
            self.insert(entry.name_range.end, &format!("{separator}{encoded}"));
        }
        Ok(())
    }

    fn replace_occurrences(&mut self, ids: &[usize], values: &[String]) -> Result<(), Error> {
        for (id, value) in ids.iter().zip(values) {
            self.set_entry_value(*id, value)?;
        }
        for &id in ids.iter().skip(values.len()) {
            self.remove_entry(id);
        }
        if let (Some(&last), true) = (ids.last(), values.len() > ids.len()) {
            let entry = &self.parsed().entries[last];
            let (name, section) = (entry.name.clone(), entry.section);
            let lines = values[ids.len()..]
                .iter()
                .map(|value| self.entry_line(&name, value, section, Some(last)))
                .collect::<Result<Vec<_>, _>>()?;
            self.insert_after_line(entry.last_line, &lines);
        }
        Ok(())
    }

    fn replace_section(
        &mut self,
        map: &Map,
        section: &[String],
        fields: &nurt_contracts::value::Fields,
    ) -> Result<(), Error> {
        let mut new_lines = Vec::new();
        for (name, value) in fields {
            match map.get(name) {
                Some(node) => {
                    let mut path = section.to_vec();
                    if matches!(node, Node::Map(_)) {
                        path.push(name.clone());
                    }
                    self.replace(node, &path, value)?;
                }
                None => new_lines.extend(self.new_entry_lines(section, name, value)?),
            }
        }
        for (name, node) in &map.members {
            if !fields.keys().any(|field| field == name) {
                let (mut sections, mut entries) = (Vec::new(), Vec::new());
                collect_ids(node, &mut sections, &mut entries);
                let parsed = self.parsed();
                entries.retain(|&id| !sections.contains(&parsed.entries[id].section));
                for section in sections {
                    self.remove_section(section);
                }
                for id in entries {
                    self.remove_entry(id);
                }
            }
        }
        if !new_lines.is_empty() {
            match map.ids.last() {
                Some(&id) => self.insert_into_section(id, &new_lines),
                None => self.append_section(section, &new_lines)?,
            }
        }
        Ok(())
    }

    /// Lines for a new key `name` with `value` in the section `section`.
    fn new_entry_lines(
        &mut self,
        section: &[String],
        name: &str,
        value: &Value,
    ) -> Result<Vec<String>, Error> {
        self.check_key_name(name)?;
        let values = match value {
            Value::List(_) if self.is_list_profile() => texts(value)?,
            Value::List(_) | Value::Map(_) | Value::Record(_) => {
                return Err(unsupported(
                    NESTED,
                    "INI keys hold text; maps are possible only as sections",
                ));
            }
            other => texts(other)?,
        };
        let anchor = self.section_id(section);
        let reference = anchor.and_then(|id| self.parsed().sections[id].entries.last().copied());
        values
            .iter()
            .map(|text| {
                self.plan.count += 1;
                self.plan
                    .expected
                    .push((section.to_vec(), self.folded(name), Some(text.clone())));
                self.entry_text(name, text, anchor, reference)
            })
            .collect()
    }

    /// Section with the path `path` that receives new keys: the last
    /// occurrence of the section.
    fn section_id(&self, path: &[String]) -> Option<usize> {
        let mut node = &self.doc.tree;
        for name in path {
            let Node::Map(map) = node else {
                return None;
            };
            node = map.get(name)?;
        }
        match node {
            Node::Map(map) if path.is_empty() => Some(0).filter(|_| map.ids.is_empty()),
            Node::Map(map) => map.ids.last().copied(),
            _ => None,
        }
    }

    fn entry_line(
        &mut self,
        name: &str,
        value: &str,
        section: usize,
        reference: Option<usize>,
    ) -> Result<String, Error> {
        self.plan.count += 1;
        let path = self.parsed().sections[section].path.clone();
        self.plan
            .expected
            .push((path, self.folded(name), Some(value.to_owned())));
        self.entry_text(name, value, Some(section), reference)
    }

    /// Text of a new entry line, in the style of `reference` or of the
    /// other entries of the document.
    fn entry_text(
        &self,
        name: &str,
        value: &str,
        section: Option<usize>,
        reference: Option<usize>,
    ) -> Result<String, Error> {
        let encoded = encode(self.doc.profile, value, None).ok_or_else(|| {
            unsupported(
                UNREPRESENTABLE,
                "the value has line breaks, edge whitespace or a final backslash that this INI profile cannot write",
            )
        })?;
        let reference = reference.or_else(|| self.any_entry(section));
        let indent = match reference {
            Some(id) => self.indent_of(id),
            None => self.rules.default_indent.to_owned(),
        };
        let suffix = match reference.map(|id| &self.parsed().entries[id]) {
            Some(entry) if entry.list_suffix && entry.name == name => "[]",
            _ => "",
        };
        let separator = self.separator_near(section);
        Ok(format!("{indent}{name}{suffix}{separator}{encoded}"))
    }

    fn any_entry(&self, section: Option<usize>) -> Option<usize> {
        let parsed = self.parsed();
        section
            .and_then(|s| parsed.sections[s].entries.last().copied())
            .or_else(|| {
                parsed
                    .entries
                    .iter()
                    .position(|e| e.section != 0 || section == Some(0))
            })
    }

    fn indent_of(&self, id: usize) -> String {
        let entry = &self.parsed().entries[id];
        let line = self.parsed().lines[entry.first_line];
        self.text()[line.start..entry.name_range.start]
            .trim_start_matches(super::BOM)
            .to_owned()
    }

    /// Separator text of an entry with a value near `section`, or the
    /// profile default.
    fn separator_near(&self, section: Option<usize>) -> String {
        let parsed = self.parsed();
        let in_section = section
            .into_iter()
            .flat_map(|s| parsed.sections[s].entries.iter().rev().copied());
        in_section
            .chain(0..parsed.entries.len())
            .map(|id| &parsed.entries[id])
            .find(|entry| entry.value.is_some() && !entry.value_range.is_empty())
            .map_or_else(
                || self.rules.default_separator.to_owned(),
                |entry| {
                    let end = entry.name_range.end;
                    let raw = &self.text()[end..entry.value_range.start];
                    raw.trim_start_matches("[]").to_owned()
                },
            )
    }

    fn insert_after_line(&mut self, line: usize, lines: &[String]) {
        let target = self.parsed().lines[line];
        let newline = self.newline;
        let text = if target.end == target.content_end {
            lines.iter().map(|l| format!("{newline}{l}")).collect::<String>()
        } else {
            lines.iter().map(|l| format!("{l}{newline}")).collect::<String>()
        };
        self.insert(target.end, &text);
    }

    fn insert_into_section(&mut self, section: usize, lines: &[String]) {
        let section = &self.parsed().sections[section];
        let anchor = section
            .entries
            .last()
            .map(|&id| self.parsed().entries[id].last_line)
            .or(section.header_line);
        match anchor {
            Some(line) => self.insert_after_line(line, lines),
            None => self.insert_global(lines),
        }
    }

    /// Inserts keys before the first section, after the existing global
    /// keys.
    fn insert_global(&mut self, lines: &[String]) {
        let parsed = self.parsed();
        if let Some(&last) = parsed.sections[0].entries.last() {
            self.insert_after_line(parsed.entries[last].last_line, lines);
            return;
        }
        let first_header = parsed.sections.get(1).and_then(|s| s.header_line);
        let offset = match first_header {
            Some(mut line) => {
                while line > 0 && parsed.comment_lines[line - 1] {
                    line -= 1;
                }
                parsed.lines[line].start.max(bom_len(self.text()))
            }
            None => self.text().len(),
        };
        let newline = self.newline;
        let mut text = if first_header.is_none() {
            line_break_before_append(self.text(), newline).to_owned()
        } else {
            String::new()
        };
        for line in lines {
            text.push_str(line);
            text.push_str(newline);
        }
        self.insert(offset, &text);
    }

    fn append_section(&mut self, path: &[String], lines: &[String]) -> Result<(), Error> {
        let header = self.header_text(path)?;
        let text = self.text();
        let newline = self.newline;
        let mut block = line_break_before_append(text, newline).to_owned();
        let last_blank = self
            .parsed()
            .lines
            .last()
            .is_none_or(|line| line.is_blank(text));
        if !last_blank {
            block.push_str(newline);
        }
        block.push_str(&header);
        block.push_str(newline);
        for line in lines {
            block.push_str(line);
            block.push_str(newline);
        }
        self.insert(text.len(), &block);
        Ok(())
    }

    fn header_text(&self, path: &[String]) -> Result<String, Error> {
        let invalid = || {
            unsupported(
                SECTION_NAME,
                "the section name cannot be written in this INI profile; use another name",
            )
        };
        let plain = |name: &str| {
            !name.is_empty()
                && name.trim_matches([' ', '\t']) == name
                && !name.contains(['\n', '\r'])
        };
        match (self.doc.profile, path) {
            (IniProfile::Gitconfig, [section]) if git_name(section, true) => Ok(format!("[{section}]")),
            (IniProfile::Gitconfig, [section, sub]) if git_name(section, true) && !sub.contains(['\n', '\r']) => {
                let escaped = sub.replace('\\', "\\\\").replace('"', "\\\"");
                Ok(format!("[{section} \"{escaped}\"]"))
            }
            (IniProfile::Gitconfig, _) => Err(invalid()),
            (_, [name]) if plain(name) => Ok(format!("[{name}]")),
            _ => Err(invalid()),
        }
    }

    fn check_key_name(&self, name: &str) -> Result<(), Error> {
        let valid = match self.doc.profile {
            IniProfile::Gitconfig => git_name(name, false),
            _ => {
                !name.is_empty()
                    && name.trim_matches([' ', '\t']) == name
                    && !name.contains(['\n', '\r'])
                    && !name.contains(self.rules.separators)
                    && !name.starts_with('[')
                    && !name.starts_with(self.rules.comment_chars)
            }
        };
        if valid {
            Ok(())
        } else {
            Err(unsupported(
                KEY_NAME,
                "the key name cannot be written in this INI profile; use another name",
            ))
        }
    }

    fn create(&mut self, names: &[String], value: &Value) -> Result<(), Error> {
        let git = self.doc.profile == IniProfile::Gitconfig;
        let (section, key) = match (names, value) {
            ([_], Value::Map(_)) | ([_, _], Value::Map(_)) if git || names.len() == 1 => {
                (names.to_vec(), None)
            }
            ([key], _) if !git => (Vec::new(), Some(key.clone())),
            ([section, key], _) => (vec![section.clone()], Some(key.clone())),
            ([section, sub, key], _) if git => (vec![section.clone(), sub.clone()], Some(key.clone())),
            ([_], _) => {
                return Err(unsupported(
                    GLOBAL_KEY,
                    "Git configuration keys belong to a section; write `section.key`",
                ));
            }
            _ => {
                return Err(unsupported(
                    NESTED,
                    "INI documents nest keys in sections only; shorten the key path",
                ));
            }
        };
        let lines = match (&key, value) {
            (Some(key), _) => self.new_entry_lines(&section, key, value)?,
            (None, Value::Map(fields)) => {
                let mut lines = Vec::new();
                for (name, field) in fields {
                    lines.extend(self.new_entry_lines(&section, name, field)?);
                }
                lines
            }
            (None, _) => Vec::new(),
        };
        match self.section_id(&section) {
            Some(id) => self.insert_into_section(id, &lines),
            None if section.is_empty() => self.insert_global(&lines),
            None => self.append_section(&section, &lines)?,
        }
        Ok(())
    }

    fn append(&mut self, parent: &KeyPath, value: &Value, create: bool) -> Result<(), Error> {
        let value_of = |node: &Node| self.doc.node_value(node);
        let hits = match resolve(&self.doc.tree, parent, &value_of) {
            Ok(hits) => hits,
            Err(error) if error.code() == ErrorCode::KeyNotFound && create => {
                return self.create(&key_names(parent)?, value);
            }
            Err(error) => return Err(error),
        };
        for hit in hits {
            let last = match hit.node {
                Node::List(elements) => elements.last(),
                Node::Leaf(_) if self.is_list_profile() => Some(hit.node),
                _ => None,
            };
            let Some(Node::Leaf(last)) = last else {
                return Err(key_conflict(
                    &format!("`{}` is not a list; `[+]` adds an element to a list", hit.key()),
                    "not a list",
                    &[],
                ));
            };
            let entry = &self.parsed().entries[*last];
            let (name, section, line) = (entry.name.clone(), entry.section, entry.last_line);
            let lines = texts(value)?
                .iter()
                .map(|text| self.entry_line(&name, text, section, Some(*last)))
                .collect::<Result<Vec<_>, _>>()?;
            self.insert_after_line(line, &lines);
        }
        Ok(())
    }

    /// Removes an entry with its continuation lines and the comment lines
    /// directly above it.
    fn remove_entry(&mut self, id: usize) {
        self.plan.count += 1;
        let entry = &self.parsed().entries[id];
        let range = self.block_range(entry.first_line, entry.last_line + 1);
        self.plan.splices.push(Splice::new(range, ""));
    }

    /// Removes a section header with its entries up to the comment block of
    /// the next section.
    fn remove_section(&mut self, id: usize) {
        let parsed = self.parsed();
        let Some(header) = parsed.sections[id].header_line else {
            return;
        };
        self.plan.count += 1;
        let end_line = parsed
            .sections
            .get(id + 1)
            .and_then(|next| next.header_line)
            .map_or(parsed.lines.len(), |mut line| {
                while line > header + 1 && parsed.comment_lines[line - 1] {
                    line -= 1;
                }
                line
            });
        let range = self.block_range(header, end_line);
        self.plan.splices.push(Splice::new(range, ""));
    }

    /// Byte range of the lines `first..end_line` with the comment lines
    /// directly above `first`; at the end of a document without a final
    /// line break the preceding line break is included instead.
    fn block_range(&self, mut first: usize, end_line: usize) -> Range<usize> {
        let parsed = self.parsed();
        while first > 0 && parsed.comment_lines[first - 1] {
            first -= 1;
        }
        let last = parsed.lines[end_line - 1];
        let mut start = parsed.lines[first].start.max(if first == 0 { bom_len(self.text()) } else { 0 });
        if end_line == parsed.lines.len() && last.end == last.content_end && first > 0 {
            start = parsed.lines[first - 1].content_end;
        }
        start..last.end
    }
}

fn bom_len(text: &str) -> usize {
    if text.starts_with(super::BOM) { BOM_LEN } else { 0 }
}

/// Git section names: letters, digits, `-` and `.`; variable names: letters,
/// digits and `-`, starting with a letter.
fn git_name(name: &str, section: bool) -> bool {
    let mut chars = name.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || (section && (c.is_ascii_digit() || c == '-' || c == '.')));
    first_ok && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || (section && c == '.'))
}

/// Applies the plan and checks the result: the changed document must parse,
/// every entry outside the changed regions must read back unchanged and in
/// order, and the changed regions must hold exactly the expected entries.
///
/// Returns `None` when the plan changes nothing.
pub(crate) fn apply_and_verify(doc: &IniDocument, plan: &Plan) -> Result<Option<String>, Error> {
    let changes: Vec<&Splice> = plan
        .splices
        .iter()
        .filter(|s| doc.text.get(s.range.clone()) != Some(s.text.as_str()))
        .collect();
    if changes.is_empty() {
        return Ok(None);
    }
    let owned: Vec<Splice> = changes.into_iter().cloned().collect();
    let text = apply_splices(&doc.text, &owned).ok_or_else(|| {
        key_conflict(
            "the changes overlap; address keys that do not contain each other",
            "overlapping changes",
            &[],
        )
    })?;
    let failed = || {
        unsupported(
            VERIFY,
            "the changed document does not read back with the expected keys; nothing was changed",
        )
    };
    let parsed = parse(&text, doc.profile).map_err(|_| failed())?;
    let old_ranges: Vec<Range<usize>> = owned.iter().map(|s| s.range.clone()).collect();
    let new_ranges = shifted_ranges(&owned);
    let fold = doc.profile.rules().fold_key_names;
    let (old_kept, _) = partition(&doc.parsed, &old_ranges, fold);
    let (new_kept, mut new_changed) = partition(&parsed, &new_ranges, fold);
    let mut expected = plan.expected.clone();
    expected.sort();
    new_changed.sort();
    if old_kept == new_kept && expected == new_changed {
        Ok(Some(text))
    } else {
        Err(failed())
    }
}

/// Ranges of the replacement texts in the changed document.
fn shifted_ranges(splices: &[Splice]) -> Vec<Range<usize>> {
    let mut order: Vec<&Splice> = splices.iter().collect();
    order.sort_by_key(|s| (s.range.start, s.range.end));
    let mut delta: isize = 0;
    order
        .into_iter()
        .map(|splice| {
            let start = splice.range.start.saturating_add_signed(delta);
            let removed = isize::try_from(splice.range.len()).unwrap_or(isize::MAX);
            let added = isize::try_from(splice.text.len()).unwrap_or(isize::MAX);
            delta += added - removed;
            start..start + splice.text.len()
        })
        .collect()
}

/// Splits the entries into those outside every changed range and those
/// touching one.
fn partition(
    parsed: &Parsed,
    ranges: &[Range<usize>],
    fold: bool,
) -> (Vec<Signature>, Vec<Signature>) {
    let mut kept = Vec::new();
    let mut changed = Vec::new();
    for entry in &parsed.entries {
        let span = parsed.lines[entry.first_line].start..parsed.lines[entry.last_line].end;
        let touched = ranges.iter().any(|r| overlaps(&span, r));
        let name = if fold {
            entry.name.to_ascii_lowercase()
        } else {
            entry.name.clone()
        };
        let signature = (
            parsed.sections[entry.section].path.clone(),
            name,
            entry.value.clone(),
        );
        if touched {
            changed.push(signature);
        } else {
            kept.push(signature);
        }
    }
    (kept, changed)
}

/// Returns `true` when a change range touches a line span; an insertion
/// touches the span when it falls strictly inside it.
fn overlaps(span: &Range<usize>, change: &Range<usize>) -> bool {
    if change.is_empty() {
        span.start < change.start && change.start < span.end
    } else {
        span.start < change.end && change.start < span.end
    }
}
