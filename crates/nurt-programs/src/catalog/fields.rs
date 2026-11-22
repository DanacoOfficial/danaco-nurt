// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Typed extraction of the members of a catalog object, with every violation
//! recorded under the path of the member.

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::text::strip_controls;

/// Violation of the format by one member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fault {
    /// Path of the member, for example `programs[3].aliases[1].kind`.
    pub(crate) field: String,
    /// What is wrong and how to correct it.
    pub(crate) message: String,
}

/// Permitted length of a text, in Unicode scalar values.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Len {
    min: usize,
    max: usize,
}

/// Text of `min` to `max` characters.
pub(crate) const fn len(min: usize, max: usize) -> Len {
    Len { min, max }
}

/// Check of a text against a pattern of the format, with the rule stated in
/// messages.
#[derive(Clone, Copy)]
pub(crate) struct Pattern {
    pub(crate) check: fn(&str) -> bool,
    pub(crate) rule: &'static str,
}

/// Reader of the members of one object.
pub(crate) struct Fields<'m> {
    map: &'m Map<String, Value>,
    pointer: String,
    used: BTreeSet<&'static str>,
    faults: Vec<Fault>,
    unknown: Vec<Fault>,
    cleaned: Vec<String>,
}

/// Result of reading an object.
#[derive(Debug, Clone, Default)]
pub(crate) struct Findings {
    /// Violations; the object is invalid when there is any.
    pub(crate) faults: Vec<Fault>,
    /// Members and enumeration values the format does not define.
    pub(crate) unknown: Vec<Fault>,
    /// Members whose control characters were removed.
    pub(crate) cleaned: Vec<String>,
}

impl<'m> Fields<'m> {
    /// Reader of `map`, an object at `pointer`.
    pub(crate) fn new(map: &'m Map<String, Value>, pointer: impl Into<String>) -> Self {
        Self {
            map,
            pointer: pointer.into(),
            used: BTreeSet::new(),
            faults: Vec::new(),
            unknown: Vec::new(),
            cleaned: Vec::new(),
        }
    }

    /// Reader of an object value, or a fault when `value` is not an object.
    pub(crate) fn of(value: &'m Value, pointer: String) -> Result<Self, Fault> {
        match value {
            Value::Object(map) => Ok(Self::new(map, pointer)),
            _ => Err(Fault {
                field: pointer,
                message: "must be an object; write the entry as {...}".to_owned(),
            }),
        }
    }

    /// Path of a member of this object.
    pub(crate) fn path(&self, key: &str) -> String {
        if self.pointer.is_empty() {
            key.to_owned()
        } else {
            format!("{}.{key}", self.pointer)
        }
    }

    /// Records a violation.
    pub(crate) fn fault(&mut self, field: String, message: impl Into<String>) {
        self.faults.push(Fault {
            field,
            message: message.into(),
        });
    }

