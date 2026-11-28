// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-conformance::normalize
// Responsibility:   Normalization of values that differ between runs of the same program.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Normalization of values that differ between runs of the same program.
//!
//! Members marked `"x-nurt-volatile": true` in a published schema, such as
//! the journal entry number `op` of a report, and paths below the working
//! directory of a case are rewritten before an expected value is compared
//! with an actual one.

use serde_json::Value;

use crate::pointer;

/// Text that replaces a volatile value.
pub const VOLATILE: &str = "<volatile>";

/// Schema annotation that marks a volatile member.
pub const VOLATILE_ANNOTATION: &str = "x-nurt-volatile";

/// Placeholder that replaces the working directory of a case.
pub const BASE_PLACEHOLDER: &str = "${base}";

/// Placeholder that replaces the address of the local HTTP server of a case.
pub const HTTP_BASE_PLACEHOLDER: &str = "${http_base}";

/// Placeholder that replaces the directory of a case on the second volume.
pub const SECOND_VOLUME_PLACEHOLDER: &str = "${second_volume}";

/// Placeholder of the configuration of a case that stands for the mount
/// point of the working directory.
pub const WORK_MOUNT_PLACEHOLDER: &str = "${work_mount}";

/// Volatile members of an event `nurt/event`.
pub const EVENT_VOLATILE: [&str; 2] = ["/seq", "/time"];

/// Locations of volatile values, as pointer patterns where `*` matches any
/// member or index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VolatilePaths {
    patterns: Vec<Vec<String>>,
}

impl VolatilePaths {
    /// Collects the members marked volatile in a schema.
    ///
    /// The patterns are prefixed with `prefix`, the location of the
    /// documents of the schema inside the compared value. References within
    /// the schema (`#/$defs/...`) are followed; a reference that is already
    /// being followed on the current path is not followed again.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_conformance::normalize::VolatilePaths;
    /// use serde_json::json;
    ///
    /// let schema = json!({"properties": {"op": {"type": "integer", "x-nurt-volatile": true}}});
    /// let paths = VolatilePaths::from_schema(&schema, "/report");
    /// assert_eq!(paths.patterns(), vec!["/report/op".to_owned()]);
    /// ```
    #[must_use]
    pub fn from_schema(schema: &Value, prefix: &str) -> Self {
        let mut collector = Collector {
            root: schema,
            visiting: Vec::new(),
            patterns: Vec::new(),
        };
        collector.walk(schema, &mut pointer::segments(prefix));
        let mut patterns = collector.patterns;
        patterns.sort();
        patterns.dedup();
        Self { patterns }
    }

    /// Adds patterns written as JSON pointers.
    pub fn extend<'a>(&mut self, pointers: impl IntoIterator<Item = &'a str>) {
        self.patterns
            .extend(pointers.into_iter().map(pointer::segments));
    }

    /// Returns the patterns as JSON pointers.
    #[must_use]
    pub fn patterns(&self) -> Vec<String> {
        self.patterns.iter().map(|p| pointer::join(p)).collect()
    }

    /// Replaces every volatile value present in `value` by [`VOLATILE`].
    pub fn apply(&self, value: &mut Value) {
        for pattern in &self.patterns {
            pointer::for_each_match(value, pattern, &mut |found| {
                *found = Value::String(VOLATILE.to_owned());
            });
        }
    }
}

struct Collector<'a> {
    root: &'a Value,
    visiting: Vec<String>,
    patterns: Vec<Vec<String>>,
}

const COMBINATORS: [&str; 6] = ["allOf", "anyOf", "oneOf", "if", "then", "else"];
const LOCAL_REFERENCE: &str = "#";

impl<'a> Collector<'a> {
    fn walk(&mut self, schema: &'a Value, path: &mut Vec<String>) {
        let Some(members) = schema.as_object() else {
            return;
        };
        if members.get(VOLATILE_ANNOTATION) == Some(&Value::Bool(true)) {
            self.patterns.push(path.clone());
            return;
        }
        if let Some(reference) = members.get("$ref").and_then(Value::as_str) {
            self.follow(reference, path);
        }
        for name in COMBINATORS {
            match members.get(name) {
                Some(Value::Array(branches)) => branches.iter().for_each(|b| self.walk(b, path)),
                Some(branch) => self.walk(branch, path),
                None => {}
            }
        }
        if let Some(Value::Object(properties)) = members.get("properties") {
            for (name, property) in properties {
                path.push(name.clone());
                self.walk(property, path);
                path.pop();
            }
        }
        for name in ["items", "additionalProperties"] {
            if let Some(child) = members.get(name) {
                self.walk_any(child, path);
            }
        }
        if let Some(Value::Object(patterns)) = members.get("patternProperties") {
            patterns
                .values()
                .for_each(|child| self.walk_any(child, path));
        }
    }

    fn walk_any(&mut self, child: &'a Value, path: &mut Vec<String>) {
        path.push(pointer::WILDCARD.to_owned());
        self.walk(child, path);
        path.pop();
    }

    fn follow(&mut self, reference: &str, path: &mut Vec<String>) {
        let Some(local) = reference.strip_prefix(LOCAL_REFERENCE) else {
            return;
        };
        if self.visiting.iter().any(|r| r == reference) {
            return;
        }
        if let Some(target) = self.root.pointer(local) {
            self.visiting.push(reference.to_owned());
            self.walk(target, path);
            self.visiting.pop();
        }
    }
}

