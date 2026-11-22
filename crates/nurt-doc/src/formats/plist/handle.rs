// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Registered format `plist` and its document handle.
//!
//! Every change goes through [`guard::commit`]: the edits must lie in the
//! changed values and their containers, and the new bytes, read by the
//! `plist` crate (XML and binary) or read again (ASCII), must hold the
//! expected value.

use std::borrow::Cow;
use std::io::Cursor;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::report::Notable;
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::Value;

use super::{
    EXTENSIONS, FILE_NAMES, Planned, PlistDocument, PlistKind, UTF8_BOM, ascii, sniff, value,
};
use crate::constructs::{self, DOC_SEMANTIC_MISMATCH};
use crate::edit::guard::{self, EditPlan, Expectation, ReferenceParser, Surroundings};
use crate::edit::{ByteEdit, EditReport};
use crate::model::{DocHandle, DocTree, FormatSemantics, SetOptions, Span};
use crate::registry::{Confidence, DocFormat, DocOptions, InputKind};

use super::plan::SEMANTICS;

/// Flag of a change that created intermediate keys.
const KEYS_CREATED_FLAG: &str = "keys_created";

/// Separator of ASCII dictionary entries, which a removal takes with the
/// entry.
const ENTRY_SEPARATOR: &[u8] = b";";

/// The format `plist`: XML, binary and ASCII property lists.
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::plist::PlistFormat;
/// use nurt_doc::registry::{Confidence, DocFormat, DocOptions};
///
/// assert_eq!(PlistFormat.sniff(b"bplist00"), Confidence::High);
/// let handle = PlistFormat.open(b"{ a = b; }".to_vec(), &DocOptions::default())?;
/// assert_eq!(handle.text(), b"{ a = b; }");
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PlistFormat;

impl DocFormat for PlistFormat {
    fn id(&self) -> DocFormatId {
        DocFormatId::PLIST
    }

    fn extensions(&self) -> &[&'static str] {
        &EXTENSIONS
    }

    fn file_names(&self) -> &[&'static str] {
        &FILE_NAMES
    }

    fn sniff(&self, content: &[u8]) -> Confidence {
        if sniff(content).is_some() {
            Confidence::High
        } else {
            Confidence::None
        }
    }

    fn input(&self) -> InputKind {
        InputKind::Bytes
    }

    fn open(&self, content: Vec<u8>, _options: &DocOptions) -> Result<Box<dyn DocHandle>> {
        Ok(Box::new(PlistHandle::new(PlistDocument::parse(&content)?)))
    }
}

/// Open property list.
///
/// # Examples
///
/// ```
/// use nurt_contracts::keypath::KeyPath;
/// use nurt_contracts::value::Value;
/// use nurt_doc::formats::plist::{PlistDocument, PlistHandle};
/// use nurt_doc::model::{DocHandle, SetOptions};
///
/// let mut handle = PlistHandle::new(PlistDocument::parse(b"{\n\tname = app;\n}\n")?);
/// handle.set(&KeyPath::parse("version")?, &Value::Text("1.0".to_owned()), &SetOptions { create: true })?;
/// assert_eq!(handle.text(), b"{\n\tname = app;\n\tversion = 1.0;\n}\n");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct PlistHandle {
    document: PlistDocument,
}

impl PlistHandle {
    /// Handle of a parsed property list.
    #[must_use]
    pub const fn new(document: PlistDocument) -> Self {
        Self { document }
    }

    /// The parsed property list.
    #[must_use]
    pub const fn document(&self) -> &PlistDocument {
        &self.document
    }

