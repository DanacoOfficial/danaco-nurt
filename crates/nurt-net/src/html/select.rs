// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-net::html::select
// Responsibility:   Selection of parts of a page: sections, pattern matches, the blocks that.
// Dependencies:     regex
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Selection of parts of a page: sections, pattern matches, the blocks that
//! best match a query, and the outline of a long page.

use std::collections::HashMap;

use regex::Regex;

use super::markdown::render_block;
use super::{Block, BlockKind, TextForm};

/// BM25 term frequency saturation.
const BM25_K1: f64 = 1.2;
/// BM25 length normalization.
const BM25_B: f64 = 0.75;
/// Shortest word whose inflected forms are matched by a common prefix.
const STEM_MIN: usize = 4;
/// Longest differing ending of words shorter than [`LONG_WORD`].
const SHORT_ENDING: usize = 4;
/// Length from which words may differ in a longer ending.
const LONG_WORD: usize = 6;
/// Longest differing ending of words of [`LONG_WORD`] characters or more.
const LONG_ENDING: usize = 5;
/// Characters of the start of the content shown in an outline.
const OUTLINE_START: usize = 300;

/// Returns the section under the first heading equal to `title`, or else
/// containing it; texts are compared without case and diacritics. The section
/// ends before the next heading of the same or a higher level.
///
/// # Errors
///
/// The texts of all headings when no heading matches.
///
/// ```
/// use nurt_net::html::{convert_markdown, select};
///
/// let page = convert_markdown("# A\n\none\n\n## Setup\n\ntwo\n\n# B\n\nthree");
/// let found = select::section(&page.blocks, "setup").unwrap();
/// assert_eq!(found.len(), 2);
/// assert!(select::section(&page.blocks, "missing").is_err());
/// ```
pub fn section<'a>(blocks: &'a [Block], title: &str) -> Result<&'a [Block], Vec<String>> {
    let wanted = fold(title.trim());
    let headings: Vec<(usize, u8)> = blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| block.heading_level().map(|level| (index, level)))
        .collect();
    let matching = |exact: bool| {
        headings.iter().find(|(index, _)| {
            let text = fold(&blocks[*index].text);
            if exact {
                text == wanted
            } else {
                text.contains(&wanted)
            }
        })
    };
    let Some(&(start, level)) = matching(true).or_else(|| matching(false)) else {
        return Err(headings
            .iter()
            .map(|(index, _)| blocks[*index].text.clone())
            .collect());
    };
    let end = headings
        .iter()
        .find(|(index, other)| *index > start && *other <= level)
        .map_or(blocks.len(), |(index, _)| *index);
    Ok(&blocks[start..end])
}

/// Blocks whose text matches `pattern`, each with up to `context` blocks
/// before and after it. Groups that are not adjacent are separated by a
/// paragraph holding `…`.
#[must_use]
pub fn find(blocks: &[Block], pattern: &Regex, context: usize) -> Vec<Block> {
    let mut keep = vec![false; blocks.len()];
    for (index, block) in blocks.iter().enumerate() {
        if pattern.is_match(&block.text) || pattern.is_match(&block.markdown) {
            let from = index.saturating_sub(context);
            let to = index
                .saturating_add(context)
                .min(blocks.len().saturating_sub(1));
            keep[from..=to].iter_mut().for_each(|slot| *slot = true);
        }
    }
    let mut out = Vec::new();
    let mut previous: Option<usize> = None;
    for (index, block) in blocks.iter().enumerate().filter(|(index, _)| keep[*index]) {
        if previous.is_some_and(|previous| previous + 1 != index) {
            out.push(gap());
        }
        out.push(block.clone());
        previous = Some(index);
    }
    out
}

fn gap() -> Block {
    Block::new(
        BlockKind::Paragraph,
        "…".to_owned(),
        "…".to_owned(),
        Vec::new(),
    )
}

