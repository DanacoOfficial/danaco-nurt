// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Guards that every change of a document passes before it is accepted.
//!
//! A format describes a change as an [`EditPlan`]: the spans of the nodes it
//! changes (targets), the byte edits, and the effects to report. Together
//! with an [`Expectation`] (the value of the document before the change and
//! the operations applied to it) and the [`ReferenceParser`] of the format,
//! [`commit`] checks the plan:
//!
//! 1. Edits equal to the bytes they replace are dropped, so setting the
//!    current value changes nothing.
//! 2. Scope: every edit lies in a target, extended over the permitted
//!    surrounding bytes ([`Surroundings`]).
//! 3. The edits are applied to the original bytes.
//! 4. Meaning: the reference parser reads the new bytes, and the value
//!    must be equivalent to the expected value.
//!
//! [`check_round_trip`] is the identity guard applied when a document is opened.
//! Every violation fails with `E_UNSUPPORTED_CONSTRUCT` naming the
//! construct; the original bytes stay unchanged.
//!
//! ```
//! use nurt_contracts::error::Result;
//! use nurt_contracts::keypath::KeyPath;
//! use nurt_contracts::value::Value;
//! use nurt_doc::edit::ByteEdit;
//! use nurt_doc::edit::guard::{EditPlan, Expectation, ReferenceParser, commit};
//! use nurt_doc::model::{MapSemantics, Span};
//!
//! /// Reads `name=value` lines.
//! struct Lines;
//!
//! impl ReferenceParser for Lines {
//!     fn parse(&self, text: &[u8]) -> Result<Vec<Value>> {
//!         let text = String::from_utf8_lossy(text);
//!         let map = text
//!             .lines()
//!             .filter_map(|line| line.split_once('='))
//!             .map(|(name, value)| (name.to_owned(), Value::Text(value.to_owned())))
//!             .collect();
//!         Ok(vec![Value::Map(map)])
//!     }
//! }
//!
//! let original = b"a=1\nb=2\n";
//! let before = Lines.parse(original)?;
//! let mut plan = EditPlan::new();
//! plan.target(Span::new(2, 3));
//! plan.edit(ByteEdit::replace(Span::new(2, 3), "9"));
//! plan.report.replacements = 1;
//! let expectation = Expectation::new(before, &MapSemantics::LAST_WINS)
//!     .set(KeyPath::parse("a")?, Value::Text("9".to_owned()));
//! let committed = commit(original, plan, &expectation, &Lines)?;
//! assert_eq!(committed.bytes, b"a=9\nb=2\n");
//! assert_eq!(committed.report.replacements, 1);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use nurt_contracts::error::{Error, Result};
use nurt_contracts::keypath::{DocSelector, KeyPath};
use nurt_contracts::value::Value;

use super::{ByteEdit, EditReport, apply, diff};
use crate::constructs::{
    self, DOC_OUT_OF_SCOPE_CHANGE, DOC_ROUND_TRIP_LOSS, DOC_SEMANTIC_MISMATCH,
};
use crate::model::{FormatSemantics, LineIndex, Span, key_conflict, key_not_found};
use crate::value::{apply_remove, apply_set, first_difference, to_document_value};

/// Independent reader of a format, used to check the meaning of changed
/// bytes.
///
/// The reader must not share the parser of the editing code where an
/// independent parser is available (for example `serde_json` for JSON,
/// `saphyr` for YAML, `toml` tables from `toml_edit` for TOML).
pub trait ReferenceParser: Send + Sync {
    /// Reads `text` into one value per document of the file.
    ///
    /// # Errors
    ///
    /// Fails with `E_DOC_PARSE` when the text is not a valid document.
    fn parse(&self, text: &[u8]) -> Result<Vec<Value>>;
}

/// Bytes that may change next to a target: separators and blanks that a
/// change adds or removes together with a node.
///
/// ```
/// use nurt_doc::edit::guard::Surroundings;
///
/// assert!(Surroundings::DEFAULT.allows(b','));
/// assert!(Surroundings::DEFAULT.with(b";").allows(b';'));
/// assert!(!Surroundings::DEFAULT.allows(b'x'));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Surroundings {
    bytes: &'static [u8],
    extra: &'static [u8],
}

impl Surroundings {
    /// Space, tab, line breaks and the comma.
    pub const DEFAULT: Self = Self {
        bytes: b" \t\r\n,",
        extra: b"",
    };

