// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Property lists in XML, binary and ASCII form.
//!
//! A [`PlistDocument`] reads the three forms of a property list into one
//! node tree. Key paths follow the property list structure: `a.b[0]` is
//! element 0 of the array under key `b` of the dictionary under key `a`, in
//! every form.
//!
//! - XML (`<plist>`): changes replace byte ranges of the source text. A
//!   scalar replaced by a value of the same type keeps its tags; a value of
//!   another type replaces the value element. New entries take the
//!   indentation of their siblings.
//! - ASCII (OpenStep, `{ a = b; }`, `( … )`, `<hex>`): changes replace byte
//!   ranges; quoting, typed GNUstep values, separators and comments of
//!   unchanged entries keep their bytes.
//! - Binary (`bplist00`): the format has no layout. A change writes the
//!   whole file again with the `plist` crate, deterministically, and carries
//!   the flag `binary_rewritten`; types (`integer`, `real`, `date`, `data`,
//!   `UID`) and the order of dictionary keys are kept.
//!
//! A repeated dictionary key selects its last entry.
//!
//! # Examples
//!
//! ```
//! use nurt_contracts::keypath::KeyPath;
//! use nurt_contracts::value::Value;
//! use nurt_doc::formats::plist::{PlistDocument, PlistKind};
//!
//! let text = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n<dict>\n\t<key>CFBundleVersion</key>\n\t<string>1</string>\n</dict>\n</plist>\n";
//! let document = PlistDocument::parse(text)?;
//! assert_eq!(document.kind(), PlistKind::Xml);
//!
//! let items = document.read(&KeyPath::parse("CFBundleVersion")?)?;
//! assert_eq!(items[0].value, Value::Text("1".to_owned()));
//!
//! let change = document.set(&KeyPath::parse("CFBundleVersion")?, &Value::Text("2".to_owned()), false)?;
//! let changed = String::from_utf8(change.apply(text))?;
//! assert!(changed.contains("\t<string>2</string>\n"));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod ascii;
mod base64;
mod binary;
mod handle;
mod layout;
mod plan;
mod value;
mod xml;

use std::ops::Range;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::keypath::{KeyPath, KeySegment};
use nurt_contracts::services::DocItem;
use nurt_contracts::value::Value;

pub use handle::{PlistFormat, PlistHandle};

use crate::model::{DocNode, DocTree, NodeId, NodeKind, Span, key_not_found};
use crate::registry::DocFormat;
use plan::{Op, SEMANTICS};
use value::{Hint, UID_TAG};

/// File name extensions of property lists, without the period.
pub const EXTENSIONS: [&str; 2] = ["plist", "entitlements"];

/// File names of property lists, lower case.
pub const FILE_NAMES: [&str; 1] = ["info.plist"];

/// Flag of a change that wrote a binary property list anew.
pub const BINARY_REWRITTEN: &str = "binary_rewritten";

/// Bytes at the start of a file searched for an XML property list marker.
const SNIFF_BYTES: usize = 1024;

/// Byte order mark of UTF-8.
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// The format `plist`, for registration in a
/// [`Registry`](crate::registry::Registry).
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::plist;
///
/// let ids: Vec<String> = plist::formats().iter().map(|format| format.id().to_string()).collect();
/// assert_eq!(ids, ["plist"]);
/// ```
#[must_use]
pub fn formats() -> Vec<Box<dyn DocFormat>> {
    vec![Box::new(PlistFormat)]
}

/// Form of a property list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlistKind {
    /// XML with the document element `plist`.
    Xml,
    /// Binary, signature `bplist00`.
    Binary,
    /// ASCII (OpenStep), including strings files without braces.
    Ascii,
}

impl PlistKind {
    /// Form of the property list in `bytes`: binary for the signature
    /// `bplist`, XML for text that starts with `<?`, `<!` or `<plist` after
    /// an optional byte order mark and white space, ASCII otherwise.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_doc::formats::plist::PlistKind;
    ///
    /// assert_eq!(PlistKind::of(b"bplist00"), PlistKind::Binary);
    /// assert_eq!(PlistKind::of(b"  <?xml version=\"1.0\"?><plist/>"), PlistKind::Xml);
    /// assert_eq!(PlistKind::of(b"<0fa1>"), PlistKind::Ascii);
    /// assert_eq!(PlistKind::of(b"{ a = b; }"), PlistKind::Ascii);
    /// ```
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        if bytes.starts_with(binary::SIGNATURE_PREFIX) {
            return Self::Binary;
        }
        let text = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
        let start = text
            .iter()
            .position(|byte| !byte.is_ascii_whitespace())
            .unwrap_or(text.len());
        let rest = &text[start..];
        if rest.starts_with(b"<?") || rest.starts_with(b"<!") || rest.starts_with(b"<plist") {
            Self::Xml
        } else {
            Self::Ascii
        }
    }
}

