// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::keypath::{KeyPath, KeySegment};
use nurt_contracts::value::{Fields, Value};

use crate::edit::guard::{Expectation, Surroundings};
use crate::model::{DocTree, FormatSemantics, NodeId, NodeKind, SetOptions, Span};
use crate::registry::{DocFormat, DocOptions, OpenDocument, matches_file, open_with_path};
use crate::value::first_difference;

/// Increment of the SplitMix64 generator.
const SPLITMIX_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;
/// First multiplier of the SplitMix64 output function.
const SPLITMIX_MUL1: u64 = 0xBF58_476D_1CE4_E5B9;
/// Second multiplier of the SplitMix64 output function.
const SPLITMIX_MUL2: u64 = 0x94D0_49BB_1331_11EB;
/// Offset basis of the 64-bit FNV-1a hash.
const FNV_OFFSET: u64 = 0xCBF2_9CE4_8422_2325;
/// Prime of the 64-bit FNV-1a hash.
const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;
/// Nodes selected per file by default.
const DEFAULT_NODES_PER_FILE: usize = 50;
/// Seed of the node selection by default.
const DEFAULT_SEED: u64 = 0x6E75_7274;
/// Name of the member created by [`HarnessOperation::InsertKey`].
const NEW_KEY: &str = "nurt_new_key";
/// Text written by [`HarnessOperation::ReplaceScalar`] and
/// [`HarnessOperation::ChangeType`].
const NEW_TEXT: &str = "nurt-test";

/// Edit operation applied by the harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HarnessOperation {
    /// Setting the current value of a node; must change no byte.
    SetSame,
    /// Setting another value of the same type on a scalar.
    ReplaceScalar,
    /// Setting a map on a scalar, or a text on a map or list.
    ChangeType,
    /// Creating a new member in a map or element.
    InsertKey,
    /// Appending an element to a list (`[+]`).
    Append,
    /// Removing a node.
    Remove,
}

impl HarnessOperation {
    /// Every operation.
    pub const ALL: [Self; 6] = [
        Self::SetSame,
        Self::ReplaceScalar,
        Self::ChangeType,
        Self::InsertKey,
        Self::Append,
        Self::Remove,
    ];
}

/// Which files of a corpus directory are run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FileFilter {
    /// Files whose name or extension belongs to the format.
    #[default]
    ByFormat,
    /// Every file.
    All,
}

/// Settings of a harness run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessOptions {
    /// Seed of the node selection; the selection also depends on the file
    /// name, so it is stable when files are added.
    pub seed: u64,
    /// Largest number of nodes selected per file.
    pub nodes_per_file: usize,
    /// Operations applied to every selected node.
    pub operations: Vec<HarnessOperation>,
    /// Files of a corpus directory that are run.
    pub filter: FileFilter,
    /// Options of opening documents.
    pub documents: DocOptions,
}

impl Default for HarnessOptions {
    fn default() -> Self {
        Self {
            seed: DEFAULT_SEED,
            nodes_per_file: DEFAULT_NODES_PER_FILE,
            operations: HarnessOperation::ALL.to_vec(),
            filter: FileFilter::ByFormat,
            documents: DocOptions::default(),
        }
    }
}

/// Guard whose independent check failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Guard {
    /// Bytes outside the lines of the node changed.
    Scope,
    /// The changed document reads back as another value.
    Meaning,
    /// A document does not keep its bytes on opening.
    Identity,
    /// Setting the current value changed bytes.
    Idempotence,
}

/// Failed independent check of an accepted change or of an opened file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// File.
    pub path: PathBuf,
    /// Key of the node; empty for violations of opening.
    pub key: String,
    /// Operation; `None` for violations of opening.
    pub operation: Option<HarnessOperation>,
    /// Failed check.
    pub guard: Guard,
    /// Description.
    pub detail: String,
}

/// Results of a harness run.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Summary {
    /// Files run.
    pub files: usize,
    /// Files that the format refused to open, with the error message
    /// (expected for corpus files of invalid documents).
    pub not_opened: Vec<(PathBuf, String)>,
    /// Operations attempted.
    pub operations: usize,
    /// Operations accepted by the format.
    pub accepted: usize,
    /// Failed independent checks.
    pub violations: Vec<Violation>,
    /// Refusals with `E_UNSUPPORTED_CONSTRUCT`, by construct name.
    pub unsupported: BTreeMap<String, usize>,
    /// Other refusals, by error code.
    pub refused: BTreeMap<String, usize>,
}

impl Summary {
    /// Adds the results of `other`.
    pub fn merge(&mut self, other: Self) {
        self.files += other.files;
        self.not_opened.extend(other.not_opened);
        self.operations += other.operations;
        self.accepted += other.accepted;
        self.violations.extend(other.violations);
        for (name, count) in other.unsupported {
            *self.unsupported.entry(name).or_default() += count;
        }
        for (code, count) in other.refused {
            *self.refused.entry(code).or_default() += count;
        }
    }

