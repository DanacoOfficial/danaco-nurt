// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Text and metadata of PDF documents.
//!
//! A document is parsed from memory and its text is extracted one page at a
//! time. [`PdfTextMode::Text`] yields words and lines in reading order;
//! [`PdfTextMode::Layout`] places every glyph on a character grid by its
//! position on the page, which keeps columns and tables aligned within the
//! precision of the glyph positions stored in the document.
//!
//! A document that requires a password to be opened fails with
//! [`PdfError::Encrypted`]; a document encrypted only to restrict printing
//! or copying opens with the empty password and is read. Text is extracted
//! from the font encodings and `ToUnicode` maps of the document; pages that
//! carry text only as images yield no text.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::{Bound, RangeBounds, RangeInclusive};
use std::panic::{self, AssertUnwindSafe};

use nurt_contracts::error::ErrorCode;
use pdf_extract::{
    Document, MediaBox, ObjectId, OutputDev, OutputError, PlainTextOutput, Transform,
};

const INFO_KEY: &[u8] = b"Info";

/// Detail reported when the PDF library rejects a document structure by
/// panicking instead of returning an error.
const UNSUPPORTED_STRUCTURE: &str = "unsupported document structure";

/// Glyphs whose baselines differ by less than this fraction of the font size
/// belong to one row of the layout.
const ROW_TOLERANCE: f64 = 0.5;

/// A gap between glyphs wider than this fraction of the font size is a space
/// between words.
const WORD_GAP: f64 = 0.1;

/// A gap between baselines wider than this multiple of the font size starts
/// a new paragraph, shown as an empty line.
const PARAGRAPH_GAP: f64 = 2.0;

/// Width of a grid cell as a fraction of the font size. Half of the font
/// size is the average advance of Latin text in proportional fonts.
const CELL_WIDTH: f64 = 0.5;

/// Advance widths outside this range of multiples of the font size come
/// from fonts with incomplete metrics and are replaced by [`CELL_WIDTH`].
const PLAUSIBLE_ADVANCE: std::ops::RangeInclusive<f64> = 0.05..=2.0;

/// Column limit of the layout grid, which bounds the length of a line for
/// glyphs placed far outside the page.
const MAX_LAYOUT_COLUMNS: f64 = 1_000.0;

/// Way of turning the glyphs of a page into text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum PdfTextMode {
    /// Words and lines in reading order.
    #[default]
    Text,
    /// Glyphs on a character grid that keeps their horizontal positions.
    Layout,
}

/// Failure to read a PDF document.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PdfError {
    /// The document requires a password.
    Encrypted,
    /// The content is not a PDF document or its structure is damaged or
    /// unsupported.
    Malformed {
        /// Description of the defect.
        detail: String,
    },
    /// The requested page does not exist.
    PageOutOfRange {
        /// Requested page number, counted from 1.
        page: u32,
        /// Number of pages of the document.
        pages: u32,
    },
}

impl PdfError {
    /// Error code of the language for this failure.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_contracts::error::ErrorCode;
    /// use nurt_files::read::pdf::PdfError;
    ///
    /// assert_eq!(PdfError::Encrypted.code(), ErrorCode::Encrypted);
    /// ```
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Encrypted => ErrorCode::Encrypted,
            Self::Malformed { .. } => ErrorCode::Format,
            Self::PageOutOfRange { .. } => ErrorCode::InvalidValue,
        }
    }

    fn from_library(error: pdf_extract::Error) -> Self {
        match error {
            pdf_extract::Error::Decryption(_) | pdf_extract::Error::InvalidPassword => {
                Self::Encrypted
            }
            other => Self::Malformed {
                detail: other.to_string(),
            },
        }
    }

    fn from_output(error: OutputError) -> Self {
        match error {
            OutputError::PdfError(error) => Self::from_library(error),
            other => Self::Malformed {
                detail: other.to_string(),
            },
        }
    }
}

