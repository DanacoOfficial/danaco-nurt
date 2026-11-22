// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! JSON pointers (RFC 6901), pointer patterns and placeholders of case files.

use serde_json::{Map, Value};

/// Segment of a pointer pattern that matches any member or index.
pub(crate) const WILDCARD: &str = "*";

/// Splits a JSON pointer into its unescaped segments.
pub(crate) fn segments(pointer: &str) -> Vec<String> {
    if pointer.is_empty() {
        return Vec::new();
    }
    pointer
        .trim_start_matches('/')
        .split('/')
        .map(|segment| segment.replace("~1", "/").replace("~0", "~"))
        .collect()
}

/// Joins unescaped segments into a JSON pointer.
pub(crate) fn join(segments: &[String]) -> String {
    segments
        .iter()
        .map(|segment| format!("/{}", segment.replace('~', "~0").replace('/', "~1")))
        .collect()
}

/// Stores `value` at `pointer`, creating objects for missing members.
///
/// An existing object at the target is merged member by member with an
/// object value; any other existing value is replaced.
pub(crate) fn insert(target: &mut Value, pointer: &str, value: Value) {
    let mut current = target;
    for segment in segments(pointer) {
        if !current.is_object() && !current.is_array() {
            *current = Value::Object(Map::new());
        }
        current = match current {
            Value::Array(items) => match segment.parse::<usize>() {
                Ok(index) if index < items.len() => &mut items[index],
                _ => return,
            },
            Value::Object(members) => members.entry(segment).or_insert(Value::Null),
            _ => return,
        };
    }
    merge(current, value);
}

/// Merges `value` into `target`: objects member by member, other values by
/// replacement.
pub(crate) fn merge(target: &mut Value, value: Value) {
    match (target, value) {
        (Value::Object(existing), Value::Object(added)) => {
            for (name, member) in added {
                merge(existing.entry(name).or_insert(Value::Null), member);
            }
        }
        (target, value) => *target = value,
    }
}

/// Calls `apply` for every value whose location matches the pattern.
pub(crate) fn for_each_match(
    value: &mut Value,
    pattern: &[String],
    apply: &mut dyn FnMut(&mut Value),
) {
    let Some((first, rest)) = pattern.split_first() else {
        apply(value);
        return;
    };
    match value {
        Value::Object(members) => {
            if first == WILDCARD {
                for member in members.values_mut() {
                    for_each_match(member, rest, apply);
                }
            } else if let Some(member) = members.get_mut(first) {
                for_each_match(member, rest, apply);
            }
        }
        Value::Array(items) => {
            if first == WILDCARD {
                for item in items {
                    for_each_match(item, rest, apply);
                }
            } else if let Some(item) = first.parse::<usize>().ok().and_then(|i| items.get_mut(i)) {
                for_each_match(item, rest, apply);
            }
        }
        _ => {}
    }
}

/// Returns the pointers of the values whose location matches the pattern.
pub(crate) fn find_matches(value: &Value, pattern: &[String]) -> Vec<String> {
    let mut found = Vec::new();
    collect_matches(value, pattern, &mut Vec::new(), &mut found);
    found
}

fn collect_matches(
    value: &Value,
    pattern: &[String],
    path: &mut Vec<String>,
    found: &mut Vec<String>,
) {
    let Some((first, rest)) = pattern.split_first() else {
        found.push(join(path));
        return;
    };
    let mut visit = |name: String, child: &Value, path: &mut Vec<String>| {
        path.push(name);
        collect_matches(child, rest, path, found);
        path.pop();
    };
    match value {
        Value::Object(members) if first == WILDCARD => {
            for (name, member) in members {
                visit(name.clone(), member, path);
            }
        }
        Value::Object(members) => {
            if let Some(member) = members.get(first) {
                visit(first.clone(), member, path);
            }
        }
        Value::Array(items) if first == WILDCARD => {
            for (index, item) in items.iter().enumerate() {
                visit(index.to_string(), item, path);
            }
        }
        Value::Array(items) => {
            if let Some(item) = first.parse::<usize>().ok().and_then(|i| items.get(i)) {
                visit(first.clone(), item, path);
            }
        }
        _ => {}
    }
}