/// Rewrites every text that starts with `base` to start with
/// [`BASE_PLACEHOLDER`], writing the separators of the rest as `/`.
///
/// `base` is matched in its native form and with `/` separators.
///
/// # Examples
///
/// ```
/// use nurt_conformance::normalize::rewrite_base;
/// use serde_json::json;
///
/// let mut value = json!({"path": "/tmp/case/work/src/a.go", "other": "/srv"});
/// rewrite_base(&mut value, "/tmp/case/work");
/// assert_eq!(value, json!({"path": "${base}/src/a.go", "other": "/srv"}));
/// ```
pub fn rewrite_base(value: &mut Value, base: &str) {
    rewrite_path(value, base, BASE_PLACEHOLDER);
}

/// Rewrites every text that starts with the directory `path` to start with
/// `placeholder`, writing the separators of the rest as `/`.
///
/// `path` is matched in its native form and with `/` separators.
pub fn rewrite_path(value: &mut Value, path: &str, placeholder: &str) {
    let forms = path_forms(path);
    rewrite(value, &mut |text| rewrite_text(text, &forms, placeholder));
}

/// Replaces every occurrence of `text` inside the texts of `value` by
/// `placeholder`.
///
/// # Examples
///
/// ```
/// use nurt_conformance::normalize::rewrite_occurrences;
/// use serde_json::json;
///
/// let mut value = json!({"url": "http://127.0.0.1:4000/echo", "n": 1});
/// rewrite_occurrences(&mut value, "http://127.0.0.1:4000", "${http_base}");
/// assert_eq!(value, json!({"url": "${http_base}/echo", "n": 1}));
/// ```
pub fn rewrite_occurrences(value: &mut Value, text: &str, placeholder: &str) {
    if text.is_empty() {
        return;
    }
    rewrite(value, &mut |found| {
        found
            .contains(text)
            .then(|| found.replace(text, placeholder))
    });
}

fn path_forms(path: &str) -> Vec<String> {
    let trimmed = path.trim_end_matches(['/', '\\']);
    let mut forms = vec![trimmed.to_owned(), trimmed.replace('\\', "/")];
    forms.dedup();
    forms.retain(|form| !form.is_empty());
    forms
}

fn rewrite(value: &mut Value, change: &mut dyn FnMut(&str) -> Option<String>) {
    match value {
        Value::String(text) => {
            if let Some(rewritten) = change(text) {
                *text = rewritten;
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| rewrite(item, change)),
        Value::Object(members) => members
            .values_mut()
            .for_each(|member| rewrite(member, change)),
        _ => {}
    }
}

fn rewrite_text(text: &str, forms: &[String], placeholder: &str) -> Option<String> {
    forms.iter().find_map(|form| {
        let rest = text.strip_prefix(form.as_str())?;
        let boundary = rest.is_empty() || rest.starts_with(['/', '\\']);
        boundary.then(|| format!("{placeholder}{}", rest.replace('\\', "/")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn report_schema_marks_the_documented_volatile_members() {
        let schema: Value = serde_json::from_str(nurt_contracts::schema::REPORT).unwrap();
        let patterns = VolatilePaths::from_schema(&schema, "").patterns();
        for member in ["/op", "/result_file", "/next_cursor", "/timing"] {
            assert!(
                patterns.contains(&member.to_owned()),
                "{member} in {patterns:?}"
            );
        }
    }

    #[test]
    fn references_wildcards_and_recursion_are_followed() {
        let schema = json!({
            "properties": {
                "rows": {"type": "array", "items": {"$ref": "#/$defs/row"}},
                "map": {"additionalProperties": {"$ref": "#/$defs/row"}},
                "pat": {"patternProperties": {"^x": {"x-nurt-volatile": true}}},
                "either": {"oneOf": [{"properties": {"t": {"x-nurt-volatile": true}}}]},
                "remote": {"$ref": "other.json#/a"},
                "dangling": {"$ref": "#/$defs/none"}
            },
            "$defs": {
                "row": {"properties": {"time": {"x-nurt-volatile": true}, "child": {"$ref": "#/$defs/row"}}}
            }
        });
        let patterns = VolatilePaths::from_schema(&schema, "/report").patterns();
        assert_eq!(
            patterns,
            vec![
                "/report/either/t",
                "/report/map/*/time",
                "/report/pat/*",
                "/report/rows/*/time",
            ]
        );
    }

    #[test]
    fn apply_replaces_present_values_only() {
        let mut paths = VolatilePaths::default();
        paths.extend(["/report/op", "/report/rows/*/t"]);
        let mut value = json!({"report": {"op": 21, "rows": [{"t": 1}, {"u": 2}]}});
        paths.apply(&mut value);
        assert_eq!(
            value,
            json!({"report": {"op": VOLATILE, "rows": [{"t": VOLATILE}, {"u": 2}]}})
        );
        let mut absent = json!({"report": {}});
        paths.apply(&mut absent);
        assert_eq!(absent, json!({"report": {}}));
    }

    #[test]
    fn base_is_rewritten_in_native_and_slash_form() {
        let mut value = json!([
            "C:\\tmp\\work\\a\\b.txt",
            "C:/tmp/work/c.txt",
            "C:\\tmp\\work",
            "C:\\tmp\\workshop\\x",
            7
        ]);
        rewrite_base(&mut value, "C:\\tmp\\work\\");
        assert_eq!(
            value,
            json!([
                "${base}/a/b.txt",
                "${base}/c.txt",
                "${base}",
                "C:\\tmp\\workshop\\x",
                7
            ])
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