/// Form recognized from the start of a file without its name: binary for
/// the signature `bplist`, XML when the first kilobyte holds
/// `<!DOCTYPE plist` or `<plist`; `None` otherwise, ASCII property lists
/// included, which are recognized by name only.
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::plist::{PlistKind, sniff};
///
/// assert_eq!(sniff(b"bplist00\xd0\x08"), Some(PlistKind::Binary));
/// assert_eq!(sniff(b"<?xml version=\"1.0\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"\">"), Some(PlistKind::Xml));
/// assert_eq!(sniff(b"<project/>"), None);
/// ```
#[must_use]
pub fn sniff(content: &[u8]) -> Option<PlistKind> {
    if content.starts_with(binary::SIGNATURE_PREFIX) {
        return Some(PlistKind::Binary);
    }
    let head = &content[..content.len().min(SNIFF_BYTES)];
    let marked = contains(head, b"<!DOCTYPE plist") || contains(head, b"<plist");
    marked.then_some(PlistKind::Xml)
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Replacement of a byte range of a property list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlistEdit {
    /// Replaced bytes of the source; empty for an insertion.
    pub range: Range<usize>,
    /// New bytes.
    pub replacement: Vec<u8>,
}

/// Change of a property list: replacements of non-overlapping byte ranges
/// of the source, ordered by position, and the flags of the change.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlistChange {
    /// Replacements in ascending order of their ranges.
    pub edits: Vec<PlistEdit>,
    /// Markers of the change, [`BINARY_REWRITTEN`] for a binary property
    /// list written anew.
    pub flags: Vec<String>,
}

impl PlistChange {
    /// `true` when the change leaves the source as it is.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// Bytes after the change; `source` is the source of the document.
    #[must_use]
    pub fn apply(&self, source: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(source.len());
        let mut position = 0;
        for edit in &self.edits {
            out.extend_from_slice(&source[position..edit.range.start]);
            out.extend_from_slice(&edit.replacement);
            position = edit.range.end;
        }
        out.extend_from_slice(&source[position..]);
        out
    }
}

/// Syntax facts of each form; the binary form keeps its value.
#[derive(Debug, Clone)]
enum Form {
    Xml(xml::XmlSyntax),
    Ascii(ascii::AsciiSyntax),
    Binary(plist::Value),
}

/// Planned change with what the guards check: the changed spans of the
/// source, the operations on the document value and the effects.
#[derive(Debug, Clone, Default)]
pub(crate) struct Planned {
    /// The change.
    pub(crate) change: PlistChange,
    /// Spans of the source the change may alter.
    pub(crate) targets: Vec<Span>,
    /// Values set, by concrete key, as the document holds them afterwards.
    pub(crate) sets: Vec<(KeyPath, Value)>,
    /// Concrete keys removed, in the order of removal.
    pub(crate) removes: Vec<KeyPath>,
    /// Nodes changed, created or removed.
    pub(crate) replacements: u64,
    /// Intermediate keys were created.
    pub(crate) keys_created: bool,
}

/// Property list in XML, binary or ASCII form with its node tree.
#[derive(Debug, Clone)]
pub struct PlistDocument {
    kind: PlistKind,
    source: Vec<u8>,
    tree: DocTree,
    form: Form,
}