    /// Checks the planned change with the guards, then takes the new bytes.
    fn commit(&mut self, planned: Planned, key: &KeyPath) -> Result<EditReport> {
        let mut plan = EditPlan::new();
        plan.surroundings = Surroundings::DEFAULT.with(ENTRY_SEPARATOR);
        for target in planned.targets {
            plan.target(target);
        }
        for edit in planned.change.edits {
            plan.edit(ByteEdit::replace(
                Span::new(edit.range.start, edit.range.end),
                edit.replacement,
            ));
        }
        plan.report.replacements = planned.replacements;
        for flag in &planned.change.flags {
            plan.report.flag(flag);
        }
        if planned.keys_created {
            plan.report
                .note(Notable::KeysCreated, Some(key.to_string()));
            plan.report.flag(KEYS_CREATED_FLAG);
        }
        let mut expectation = Expectation::new(vec![self.document.to_value()], &SEMANTICS);
        for (concrete, value) in planned.sets {
            expectation = expectation.set(concrete, value);
        }
        for concrete in planned.removes {
            expectation = expectation.remove(concrete);
        }
        let reference = PlistReference(self.document.kind());
        let committed = guard::commit(self.document.source(), plan, &expectation, &reference)?;
        if committed.report.is_unchanged() {
            return Ok(committed.report);
        }
        self.document = PlistDocument::parse(&committed.bytes).map_err(|error| {
            constructs::unsupported(
                DOC_SEMANTIC_MISMATCH,
                "the changed property list does not parse",
            )
            .with_cause(error)
        })?;
        Ok(committed.report)
    }
}

impl DocHandle for PlistHandle {
    fn format(&self) -> DocFormatId {
        DocFormatId::PLIST
    }

    fn text(&self) -> &[u8] {
        self.document.source()
    }

    fn tree(&self) -> &DocTree {
        self.document.tree()
    }

    fn semantics(&self) -> &dyn FormatSemantics {
        &SEMANTICS
    }

    fn set(&mut self, key: &KeyPath, value: &Value, options: &SetOptions) -> Result<EditReport> {
        let planned = self.document.plan_set(key, value, options.create)?;
        self.commit(planned, key)
    }

    fn remove(&mut self, key: &KeyPath) -> Result<EditReport> {
        let planned = self.document.plan_remove(key)?;
        self.commit(planned, key)
    }

    fn to_value(&self) -> Result<Value> {
        Ok(self.document.to_value())
    }
}

/// Reads XML and binary property lists with the `plist` crate and ASCII
/// property lists again, for the meaning guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlistReference(PlistKind);

impl ReferenceParser for PlistReference {
    fn parse(&self, text: &[u8]) -> Result<Vec<Value>> {
        if self.0 == PlistKind::Ascii {
            let source = std::str::from_utf8(text).map_err(|error| {
                Error::new(
                    ErrorCode::DocParse,
                    format!(
                        "invalid plist document: not UTF-8 at byte {}",
                        error.valid_up_to()
                    ),
                )
            })?;
            let base = if text.starts_with(UTF8_BOM) {
                UTF8_BOM.len()
            } else {
                0
            };
            let (tree, _) = ascii::build(source, base)?;
            let value = tree
                .roots()
                .first()
                .map(|root| tree.value_of(*root, &SEMANTICS, None))
                .unwrap_or_default();
            return Ok(vec![value]);
        }
        let text = if self.0 == PlistKind::Xml {
            without_cdata(text)
        } else {
            Cow::Borrowed(text)
        };
        let value = plist::Value::from_reader(Cursor::new(text.as_ref())).map_err(|error| {
            Error::new(
                ErrorCode::DocParse,
                format!("invalid plist document: {error}"),
            )
        })?;
        Ok(vec![value::to_value(&value)])
    }
}

