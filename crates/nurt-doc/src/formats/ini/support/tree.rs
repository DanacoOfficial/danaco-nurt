// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Key tree of a document and resolution of key paths against it.
//!
//! A format builds the tree from its parsed form: leaves refer to scalar
//! entries of the format by index, maps keep the order of the document.

use nurt_contracts::error::Error;
use nurt_contracts::keypath::{CompareOp, FilterExpr, FilterLiteral, KeyPath, KeySegment};
use nurt_contracts::value::Value;

use super::errors::{key_conflict, key_not_found};

/// Node of a key tree.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Node {
    /// Scalar entry with the given index in the format's entry table.
    Leaf(usize),
    /// Members in document order.
    Map(Map),
    /// Elements in document order.
    List(Vec<Node>),
    /// Key written more than once where the format does not define the
    /// repetition as a list; selecting it requires an index.
    Repeated(Vec<Node>),
}

/// Members of a map node.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Map {
    pub(crate) members: Vec<(String, Node)>,
    /// Member names are compared without regard to ASCII case; an exact
    /// match takes precedence.
    pub(crate) fold_case: bool,
    /// Format-defined identifiers of the constructs that form the map, for
    /// example the sections of an INI file that share a name.
    pub(crate) ids: Vec<usize>,
}

impl Map {
    /// Empty map comparing names exactly or without regard to ASCII case.
    pub(crate) fn new(fold_case: bool) -> Self {
        Self {
            members: Vec::new(),
            fold_case,
            ids: Vec::new(),
        }
    }

    /// Position of the member named `name`.
    pub(crate) fn position(&self, name: &str) -> Option<usize> {
        self.members
            .iter()
            .position(|(member, _)| member == name)
            .or_else(|| {
                self.members
                    .iter()
                    .position(|(member, _)| self.fold_case && member.eq_ignore_ascii_case(name))
            })
    }

    /// Member named `name`.
    pub(crate) fn get(&self, name: &str) -> Option<&Node> {
        self.position(name).map(|i| &self.members[i].1)
    }

    /// Member named `name`, inserted with `make` when missing.
    pub(crate) fn entry(&mut self, name: &str, make: impl FnOnce() -> Node) -> &mut Node {
        let index = match self.position(name) {
            Some(index) => index,
            None => {
                self.members.push((name.to_owned(), make()));
                self.members.len() - 1
            }
        };
        &mut self.members[index].1
    }
}

/// Node selected by a key path, with its concrete path.
#[derive(Debug, Clone)]
pub(crate) struct Hit<'a> {
    pub(crate) path: Vec<KeySegment>,
    pub(crate) node: &'a Node,
}

impl Hit<'_> {
    /// Canonical text of the concrete path.
    pub(crate) fn key(&self) -> String {
        print_path(&self.path)
    }
}

/// Canonical text of a list of segments.
pub(crate) fn print_path(path: &[KeySegment]) -> String {
    KeyPath::new(None, path.to_vec()).map_or_else(|_| String::new(), |key| key.to_string())
}

/// Resolves `key` to the selected nodes.
///
/// `value_of` converts a node to its value for filter segments.
///
/// # Errors
///
/// Fails with `E_KEY_NOT_FOUND` when a concrete path names a missing member,
/// `E_KEY_CONFLICT` when the path passes through a scalar, indexes a map,
/// selects a repeated key without an index or ends with `[+]`.
pub(crate) fn resolve<'a>(
    root: &'a Node,
    key: &KeyPath,
    value_of: &dyn Fn(&Node) -> Value,
) -> Result<Vec<Hit<'a>>, Error> {
    let mut frontier = vec![Hit {
        path: Vec::new(),
        node: root,
    }];
    let concrete = key.is_concrete();
    for segment in key.segments() {
        let mut next = Vec::new();
        for hit in frontier {
            step(&hit, segment, concrete, value_of, &mut next)?;
        }
        frontier = next;
    }
    if let Some(hit) = frontier
        .iter()
        .find(|hit| matches!(hit.node, Node::Repeated(_)))
    {
        return Err(repeated_conflict(hit));
    }
    Ok(frontier)
}