    /// Returns `true` when the object has the member.
    pub(crate) fn has(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    /// Returns `true` when no violation was recorded so far.
    pub(crate) fn is_clean(&self) -> bool {
        self.faults.is_empty()
    }

    /// Value of a member, marking it as defined by the format.
    pub(crate) fn get(&mut self, key: &'static str) -> Option<&'m Value> {
        self.used.insert(key);
        self.map.get(key)
    }

    /// Optional text member.
    pub(crate) fn text(&mut self, key: &'static str, limit: Len) -> Option<String> {
        let value = self.get(key)?;
        let path = self.path(key);
        self.text_value(value, path, limit)
    }

    /// Required text member.
    pub(crate) fn required_text(&mut self, key: &'static str, limit: Len) -> Option<String> {
        if self.map.contains_key(key) {
            self.text(key, limit)
        } else {
            self.used.insert(key);
            let path = self.path(key);
            self.fault(path, format!("member `{key}` is required; add it"));
            None
        }
    }

    /// Optional text member that must match `pattern`.
    pub(crate) fn pattern(&mut self, key: &'static str, pattern: Pattern) -> Option<String> {
        let value = self.get(key)?;
        let path = self.path(key);
        self.pattern_value(value, path, pattern)
    }

    /// Required text member that must match `pattern`.
    pub(crate) fn required_pattern(&mut self, key: &'static str, pattern: Pattern) -> Option<String> {
        if self.map.contains_key(key) {
            self.pattern(key, pattern)
        } else {
            self.used.insert(key);
            let path = self.path(key);
            self.fault(path, format!("member `{key}` is required; add it"));
            None
        }
    }

    /// Optional boolean member.
    pub(crate) fn boolean(&mut self, key: &'static str) -> Option<bool> {
        let value = self.get(key)?;
        if let Value::Bool(flag) = value {
            return Some(*flag);
        }
        let path = self.path(key);
        self.fault(path, "must be true or false");
        None
    }

    /// Optional integer member of at least `min`.
    pub(crate) fn integer(&mut self, key: &'static str, min: i64) -> Option<i64> {
        let value = self.get(key)?;
        let path = self.path(key);
        match value.as_i64() {
            Some(number) if number >= min => Some(number),
            Some(_) => {
                self.fault(path, format!("must be an integer of at least {min}"));
                None
            }
            None => {
                self.fault(path, "must be an integer");
                None
            }
        }
    }

    /// Optional array member of at most `max_items` items, with the path of
    /// the member.
    pub(crate) fn array(&mut self, key: &'static str, max_items: usize) -> Option<(&'m [Value], String)> {
        let value = self.get(key)?;
        let path = self.path(key);
        match value {
            Value::Array(items) if items.len() <= max_items => Some((items.as_slice(), path)),
            Value::Array(_) => {
                self.fault(path, format!("has more than {max_items} items; remove some"));
                None
            }
            _ => {
                self.fault(path, "must be an array; write it as [...]");
                None
            }
        }
    }

    /// Optional array of texts.
    pub(crate) fn text_list(&mut self, key: &'static str, max_items: usize, limit: Len) -> Vec<String> {
        let Some((items, path)) = self.array(key, max_items) else {
            return Vec::new();
        };
        items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| self.text_value(item, format!("{path}[{index}]"), limit))
            .collect()
    }

    /// Optional array of texts that match `pattern`; with `unique` a repeated
    /// item is a violation.
    pub(crate) fn pattern_list(
        &mut self,
        key: &'static str,
        max_items: usize,
        pattern: Pattern,
        unique: bool,
    ) -> Vec<String> {
        let Some((items, path)) = self.array(key, max_items) else {
            return Vec::new();
        };
        let mut out: Vec<String> = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let item_path = format!("{path}[{index}]");
            if let Some(text) = self.pattern_value(item, item_path.clone(), pattern) {
                if unique && out.contains(&text) {
                    self.fault(item_path, format!("`{text}` is repeated; list it once"));
                } else {
                    out.push(text);
                }
            }
        }
        out
    }

    /// Optional extension member `x`, which must be an object.
    pub(crate) fn extension(&mut self) {
        if let Some(value) = self.get("x") {
            if !value.is_object() {
                let path = self.path("x");
                self.fault(path, "must be an object");
            }
        }
    }

    /// Text of `value` at `path`.
    pub(crate) fn text_value(&mut self, value: &Value, path: String, limit: Len) -> Option<String> {
        let Value::String(raw) = value else {
            self.fault(path, "must be text");
            return None;
        };
        let (text, removed) = clean(raw);
        let count = text.chars().count();
        if count < limit.min {
            self.fault(path, "must not be empty");
            return None;
        }
        if count > limit.max {
            self.fault(path, format!("is longer than {} characters; shorten it", limit.max));
            return None;
        }
        if removed {
            self.cleaned.push(path);
        }
        Some(text)
    }

    /// Text of `value` at `path` that must match `pattern`.
    pub(crate) fn pattern_value(&mut self, value: &Value, path: String, pattern: Pattern) -> Option<String> {
        let Value::String(text) = value else {
            self.fault(path, "must be text");
            return None;
        };
        if (pattern.check)(text) {
            Some(text.clone())
        } else {
            self.fault(path, format!("`{text}` is not valid: {}", pattern.rule));
            None
        }
    }

    /// Adds the findings of a nested object.
    pub(crate) fn absorb(&mut self, nested: Findings) {
        self.faults.extend(nested.faults);
        self.unknown.extend(nested.unknown);
        self.cleaned.extend(nested.cleaned);
    }

    /// Records a value of an enumeration that the format does not define.
    pub(crate) fn unknown_value(&mut self, field: String, value: &str, allowed: &str) {
        self.unknown.push(Fault {
            field,
            message: format!("value `{value}` is not defined by the format; use {allowed}"),
        });
    }

    /// Finishes reading: members that were not requested are unknown.
    pub(crate) fn finish(mut self) -> Findings {
        let keys: Vec<String> = self
            .map
            .keys()
            .filter(|key| !self.used.contains(key.as_str()))
            .map(|key| self.path(key))
            .collect();
        for field in keys {
            self.unknown.push(Fault {
                field,
                message: "member is not defined by the format; remove it or move it into `x`"
                    .to_owned(),
            });
        }
        Findings {
            faults: self.faults,
            unknown: self.unknown,
            cleaned: self.cleaned,
        }
    }
}

/// Removes control characters other than line feed and tab; returns whether
/// any was removed.
pub(crate) fn clean(text: &str) -> (String, bool) {
    let cleaned = strip_controls(text);
    let removed = cleaned.len() != text.len();
    (cleaned, removed)
}