    /// Default surroundings and the format-specific `extra` bytes (for
    /// example `;` for statement-like separators).
    #[must_use]
    pub const fn with(self, extra: &'static [u8]) -> Self {
        Self {
            bytes: self.bytes,
            extra,
        }
    }

    /// Returns `true` when `byte` may change next to a target.
    #[must_use]
    pub fn allows(self, byte: u8) -> bool {
        self.bytes.contains(&byte) || self.extra.contains(&byte)
    }
}

impl Default for Surroundings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Change of a document proposed by a format: target spans, byte edits and
/// the effects to report.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EditPlan {
    /// Spans of the original text that the change may alter: the changed
    /// nodes, and empty spans at insertion points.
    pub targets: Vec<Span>,
    /// Byte edits.
    pub edits: Vec<ByteEdit>,
    /// Bytes that may change next to a target.
    pub surroundings: Surroundings,
    /// Replacements, notable effects and flags; the changed spans are
    /// filled by [`commit`].
    pub report: EditReport,
}

impl EditPlan {
    /// Empty plan with the default surroundings.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Plan for a text rewritten by a library: the edits are the
    /// differences between `original` and `rewritten` ([`diff`]).
    #[must_use]
    pub fn from_rewrite(original: &[u8], rewritten: &[u8], targets: Vec<Span>) -> Self {
        Self {
            targets,
            edits: diff(original, rewritten),
            ..Self::default()
        }
    }

    /// Declares a target span.
    pub fn target(&mut self, span: Span) -> &mut Self {
        self.targets.push(span);
        self
    }

    /// Adds an edit.
    pub fn edit(&mut self, edit: ByteEdit) -> &mut Self {
        self.edits.push(edit);
        self
    }
}

/// Operation whose result the semantic guard expects.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Operation {
    /// The value at a concrete key is set (created when missing).
    Set {
        /// Concrete key.
        key: KeyPath,
        /// New value.
        value: Value,
    },
    /// The member or element at a concrete key is removed.
    Remove {
        /// Concrete key.
        key: KeyPath,
    },
}

/// Value of a document before a change and the operations of the change.
pub struct Expectation<'s> {
    documents: Vec<Value>,
    semantics: &'s dyn FormatSemantics,
    operations: Vec<Operation>,
}

impl std::fmt::Debug for Expectation<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Expectation")
            .field("documents", &self.documents)
            .field("operations", &self.operations)
            .finish_non_exhaustive()
    }
}

impl<'s> Expectation<'s> {
    /// Expectation for a file with the given documents, before any
    /// operation.
    #[must_use]
    pub fn new(documents: Vec<Value>, semantics: &'s dyn FormatSemantics) -> Self {
        Self {
            documents,
            semantics,
            operations: Vec::new(),
        }
    }

    /// Adds a set operation.
    #[must_use]
    pub fn set(mut self, key: KeyPath, value: Value) -> Self {
        self.operations.push(Operation::Set { key, value });
        self
    }

    /// Adds a remove operation.
    #[must_use]
    pub fn remove(mut self, key: KeyPath) -> Self {
        self.operations.push(Operation::Remove { key });
        self
    }

    /// Operations in order.
    #[must_use]
    pub fn operations(&self) -> &[Operation] {
        &self.operations
    }

    /// Documents after the operations.
    ///
    /// # Errors
    ///
    /// Fails with `E_KEY_CONFLICT` or `E_KEY_NOT_FOUND` when an operation
    /// does not apply to the value, as described for
    /// [`apply_set`](crate::value::apply_set).
    pub fn expected(&self) -> Result<Vec<Value>> {
        let mut documents = self.documents.clone();
        for operation in &self.operations {
            let key = match operation {
                Operation::Set { key, .. } | Operation::Remove { key } => key,
            };
            let index = match key.document() {
                None => 0,
                Some(DocSelector::Index(index)) => usize::try_from(index).unwrap_or(usize::MAX),
                Some(DocSelector::All) => {
                    return Err(key_conflict(
                        key,
                        "pattern",
                        "selects every document; a change needs one document",
                    ));
                }
            };
            let local = KeyPath::new(None, key.segments().to_vec()).unwrap_or_default();
            let document = documents
                .get_mut(index)
                .ok_or_else(|| key_not_found(key, &[]))?;
            match operation {
                Operation::Set { value, .. } => {
                    apply_set(
                        document,
                        &local,
                        to_document_value(value.clone()),
                        self.semantics,
                    )?;
                }
                Operation::Remove { .. } => apply_remove(document, &local, self.semantics)?,
            }
        }
        Ok(documents)
    }

