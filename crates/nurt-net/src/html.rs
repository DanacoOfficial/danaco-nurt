// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-net::html
// Responsibility:   Conversion of HTML, Markdown and plain text documents to blocks of text.
// Dependencies:     blocks, charset, markdown
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Conversion of HTML, Markdown and plain text documents to blocks of text,
//! and selection of parts of them.
//!
//! A document becomes a [`Page`]: a sequence of [`Block`]s (headings,
//! paragraphs, list items, code, quotes, tables), the links and tables found
//! and the metadata of the page. Blocks are rendered as Markdown or plain
//! text ([`render`]); [`select`] picks a section, the blocks matching a
//! pattern or the blocks most relevant to a query, and builds the outline of
//! long pages.
//!
//! HTML is parsed with the `tree-sitter-html` grammar, which tolerates
//! malformed markup.

mod blocks;
mod charset;
mod dom;
mod markdown;
pub mod select;

pub use blocks::convert_html;
pub use charset::{Decoded, decode, is_textual};
pub use markdown::{convert_markdown, convert_text, render};

/// How links are written when a document is converted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinkStyle {
    /// `[text](address)` in Markdown; the text alone in plain text.
    #[default]
    Inline,
    /// The text followed by a reference number, and a list of numbered
    /// addresses after the content.
    List,
    /// The text alone.
    None,
}

impl LinkStyle {
    /// Parses `inline`, `list` or `none`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "inline" => Some(Self::Inline),
            "list" => Some(Self::List),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

/// How images are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageStyle {
    /// Images are left out.
    Drop,
    /// The alternative text is kept.
    #[default]
    Alt,
}

impl ImageStyle {
    /// Parses `drop` or `alt`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "drop" => Some(Self::Drop),
            "alt" => Some(Self::Alt),
            _ => None,
        }
    }
}

/// Options of the HTML conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertOptions {
    /// Keep only the main content: without navigation, headers and footers
    /// of the page, sidebars, forms, consent and login dialogs,
    /// recommended and sponsored blocks, and tables of contents.
    pub main_only: bool,
    /// Writing of links.
    pub links: LinkStyle,
    /// Writing of images.
    pub images: ImageStyle,
    /// Address against which relative links are resolved.
    pub base: Option<url::Url>,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            main_only: true,
            links: LinkStyle::Inline,
            images: ImageStyle::Alt,
            base: None,
        }
    }
}

/// Kind of a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// Heading of level 1 to 6.
    Heading(u8),
    /// Paragraph.
    Paragraph,
    /// Item of a list nested at the given depth, from 1.
    ListItem(u8),
    /// Preformatted text or code.
    Code,
    /// Quotation.
    Quote,
    /// Table.
    Table,
}

/// A link of a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// Number of the link in the page, from 1.
    pub number: usize,
    /// Text of the link.
    pub text: String,
    /// Absolute address.
    pub url: String,
}

/// A block of a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// Kind.
    pub kind: BlockKind,
    /// Content in Markdown, without the heading or list marker.
    pub markdown: String,
    /// Content as plain text.
    pub text: String,
    /// Links of the block.
    pub links: Vec<Link>,
}

impl Block {
    pub(crate) fn new(kind: BlockKind, markdown: String, text: String, links: Vec<Link>) -> Self {
        Self {
            kind,
            markdown,
            text,
            links,
        }
    }

    /// Heading level, when the block is a heading.
    #[must_use]
    pub fn heading_level(&self) -> Option<u8> {
        match self.kind {
            BlockKind::Heading(level) => Some(level),
            _ => None,
        }
    }
}

/// A table of a page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Table {
    /// Caption.
    pub caption: Option<String>,
    /// Column names; numbers from `1` when the table has no header row.
    pub columns: Vec<String>,
    /// Rows of cells.
    pub rows: Vec<Vec<String>>,
}

/// Metadata of a page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Metadata {
    /// Title.
    pub title: Option<String>,
    /// Description.
    pub description: Option<String>,
    /// Language.
    pub language: Option<String>,
    /// Canonical address.
    pub canonical: Option<String>,
}

/// A converted document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Page {
    /// Blocks in document order.
    pub blocks: Vec<Block>,
    /// Links, numbered in order of first appearance.
    pub links: Vec<Link>,
    /// Tables.
    pub tables: Vec<Table>,
    /// Metadata.
    pub metadata: Metadata,
}

/// Form of rendered text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextForm {
    /// Markdown.
    Markdown,
    /// Plain text.
    Text,
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
