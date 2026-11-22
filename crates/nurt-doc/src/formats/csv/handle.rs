// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Registered formats `csv` and `tsv` and their document handle.
//!
//! Every change goes through [`guard::commit`]: the edits must lie in the
//! changed rows, and the `csv` crate, reading the new text with the
//! separator and quote character of the file, must read the expected table.

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::Value;

use super::expect::rows_value;
use super::scan::{Dialect, UTF8_BOM};
use super::{CSV_EXTENSIONS, CsvDocument, CsvOptions, CsvPlan, TSV_EXTENSIONS, dialect};
use crate::constructs::{self, DOC_SEMANTIC_MISMATCH};
use crate::edit::guard::{self, EditPlan, Expectation, ReferenceParser};
use crate::edit::{ByteEdit, EditReport};
use crate::model::{DocHandle, DocTree, FormatSemantics, MapSemantics, SetOptions, Span};
use crate::registry::{Confidence, DocFormat, DocOptions};

/// Rules of key paths in tables: a repeated column name selects its first
/// column, which [`CsvDocument`] resolves itself.
const SEMANTICS: MapSemantics = MapSemantics::LAST_WINS;

/// The format `csv` or `tsv`, read with a header row and a detected
/// separator.
///
/// # Examples
///
/// ```
/// use nurt_doc::formats::csv::CsvFormat;
/// use nurt_doc::registry::{DocFormat, DocOptions};
///
/// let handle = CsvFormat::TSV.open(b"a\tb\n1\t2\n".to_vec(), &DocOptions::default())?;
/// assert_eq!(handle.text(), b"a\tb\n1\t2\n");
/// assert_eq!(CsvFormat::CSV.extensions(), ["csv"]);
/// # Ok::<(), nurt_contracts::error::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CsvFormat {
    tabs: bool,
}

impl CsvFormat {
    /// Comma-separated values, with `;` and `|` detected as separators too.
    pub const CSV: Self = Self { tabs: false };

    /// Tab-separated values.
    pub const TSV: Self = Self { tabs: true };

    const fn delimiter(self) -> u8 {
        if self.tabs { b'\t' } else { b',' }
    }
}

impl DocFormat for CsvFormat {
    fn id(&self) -> DocFormatId {
        if self.tabs {
            DocFormatId::TSV
        } else {
            DocFormatId::CSV
        }
    }

    fn extensions(&self) -> &[&'static str] {
        if self.tabs {
            &TSV_EXTENSIONS
        } else {
            &CSV_EXTENSIONS
        }
    }

    /// Text whose lines split into the same number of fields, at least two,
    /// could be a table of the format; it is never more certain, because
    /// such text is also valid in many other formats.
    fn sniff(&self, content: &[u8]) -> Confidence {
        let content = content.strip_prefix(UTF8_BOM).unwrap_or(content);
        let detected = dialect::detect(content, None, self.delimiter()).delimiter;
        let ours = if self.tabs {
            detected == b'\t'
        } else {
            detected != b'\t'
        };
        if ours && dialect::uniform(content, detected) {
            Confidence::Low
        } else {
            Confidence::None
        }
    }

    fn open(&self, content: Vec<u8>, _options: &DocOptions) -> Result<Box<dyn DocHandle>> {
        let document = CsvDocument::parse(&content, &self.id(), &CsvOptions::default())?;
        Ok(Box::new(CsvHandle::new(document)))
    }
}

/// Open CSV or TSV document.
///
/// # Examples
///
/// ```
/// use nurt_contracts::keypath::KeyPath;
/// use nurt_contracts::services::DocFormatId;
/// use nurt_contracts::value::Value;
/// use nurt_doc::formats::csv::{CsvDocument, CsvHandle, CsvOptions};
/// use nurt_doc::model::{DocHandle, SetOptions};
///
/// let document = CsvDocument::parse(b"id,name\n1,Ann\n", &DocFormatId::CSV, &CsvOptions::default())?;
/// let mut handle = CsvHandle::new(document);
/// handle.set(&KeyPath::parse("[0].name")?, &Value::Text("Bo".to_owned()), &SetOptions::default())?;
/// assert_eq!(handle.text(), b"id,name\n1,Bo\n");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone)]
pub struct CsvHandle {
    document: CsvDocument,
}

impl CsvHandle {
    /// Handle of a parsed document.
    #[must_use]
    pub const fn new(document: CsvDocument) -> Self {
        Self { document }
    }

    /// The parsed document.
    #[must_use]
    pub const fn document(&self) -> &CsvDocument {
        &self.document
    }

    /// Checks the planned change with the guards, then takes the new text.
    fn commit(&mut self, planned: CsvPlan) -> Result<EditReport> {
        let mut plan = EditPlan::new();
        for target in planned.targets {
            plan.target(target);
        }
        for edit in planned.change.edits {
            plan.edit(ByteEdit::replace(
                Span::new(edit.range.start, edit.range.end),
                edit.replacement.into_bytes(),
            ));
        }
        plan.report.replacements = planned.replacements;
        let expectation = Expectation::new(vec![self.document.to_value()], &SEMANTICS)
            .set(KeyPath::root(), planned.expected);
        let table = &self.document.table;
        let reference = CsvReference {
            dialect: table.dialect,
            header: table.header,
        };
        let committed = guard::commit(self.document.text(), plan, &expectation, &reference)?;
        if committed.report.is_unchanged() {
            return Ok(committed.report);
        }
        let options = CsvOptions {
            header: table.header,
            delimiter: Some(self.document.delimiter()),
        };
        let format = self.document.format().clone();
        self.document =
            CsvDocument::parse(&committed.bytes, &format, &options).map_err(|error| {
                constructs::unsupported(DOC_SEMANTIC_MISMATCH, "the changed table does not parse")
                    .with_cause(error)
            })?;
        Ok(committed.report)
    }
}