    fn describe(&self) -> String {
        let keys: Vec<String> = self
            .operations
            .iter()
            .map(|operation| match operation {
                Operation::Set { key, .. } | Operation::Remove { key } => format!("`{key}`"),
            })
            .collect();
        if keys.is_empty() {
            "the change".to_owned()
        } else {
            format!("the change of {}", keys.join(", "))
        }
    }
}

/// Accepted change: the new bytes and the report.
#[derive(Debug, Clone, PartialEq)]
pub struct Committed {
    /// Text after the change.
    pub bytes: Vec<u8>,
    /// Changed spans of the new text, replacements, notable effects and
    /// flags.
    pub report: EditReport,
}

/// Checks a planned change with the scope and meaning guards and applies it.
///
/// A plan whose edits all equal the bytes they replace yields the original
/// bytes and an empty list of changed spans, and its replacements are
/// reported as zero.
///
/// # Errors
///
/// Fails with `E_UNSUPPORTED_CONSTRUCT` naming `doc.out_of_scope_change`
/// when an edit lies outside the targets and their surroundings, and naming
/// `doc.semantic_mismatch` when the new bytes do not parse or read back as
/// another value. Fails with `E_KEY_CONFLICT` for overlapping edits and with
/// the errors of [`Expectation::expected`].
pub fn commit(
    original: &[u8],
    plan: EditPlan,
    expectation: &Expectation<'_>,
    reference: &dyn ReferenceParser,
) -> Result<Committed> {
    let EditPlan {
        targets,
        edits,
        surroundings,
        mut report,
    } = plan;
    let edits: Vec<ByteEdit> = edits
        .into_iter()
        .filter(|edit| !edit.is_noop(original))
        .collect();
    check_scope(original, &targets, &edits, surroundings)?;
    let applied = apply(original, &edits)?;
    check_meaning(&applied.bytes, expectation, reference)?;
    report.changed = applied.offsets.changed().to_vec();
    if edits.is_empty() {
        report.replacements = 0;
    }
    Ok(Committed {
        bytes: applied.bytes,
        report,
    })
}

/// Scope guard: every edit lies in a target extended over permitted
/// surrounding bytes.
///
/// # Errors
///
/// Fails with `E_UNSUPPORTED_CONSTRUCT` naming `doc.out_of_scope_change`,
/// with the line of the first edit outside the targets.
pub fn check_scope(
    original: &[u8],
    targets: &[Span],
    edits: &[ByteEdit],
    surroundings: Surroundings,
) -> Result<()> {
    let reaches = reaches(original, targets, surroundings);
    // The reaches are sorted and disjoint, so only the last one that starts at
    // or before the edit can hold it.
    let outside = edits.iter().find(|edit| {
        let candidate = reaches.partition_point(|reach| reach.start <= edit.span.start);
        !candidate
            .checked_sub(1)
            .is_some_and(|at| edit.span.end <= reaches[at].end)
    });
    match outside {
        None => Ok(()),
        Some(edit) => {
            let position = LineIndex::new(original).position(original, edit.span.start);
            Err(constructs::unsupported(
                DOC_OUT_OF_SCOPE_CHANGE,
                format!(
                    "the change would alter bytes outside the changed node at line {}, column {}",
                    position.line, position.column
                ),
            )
            .with_detail("byte", edit.span.start.into()))
        }
    }
}

/// Targets extended over surrounding bytes and merged where they touch.
fn reaches(original: &[u8], targets: &[Span], surroundings: Surroundings) -> Vec<Span> {
    let mut extended: Vec<Span> = targets
        .iter()
        .map(|target| {
            let mut start = target.start.min(original.len());
            while start > 0 && surroundings.allows(original[start - 1]) {
                start -= 1;
            }
            let mut end = target.end.min(original.len());
            while end < original.len() && surroundings.allows(original[end]) {
                end += 1;
            }
            Span::new(start, end)
        })
        .collect();
    extended.sort();
    let mut merged: Vec<Span> = Vec::with_capacity(extended.len());
    for span in extended {
        match merged.last_mut() {
            Some(last) if span.start <= last.end => last.end = last.end.max(span.end),
            _ => merged.push(span),
        }
    }
    merged
}