/// Returns the `limit` blocks with the highest BM25 score for `query`, in
/// document order, each preceded by the heading of its section.
///
/// Words are compared without case and diacritics; words of at least four
/// characters also match their inflected forms through a common beginning
/// (an ending of up to four characters, or five for words of six characters
/// or more).
///
/// ```
/// use nurt_net::html::{convert_markdown, select};
///
/// let page = convert_markdown("# Setup\n\nInstallation of the package.\n\n# Other\n\nUnrelated text.");
/// let best = select::query(&page.blocks, "install package", 1);
/// assert_eq!(best[0].text, "Setup");
/// assert_eq!(best[1].text, "Installation of the package.");
/// ```
#[must_use]
pub fn query(blocks: &[Block], query: &str, limit: usize) -> Vec<Block> {
    let terms: Vec<String> = tokens(query);
    let candidates: Vec<(usize, Vec<String>)> = blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| block.heading_level().is_none())
        .map(|(index, block)| (index, tokens(&block.text)))
        .collect();
    if terms.is_empty() || candidates.is_empty() {
        return Vec::new();
    }
    let scores = bm25(&terms, &candidates);
    let mut ranked: Vec<(usize, f64)> = scores
        .into_iter()
        .filter(|(_, score)| *score > 0.0)
        .collect();
    ranked.sort_by(|left, right| right.1.total_cmp(&left.1).then(left.0.cmp(&right.0)));
    ranked.truncate(limit);
    let mut chosen: Vec<usize> = ranked.into_iter().map(|(index, _)| index).collect();
    chosen.sort_unstable();
    with_headings(blocks, &chosen)
}

fn bm25(terms: &[String], documents: &[(usize, Vec<String>)]) -> Vec<(usize, f64)> {
    let count = documents.len() as f64;
    let average = documents
        .iter()
        .map(|(_, words)| words.len())
        .sum::<usize>() as f64
        / count;
    let frequencies: Vec<HashMap<&str, usize>> = documents
        .iter()
        .map(|(_, words)| {
            let mut found = HashMap::new();
            for term in terms {
                let hits = words.iter().filter(|word| matches(term, word)).count();
                if hits > 0 {
                    found.insert(term.as_str(), hits);
                }
            }
            found
        })
        .collect();
    let idf: HashMap<&str, f64> = terms
        .iter()
        .map(|term| {
            let with = frequencies
                .iter()
                .filter(|found| found.contains_key(term.as_str()))
                .count() as f64;
            let value = ((count - with + 0.5) / (with + 0.5) + 1.0).ln();
            (term.as_str(), value)
        })
        .collect();
    documents
        .iter()
        .zip(&frequencies)
        .map(|((index, words), found)| {
            let length = words.len() as f64;
            let score = found
                .iter()
                .map(|(term, hits)| {
                    let hits = *hits as f64;
                    let norm = BM25_K1 * (1.0 - BM25_B + BM25_B * length / average.max(1.0));
                    idf.get(term).copied().unwrap_or_default() * hits * (BM25_K1 + 1.0)
                        / (hits + norm)
                })
                .sum();
            (*index, score)
        })
        .collect()
}

fn with_headings(blocks: &[Block], chosen: &[usize]) -> Vec<Block> {
    let mut out = Vec::new();
    let mut last_heading: Option<usize> = None;
    for &index in chosen {
        let heading = blocks[..index]
            .iter()
            .rposition(|block| block.heading_level().is_some());
        if heading.is_some() && heading != last_heading {
            if let Some(heading) = heading {
                out.push(blocks[heading].clone());
            }
            last_heading = heading;
        }
        out.push(blocks[index].clone());
    }
    out
}

fn matches(term: &str, word: &str) -> bool {
    if term == word {
        return true;
    }
    let term_chars = term.chars().count();
    let word_chars = word.chars().count();
    if term_chars < STEM_MIN || word_chars < STEM_MIN {
        return false;
    }
    let common = term
        .chars()
        .zip(word.chars())
        .take_while(|(left, right)| left == right)
        .count();
    let ending = if term_chars.min(word_chars) >= LONG_WORD {
        LONG_ENDING
    } else {
        SHORT_ENDING
    };
    common >= STEM_MIN && term_chars - common <= ending && word_chars - common <= ending
}

fn tokens(text: &str) -> Vec<String> {
    fold(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Lower case without diacritics of Latin letters.
#[must_use]
pub fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            '\u{105}' | 'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' => 'a',
            '\u{107}' | 'ç' | 'č' | 'ĉ' => 'c',
            'ď' | 'đ' => 'd',
            '\u{119}' | 'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ě' => 'e',
            'ì' | 'í' | 'î' | 'ï' | 'ī' => 'i',
            '\u{142}' | 'ľ' | 'ĺ' => 'l',
            '\u{144}' | 'ñ' | 'ň' => 'n',
            '\u{f3}' | 'ò' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => 'o',
            'ř' | 'ŕ' => 'r',
            '\u{15b}' | 'š' | 'ş' => 's',
            'ť' | 'ţ' => 't',
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => 'u',
            'ý' | 'ÿ' => 'y',
            '\u{17a}' | '\u{17c}' | 'ž' => 'z',
            other => other,
        })
        .collect()
}