impl PlistDocument {
    /// Parses a property list of any form; see [`PlistKind::of`].
    ///
    /// # Errors
    ///
    /// Returns `E_ENCODING` for a text form that is not UTF-8, `E_DOC_PARSE`
    /// for a document that is not a property list and
    /// `E_UNSUPPORTED_CONSTRUCT` naming `plist.binary_layout` for a binary
    /// version other than `bplist00`.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let kind = PlistKind::of(bytes);
        if kind == PlistKind::Binary {
            let root = binary::read(bytes)?;
            let tree = binary::tree(bytes, &root)?;
            return Ok(Self {
                kind,
                source: bytes.to_vec(),
                tree,
                form: Form::Binary(root),
            });
        }
        let text = utf8(bytes)?;
        let base = if bytes.starts_with(UTF8_BOM) {
            UTF8_BOM.len()
        } else {
            0
        };
        let (tree, form) = if kind == PlistKind::Xml {
            let (tree, syntax) = xml::build(text, base)?;
            (tree, Form::Xml(syntax))
        } else {
            let (tree, syntax) = ascii::build(text, base)?;
            (tree, Form::Ascii(syntax))
        };
        Ok(Self {
            kind,
            source: bytes.to_vec(),
            tree,
            form,
        })
    }

    /// Form of the document.
    #[must_use]
    pub const fn kind(&self) -> PlistKind {
        self.kind
    }

    /// Source bytes.
    #[must_use]
    pub fn source(&self) -> &[u8] {
        &self.source
    }

    /// Node tree; nodes of a binary property list have empty spans.
    #[must_use]
    pub const fn tree(&self) -> &DocTree {
        &self.tree
    }

    /// Value of the whole document.
    #[must_use]
    pub fn to_value(&self) -> Value {
        self.tree
            .roots()
            .first()
            .map(|root| self.tree.value_of(*root, &SEMANTICS, None))
            .unwrap_or_default()
    }

    /// Values at `key`, one item per selected node; a `UID` carries the tag
    /// `uid`.
    ///
    /// # Errors
    ///
    /// Returns `E_KEY_NOT_FOUND` when the key selects nothing and the
    /// errors of [`DocTree::resolve`].
    pub fn read(&self, key: &KeyPath) -> Result<Vec<DocItem>> {
        let found = self.tree.resolve(key, &SEMANTICS)?;
        if found.is_empty() {
            return Err(key_not_found(key, &self.tree.similar_keys(key, &SEMANTICS)));
        }
        Ok(found
            .into_iter()
            .map(|found| self.item(found.node, found.key))
            .collect())
    }

    /// Change that stores `value` at `key`.
    ///
    /// A key that selects nodes replaces their values. A missing dictionary
    /// key is added with `create`; `[+]` and the index equal to the length
    /// of an array append. With `create`, missing intermediate keys become
    /// dictionaries (arrays for `[0]` and `[+]`). Storing the current value
    /// changes nothing.
    ///
    /// # Errors
    ///
    /// Returns `E_KEY_NOT_FOUND` for a missing key without `create`,
    /// `E_KEY_CONFLICT` for a path through a value of another kind, `E_TYPE`
    /// for a value without a property list type (null, regular expression)
    /// and `E_INVALID_VALUE` for an integer outside the 64-bit range or a
    /// date that is not RFC 3339.
    pub fn set(&self, key: &KeyPath, value: &Value, create: bool) -> Result<PlistChange> {
        Ok(self.plan_set(key, value, create)?.change)
    }

    /// Change that removes every node selected by `key`, with its key,
    /// separator and line.
    ///
    /// # Errors
    ///
    /// Returns `E_KEY_NOT_FOUND` when the key selects nothing and
    /// `E_KEY_CONFLICT` for the root value.
    pub fn remove(&self, key: &KeyPath) -> Result<PlistChange> {
        Ok(self.plan_remove(key)?.change)
    }

    /// Plans storing `value` at `key`.
    pub(crate) fn plan_set(&self, key: &KeyPath, value: &Value, create: bool) -> Result<Planned> {
        let ops = plan::plan_set(&self.tree, key, value, create)?;
        let mut planned = Planned::default();
        for op in &ops {
            let (target, concrete, stored) = self.expectation_of(op)?;
            planned.keys_created |= !matches!(op, Op::Replace { .. })
                && concrete.segments().len() < key.segments().len();
            planned.targets.push(target);
            planned.sets.push((concrete, stored));
        }
        planned.change = match &self.form {
            Form::Xml(syntax) => {
                text_change(plan::text_set(syntax, self.text()?, &self.tree, key, &ops)?)
            }
            Form::Ascii(syntax) => {
                text_change(plan::text_set(syntax, self.text()?, &self.tree, key, &ops)?)
            }
            Form::Binary(root) => {
                let mut changed = root.clone();
                for op in &ops {
                    self.apply_binary(&mut changed, op)?;
                }
                self.binary_change(root, &changed)?
            }
        };
        planned.replacements = count(&planned.change, ops.len());
        Ok(planned)
    }

    /// Plans removing the nodes selected by `key`.
    pub(crate) fn plan_remove(&self, key: &KeyPath) -> Result<Planned> {
        let mut nodes = with_repeated(&self.tree, plan::plan_remove(&self.tree, key)?);
        // Later nodes first: removing them keeps the indexes of earlier
        // siblings and the paths of enclosing nodes valid.
        nodes.sort_unstable_by(|left, right| right.cmp(left));
        nodes.dedup();
        let mut planned = Planned::default();
        for node in &nodes {
            let path = self.tree.path_of(*node, &SEMANTICS);
            if !planned.removes.contains(&path) {
                planned.removes.push(path);
            }
            let container = self.tree[*node].parent.unwrap_or(*node);
            planned.targets.push(self.target(container));
        }
        planned.change = match &self.form {
            Form::Xml(syntax) => text_change(plan::text_remove(
                syntax,
                self.text()?,
                &self.tree,
                key,
                &nodes,
            )?),
            Form::Ascii(syntax) => text_change(plan::text_remove(
                syntax,
                self.text()?,
                &self.tree,
                key,
                &nodes,
            )?),
            Form::Binary(root) => {
                let mut changed = root.clone();
                for path in &planned.removes {
                    binary::remove(&mut changed, path.segments())?;
                }
                self.binary_change(root, &changed)?
            }
        };
        planned.replacements = count(&planned.change, nodes.len());
        Ok(planned)
    }

    /// Target span, concrete key and stored value of one operation.
    fn expectation_of(&self, op: &Op) -> Result<(Span, KeyPath, Value)> {
        Ok(match op {
            Op::Replace { node, value } => (
                self.target(*node),
                self.tree.path_of(*node, &SEMANTICS),
                self.stored(Some(*node), value)?,
            ),
            Op::Insert { map, name, value } => {
                let key = child(
                    &self.tree.path_of(*map, &SEMANTICS),
                    KeySegment::Key(name.clone()),
                )?;
                (self.target(*map), key, self.stored(None, value)?)
            }
            Op::Append { list, value } => {
                let key = child(&self.tree.path_of(*list, &SEMANTICS), KeySegment::Append)?;
                (self.target(*list), key, self.stored(None, value)?)
            }
        })
    }

    /// Value the document holds after `value` is written over `node`, or as
    /// a new member when `node` is `None`.
    fn stored(&self, node: Option<NodeId>, value: &Value) -> Result<Value> {
        match &self.form {
            Form::Ascii(syntax) => syntax.stored(self.text()?, &self.tree, node, value),
            Form::Xml(_) | Form::Binary(_) => {
                let hint = node.map_or(Hint::Other, |node| hint_of(&self.tree[node]));
                Ok(value::to_value(&value::from_value(value, hint)?))
            }
        }
    }

    /// Span of the source a change of `node` may alter: its value, or the
    /// whole file for a binary property list.
    fn target(&self, node: NodeId) -> Span {
        match self.form {
            Form::Binary(_) => Span::new(0, self.source.len()),
            Form::Xml(_) | Form::Ascii(_) => self.tree[node].value_span,
        }
    }

    fn apply_binary(&self, root: &mut plist::Value, op: &Op) -> Result<()> {
        match op {
            Op::Replace { node, value } => {
                binary::replace(root, self.tree.path_of(*node, &SEMANTICS).segments(), value)
            }
            Op::Insert { map, name, value } => binary::insert(
                root,
                self.tree.path_of(*map, &SEMANTICS).segments(),
                name,
                value,
            ),
            Op::Append { list, value } => {
                binary::append(root, self.tree.path_of(*list, &SEMANTICS).segments(), value)
            }
        }
    }

    fn binary_change(&self, root: &plist::Value, changed: &plist::Value) -> Result<PlistChange> {
        if changed == root {
            return Ok(PlistChange::default());
        }
        let bytes = binary::write(changed)?;
        Ok(PlistChange {
            edits: vec![PlistEdit {
                range: 0..self.source.len(),
                replacement: bytes,
            }],
            flags: vec![BINARY_REWRITTEN.to_owned()],
        })
    }

    fn text(&self) -> Result<&str> {
        utf8(&self.source)
    }

    fn item(&self, node: NodeId, key: KeyPath) -> DocItem {
        let doc_node = &self.tree[node];
        let span = (self.kind != PlistKind::Binary)
            .then(|| self.tree.text_span(&self.source, doc_node.value_span));
        DocItem {
            key,
            value: self.tree.value_of(node, &SEMANTICS, None),
            span,
            comment: None,
            document: None,
            anchor: None,
            alias_of: None,
            tag: doc_node.tag.clone(),
        }
    }
}