    fn refusal(&mut self, error: &Error) {
        if error.code == ErrorCode::UnsupportedConstruct {
            let name = error.found.clone().unwrap_or_default();
            *self.unsupported.entry(name).or_default() += 1;
        } else {
            *self
                .refused
                .entry(error.code.as_str().to_owned())
                .or_default() += 1;
        }
    }
}

/// Runs the harness on every file under `dir`, recursively, in name order.
///
/// # Errors
///
/// Fails with the error of reading the directory or a file.
pub fn run_corpus(
    dir: &Path,
    format: &dyn DocFormat,
    options: &HarnessOptions,
) -> io::Result<Summary> {
    let mut files = Vec::new();
    collect(dir, &mut files)?;
    files.sort();
    let mut summary = Summary::default();
    for path in files {
        if options.filter == FileFilter::ByFormat && !matches_file(format, &path) {
            continue;
        }
        let bytes = std::fs::read(&path)?;
        summary.merge(run_document(&path, &bytes, format, options));
    }
    Ok(summary)
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect(&entry.path(), files)?;
        } else if kind.is_file() {
            files.push(entry.path());
        }
    }
    Ok(())
}

/// Runs the harness on one document.
pub fn run_document(
    path: impl AsRef<Path>,
    bytes: &[u8],
    format: &dyn DocFormat,
    options: &HarnessOptions,
) -> Summary {
    let path = path.as_ref();
    let mut summary = Summary {
        files: 1,
        ..Summary::default()
    };
    let document = match open_with_path(format, Some(path), bytes, &options.documents) {
        Ok(document) => document,
        Err(error) => {
            if error.code == ErrorCode::UnsupportedConstruct {
                summary
                    .violations
                    .push(opening(path, Guard::Identity, &error.message));
            } else {
                summary
                    .not_opened
                    .push((path.to_owned(), error.message.clone()));
            }
            return summary;
        }
    };
    if document.bytes() != bytes {
        summary.violations.push(opening(
            path,
            Guard::Identity,
            "the bytes of the opened document differ",
        ));
        return summary;
    }
    let run = Run {
        path,
        bytes,
        format,
        options,
    };
    let tree = document.handle.tree();
    for node in select(tree, document.handle.semantics(), options, path) {
        for operation in &options.operations {
            run.operation(&document, node, *operation, &mut summary);
        }
    }
    summary
}

fn opening(path: &Path, guard: Guard, detail: &str) -> Violation {
    Violation {
        path: path.to_owned(),
        key: String::new(),
        operation: None,
        guard,
        detail: detail.to_owned(),
    }
}

/// Nodes of the tree in a seeded order, at most `nodes_per_file`.
///
/// Nodes that their own key path does not select, such as members shadowed
/// by a later member of the same name, are skipped: an operation on their
/// key would change another node.
fn select(
    tree: &DocTree,
    semantics: &dyn FormatSemantics,
    options: &HarnessOptions,
    path: &Path,
) -> Vec<NodeId> {
    let mut nodes: Vec<NodeId> = tree
        .iter()
        .filter(|(_, node)| node.kind != NodeKind::Comment)
        .map(|(id, _)| id)
        .collect();
    let mut state = options.seed ^ fnv1a(path.to_string_lossy().as_bytes());
    let mut selected = Vec::with_capacity(nodes.len().min(options.nodes_per_file));
    let mut index = 0;
    while selected.len() < options.nodes_per_file && index < nodes.len() {
        let remaining = u64::try_from(nodes.len() - index).unwrap_or(u64::MAX);
        let pick = index + usize::try_from(splitmix(&mut state) % remaining).unwrap_or(0);
        nodes.swap(index, pick);
        if reachable(tree, semantics, nodes[index]) {
            selected.push(nodes[index]);
        }
        index += 1;
    }
    selected
}

/// Returns `true` when the key path of `node` selects exactly that node
/// (or an alias with the same target).
fn reachable(tree: &DocTree, semantics: &dyn FormatSemantics, node: NodeId) -> bool {
    let key = tree.path_of(node, semantics);
    tree.resolve(&key, semantics).is_ok_and(
        |found| matches!(found.as_slice(), [only] if tree.target(only.node) == tree.target(node)),
    )
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(SPLITMIX_GAMMA);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(SPLITMIX_MUL1);
    z = (z ^ (z >> 27)).wrapping_mul(SPLITMIX_MUL2);
    z ^ (z >> 31)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FNV_OFFSET, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

struct Run<'r> {
    path: &'r Path,
    bytes: &'r [u8],
    format: &'r dyn DocFormat,
    options: &'r HarnessOptions,
}