fn step<'a>(
    hit: &Hit<'a>,
    segment: &KeySegment,
    concrete: bool,
    value_of: &dyn Fn(&Node) -> Value,
    out: &mut Vec<Hit<'a>>,
) -> Result<(), Error> {
    match segment {
        KeySegment::Key(name) => step_key(hit, name, concrete, out),
        KeySegment::Index(index) => step_index(hit, *index, concrete, out),
        KeySegment::Wildcard => {
            children(hit, out);
            Ok(())
        }
        KeySegment::Recursive => {
            descendants(hit, out);
            Ok(())
        }
        KeySegment::Filter(expr) => {
            let mut all = Vec::new();
            children(hit, &mut all);
            out.extend(
                all.into_iter()
                    .filter(|child| holds(expr, &value_of(child.node))),
            );
            Ok(())
        }
        KeySegment::Append => Err(key_conflict(
            "`[+]` addresses no existing value; use it only to add an element",
            "append marker",
            &[],
        )),
        _ => Err(key_conflict(
            "the key path segment is not supported by this format",
            "segment",
            &[],
        )),
    }
}

fn step_key<'a>(
    hit: &Hit<'a>,
    name: &str,
    concrete: bool,
    out: &mut Vec<Hit<'a>>,
) -> Result<(), Error> {
    match hit.node {
        Node::Map(map) => match map.get(name) {
            Some(node) => {
                out.push(child(hit, KeySegment::Key(name.to_owned()), node));
                Ok(())
            }
            None if concrete => Err(key_not_found(
                &print_path(&extend(&hit.path, KeySegment::Key(name.to_owned()))),
                &closest(map, name, &hit.path),
            )),
            None => Ok(()),
        },
        Node::List(_) => match name.parse::<i64>() {
            Ok(index) => step_index(hit, index, concrete, out),
            Err(_) => conflict_or_skip(concrete, hit, "list"),
        },
        Node::Repeated(_) => Err(repeated_conflict(hit)),
        Node::Leaf(_) => conflict_or_skip(concrete, hit, "scalar"),
    }
}

fn step_index<'a>(
    hit: &Hit<'a>,
    index: i64,
    concrete: bool,
    out: &mut Vec<Hit<'a>>,
) -> Result<(), Error> {
    let elements = match hit.node {
        Node::List(elements) | Node::Repeated(elements) => elements,
        Node::Map(_) => return conflict_or_skip(concrete, hit, "map"),
        Node::Leaf(_) => return conflict_or_skip(concrete, hit, "scalar"),
    };
    match normalize_index(index, elements.len()) {
        Some(position) => {
            out.push(child(
                hit,
                KeySegment::Index(to_i64(position)),
                &elements[position],
            ));
            Ok(())
        }
        None if concrete => Err(key_not_found(
            &print_path(&extend(&hit.path, KeySegment::Index(index))),
            &[],
        )),
        None => Ok(()),
    }
}

/// Position of `index` (negative counts from the end) in a list of `len`
/// elements.
pub(crate) fn normalize_index(index: i64, len: usize) -> Option<usize> {
    let len = to_i64(len);
    let position = if index < 0 { len + index } else { index };
    if (0..len).contains(&position) {
        usize::try_from(position).ok()
    } else {
        None
    }
}

fn to_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn conflict_or_skip(concrete: bool, hit: &Hit<'_>, found: &str) -> Result<(), Error> {
    if concrete {
        Err(key_conflict(
            &format!(
                "key path passes through a {found} at `{}`; address an existing member",
                hit.key()
            ),
            found,
            &[],
        ))
    } else {
        Ok(())
    }
}