/// Type of a replaced node where it decides the type of the new value.
fn hint_of(node: &DocNode) -> Hint {
    if node.tag.as_deref() == Some(UID_TAG) {
        Hint::Uid
    } else if matches!(node.value, Some(Value::Number(_))) {
        Hint::Real
    } else {
        Hint::Other
    }
}

/// `nodes` with every dictionary entry that shares the key of one of them:
/// a repeated key is removed with all its entries, so that no earlier entry
/// takes the place of the removed one.
fn with_repeated(tree: &DocTree, nodes: Vec<NodeId>) -> Vec<NodeId> {
    let mut out = nodes.clone();
    for node in nodes {
        let (Some(parent), Some(name)) = (tree[node].parent, tree[node].name.as_deref()) else {
            continue;
        };
        if tree[parent].kind != NodeKind::Map {
            continue;
        }
        out.extend(
            tree.members(parent)
                .filter(|member| *member != node && tree[*member].name.as_deref() == Some(name)),
        );
    }
    out
}

fn text_change(edits: Vec<PlistEdit>) -> PlistChange {
    PlistChange {
        edits,
        flags: Vec::new(),
    }
}

/// Number of nodes changed: the operations when the change has edits.
fn count(change: &PlistChange, operations: usize) -> u64 {
    if change.is_empty() {
        0
    } else {
        u64::try_from(operations).unwrap_or(u64::MAX)
    }
}