/// Change requested by one operation on one node.
enum Change {
    Set(KeyPath, Value),
    Remove(KeyPath),
}

impl Run<'_> {
    fn operation(
        &self,
        original: &OpenDocument,
        node: NodeId,
        operation: HarnessOperation,
        summary: &mut Summary,
    ) {
        let tree = original.handle.tree();
        let semantics = original.handle.semantics();
        let key = tree.path_of(node, semantics);
        let Some(change) = change_for(tree, node, &key, operation, semantics) else {
            return;
        };
        let Ok(mut document) = open_with_path(
            self.format,
            Some(self.path),
            self.bytes,
            &self.options.documents,
        ) else {
            return;
        };
        summary.operations += 1;
        let result = match &change {
            Change::Set(key, value) => {
                document
                    .handle
                    .set(key, value, &SetOptions { create: true })
            }
            Change::Remove(key) => document.handle.remove(key),
        };
        match result {
            Ok(report) => {
                summary.accepted += 1;
                let after = document.handle.text().to_vec();
                let violation = if operation == HarnessOperation::SetSame {
                    (!report.is_unchanged() || after != original.handle.text()).then(|| {
                        (
                            Guard::Idempotence,
                            "setting the current value changed bytes".to_owned(),
                        )
                    })
                } else {
                    self.verify(original, node, &change, &after)
                };
                if let Some((guard, detail)) = violation {
                    summary.violations.push(Violation {
                        path: self.path.to_owned(),
                        key: key.to_string(),
                        operation: Some(operation),
                        guard,
                        detail,
                    });
                }
            }
            Err(error) => summary.refusal(&error),
        }
    }

    /// Independent scope and meaning checks of an accepted change.
    fn verify(
        &self,
        original: &OpenDocument,
        node: NodeId,
        change: &Change,
        after: &[u8],
    ) -> Option<(Guard, String)> {
        let text = original.handle.text();
        let tree = original.handle.tree();
        let region = lines_of(text, tree[node].span);
        if let Some(offset) = first_change_outside(text, after, region) {
            let position = tree.lines().position(text, offset);
            return Some((
                Guard::Scope,
                format!(
                    "bytes outside the lines of the node changed at line {}, column {}",
                    position.line, position.column
                ),
            ));
        }
        let semantics = original.handle.semantics();
        let documents: Vec<Value> = tree
            .roots()
            .iter()
            .map(|root| tree.value_of(*root, semantics, None))
            .collect();
        let expectation = match change {
            Change::Set(key, value) => {
                Expectation::new(documents, semantics).set(key.clone(), value.clone())
            }
            Change::Remove(key) => Expectation::new(documents, semantics).remove(key.clone()),
        };
        let expected = match expectation.expected() {
            Ok(expected) => expected,
            Err(error) => return Some((Guard::Meaning, error.message.clone())),
        };
        let reopened =
            match open_with_path(self.format, Some(self.path), after, &self.options.documents) {
                Ok(reopened) => reopened,
                Err(error) => {
                    return Some((
                        Guard::Meaning,
                        format!("the changed document does not open: {}", error.message),
                    ));
                }
            };
        let reopened_tree = reopened.handle.tree();
        let reopened_semantics = reopened.handle.semantics();
        let actual: Vec<Value> = reopened_tree
            .roots()
            .iter()
            .map(|root| {
                reopened_semantics.canonical(reopened_tree.value_of(
                    *root,
                    reopened_semantics,
                    None,
                ))
            })
            .collect();
        let expected: Vec<Value> = expected
            .into_iter()
            .map(|document| semantics.canonical(document))
            .collect();
        first_difference(&Value::List(expected), &Value::List(actual)).map(|path| {
            (
                Guard::Meaning,
                format!("the changed document differs at `{path}`"),
            )
        })
    }
}

/// Offset in `original` of the first byte outside `region` that `changed`
/// does not keep, or `None` when every byte outside `region` is kept.
///
/// The bytes after `region` must end `changed` and the bytes before it must
/// start `changed`. The texts are not aligned line by line, so a removed
/// entry next to similar entries is checked at its own place. One run of
/// separator bytes ([`Surroundings::DEFAULT`]: blanks, line breaks, the
/// comma) may be removed or added between the region and the last
/// unchanged byte before it, as when removing the last member of a list
/// also removes the comma of the previous member, past a comment.
///
/// # Examples
///
/// ```
/// use nurt_doc::model::Span;
/// use nurt_doc::testing::first_change_outside;
///
/// let original = b"[1, // one\n 2\n]";
/// let region = Span::new(11, 14);
/// assert_eq!(first_change_outside(original, b"[1 // one\n]", region), None);
/// assert_eq!(first_change_outside(original, b"[0, // one\n]", region), Some(1));
/// ```
#[must_use]
pub fn first_change_outside(original: &[u8], changed: &[u8], region: Span) -> Option<usize> {
    let start = region.start.min(original.len());
    let end = region.end.clamp(start, original.len());
    let tail = &original[end..];
    if !changed.ends_with(tail) {
        let kept = common_suffix(original, changed);
        return Some(original.len() - kept - 1);
    }
    let changed_end = changed.len() - tail.len();
    let prefix = common_prefix(&original[..start], &changed[..changed_end.min(start)]);
    let kept = prefix == start
        || separators_removed(original, changed, prefix, start, changed_end)
        || separators_added(original, changed, prefix, start, changed_end);
    (!kept).then_some(prefix)
}