impl fmt::Display for PdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encrypted => f.write_str(
                "the PDF document is protected by a password; read a copy without a password",
            ),
            Self::Malformed { detail } => write!(
                f,
                "malformed PDF document: {detail}; check that the file is a complete PDF"
            ),
            Self::PageOutOfRange { pages: 0, .. } => f.write_str("the PDF document has no pages"),
            Self::PageOutOfRange { page, pages } => write!(
                f,
                "page {page} is outside the document; use pages 1 to {pages}"
            ),
        }
    }
}

impl std::error::Error for PdfError {}

/// Document information of a PDF.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PdfMetadata {
    /// PDF version from the file header, for example `1.7`.
    pub version: String,
    /// Number of pages.
    pub pages: u32,
    /// Document title.
    pub title: Option<String>,
    /// Name of the author.
    pub author: Option<String>,
    /// Subject of the document.
    pub subject: Option<String>,
    /// Keywords, as written in the document.
    pub keywords: Option<String>,
    /// Application that created the original document.
    pub creator: Option<String>,
    /// Application that produced the PDF.
    pub producer: Option<String>,
}

/// Text of one page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfPage {
    /// Page number, counted from 1.
    pub number: u32,
    /// Text of the page without trailing spaces on lines and without
    /// leading or trailing empty lines.
    pub text: String,
}

/// Parsed PDF document.
///
/// # Examples
///
/// ```
/// use nurt_files::read::pdf::{PdfDocument, PdfError};
///
/// let error = PdfDocument::parse(b"not a PDF document").unwrap_err();
/// assert!(matches!(error, PdfError::Malformed { .. }));
/// ```
pub struct PdfDocument {
    document: Document,
    pages: BTreeMap<u32, ObjectId>,
}

impl fmt::Debug for PdfDocument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PdfDocument")
            .field("version", &self.document.version)
            .field("pages", &self.pages.len())
            .finish()
    }
}

impl PdfDocument {
    /// Parses a document held in memory.
    ///
    /// # Errors
    ///
    /// Returns [`PdfError::Encrypted`] for a document that requires a
    /// password and [`PdfError::Malformed`] for content that is not a
    /// readable PDF document.
    pub fn parse(bytes: &[u8]) -> Result<Self, PdfError> {
        guarded(|| {
            let document = Document::load_mem(bytes).map_err(PdfError::from_library)?;
            if document.is_encrypted() {
                return Err(PdfError::Encrypted);
            }
            let pages = document.get_pages();
            Ok(Self { document, pages })
        })
    }

    /// Number of pages.
    pub fn page_count(&self) -> u32 {
        u32::try_from(self.pages.len()).unwrap_or(u32::MAX)
    }

    /// Version and document information.
    ///
    /// Entries of the information dictionary that are missing, empty or not
    /// text strings are `None`.
    pub fn metadata(&self) -> PdfMetadata {
        let info = self.info();
        let entry = |key: &[u8]| info.and_then(|info| self.text_entry(info, key));
        PdfMetadata {
            version: self.document.version.clone(),
            pages: self.page_count(),
            title: entry(b"Title"),
            author: entry(b"Author"),
            subject: entry(b"Subject"),
            keywords: entry(b"Keywords"),
            creator: entry(b"Creator"),
            producer: entry(b"Producer"),
        }
    }

    /// Resolves a range of page numbers, counted from 1, against the
    /// document.
    ///
    /// An unbounded start is page 1 and an unbounded end is the last page;
    /// an end beyond the last page is reduced to the last page. A range whose
    /// end precedes its start is empty.
    ///
    /// # Errors
    ///
    /// Returns [`PdfError::PageOutOfRange`] when the first page is 0 or
    /// beyond the last page.
    pub fn page_range(
        &self,
        pages: impl RangeBounds<u32>,
    ) -> Result<RangeInclusive<u32>, PdfError> {
        let count = self.page_count();
        let first = match pages.start_bound() {
            Bound::Included(&page) => page,
            Bound::Excluded(&page) => page.saturating_add(1),
            Bound::Unbounded => 1,
        };
        let last = match pages.end_bound() {
            Bound::Included(&page) => page,
            Bound::Excluded(&page) => page.saturating_sub(1),
            Bound::Unbounded => count,
        };
        if first == 0 || first > count {
            return Err(PdfError::PageOutOfRange {
                page: first,
                pages: count,
            });
        }
        Ok(first..=last.min(count))
    }