impl DocHandle for CsvHandle {
    fn format(&self) -> DocFormatId {
        self.document.format().clone()
    }

    fn text(&self) -> &[u8] {
        self.document.text()
    }

    fn tree(&self) -> &DocTree {
        self.document.tree()
    }

    fn semantics(&self) -> &dyn FormatSemantics {
        &SEMANTICS
    }

    fn set(&mut self, key: &KeyPath, value: &Value, _options: &SetOptions) -> Result<EditReport> {
        let planned = self.document.plan(key, Some(value))?;
        self.commit(planned)
    }

    fn remove(&mut self, key: &KeyPath) -> Result<EditReport> {
        let planned = self.document.plan(key, None)?;
        self.commit(planned)
    }

    fn to_value(&self) -> Result<Value> {
        Ok(self.document.to_value())
    }
}

/// Reads a table with the `csv` crate in flexible mode, for the meaning
/// guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CsvReference {
    /// Separator and quote character of the file.
    pub(crate) dialect: Dialect,
    /// The first record names the columns.
    pub(crate) header: bool,
}

impl ReferenceParser for CsvReference {
    fn parse(&self, text: &[u8]) -> Result<Vec<Value>> {
        let text = text.strip_prefix(UTF8_BOM).unwrap_or(text);
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .delimiter(self.dialect.delimiter)
            .quote(self.dialect.quote)
            .from_reader(text);
        let mut records: Vec<Vec<String>> = Vec::new();
        for record in reader.records() {
            let record = record.map_err(|error| {
                Error::new(
                    ErrorCode::DocParse,
                    format!("invalid delimited text: {error}"),
                )
            })?;
            records.push(record.iter().map(str::to_owned).collect());
        }
        let header = (self.header && !records.is_empty()).then(|| records.remove(0));
        Ok(vec![rows_value(
            header.as_deref(),
            self.header,
            records.iter().map(Vec::as_slice),
        )])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(text: &str) -> CsvHandle {
        CsvHandle::new(
            CsvDocument::parse(text.as_bytes(), &DocFormatId::CSV, &CsvOptions::default()).unwrap(),
        )
    }

    fn key(text: &str) -> KeyPath {
        KeyPath::parse(text).unwrap()
    }

    #[test]
    fn changes_pass_the_guards() {
        let mut doc = handle("id,name\r\n1,Ann\r\n2,Bo\r\n");
        let report = doc
            .set(
                &key("[*].name"),
                &Value::Text("x, y".into()),
                &SetOptions::default(),
            )
            .unwrap();
        assert_eq!(report.replacements, 2);
        assert_eq!(doc.text(), b"id,name\r\n1,\"x, y\"\r\n2,\"x, y\"\r\n");
        doc.set(
            &key("[+]"),
            &Value::List(vec![Value::Int(3), Value::Text("Cy".into())]),
            &SetOptions::default(),
        )
        .unwrap();
        doc.remove(&key("[0]")).unwrap();
        assert_eq!(doc.text(), b"id,name\r\n2,\"x, y\"\r\n3,Cy\r\n");
        doc.remove(&key("[1].name")).unwrap();
        assert_eq!(doc.text(), b"id,name\r\n2,\"x, y\"\r\n3,\r\n");
        let unchanged = doc
            .set(&key("[0].id"), &Value::Int(2), &SetOptions::default())
            .unwrap();
        assert!(unchanged.is_unchanged());
        assert_eq!(doc.to_value().unwrap(), doc.document().to_value());
    }

    #[test]
    fn reference_reads_the_same_table() {
        for text in [
            "a,b\n1\n1,2,3\n",
            "a;b\n\"x;y\";2\n",
            "\u{feff}a,b\n1,2",
            "a,a\n1,2\n\n\"\"\n",
        ] {
            let document =
                CsvDocument::parse(text.as_bytes(), &DocFormatId::CSV, &CsvOptions::default())
                    .unwrap();
            let reference = CsvReference {
                dialect: document.table.dialect,
                header: true,
            };
            assert_eq!(
                reference.parse(text.as_bytes()).unwrap(),
                vec![document.to_value()],
                "{text}"
            );
        }
    }

    #[test]
    fn sniffing_is_at_most_low() {
        assert_eq!(CsvFormat::CSV.sniff(b"a,b\n1,2\n"), Confidence::Low);
        assert_eq!(CsvFormat::CSV.sniff(b"a;b\n1;2\n"), Confidence::Low);
        assert_eq!(CsvFormat::TSV.sniff(b"a\tb\n1\t2\n"), Confidence::Low);
        assert_eq!(CsvFormat::TSV.sniff(b"a,b\n1,2\n"), Confidence::None);
        assert_eq!(CsvFormat::CSV.sniff(b"plain text\n"), Confidence::None);
        assert_eq!(CsvFormat::TSV.extensions(), ["tsv", "tab"]);
    }
}
