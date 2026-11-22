// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Comparison of expected and actual values and `ast_hash`.

use nurt_contracts::ast::{AST_HASH_PREFIX, strip_for_hash};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::pointer;

/// Largest number of differences reported for one comparison.
pub const MAX_DIFFERENCES: usize = 20;

/// Comparison mode of an expectation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Match {
    /// Expected object members must be present and match; other members are
    /// ignored. Arrays must have the same length.
    #[default]
    Subset,
    /// The values must be equal.
    Exact,
    /// The hash forms of two syntax trees must be equal.
    Tree,
}

impl Match {
    /// Reads the mode from its name in a case file.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "subset" => Some(Self::Subset),
            "exact" => Some(Self::Exact),
            "tree" => Some(Self::Tree),
            _ => None,
        }
    }
}

/// Difference between an expected and an actual value.
#[derive(Debug, Clone, PartialEq)]
pub struct Difference {
    /// JSON pointer of the difference, relative to the compared values.
    pub path: String,
    /// Description of the difference.
    pub message: String,
    /// Expected value, when one exists at the path.
    pub expected: Option<Value>,
    /// Actual value, when one exists at the path.
    pub actual: Option<Value>,
}

/// Compares `actual` with `expected` in the given mode.
///
/// Returns at most [`MAX_DIFFERENCES`] differences; an empty list means the
/// values match.
///
/// # Examples
///
/// ```
/// use nurt_conformance::compare::{Match, compare};
/// use serde_json::json;
///
/// let expected = json!({"report": {"status": "ok"}});
/// let actual = json!({"report": {"status": "ok", "kind": "find"}});
/// assert!(compare(&expected, &actual, Match::Subset).is_empty());
/// assert_eq!(compare(&expected, &actual, Match::Exact).len(), 1);
/// ```
#[must_use]
pub fn compare(expected: &Value, actual: &Value, mode: Match) -> Vec<Difference> {
    let mut differences = Vec::new();
    match mode {
        Match::Subset => walk(expected, actual, true, &mut Vec::new(), &mut differences),
        Match::Exact => walk(expected, actual, false, &mut Vec::new(), &mut differences),
        Match::Tree => walk(
            &strip_for_hash(expected),
            &strip_for_hash(actual),
            false,
            &mut Vec::new(),
            &mut differences,
        ),
    }
    differences.truncate(MAX_DIFFERENCES);
    differences
}

fn walk(
    expected: &Value,
    actual: &Value,
    subset: bool,
    path: &mut Vec<String>,
    out: &mut Vec<Difference>,
) {
    if out.len() >= MAX_DIFFERENCES {
        return;
    }
    match (expected, actual) {
        (Value::Object(wanted), Value::Object(found)) => {
            for (name, member) in wanted {
                path.push(name.clone());
                match found.get(name) {
                    Some(value) => walk(member, value, subset, path, out),
                    None => out.push(difference(path, "member is missing", Some(member), None)),
                }
                path.pop();
            }
            if !subset {
                for (name, value) in found.iter().filter(|(name, _)| !wanted.contains_key(*name)) {
                    path.push(name.clone());
                    out.push(difference(path, "unexpected member", None, Some(value)));
                    path.pop();
                }
            }
        }
        (Value::Array(wanted), Value::Array(found)) => {
            if wanted.len() != found.len() {
                let message = format!("{} items instead of {}", found.len(), wanted.len());
                out.push(difference(path, &message, Some(expected), Some(actual)));
                return;
            }
            for (index, (item, value)) in wanted.iter().zip(found).enumerate() {
                path.push(index.to_string());
                walk(item, value, subset, path, out);
                path.pop();
            }
        }
        (Value::Number(wanted), Value::Number(found)) => {
            if !numbers_equal(wanted, found) {
                out.push(difference(
                    path,
                    "values differ",
                    Some(expected),
                    Some(actual),
                ));
            }
        }
        _ => {
            if expected != actual {
                out.push(difference(
                    path,
                    "values differ",
                    Some(expected),
                    Some(actual),
                ));
            }
        }
    }
}

fn numbers_equal(a: &serde_json::Number, b: &serde_json::Number) -> bool {
    match (a.as_i64(), b.as_i64(), a.as_u64(), b.as_u64()) {
        (Some(x), Some(y), _, _) => x == y,
        (_, _, Some(x), Some(y)) => x == y,
        _ => a.as_f64() == b.as_f64(),
    }
}