    /// Extracts the text of one page.
    ///
    /// # Errors
    ///
    /// Returns [`PdfError::PageOutOfRange`] for a page that does not exist
    /// and [`PdfError::Malformed`] when the content of the page cannot be
    /// interpreted.
    pub fn page_text(&self, page: u32, mode: PdfTextMode) -> Result<String, PdfError> {
        if !self.pages.contains_key(&page) {
            return Err(PdfError::PageOutOfRange {
                page,
                pages: self.page_count(),
            });
        }
        guarded(|| {
            let raw = match mode {
                PdfTextMode::Text => {
                    let mut text = String::new();
                    let mut output = PlainTextOutput::new(&mut text);
                    pdf_extract::output_doc_page(&self.document, &mut output, page)
                        .map_err(PdfError::from_output)?;
                    text
                }
                PdfTextMode::Layout => {
                    let mut output = LayoutOutput::default();
                    pdf_extract::output_doc_page(&self.document, &mut output, page)
                        .map_err(PdfError::from_output)?;
                    output.into_text()
                }
            };
            Ok(tidy(&raw))
        })
    }

    /// Extracts the text of a range of pages, one page at a time.
    ///
    /// # Errors
    ///
    /// Fails as [`PdfDocument::page_range`] before any page is read; each
    /// item fails as [`PdfDocument::page_text`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use nurt_files::read::pdf::{PdfDocument, PdfTextMode};
    ///
    /// # fn example(bytes: &[u8]) -> Result<(), nurt_files::read::pdf::PdfError> {
    /// let document = PdfDocument::parse(bytes)?;
    /// for page in document.pages(2..=3, PdfTextMode::Text)? {
    ///     let page = page?;
    ///     assert!(page.number >= 2);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn pages(
        &self,
        pages: impl RangeBounds<u32>,
        mode: PdfTextMode,
    ) -> Result<impl Iterator<Item = Result<PdfPage, PdfError>> + '_, PdfError> {
        let range = self.page_range(pages)?;
        Ok(range.map(move |number| {
            self.page_text(number, mode)
                .map(|text| PdfPage { number, text })
        }))
    }

    fn info(&self) -> Option<&pdf_extract::Dictionary> {
        let info = self.document.trailer.get(INFO_KEY).ok()?;
        let (_, info) = self.document.dereference(info).ok()?;
        info.as_dict().ok()
    }

    fn text_entry(&self, info: &pdf_extract::Dictionary, key: &[u8]) -> Option<String> {
        let value = info.get(key).ok()?;
        let (_, value) = self.document.dereference(value).ok()?;
        let text = pdf_extract::decode_text_string(value).ok()?;
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_owned())
    }
}

/// Runs PDF parsing code of the PDF library, which panics on some damaged
/// documents instead of returning an error, and reports such a panic as a
/// malformed document.
fn guarded<T>(operation: impl FnOnce() -> Result<T, PdfError>) -> Result<T, PdfError> {
    panic::catch_unwind(AssertUnwindSafe(operation)).unwrap_or_else(|_| {
        Err(PdfError::Malformed {
            detail: UNSUPPORTED_STRUCTURE.to_owned(),
        })
    })
}

/// Removes trailing spaces from lines and empty lines at the start and end.
fn tidy(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let first = lines.iter().position(|line| !line.is_empty());
    let last = lines.iter().rposition(|line| !line.is_empty());
    match (first, last) {
        (Some(first), Some(last)) => lines[first..=last].join("\n"),
        _ => String::new(),
    }
}

#[derive(Debug)]
struct Glyph {
    x: f64,
    y: f64,
    size: f64,
    advance: f64,
    text: String,
}

impl Glyph {
    fn end(&self) -> f64 {
        self.x + self.advance
    }
}

/// Collects the glyphs of a page with their positions in page space.
#[derive(Debug, Default)]
struct LayoutOutput {
    glyphs: Vec<Glyph>,
}