/// Meaning guard: the new bytes read back as the expected value.
///
/// Both values are compared in the form given by
/// [`FormatSemantics::canonical`] of the semantics of the expectation.
///
/// # Errors
///
/// Fails with `E_UNSUPPORTED_CONSTRUCT` naming `doc.semantic_mismatch`,
/// with the first differing key in the message, and with the errors of
/// [`Expectation::expected`].
pub fn check_meaning(
    bytes: &[u8],
    expectation: &Expectation<'_>,
    reference: &dyn ReferenceParser,
) -> Result<()> {
    let expected = expectation.expected()?;
    let actual = reference.parse(bytes).map_err(|error| {
        constructs::unsupported(
            DOC_SEMANTIC_MISMATCH,
            format!(
                "{} would make the document invalid ({})",
                expectation.describe(),
                error.message
            ),
        )
        .with_cause(error)
    })?;
    let semantics = expectation.semantics;
    let canonical = |documents: Vec<Value>| {
        Value::List(
            documents
                .into_iter()
                .map(|document| semantics.canonical(document))
                .collect(),
        )
    };
    match first_difference(&canonical(expected), &canonical(actual)) {
        None => Ok(()),
        Some(path) => Err(mismatch(expectation, &path)),
    }
}

fn mismatch(expectation: &Expectation<'_>, path: &KeyPath) -> Error {
    let place = path.to_string();
    constructs::unsupported(
        DOC_SEMANTIC_MISMATCH,
        format!(
            "{} would read back with a different value at `{place}` (document and key)",
            expectation.describe()
        ),
    )
    .with_detail("difference", place.into())
}

/// Identity guard: the bytes of an opened document equal the bytes it was opened
/// from.
///
/// # Errors
///
/// Fails with `E_UNSUPPORTED_CONSTRUCT` naming `doc.round_trip_loss`, with
/// the line and column of the first differing byte.
///
/// # Examples
///
/// ```
/// use nurt_doc::edit::guard::check_round_trip;
///
/// assert!(check_round_trip(b"a: 1\n", b"a: 1\n").is_ok());
/// let error = check_round_trip(b"a: 1\n", b"a: 1").unwrap_err();
/// assert_eq!(error.found.as_deref(), Some("doc.round_trip_loss"));
/// ```
pub fn check_round_trip(original: &[u8], reopened: &[u8]) -> Result<()> {
    if original == reopened {
        return Ok(());
    }
    let offset = original
        .iter()
        .zip(reopened)
        .take_while(|(a, b)| a == b)
        .count();
    let position = LineIndex::new(original).position(original, offset);
    Err(constructs::unsupported(
        DOC_ROUND_TRIP_LOSS,
        format!(
            "the document would not be kept byte for byte from line {}, column {}",
            position.line, position.column
        ),
    )
    .with_detail("byte", offset.into()))
}

#[cfg(test)]
mod tests {
    use nurt_contracts::error::ErrorCode;

    use super::*;
    use crate::model::MapSemantics;

    /// Reads `name=value` lines.
    struct Lines;

    impl ReferenceParser for Lines {
        fn parse(&self, text: &[u8]) -> Result<Vec<Value>> {
            let text = String::from_utf8_lossy(text);
            let map = text
                .lines()
                .filter_map(|line| line.split_once('='))
                .map(|(name, value)| (name.to_owned(), Value::Text(value.to_owned())))
                .collect();
            Ok(vec![Value::Map(map)])
        }
    }

    /// Rejects every text.
    struct Failing;

    impl ReferenceParser for Failing {
        fn parse(&self, _text: &[u8]) -> Result<Vec<Value>> {
            Err(Error::new(ErrorCode::DocParse, "not a document"))
        }
    }

    const ORIGINAL: &[u8] = b"a=1\nb=2\n";