fn repeated_conflict(hit: &Hit<'_>) -> Error {
    let count = match hit.node {
        Node::Repeated(elements) => elements.len(),
        _ => 0,
    };
    let candidates: Vec<String> = (0..count)
        .map(|i| print_path(&extend(&hit.path, KeySegment::Index(to_i64(i)))))
        .collect();
    key_conflict(
        &format!(
            "key `{}` is written {count} times; select one occurrence by index or all with `[*]`",
            hit.key()
        ),
        "repeated key",
        &candidates,
    )
}

fn extend(path: &[KeySegment], segment: KeySegment) -> Vec<KeySegment> {
    let mut path = path.to_vec();
    path.push(segment);
    path
}

fn child<'a>(hit: &Hit<'a>, segment: KeySegment, node: &'a Node) -> Hit<'a> {
    Hit {
        path: extend(&hit.path, segment),
        node,
    }
}

fn children<'a>(hit: &Hit<'a>, out: &mut Vec<Hit<'a>>) {
    match hit.node {
        Node::Map(map) => {
            for (name, node) in &map.members {
                let member = child(hit, KeySegment::Key(name.clone()), node);
                if let Node::Repeated(elements) = node {
                    for (i, element) in elements.iter().enumerate() {
                        out.push(child(&member, KeySegment::Index(to_i64(i)), element));
                    }
                } else {
                    out.push(member);
                }
            }
        }
        Node::List(elements) | Node::Repeated(elements) => {
            for (i, element) in elements.iter().enumerate() {
                out.push(child(hit, KeySegment::Index(to_i64(i)), element));
            }
        }
        Node::Leaf(_) => {}
    }
}

fn descendants<'a>(hit: &Hit<'a>, out: &mut Vec<Hit<'a>>) {
    if !matches!(hit.node, Node::Repeated(_)) {
        out.push(hit.clone());
    }
    let mut direct = Vec::new();
    children(hit, &mut direct);
    for child in &direct {
        descendants(child, out);
    }
}

fn closest(map: &Map, name: &str, path: &[KeySegment]) -> Vec<String> {
    let mut ranked: Vec<(usize, &str)> = map
        .members
        .iter()
        .map(|(member, _)| (distance(&member.to_lowercase(), &name.to_lowercase()), member.as_str()))
        .collect();
    ranked.sort();
    ranked
        .into_iter()
        .map(|(_, member)| print_path(&extend(path, KeySegment::Key(member.to_owned()))))
        .collect()
}

/// Levenshtein distance in Unicode scalar values.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitution = previous + usize::from(ca != *cb);
            previous = row[j + 1];
            row[j + 1] = substitution.min(row[j] + 1).min(previous + 1);
        }
    }
    row[b.len()]
}

/// Evaluates a filter expression for one candidate value.
fn holds(expr: &FilterExpr, value: &Value) -> bool {
    match expr {
        FilterExpr::And { left, right } => holds(left, value) && holds(right, value),
        FilterExpr::Or { left, right } => holds(left, value) || holds(right, value),
        FilterExpr::Not { operand } => !holds(operand, value),
        FilterExpr::Compare { op, left, right } => {
            match (operand(left, value), operand(right, value)) {
                (Some(l), Some(r)) => compare(*op, &l, &r),
                _ => false,
            }
        }
        other => operand(other, value)
            .is_some_and(|v| !matches!(v, Value::Null | Value::Bool(false))),
    }
}

fn operand(expr: &FilterExpr, value: &Value) -> Option<Value> {
    match expr {
        FilterExpr::Field { path } => path
            .iter()
            .try_fold(value, |current, name| current.get(name))
            .cloned(),
        FilterExpr::Literal { value: literal } => Some(match literal {
            FilterLiteral::Text(text) => Value::Text(text.clone()),
            FilterLiteral::Int(int) => Value::Int(*int),
            FilterLiteral::Number(number) => Value::Number(*number),
            FilterLiteral::Bool(flag) => Value::Bool(*flag),
            _ => Value::Null,
        }),
        other => Some(Value::Bool(holds(other, value))),
    }
}