/// XML text with every CDATA section written as escaped character data,
/// because the `plist` crate reads a CDATA section as empty text. A section
/// without its end is left as it is.
fn without_cdata(text: &[u8]) -> Cow<'_, [u8]> {
    const OPEN: &[u8] = b"<![CDATA[";
    const CLOSE: &[u8] = b"]]>";
    if find(text, OPEN).is_none() {
        return Cow::Borrowed(text);
    }
    let mut out = Vec::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = find(rest, OPEN) {
        let body = &rest[start + OPEN.len()..];
        let Some(end) = find(body, CLOSE) else {
            break;
        };
        out.extend_from_slice(&rest[..start]);
        for &byte in &body[..end] {
            match byte {
                b'&' => out.extend_from_slice(b"&amp;"),
                b'<' => out.extend_from_slice(b"&lt;"),
                b'>' => out.extend_from_slice(b"&gt;"),
                _ => out.push(byte),
            }
        }
        rest = &body[end + CLOSE.len()..];
    }
    out.extend_from_slice(rest);
    Cow::Owned(out)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats::plist::BINARY_REWRITTEN;

    fn key(text: &str) -> KeyPath {
        KeyPath::parse(text).unwrap()
    }

    const XML: &[u8] = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n<dict>\n\t<key>a</key>\n\t<integer>1</integer>\n\t<key>b</key>\n\t<array>\n\t\t<string>x</string>\n\t</array>\n</dict>\n</plist>\n";

    #[test]
    fn xml_changes_pass_the_guards() {
        let mut handle = PlistHandle::new(PlistDocument::parse(XML).unwrap());
        let report = handle
            .set(
                &key("a"),
                &Value::Text("one".into()),
                &SetOptions::default(),
            )
            .unwrap();
        assert_eq!(report.replacements, 1);
        assert!(
            String::from_utf8_lossy(handle.text())
                .contains("\t<string>one</string>\n\t<key>b</key>")
        );
        let report = handle
            .set(&key("c.d"), &Value::Int(4), &SetOptions { create: true })
            .unwrap();
        assert_eq!(report.flags, [KEYS_CREATED_FLAG]);
        handle.remove(&key("b[0]")).unwrap();
        assert_eq!(
            handle.to_value().unwrap(),
            PlistReference(PlistKind::Xml).parse(handle.text()).unwrap()[0]
        );
        let unchanged = handle
            .set(
                &key("a"),
                &Value::Text("one".into()),
                &SetOptions::default(),
            )
            .unwrap();
        assert!(unchanged.is_unchanged());
    }

    #[test]
    fn cdata_sections_pass_the_meaning_guard() {
        let xml = b"<plist version=\"1.0\">\n<dict>\n\t<key>expr</key>\n\t<string><![CDATA[a < b && c]]></string>\n\t<key>n</key>\n\t<integer>1</integer>\n</dict>\n</plist>\n";
        let mut handle = PlistHandle::new(PlistDocument::parse(xml).unwrap());
        assert_eq!(
            PlistReference(PlistKind::Xml).parse(xml).unwrap(),
            vec![handle.to_value().unwrap()]
        );
        let report = handle
            .set(&key("n"), &Value::Int(2), &SetOptions::default())
            .unwrap();
        assert_eq!(report.replacements, 1);
        assert_eq!(
            without_cdata(b"a<![CDATA[x<y]]>b<![CDATA[open").as_ref(),
            b"ax&lt;yb<![CDATA[open"
        );
    }

    #[test]
    fn binary_changes_report_the_rewrite() {
        let mut dictionary = plist::Dictionary::new();
        dictionary.insert("n".into(), plist::Value::Real(1.5));
        let mut bytes = Vec::new();
        plist::Value::Dictionary(dictionary)
            .to_writer_binary(&mut bytes)
            .unwrap();
        let mut handle = PlistHandle::new(PlistDocument::parse(&bytes).unwrap());
        let report = handle
            .set(&key("n"), &Value::Int(2), &SetOptions::default())
            .unwrap();
        assert_eq!(report.flags, [BINARY_REWRITTEN]);
        assert_eq!(
            handle.document().read(&key("n")).unwrap()[0].value,
            Value::Number(2.0)
        );
        assert!(handle.remove(&key("n")).is_ok());
        assert_eq!(
            handle.to_value().unwrap(),
            Value::Map(nurt_contracts::value::Fields::new())
        );
    }

    #[test]
    fn ascii_changes_pass_the_guards() {
        let mut handle = PlistHandle::new(PlistDocument::parse(b"{ a = (x, y); b = 1; }").unwrap());
        handle
            .set(&key("a[+]"), &Value::Int(3), &SetOptions::default())
            .unwrap();
        assert_eq!(handle.text(), b"{ a = (x, y, 3); b = 1; }");
        handle.remove(&key("b")).unwrap();
        assert_eq!(handle.text(), b"{ a = (x, y, 3); }");
    }

    #[test]
    fn format_detects_by_content_and_name() {
        assert_eq!(PlistFormat.sniff(b"<plist>"), Confidence::High);
        assert_eq!(PlistFormat.sniff(b"{ a = b; }"), Confidence::None);
        assert_eq!(PlistFormat.input(), InputKind::Bytes);
        assert_eq!(PlistFormat.file_names(), ["info.plist"]);
        assert!(
            PlistFormat
                .open(b"<plist><foo/></plist>".to_vec(), &DocOptions::default())
                .is_err()
        );
        assert!(PlistReference(PlistKind::Xml).parse(b"<plist>").is_err());
    }
}