    fn expectation() -> Expectation<'static> {
        Expectation::new(Lines.parse(ORIGINAL).unwrap(), &MapSemantics::LAST_WINS)
    }

    fn plan(edit: ByteEdit, target: Span) -> EditPlan {
        let mut plan = EditPlan::new();
        plan.target(target);
        plan.edit(edit);
        plan.report.replacements = 1;
        plan
    }

    fn key(text: &str) -> KeyPath {
        KeyPath::parse(text).unwrap()
    }

    #[test]
    fn accepts_a_change_that_reads_back_as_expected() {
        let plan = plan(
            ByteEdit::replace(Span::new(2, 3), "9"),
            Span::new(2, 3),
        );
        let expectation = expectation().set(key("a"), Value::Text("9".to_owned()));
        let committed = commit(ORIGINAL, plan, &expectation, &Lines).unwrap();
        assert_eq!(committed.bytes, b"a=9\nb=2\n");
        assert_eq!(committed.report.replacements, 1);
        assert_eq!(committed.report.changed, vec![Span::new(2, 3)]);
    }

    #[test]
    fn edits_equal_to_the_bytes_they_replace_are_dropped() {
        let plan = plan(ByteEdit::replace(Span::new(2, 3), "1"), Span::new(2, 3));
        let committed = commit(ORIGINAL, plan, &expectation(), &Lines).unwrap();
        assert_eq!(committed.bytes, ORIGINAL);
        assert!(committed.report.is_unchanged());
        assert_eq!(committed.report.replacements, 0);
    }

    #[test]
    fn rejects_edits_outside_the_targets() {
        let plan = plan(ByteEdit::replace(Span::new(6, 7), "9"), Span::new(2, 3));
        let error = commit(ORIGINAL, plan, &expectation(), &Lines).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedConstruct);
        assert_eq!(error.found.as_deref(), Some(DOC_OUT_OF_SCOPE_CHANGE));
    }

    #[test]
    fn blanks_and_separators_next_to_a_target_may_change() {
        let mut plan = EditPlan::new();
        plan.target(Span::new(2, 3));
        plan.edit(ByteEdit::replace(Span::new(2, 4), "1\n\n"));
        let committed = commit(ORIGINAL, plan, &expectation(), &Lines).unwrap();
        assert_eq!(committed.bytes, b"a=1\n\nb=2\n");
        assert!(!Surroundings::DEFAULT.allows(b'x'));
        assert!(Surroundings::DEFAULT.with(b";").allows(b';'));
    }

    #[test]
    fn rejects_a_change_that_reads_back_with_another_value() {
        let plan = plan(ByteEdit::replace(Span::new(2, 3), "9"), Span::new(2, 3));
        let expectation = expectation().set(key("a"), Value::Text("8".to_owned()));
        let error = commit(ORIGINAL, plan, &expectation, &Lines).unwrap_err();
        assert_eq!(error.found.as_deref(), Some(DOC_SEMANTIC_MISMATCH));
    }

    #[test]
    fn rejects_a_change_that_makes_the_document_invalid() {
        let plan = plan(ByteEdit::replace(Span::new(2, 3), "9"), Span::new(2, 3));
        let error = commit(ORIGINAL, plan, &expectation(), &Failing).unwrap_err();
        assert_eq!(error.found.as_deref(), Some(DOC_SEMANTIC_MISMATCH));
    }

    #[test]
    fn expectations_apply_their_operations_in_order() {
        let expectation = expectation()
            .set(key("c"), Value::Text("3".to_owned()))
            .remove(key("a"));
        assert_eq!(expectation.operations().len(), 2);
        let documents = expectation.expected().unwrap();
        let Value::Map(fields) = &documents[0] else {
            panic!("map expected")
        };
        assert_eq!(fields.keys().collect::<Vec<_>>(), ["b", "c"]);
        let missing = Expectation::new(Vec::new(), &MapSemantics::LAST_WINS)
            .set(key("a"), Value::Null)
            .expected()
            .unwrap_err();
        assert_eq!(missing.code, ErrorCode::KeyNotFound);
    }

    #[test]
    fn a_rewrite_becomes_the_edits_between_the_texts() {
        let plan = EditPlan::from_rewrite(ORIGINAL, b"a=9\nb=2\n", vec![Span::new(2, 3)]);
        assert_eq!(
            plan.edits,
            vec![ByteEdit::replace(Span::new(2, 3), "9")]
        );
        assert_eq!(plan.targets, vec![Span::new(2, 3)]);
    }

    #[test]
    fn the_identity_guard_reports_the_first_differing_byte() {
        assert!(check_round_trip(b"a: 1\n", b"a: 1\n").is_ok());
        let error = check_round_trip(b"a: 1\nb: 2\n", b"a: 1\nb: 3\n").unwrap_err();
        assert_eq!(error.found.as_deref(), Some(DOC_ROUND_TRIP_LOSS));
        assert_eq!(
            error.details.get("byte").and_then(serde_json::Value::as_u64),
            Some(8)
        );
        let shorter = check_round_trip(b"a: 1\n", b"").unwrap_err();
        assert_eq!(shorter.found.as_deref(), Some(DOC_ROUND_TRIP_LOSS));
    }
}