/// Removes every object member whose location matches the pattern. Array
/// items are not removed, so the indexes of other items stay valid.
pub(crate) fn remove_matches(value: &mut Value, pattern: &[String]) {
    let Some((last, parent)) = pattern.split_last() else {
        return;
    };
    for_each_match(value, parent, &mut |found| {
        if let Value::Object(members) = found {
            if last == WILDCARD {
                members.clear();
            } else {
                members.remove(last);
            }
        }
    });
}

/// Names of the placeholders that stand for a value of the environment of
/// a case.
pub(crate) mod variable {
    /// Working directory of the case.
    pub(crate) const BASE: &str = "base";
    /// Address of the local HTTP server of the case.
    pub(crate) const HTTP_BASE: &str = "http_base";
    /// Directory of the case on the second volume.
    pub(crate) const SECOND_VOLUME: &str = "second_volume";
    /// Every name.
    pub(crate) const ALL: [&str; 3] = [BASE, HTTP_BASE, SECOND_VOLUME];
}

/// Values that replace placeholders.
pub(crate) trait Bindings {
    /// Value of the placeholder of the environment with the given name, one
    /// of [`variable::ALL`].
    fn variable(&self, name: &str) -> Option<String>;
    /// Value at `pointer` in the result of the step with the given number,
    /// counted from 1.
    fn step(&self, number: usize, pointer: &str) -> Option<Value>;
}

/// Error of a placeholder that cannot be replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlaceholderError(pub(crate) String);

const OPEN: &str = "${";
const CLOSE: char = '}';
const STEP: &str = "step";

/// Replaces the placeholders of the environment (`${base}`, `${http_base}`,
/// `${second_volume}`) and `${stepN/pointer}` in every text of `value`.
pub(crate) fn substitute(
    value: &Value,
    bindings: &dyn Bindings,
) -> Result<Value, PlaceholderError> {
    match value {
        Value::String(text) => substitute_text(text, bindings),
        Value::Array(items) => items
            .iter()
            .map(|item| substitute(item, bindings))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(members) => members
            .iter()
            .map(|(name, member)| Ok((name.clone(), substitute(member, bindings)?)))
            .collect::<Result<Map<_, _>, _>>()
            .map(Value::Object),
        other => Ok(other.clone()),
    }
}