fn child(path: &KeyPath, segment: KeySegment) -> Result<KeyPath> {
    let mut segments = path.segments().to_vec();
    segments.push(segment);
    KeyPath::new(path.document(), segments).map_err(|error| {
        Error::new(
            ErrorCode::KeyConflict,
            format!("key {path} cannot address a new member: {error}"),
        )
    })
}

fn utf8(bytes: &[u8]) -> Result<&str> {
    std::str::from_utf8(bytes).map_err(|error| {
        Error::new(
            ErrorCode::Encoding,
            format!(
                "property list text is not valid UTF-8 at byte {}; save it as UTF-8 or convert it to binary",
                error.valid_up_to()
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>Name</key>\n\t<string>App</string>\n\t<key>Scale</key>\n\t<real>1.5</real>\n\t<key>List</key>\n\t<array>\n\t\t<integer>1</integer>\n\t\t<integer>2</integer>\n\t</array>\n</dict>\n</plist>\n";

    fn key(text: &str) -> KeyPath {
        KeyPath::parse(text).unwrap()
    }

    fn binary_sample() -> Vec<u8> {
        let mut dictionary = plist::Dictionary::new();
        dictionary.insert("name".into(), plist::Value::String("app".into()));
        dictionary.insert("uid".into(), plist::Value::Uid(plist::Uid::new(3)));
        dictionary.insert(
            "list".into(),
            plist::Value::Array(vec![plist::Value::Integer(1.into())]),
        );
        binary::write(&plist::Value::Dictionary(dictionary)).unwrap()
    }

    #[test]
    fn detects_the_form_of_each_kind() {
        assert_eq!(PlistKind::of(b"\xEF\xBB\xBF<plist/>"), PlistKind::Xml);
        assert_eq!(PlistKind::of(b"<!DOCTYPE plist>"), PlistKind::Xml);
        assert_eq!(PlistKind::of(b"a = b;"), PlistKind::Ascii);
        assert_eq!(sniff(b"{ a = b; }"), None);
        assert_eq!(sniff(b"<plist version=\"1.0\">"), Some(PlistKind::Xml));
    }

    #[test]
    fn reads_xml_values_with_spans() {
        let document = PlistDocument::parse(XML.as_bytes()).unwrap();
        let items = document.read(&key("List[*]")).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].value, Value::Int(2));
        assert!(items[1].span.is_some());
        let error = document.read(&key("Missing")).unwrap_err();
        assert_eq!(error.code().as_str(), "E_KEY_NOT_FOUND");
    }

    #[test]
    fn plans_record_stored_values_and_targets() {
        let document = PlistDocument::parse(XML.as_bytes()).unwrap();
        let planned = document
            .plan_set(&key("Scale"), &Value::Int(2), false)
            .unwrap();
        assert_eq!(planned.sets, vec![(key("Scale"), Value::Number(2.0))]);
        assert_eq!(planned.replacements, 1);
        let planned = document
            .plan_set(&key("New.Inner"), &Value::Bool(true), true)
            .unwrap();
        assert!(planned.keys_created);
        assert_eq!(planned.sets[0].0, key("New"));
        let unchanged = document
            .plan_set(&key("Name"), &Value::Text("App".into()), false)
            .unwrap();
        assert!(unchanged.change.is_empty());
        assert_eq!(unchanged.replacements, 0);
    }

    #[test]
    fn removal_plans_later_nodes_first() {
        let document = PlistDocument::parse(XML.as_bytes()).unwrap();
        let planned = document.plan_remove(&key("List[*]")).unwrap();
        assert_eq!(planned.removes, vec![key("List[1]"), key("List[0]")]);
        let text = String::from_utf8(planned.change.apply(XML.as_bytes())).unwrap();
        assert!(text.contains("<array>\n\t</array>"), "{text}");
    }

    #[test]
    fn removing_a_repeated_key_removes_every_entry() {
        let text = b"<plist><dict><key>a</key><integer>1</integer><key>b</key><true/><key>a</key><integer>2</integer></dict></plist>";
        let document = PlistDocument::parse(text).unwrap();
        let planned = document.plan_remove(&key("a")).unwrap();
        assert_eq!(planned.removes, vec![key("a")]);
        let changed = PlistDocument::parse(&planned.change.apply(text)).unwrap();
        assert!(changed.read(&key("a")).is_err());
        assert_eq!(changed.read(&key("b")).unwrap()[0].value, Value::Bool(true));
    }

    #[test]
    fn binary_changes_rewrite_the_file_with_a_flag() {
        let bytes = binary_sample();
        let document = PlistDocument::parse(&bytes).unwrap();
        assert_eq!(document.kind(), PlistKind::Binary);
        let items = document.read(&key("uid")).unwrap();
        assert_eq!(items[0].tag.as_deref(), Some(UID_TAG));
        assert!(items[0].span.is_none());
        let change = document.set(&key("uid"), &Value::Int(9), false).unwrap();
        assert_eq!(change.flags, [BINARY_REWRITTEN]);
        let changed = PlistDocument::parse(&change.apply(&bytes)).unwrap();
        let Form::Binary(root) = &changed.form else {
            panic!("binary expected")
        };
        assert_eq!(
            root.as_dictionary().unwrap().get("uid"),
            Some(&plist::Value::Uid(plist::Uid::new(9)))
        );
        assert!(
            document
                .set(&key("name"), &Value::Text("app".into()), false)
                .unwrap()
                .is_empty()
        );
        let removed = document.remove(&key("list[0]")).unwrap();
        let changed = PlistDocument::parse(&removed.apply(&bytes)).unwrap();
        assert_eq!(
            changed.read(&key("list")).unwrap()[0].value,
            Value::List(Vec::new())
        );
        let appended = document
            .set(&key("list[+]"), &Value::Int(2), false)
            .unwrap();
        let changed = PlistDocument::parse(&appended.apply(&bytes)).unwrap();
        assert_eq!(
            changed.read(&key("list[1]")).unwrap()[0].value,
            Value::Int(2)
        );
        let inserted = document
            .set(&key("extra"), &Value::Bool(false), true)
            .unwrap();
        let changed = PlistDocument::parse(&inserted.apply(&bytes)).unwrap();
        assert_eq!(
            changed.read(&key("extra")).unwrap()[0].value,
            Value::Bool(false)
        );
    }

    #[test]
    fn ascii_documents_with_a_byte_order_mark_keep_offsets() {
        let text = b"\xEF\xBB\xBF{ a = 1; }";
        let document = PlistDocument::parse(text).unwrap();
        assert_eq!(document.kind(), PlistKind::Ascii);
        let change = document.set(&key("a"), &Value::Int(2), false).unwrap();
        assert_eq!(change.apply(text), b"\xEF\xBB\xBF{ a = 2; }");
        assert_eq!(
            document
                .plan_set(&key("a"), &Value::Int(2), false)
                .unwrap()
                .sets[0]
                .1,
            Value::Text("2".into())
        );
    }

    #[test]
    fn rejects_text_that_is_not_utf8() {
        let error = PlistDocument::parse(b"{ a = \xFF; }").unwrap_err();
        assert_eq!(error.code().as_str(), "E_ENCODING");
    }
}