/// Returns `true` when `original[at..]` starts with a run of separators
/// whose removal makes `original[..start]` a prefix of
/// `changed[..changed_end]`.
fn separators_removed(
    original: &[u8],
    changed: &[u8],
    at: usize,
    start: usize,
    changed_end: usize,
) -> bool {
    let mut skipped = at;
    while skipped < start && Surroundings::DEFAULT.allows(original[skipped]) {
        skipped += 1;
        let rest = &original[skipped..start];
        let to = at + rest.len();
        if to <= changed_end && changed[at..to] == *rest {
            return true;
        }
    }
    false
}

/// Returns `true` when `changed[at..]` starts with a run of separators
/// after which `changed[..changed_end]` continues with `original[at..start]`.
fn separators_added(
    original: &[u8],
    changed: &[u8],
    at: usize,
    start: usize,
    changed_end: usize,
) -> bool {
    let rest = &original[at..start];
    let mut skipped = at;
    while skipped < changed_end && Surroundings::DEFAULT.allows(changed[skipped]) {
        skipped += 1;
        let to = skipped + rest.len();
        if to <= changed_end && changed[skipped..to] == *rest {
            return true;
        }
    }
    false
}

fn common_prefix(left: &[u8], right: &[u8]) -> usize {
    left.iter().zip(right).take_while(|(a, b)| a == b).count()
}

fn common_suffix(left: &[u8], right: &[u8]) -> usize {
    left.iter()
        .rev()
        .zip(right.iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
}

/// `span` widened to whole lines, with the line break of the last line.
fn lines_of(text: &[u8], span: Span) -> Span {
    let start = text[..span.start.min(text.len())]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |at| at + 1);
    let end = text[span.end.min(text.len())..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(text.len(), |at| span.end + at + 1);
    Span::new(start, end)
}

fn change_for(
    tree: &DocTree,
    node: NodeId,
    key: &KeyPath,
    operation: HarnessOperation,
    semantics: &dyn FormatSemantics,
) -> Option<Change> {
    let kind = tree[tree.target(node)].kind;
    let scalar = !kind.is_container();
    let child = |segment: KeySegment| {
        let mut extended = key.clone();
        extended.push(segment).ok().map(|()| extended)
    };
    match operation {
        HarnessOperation::SetSame => Some(Change::Set(
            key.clone(),
            tree.value_of(node, semantics, None),
        )),
        HarnessOperation::ReplaceScalar if scalar => {
            let current = tree.value_of(node, semantics, None);
            Some(Change::Set(key.clone(), replacement(&current)))
        }
        HarnessOperation::ChangeType if scalar => {
            let mut fields = Fields::new();
            fields.insert(NEW_KEY.to_owned(), Value::Int(1));
            Some(Change::Set(key.clone(), Value::Map(fields)))
        }
        HarnessOperation::ChangeType => {
            Some(Change::Set(key.clone(), Value::Text(NEW_TEXT.to_owned())))
        }
        HarnessOperation::InsertKey if kind.is_named_container() => {
            child(KeySegment::Key(NEW_KEY.to_owned())).map(|key| Change::Set(key, Value::Int(1)))
        }
        HarnessOperation::Append if kind == NodeKind::List => {
            child(KeySegment::Append).map(|key| Change::Set(key, Value::Int(1)))
        }
        HarnessOperation::Remove if tree[node].parent.is_some() => {
            Some(Change::Remove(key.clone()))
        }
        _ => None,
    }
}

/// Another value of the same type.
fn replacement(current: &Value) -> Value {
    match current {
        Value::Bool(flag) => Value::Bool(!flag),
        Value::Int(number) => Value::Int(number.wrapping_add(1)),
        Value::Number(number) => Value::Number(number + 1.0),
        Value::Text(text) if text == NEW_TEXT => Value::Text(format!("{NEW_TEXT}-2")),
        _ => Value::Text(NEW_TEXT.to_owned()),
    }
}