impl OutputDev for LayoutOutput {
    fn begin_page(
        &mut self,
        _page_num: u32,
        _media_box: &MediaBox,
        _art_box: Option<(f64, f64, f64, f64)>,
    ) -> Result<(), OutputError> {
        Ok(())
    }

    fn end_page(&mut self) -> Result<(), OutputError> {
        Ok(())
    }

    fn output_character(
        &mut self,
        trm: &Transform,
        width: f64,
        _spacing: f64,
        font_size: f64,
        char: &str,
    ) -> Result<(), OutputError> {
        let scale_x = font_size * (trm.m11 + trm.m21);
        let scale_y = font_size * (trm.m12 + trm.m22);
        let size = (scale_x * scale_y).abs().sqrt();
        if size.is_finite() && size > 0.0 && trm.m31.is_finite() && trm.m32.is_finite() {
            let relative = width.abs();
            let advance = if PLAUSIBLE_ADVANCE.contains(&relative) {
                relative * size
            } else {
                CELL_WIDTH * size
            };
            self.glyphs.push(Glyph {
                x: trm.m31,
                y: trm.m32,
                size,
                advance,
                text: char.to_owned(),
            });
        }
        Ok(())
    }

    fn begin_word(&mut self) -> Result<(), OutputError> {
        Ok(())
    }

    fn end_word(&mut self) -> Result<(), OutputError> {
        Ok(())
    }

    fn end_line(&mut self) -> Result<(), OutputError> {
        Ok(())
    }
}

impl LayoutOutput {
    /// Places glyphs in rows by baseline, from the top of the page, and in
    /// columns by horizontal position, measured in cells of half the median
    /// font size of the page.
    fn into_text(mut self) -> String {
        let Some(left) = self
            .glyphs
            .iter()
            .map(|glyph| glyph.x)
            .min_by(f64::total_cmp)
        else {
            return String::new();
        };
        let cell = cell_width(&self.glyphs);
        self.glyphs
            .sort_by(|a, b| b.y.total_cmp(&a.y).then(a.x.total_cmp(&b.x)));
        let mut rows: Vec<Vec<Glyph>> = Vec::new();
        for glyph in self.glyphs {
            match rows.last_mut() {
                Some(row) if (row[0].y - glyph.y).abs() <= row[0].size * ROW_TOLERANCE => {
                    row.push(glyph);
                }
                _ => rows.push(vec![glyph]),
            }
        }
        let mut text = String::new();
        let mut previous: Option<(f64, f64)> = None;
        for mut row in rows {
            row.sort_by(|a, b| a.x.total_cmp(&b.x));
            let (baseline, size) = (row[0].y, row[0].size);
            if let Some((previous_baseline, previous_size)) = previous {
                text.push('\n');
                if previous_baseline - baseline > previous_size * PARAGRAPH_GAP {
                    text.push('\n');
                }
            }
            text.push_str(&layout_row(&row, left, cell));
            previous = Some((baseline, size));
        }
        text
    }
}

fn cell_width(glyphs: &[Glyph]) -> f64 {
    let mut sizes: Vec<f64> = glyphs.iter().map(|glyph| glyph.size).collect();
    sizes.sort_by(f64::total_cmp);
    sizes
        .get(sizes.len() / 2)
        .map_or(1.0, |size| size * CELL_WIDTH)
}