fn compare(op: CompareOp, left: &Value, right: &Value) -> bool {
    let ordering = match (left, right) {
        (Value::Text(l), Value::Text(r)) => l.partial_cmp(r),
        (Value::Bool(l), Value::Bool(r)) => l.partial_cmp(r),
        (Value::Null, Value::Null) => Some(std::cmp::Ordering::Equal),
        _ => match (left.as_number(), right.as_number()) {
            (Some(l), Some(r)) => l.partial_cmp(&r),
            _ => None,
        },
    };
    match ordering {
        Some(ordering) => match op {
            CompareOp::Eq => ordering.is_eq(),
            CompareOp::Ne => ordering.is_ne(),
            CompareOp::Lt => ordering.is_lt(),
            CompareOp::Le => ordering.is_le(),
            CompareOp::Gt => ordering.is_gt(),
            CompareOp::Ge => ordering.is_ge(),
        },
        None => op == CompareOp::Ne,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Node {
        let mut section = Map::new(false);
        section.members.push(("a".into(), Node::Leaf(0)));
        section
            .members
            .push(("dup".into(), Node::Repeated(vec![Node::Leaf(1), Node::Leaf(2)])));
        section
            .members
            .push(("list".into(), Node::List(vec![Node::Leaf(3), Node::Leaf(4)])));
        let mut root = Map::new(true);
        root.members.push(("Sec".into(), Node::Map(section)));
        Node::Map(root)
    }

    fn value_of(node: &Node) -> Value {
        match node {
            Node::Leaf(i) => Value::Text(format!("v{i}")),
            _ => Value::Null,
        }
    }

    fn keys(text: &str) -> Result<Vec<String>, Error> {
        let root = sample();
        let key = KeyPath::parse(text).unwrap();
        Ok(resolve(&root, &key, &value_of)?
            .iter()
            .map(Hit::key)
            .collect())
    }

    #[test]
    fn resolves_members_with_folded_case_at_the_root() {
        assert_eq!(keys("sec.a").unwrap(), ["Sec.a"]);
        assert_eq!(keys("sec.list[-1]").unwrap(), ["Sec.list[1]"]);
        assert_eq!(keys("sec.list.0").unwrap(), ["Sec.list[0]"]);
    }

    #[test]
    fn repeated_keys_require_an_index() {
        let error = keys("Sec.dup").unwrap_err();
        assert_eq!(error.sample, ["Sec.dup[0]", "Sec.dup[1]"]);
        assert_eq!(keys("Sec.dup[1]").unwrap(), ["Sec.dup[1]"]);
        assert_eq!(keys("Sec.dup[*]").unwrap().len(), 2);
    }

    #[test]
    fn wildcards_and_recursion_expand_every_member() {
        assert_eq!(keys("Sec.*").unwrap().len(), 5);
        assert_eq!(keys("Sec.**").unwrap().len(), 8);
        assert!(keys("Sec.a.*").unwrap().is_empty());
    }

    #[test]
    fn filters_select_by_value() {
        assert_eq!(keys("Sec.list[?@ == \"v4\"]").unwrap(), ["Sec.list[1]"]);
        assert_eq!(keys("Sec.list[?@ != \"v4\" and !(@ == 1)]").unwrap().len(), 1);
        assert_eq!(keys("Sec.list[?@ > \"v3\" or @ == null]").unwrap().len(), 1);
    }

    #[test]
    fn missing_and_mismatched_keys_fail_with_candidates() {
        let missing = keys("Sec.aa").unwrap_err();
        assert_eq!(missing.code(), nurt_contracts::error::ErrorCode::KeyNotFound);
        assert_eq!(missing.sample.first().map(String::as_str), Some("Sec.a"));
        let through = keys("Sec.a.b").unwrap_err();
        assert_eq!(through.code(), nurt_contracts::error::ErrorCode::KeyConflict);
        assert!(keys("Sec[0]").is_err());
        assert!(keys("Sec.list[5]").is_err());
        assert!(keys("Sec.list[+]").is_err());
    }

    #[test]
    fn computes_edit_distance() {
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("", "ab"), 2);
    }
}