fn substitute_text(text: &str, bindings: &dyn Bindings) -> Result<Value, PlaceholderError> {
    if !text.contains(OPEN) {
        return Ok(Value::String(text.to_owned()));
    }
    if let Some(inner) = whole_placeholder(text) {
        return resolve(inner, bindings);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + OPEN.len()..];
        let end = after
            .find(CLOSE)
            .ok_or_else(|| PlaceholderError(format!("unterminated placeholder in {text:?}")))?;
        match resolve(&after[..end], bindings)? {
            Value::String(inserted) => out.push_str(&inserted),
            other => out.push_str(&other.to_string()),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(Value::String(out))
}

fn whole_placeholder(text: &str) -> Option<&str> {
    let inner = text.strip_prefix(OPEN)?.strip_suffix(CLOSE)?;
    (!inner.contains(CLOSE) && !inner.contains(OPEN)).then_some(inner)
}

fn resolve(name: &str, bindings: &dyn Bindings) -> Result<Value, PlaceholderError> {
    if variable::ALL.contains(&name) {
        return bindings
            .variable(name)
            .map(Value::String)
            .ok_or_else(|| PlaceholderError(format!("${{{name}}} is not available here")));
    }
    let reference = name
        .strip_prefix(STEP)
        .ok_or_else(|| PlaceholderError(format!("unknown placeholder ${{{name}}}")))?;
    let digits = reference.find('/').unwrap_or(reference.len());
    let number: usize = reference[..digits]
        .parse()
        .map_err(|_| PlaceholderError(format!("invalid step number in ${{{name}}}")))?;
    bindings
        .step(number, &reference[digits..])
        .ok_or_else(|| PlaceholderError(format!("${{{name}}} names no value of an earlier step")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Fixed {
        base: Option<String>,
        results: Vec<Value>,
    }

    impl Bindings for Fixed {
        fn variable(&self, name: &str) -> Option<String> {
            match name {
                variable::BASE => self.base.clone(),
                variable::HTTP_BASE => Some("http://127.0.0.1:8080".to_owned()),
                _ => None,
            }
        }
        fn step(&self, number: usize, pointer: &str) -> Option<Value> {
            self.results
                .get(number.checked_sub(1)?)?
                .pointer(pointer)
                .cloned()
        }
    }

    fn fixed() -> Fixed {
        Fixed {
            base: Some("/tmp/case/work".to_owned()),
            results: vec![json!({"report": {"op": 7, "next_cursor": "c1"}})],
        }
    }

    #[test]
    fn segments_and_join_are_inverse() {
        let pointer = "/a~1b/c~0d/0";
        let parts = segments(pointer);
        assert_eq!(parts, vec!["a/b", "c~d", "0"]);
        assert_eq!(join(&parts), pointer);
        assert!(segments("").is_empty());
    }

    #[test]
    fn insert_creates_objects_and_merges() {
        let mut value = json!({"report": {"status": "ok"}});
        insert(&mut value, "/report/error/code", json!("E_IO"));
        insert(&mut value, "/report", json!({"kind": "find"}));
        assert_eq!(
            value,
            json!({"report": {"status": "ok", "kind": "find", "error": {"code": "E_IO"}}})
        );
        let mut scalar = json!(1);
        insert(&mut scalar, "/a", json!(2));
        assert_eq!(scalar, json!({"a": 2}));
        let mut array = json!([1, 2]);
        insert(&mut array, "/1", json!(5));
        insert(&mut array, "/9", json!(5));
        assert_eq!(array, json!([1, 5]));
    }

    #[test]
    fn wildcard_patterns_visit_every_member_and_index() {
        let mut value = json!({"rows": [{"t": 1}, {"t": 2}], "other": {"t": 3}});
        let mut seen = Vec::new();
        for_each_match(&mut value, &segments("/rows/*/t"), &mut |v| {
            seen.push(v.clone())
        });
        assert_eq!(seen, vec![json!(1), json!(2)]);
        for_each_match(&mut value, &segments("/*/t"), &mut |v| *v = json!(0));
        assert_eq!(value["other"]["t"], json!(0));
        for_each_match(&mut value, &segments("/rows/1/t"), &mut |v| *v = json!(9));
        assert_eq!(value["rows"][1]["t"], json!(9));
    }

    #[test]
    fn matches_are_found_with_their_pointers_and_removed() {
        let value = json!({"rows": [{"t": 1}, {"u": 2}], "o": {"a/b": {"t": 3}}, "s": 4});
        assert_eq!(find_matches(&value, &segments("/rows/*/t")), ["/rows/0/t"]);
        let mut nested = find_matches(&value, &segments("/*/*/t"));
        nested.sort();
        assert_eq!(nested, ["/o/a~1b/t", "/rows/0/t"]);
        assert_eq!(find_matches(&value, &segments("/rows/1")), ["/rows/1"]);
        assert_eq!(find_matches(&value, &segments("")), [""]);
        assert!(find_matches(&value, &segments("/s/x")).is_empty());
        assert!(find_matches(&value, &segments("/rows/7")).is_empty());

        let mut removed = value.clone();
        remove_matches(&mut removed, &segments("/rows/*/t"));
        remove_matches(&mut removed, &segments("/o/*"));
        remove_matches(&mut removed, &segments("/rows/0"));
        remove_matches(&mut removed, &segments(""));
        assert_eq!(removed, json!({"rows": [{}, {"u": 2}], "o": {}, "s": 4}));
    }

    #[test]
    fn whole_placeholder_keeps_the_json_type() {
        let value = json!({"op": "${step1/report/op}", "cursor": "${step1/report/next_cursor}"});
        let replaced = substitute(&value, &fixed()).unwrap();
        assert_eq!(replaced, json!({"op": 7, "cursor": "c1"}));
    }

    #[test]
    fn embedded_placeholders_are_inserted_as_text() {
        let value = json!([
            "journal.undo op: ${step1/report/op}",
            "find \"${base}/src\"",
            "http.request \"${http_base}/echo\""
        ]);
        let replaced = substitute(&value, &fixed()).unwrap();
        assert_eq!(
            replaced,
            json!([
                "journal.undo op: 7",
                "find \"/tmp/case/work/src\"",
                "http.request \"http://127.0.0.1:8080/echo\""
            ])
        );
    }

    #[test]
    fn unresolvable_placeholders_are_errors() {
        let bindings = Fixed {
            base: None,
            results: Vec::new(),
        };
        for text in [
            "${base}",
            "${second_volume}",
            "${step1/report/op}",
            "${stepx/a}",
            "${other}",
            "a ${base",
        ] {
            assert!(substitute(&json!(text), &bindings).is_err(), "{text}");
        }
    }
}