/// Only the first glyph of a word is aligned to the grid; the other glyphs
/// follow it directly, so that proportional advances do not split words.
fn layout_row(row: &[Glyph], left: f64, cell: f64) -> String {
    let mut line = String::new();
    let mut column = 0usize;
    let mut previous_end: Option<f64> = None;
    for glyph in row {
        let starts_word = previous_end.is_none_or(|end| glyph.x - end > glyph.size * WORD_GAP);
        if starts_word {
            let target = ((glyph.x - left) / cell)
                .round()
                .clamp(0.0, MAX_LAYOUT_COLUMNS) as usize;
            if target > column {
                line.extend(std::iter::repeat_n(' ', target - column));
                column = target;
            } else if previous_end.is_some() && !line.ends_with(' ') && glyph.text != " " {
                line.push(' ');
                column += 1;
            }
        }
        line.push_str(&glyph.text);
        column += glyph.text.chars().count();
        previous_end = Some(glyph.end());
    }
    line.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use pdf_extract::content::{Content, Operation};
    use pdf_extract::{
        Dictionary, EncryptionState, EncryptionVersion, Object, Permissions, Stream, StringFormat,
    };

    use super::*;

    /// Polish letters that the test font maps to the codes from 128 on, by
    /// their PostScript glyph names.
    const POLISH_GLYPHS: [(char, &str); 16] = [
        ('ą', "aogonek"),
        ('ć', "cacute"),
        ('ę', "eogonek"),
        ('ł', "lslash"),
        ('ń', "nacute"),
        ('ś', "sacute"),
        ('ź', "zacute"),
        ('ż', "zdotaccent"),
        ('Ą', "Aogonek"),
        ('Ć', "Cacute"),
        ('Ę', "Eogonek"),
        ('Ł', "Lslash"),
        ('Ń', "Nacute"),
        ('Ś', "Sacute"),
        ('Ź', "Zacute"),
        ('Ż', "Zdotaccent"),
    ];
    const FIRST_POLISH_CODE: u8 = 128;

    fn name(value: &str) -> Object {
        Object::Name(value.as_bytes().to_vec())
    }

    /// Helvetica with `WinAnsiEncoding`, in which `ó` and `Ó` already exist,
    /// and the other Polish letters added through `Differences`.
    fn font() -> Dictionary {
        let mut differences = vec![Object::Integer(i64::from(FIRST_POLISH_CODE))];
        differences.extend(POLISH_GLYPHS.iter().map(|(_, glyph)| name(glyph)));
        let mut encoding = Dictionary::new();
        encoding.set("Type", name("Encoding"));
        encoding.set("BaseEncoding", name("WinAnsiEncoding"));
        encoding.set("Differences", Object::Array(differences));
        let mut font = Dictionary::new();
        font.set("Type", name("Font"));
        font.set("Subtype", name("Type1"));
        font.set("BaseFont", name("Helvetica"));
        font.set("Encoding", Object::Dictionary(encoding));
        font
    }

    fn encode(text: &str) -> Vec<u8> {
        text.chars()
            .map(|ch| match ch {
                'ó' => 0xf3,
                'Ó' => 0xd3,
                ch if ch.is_ascii() => ch as u8,
                ch => {
                    let index = POLISH_GLYPHS
                        .iter()
                        .position(|(letter, _)| *letter == ch)
                        .unwrap();
                    FIRST_POLISH_CODE + index as u8
                }
            })
            .collect()
    }

    /// Text placed at a position in points from the lower left corner.
    struct Run<'a> {
        x: i64,
        y: i64,
        text: &'a str,
    }

    fn page_content(runs: &[Run<'_>]) -> Vec<u8> {
        let mut operations = Vec::new();
        for run in runs {
            operations.push(Operation::new("BT", vec![]));
            operations.push(Operation::new("Tf", vec![name("F1"), Object::Integer(12)]));
            operations.push(Operation::new(
                "Td",
                vec![Object::Integer(run.x), Object::Integer(run.y)],
            ));
            operations.push(Operation::new(
                "Tj",
                vec![Object::String(encode(run.text), StringFormat::Literal)],
            ));
            operations.push(Operation::new("ET", vec![]));
        }
        Content { operations }.encode().unwrap()
    }

    fn document(pages: &[&[Run<'_>]]) -> Document {
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(font());
        let mut fonts = Dictionary::new();
        fonts.set("F1", Object::Reference(font_id));
        let mut resources = Dictionary::new();
        resources.set("Font", Object::Dictionary(fonts));
        let mut kids = Vec::new();
        for runs in pages {
            let content_id =
                document.add_object(Stream::new(Dictionary::new(), page_content(runs)));
            let mut page = Dictionary::new();
            page.set("Type", name("Page"));
            page.set("Parent", Object::Reference(pages_id));
            page.set("Contents", Object::Reference(content_id));
            kids.push(Object::Reference(document.add_object(page)));
        }
        let mut tree = Dictionary::new();
        tree.set("Type", name("Pages"));
        tree.set("Count", Object::Integer(kids.len() as i64));
        tree.set("Kids", Object::Array(kids));
        tree.set("Resources", Object::Dictionary(resources));
        tree.set(
            "MediaBox",
            Object::Array([0, 0, 595, 842].map(Object::Integer).to_vec()),
        );
        document.objects.insert(pages_id, Object::Dictionary(tree));
        let mut catalog = Dictionary::new();
        catalog.set("Type", name("Catalog"));
        catalog.set("Pages", Object::Reference(pages_id));
        let catalog_id = document.add_object(catalog);
        document.trailer.set("Root", Object::Reference(catalog_id));
        let mut info = Dictionary::new();
        info.set(
            "Title",
            Object::String(b"Quarterly report".to_vec(), StringFormat::Literal),
        );
        info.set("Author", pdf_extract::text_string("Zofia Łęcka"));
        info.set("Subject", Object::String(Vec::new(), StringFormat::Literal));
        let info_id = document.add_object(info);
        document.trailer.set("Info", Object::Reference(info_id));
        document.trailer.set(
            "ID",
            Object::Array(vec![
                Object::String(vec![7; 16], StringFormat::Hexadecimal),
                Object::String(vec![7; 16], StringFormat::Hexadecimal),
            ]),
        );
        document
    }

    fn save(mut document: Document) -> Vec<u8> {
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        bytes
    }

    fn three_pages() -> Vec<u8> {
        save(document(&[
            &[Run {
                x: 72,
                y: 760,
                text: "First page",
            }],
            &[Run {
                x: 72,
                y: 760,
                text: "Zażółć gęślą jaźń",
            }],
            &[Run {
                x: 72,
                y: 760,
                text: "ŁÓDŹ, ŚRODA",
            }],
        ]))
    }

    #[test]
    fn counts_pages_and_reads_metadata() {
        let document = PdfDocument::parse(&three_pages()).unwrap();
        assert_eq!(document.page_count(), 3);
        let metadata = document.metadata();
        assert_eq!(metadata.version, "1.5");
        assert_eq!(metadata.pages, 3);
        assert_eq!(metadata.title.as_deref(), Some("Quarterly report"));
        assert_eq!(metadata.author.as_deref(), Some("Zofia Łęcka"));
        assert_eq!(metadata.subject, None);
        assert_eq!(metadata.producer, None);
        assert!(format!("{document:?}").contains("pages: 3"));
    }

    #[test]
    fn extracts_polish_letters_page_by_page() {
        let document = PdfDocument::parse(&three_pages()).unwrap();
        let pages: Vec<PdfPage> = document
            .pages(2.., PdfTextMode::Text)
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            pages,
            [
                PdfPage {
                    number: 2,
                    text: "Zażółć gęślą jaźń".to_owned()
                },
                PdfPage {
                    number: 3,
                    text: "ŁÓDŹ, ŚRODA".to_owned()
                },
            ]
        );
    }

    #[test]
    fn page_range_clamps_end_and_rejects_start_outside_document() {
        let document = PdfDocument::parse(&three_pages()).unwrap();
        assert_eq!(document.page_range(..).unwrap(), 1..=3);
        assert_eq!(document.page_range(2..=9).unwrap(), 2..=3);
        assert_eq!(document.page_range(1..3).unwrap(), 1..=2);
        assert_eq!(
            document
                .page_range((Bound::Included(3), Bound::Included(2)))
                .unwrap()
                .count(),
            0
        );
        let error = document.page_range(4..).unwrap_err();
        assert_eq!(error, PdfError::PageOutOfRange { page: 4, pages: 3 });
        assert_eq!(
            error.to_string(),
            "page 4 is outside the document; use pages 1 to 3"
        );
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert!(document.page_range(0..=1).is_err());
        assert!(document.page_text(9, PdfTextMode::Text).is_err());
    }

    #[test]
    fn layout_mode_keeps_columns() {
        let bytes = save(document(&[&[
            Run {
                x: 72,
                y: 760,
                text: "Name",
            },
            Run {
                x: 300,
                y: 760,
                text: "Size",
            },
            Run {
                x: 72,
                y: 745,
                text: "a.txt",
            },
            Run {
                x: 300,
                y: 745,
                text: "12",
            },
            Run {
                x: 72,
                y: 700,
                text: "Total",
            },
        ]]));
        let document = PdfDocument::parse(&bytes).unwrap();
        let text = document.page_text(1, PdfTextMode::Layout).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4, "{text:?}");
        assert!(
            lines[0].starts_with("Name ") && lines[0].ends_with("Size"),
            "{text:?}"
        );
        assert!(
            lines[1].starts_with("a.txt ") && lines[1].ends_with("12"),
            "{text:?}"
        );
        assert_eq!(lines[0].find("Size"), lines[1].find("12"), "{text:?}");
        assert_eq!(lines[2], "");
        assert_eq!(lines[3], "Total");
    }

    #[test]
    fn empty_page_has_no_text() {
        let document = PdfDocument::parse(&save(document(&[&[]]))).unwrap();
        assert_eq!(document.page_text(1, PdfTextMode::Text).unwrap(), "");
        assert_eq!(document.page_text(1, PdfTextMode::Layout).unwrap(), "");
    }

    #[test]
    fn document_with_user_password_is_encrypted() {
        let mut encrypted = document(&[&[Run {
            x: 72,
            y: 760,
            text: "secret",
        }]]);
        let state = EncryptionState::try_from(EncryptionVersion::V2 {
            document: &encrypted,
            owner_password: "permissions",
            user_password: "user",
            key_length: 128,
            permissions: Permissions::all(),
        })
        .unwrap();
        encrypted.encrypt(&state).unwrap();
        let error = PdfDocument::parse(&save(encrypted)).unwrap_err();
        assert_eq!(error, PdfError::Encrypted);
        assert_eq!(error.code(), ErrorCode::Encrypted);
    }

    #[test]
    fn damaged_documents_are_malformed() {
        let bytes = three_pages();
        for damaged in [&b"%PDF-1.5\n"[..], &bytes[..bytes.len() / 3], b"plain text"] {
            match PdfDocument::parse(damaged) {
                Err(error) => {
                    assert!(matches!(error, PdfError::Malformed { .. }), "{error}");
                    assert_eq!(error.code(), ErrorCode::Format);
                    assert!(error.to_string().starts_with("malformed PDF document: "));
                }
                Ok(document) => {
                    for page in 1..=document.page_count() {
                        let _ = document.page_text(page, PdfTextMode::Text);
                    }
                }
            }
        }
    }

    #[test]
    fn panics_of_the_library_become_malformed_errors() {
        let error = guarded::<()>(|| panic!("unexpected encoding")).unwrap_err();
        assert_eq!(
            error,
            PdfError::Malformed {
                detail: UNSUPPORTED_STRUCTURE.to_owned()
            }
        );
    }

    #[test]
    fn tidy_trims_lines_and_outer_blank_lines() {
        assert_eq!(tidy("\n\n a  \n\nb \n\n"), " a\n\nb");
        assert_eq!(tidy(" \n "), "");
    }

    #[test]
    fn layout_places_glyphs_in_cells_of_half_the_font_size() {
        let glyph = |x: f64, text: &str| Glyph {
            x,
            y: 100.0,
            size: 10.0,
            advance: 5.0,
            text: text.to_owned(),
        };
        let output = LayoutOutput {
            glyphs: vec![glyph(0.0, "a"), glyph(50.0, "b")],
        };
        assert_eq!(output.into_text(), format!("a{}b", " ".repeat(9)));
        assert_eq!(LayoutOutput::default().into_text(), "");
    }

    #[test]
    fn encrypted_message_tells_what_to_do() {
        assert!(
            PdfError::Encrypted
                .to_string()
                .contains("without a password")
        );
        assert_eq!(
            PdfError::PageOutOfRange { page: 1, pages: 0 }.to_string(),
            "the PDF document has no pages"
        );
    }
}