fn difference(
    path: &[String],
    message: &str,
    expected: Option<&Value>,
    actual: Option<&Value>,
) -> Difference {
    Difference {
        path: pointer::join(path),
        message: message.to_owned(),
        expected: expected.cloned(),
        actual: actual.cloned(),
    }
}

/// Computes `ast_hash` of a surface or bound syntax tree.
///
/// The hash is `"sha256:"` followed by the lowercase hexadecimal SHA-256
/// digest of the RFC 8785 canonical JSON of the hash form of the tree.
///
/// # Errors
///
/// Returns an error when the tree cannot be written as canonical JSON.
///
/// # Examples
///
/// ```
/// use nurt_conformance::compare::ast_hash;
/// use serde_json::json;
///
/// let a = json!({"nurt": "1.0", "program": {"statements": []}, "meta": {"s": 1}});
/// let b = json!({"program": {"statements": []}, "nurt": "1.0"});
/// assert_eq!(ast_hash(&a)?, ast_hash(&b)?);
/// # Ok::<(), String>(())
/// ```
pub fn ast_hash(tree: &Value) -> Result<String, String> {
    let canonical =
        serde_json_canonicalizer::to_vec(&strip_for_hash(tree)).map_err(|e| e.to_string())?;
    Ok(format!(
        "{AST_HASH_PREFIX}{}",
        hex::encode(&Sha256::digest(&canonical)[..])
    ))
}

/// Computes the SHA-256 digest of bytes as `"sha256:"` followed by
/// lowercase hexadecimal digits.
#[must_use]
pub fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(&Sha256::digest(bytes)[..]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn subset_ignores_unlisted_members_but_not_array_lengths() {
        let expected = json!({"a": [1, {"b": 2}]});
        assert!(
            compare(
                &expected,
                &json!({"a": [1, {"b": 2, "c": 3}], "d": 4}),
                Match::Subset
            )
            .is_empty()
        );
        let short = compare(&expected, &json!({"a": [1]}), Match::Subset);
        assert_eq!(short[0].path, "/a");
        assert_eq!(short[0].message, "1 items instead of 2");
    }

    #[test]
    fn missing_and_unexpected_members_are_named() {
        let missing = compare(&json!({"x": {"y": 1}}), &json!({"x": {}}), Match::Subset);
        assert_eq!(missing[0].path, "/x/y");
        assert_eq!(missing[0].message, "member is missing");
        let extra = compare(&json!({}), &json!({"z~/": 1}), Match::Exact);
        assert_eq!(extra[0].path, "/z~0~1");
        assert_eq!(extra[0].message, "unexpected member");
    }

    #[test]
    fn numbers_compare_by_value() {
        assert!(compare(&json!(1), &json!(1.0), Match::Exact).is_empty());
        assert!(compare(&json!(u64::MAX), &json!(u64::MAX), Match::Exact).is_empty());
        assert_eq!(compare(&json!(1), &json!(2), Match::Exact).len(), 1);
        assert_eq!(compare(&json!("1"), &json!(1), Match::Exact).len(), 1);
    }

    #[test]
    fn tree_mode_ignores_spans_and_meta() {
        let a = json!({"nurt": "1.0", "meta": {"x": 1}, "program": {"statements": [{"t": "pipeline", "span": [1, 1, 1, 5]}]}});
        let b = json!({"nurt": "1.0", "program": {"statements": [{"t": "pipeline", "span": [2, 1, 2, 5]}]}});
        assert!(compare(&a, &b, Match::Tree).is_empty());
        assert!(!compare(&a, &b, Match::Exact).is_empty());
    }

    #[test]
    fn number_of_differences_is_limited() {
        let expected: Value = (0..50)
            .map(|i| (i.to_string(), json!(i)))
            .collect::<serde_json::Map<_, _>>()
            .into();
        assert_eq!(
            compare(&expected, &json!({}), Match::Subset).len(),
            MAX_DIFFERENCES
        );
    }

    #[test]
    fn mode_names_are_parsed() {
        assert_eq!(Match::from_name("tree"), Some(Match::Tree));
        assert_eq!(Match::from_name("exact"), Some(Match::Exact));
        assert_eq!(Match::from_name("subset"), Some(Match::Subset));
        assert_eq!(Match::from_name("fuzzy"), None);
    }

    #[test]
    fn ast_hash_matches_a_golden_value() {
        let tree = json!({"nurt": "1.0", "program": {"statements": []}});
        let hash = ast_hash(&tree).unwrap();
        assert!(hash.starts_with("sha256:"));
        assert_eq!(hash.len(), "sha256:".len() + 64);
    }
}