/// One section of an outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineEntry {
    /// Heading level.
    pub level: u8,
    /// Heading text.
    pub title: String,
    /// Characters of the section up to the next heading of any level.
    pub chars: usize,
}

/// Sections of the blocks with their sizes in the rendered form.
#[must_use]
pub fn sections(blocks: &[Block], form: TextForm) -> Vec<OutlineEntry> {
    let mut out: Vec<OutlineEntry> = Vec::new();
    for block in blocks {
        let size = render_block(block, form).chars().count();
        match block.heading_level() {
            Some(level) => out.push(OutlineEntry {
                level,
                title: block.text.clone(),
                chars: size,
            }),
            None => {
                if let Some(last) = out.last_mut() {
                    last.chars += size;
                }
            }
        }
    }
    out
}

/// Text of an outline: title, length, sections with sizes, the start of the
/// content and the file holding the full content.
#[must_use]
pub fn outline(
    title: Option<&str>,
    content: &str,
    entries: &[OutlineEntry],
    full_content: Option<&str>,
) -> String {
    let mut lines = Vec::new();
    if let Some(title) = title {
        lines.push(format!("title: {title}"));
    }
    lines.push(format!("chars: {}", content.chars().count()));
    if !entries.is_empty() {
        lines.push("sections (chars):".to_owned());
        for entry in entries {
            lines.push(format!(
                "{}{} ({})",
                "  ".repeat(usize::from(entry.level)),
                entry.title,
                entry.chars
            ));
        }
    }
    let start: String = content.chars().take(OUTLINE_START).collect();
    lines.push(format!("start: {}", start.replace('\n', " ")));
    if let Some(path) = full_content {
        lines.push(format!("full content: {path}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::convert_markdown;

    #[test]
    fn matches_inflected_forms_and_diacritics() {
        assert!(matches("install", "installation"));
        assert!(matches("packages", "package"));
        assert!(!matches("pac", "package"));
        assert_eq!(
            fold("\u{104}\u{106}\u{118}\u{141}\u{143}\u{d3}\u{15a}\u{179}\u{17b} Caf\u{e9}"),
            "acelnoszz cafe"
        );
    }

    #[test]
    fn find_keeps_context_and_marks_gaps() {
        let page = convert_markdown("a\n\nb package\n\nc\n\nd\n\ne\n\nf package");
        let pattern = Regex::new("package").unwrap();
        let texts: Vec<String> = find(&page.blocks, &pattern, 1)
            .into_iter()
            .map(|block| block.text)
            .collect();
        assert_eq!(texts, vec!["a", "b package", "c", "…", "e", "f package"]);
    }

    #[test]
    fn find_accepts_the_largest_context() {
        let page = convert_markdown("a\n\nb package\n\nc");
        let pattern = Regex::new("package").unwrap();
        assert_eq!(find(&page.blocks, &pattern, usize::MAX).len(), 3);
        assert!(find(&[], &pattern, 1).is_empty());
    }

    #[test]
    fn section_reports_candidates_and_prefers_exact_titles() {
        let page = convert_markdown("# Install notes\n\nx\n\n# Install\n\ny");
        let found = section(&page.blocks, "INSTALL").unwrap();
        assert_eq!(found[1].text, "y");
        let candidates = section(&page.blocks, "none").unwrap_err();
        assert_eq!(candidates, vec!["Install notes", "Install"]);
    }

    #[test]
    fn query_limits_and_orders_by_document() {
        let page = convert_markdown(
            "# A\n\npackage install guide\n\nother\n\n# B\n\ninstall the package now\n\npackage",
        );
        let best = query(&page.blocks, "install package", 2);
        let texts: Vec<&str> = best.iter().map(|block| block.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["A", "package install guide", "B", "install the package now"]
        );
        assert!(query(&page.blocks, "", 2).is_empty());
    }

    #[test]
    fn outline_lists_sections_with_sizes() {
        let page = convert_markdown("# Title\n\nbody text\n\n## Part\n\nmore");
        let entries = sections(&page.blocks, TextForm::Markdown);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].chars, "# Title".len() + "body text".len());
        let text = outline(Some("T"), "content", &entries, Some("/tmp/full.md"));
        assert!(text.contains("sections (chars):\n  Title (16)\n    Part (11)"));
        assert!(text.ends_with("full content: /tmp/full.md"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
