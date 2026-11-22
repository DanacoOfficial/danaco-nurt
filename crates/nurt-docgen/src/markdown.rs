// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Parser of the Markdown subset used in `docs/`.
//!
//! The module is the only place that interprets Markdown. It produces the
//! tree defined here ([`Document`], [`Block`], [`Inline`]); the HTML and
//! manual page renderers consume the tree and do not read Markdown. The
//! supported subset is: ATX headings, paragraphs, fenced code blocks, bullet
//! and numbered lists with nesting, tables, block quotes, thematic breaks,
//! HTML comments (dropped), code spans, emphasis, strong emphasis, links,
//! autolinks and backslash escapes.

/// Inline element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    /// Plain text.
    Text(String),
    /// Code span.
    Code(String),
    /// Emphasis.
    Emphasis(Vec<Inline>),
    /// Strong emphasis.
    Strong(Vec<Inline>),
    /// Link with its text and destination.
    Link {
        /// Link text.
        text: Vec<Inline>,
        /// Destination as written.
        url: String,
    },
    /// Line break inside a paragraph, rendered as a space.
    SoftBreak,
}

/// Block element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// Heading of level 1 to 6.
    Heading {
        /// Level, from 1 for `#` to 6 for `######`.
        level: u8,
        /// Heading content.
        content: Vec<Inline>,
    },
    /// Paragraph.
    Paragraph(Vec<Inline>),
    /// Fenced code block.
    Code {
        /// Info string after the opening fence, trimmed.
        info: String,
        /// Content without the fences, lines joined with `\n`.
        text: String,
    },
    /// Bullet or numbered list.
    List {
        /// `true` for a numbered list.
        ordered: bool,
        /// Items, each a sequence of blocks.
        items: Vec<Vec<Block>>,
    },
    /// Table with a header row.
    Table {
        /// Header cells.
        header: Vec<Vec<Inline>>,
        /// Body rows.
        rows: Vec<Vec<Vec<Inline>>>,
    },
    /// Block quote.
    Quote(Vec<Block>),
    /// Thematic break.
    Rule,
}

/// Parsed Markdown document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    /// Top-level blocks in source order.
    pub blocks: Vec<Block>,
}

/// Splits a leading YAML front matter block from the text.
///
/// Returns the front matter without the `---` lines and the remaining text.
/// When the text does not start with a front matter block the first value is
/// `None` and the second is the whole text.
///
/// # Examples
///
/// ```
/// let (front, body) = nurt_docgen::markdown::split_front_matter("---\ntitle: A\n---\n# A\n");
/// assert_eq!(front, Some("title: A"));
/// assert_eq!(body, "# A\n");
/// ```
#[must_use]
pub fn split_front_matter(source: &str) -> (Option<&str>, &str) {
    let Some(rest) = source.strip_prefix("---\n") else {
        return (None, source);
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            let front = rest[..offset].trim_end_matches('\n');
            return (Some(front), &rest[offset + line.len()..]);
        }
        offset += line.len();
    }
    (None, source)
}

/// Parses Markdown text.
#[must_use]
pub fn parse(source: &str) -> Document {
    let lines: Vec<&str> = source.lines().collect();
    Document {
        blocks: parse_blocks(&lines),
    }
}

/// Returns the plain text of inline elements, without markup.
#[must_use]
pub fn plain_text(inlines: &[Inline]) -> String {
    let mut text = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(value) | Inline::Code(value) => text.push_str(value),
            Inline::Emphasis(inner) | Inline::Strong(inner) => text.push_str(&plain_text(inner)),
            Inline::Link { text: inner, .. } => text.push_str(&plain_text(inner)),
            Inline::SoftBreak => text.push(' '),
        }
    }
    text
}

/// Returns the plain text of all blocks, separated by spaces.
#[must_use]
pub fn blocks_text(blocks: &[Block]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for block in blocks {
        match block {
            Block::Heading { content, .. } | Block::Paragraph(content) => {
                parts.push(plain_text(content));
            }
            Block::Code { text, .. } => parts.push(text.clone()),
            Block::List { items, .. } => {
                for item in items {
                    parts.push(blocks_text(item));
                }
            }
            Block::Table { header, rows } => {
                for cell in header {
                    parts.push(plain_text(cell));
                }
                for row in rows {
                    for cell in row {
                        parts.push(plain_text(cell));
                    }
                }
            }
            Block::Quote(inner) => parts.push(blocks_text(inner)),
            Block::Rule => {}
        }
    }
    parts.join(" ")
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn fence_of(line: &str) -> Option<(char, usize, &str)> {
    let trimmed = line.trim_start();
    if indent_of(line) > 3 {
        return None;
    }
    let marker = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let count = trimmed.chars().take_while(|c| *c == marker).count();
    if count < 3 {
        return None;
    }
    let info = trimmed[count..].trim();
    if marker == '`' && info.contains('`') {
        return None;
    }
    Some((marker, count, info))
}

fn heading_of(line: &str) -> Option<(u8, &str)> {
    if indent_of(line) > 3 {
        return None;
    }
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &trimmed[level..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    let text = rest.trim().trim_end_matches('#').trim_end();
    let level = u8::try_from(level).ok()?;
    Some((level, text))
}

fn is_rule(line: &str) -> bool {
    let trimmed = line.trim();
    let Some(first) = trimmed.chars().next() else {
        return false;
    };
    matches!(first, '-' | '*' | '_')
        && trimmed.chars().filter(|c| !c.is_whitespace()).count() >= 3
        && trimmed.chars().all(|c| c == first || c == ' ')
}

struct ListMarker {
    indent: usize,
    ordered: bool,
    content_indent: usize,
    rest: String,
}

fn list_marker(line: &str) -> Option<ListMarker> {
    let indent = indent_of(line);
    let trimmed = &line[indent..];
    let (ordered, marker_len) = if let Some(first) = trimmed.chars().next()
        && matches!(first, '-' | '*' | '+')
    {
        (false, 1)
    } else {
        let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
        if digits == 0 || digits > 9 {
            return None;
        }
        let delimiter = trimmed[digits..].chars().next()?;
        if delimiter != '.' && delimiter != ')' {
            return None;
        }
        (true, digits + 1)
    };
    let after = &trimmed[marker_len..];
    if !after.is_empty() && !after.starts_with(' ') {
        return None;
    }
    if !ordered && is_rule(line) {
        return None;
    }
    let spaces = after.len() - after.trim_start_matches(' ').len();
    let spaces = if spaces == 0 || spaces > 4 { 1 } else { spaces };
    Some(ListMarker {
        indent,
        ordered,
        content_indent: indent + marker_len + spaces,
        rest: after.trim_start_matches(' ').to_string(),
    })
}

/// Splits a table row into trimmed cells, resolving `\|` to `|`.
#[must_use]
pub fn split_cells(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let trimmed = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut chars = trimmed.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'|') {
            current.push('|');
            chars.next();
        } else if c == '|' {
            cells.push(current.trim().to_string());
            current = String::new();
        } else {
            current.push(c);
        }
    }
    if !current.trim().is_empty() {
        cells.push(current.trim().to_string());
    }
    cells
}

fn is_delimiter_row(line: &str) -> bool {
    if !line.contains('-') || !line.contains('|') && !line.trim().starts_with('-') {
        return false;
    }
    let cells = split_cells(line);
    !cells.is_empty()
        && cells.iter().all(|cell| {
            let core = cell.trim_matches(':');
            !core.is_empty() && core.chars().all(|c| c == '-')
        })
}

fn starts_block(line: &str, next: Option<&str>) -> bool {
    let trimmed = line.trim_start();
    fence_of(line).is_some()
        || heading_of(line).is_some()
        || trimmed.starts_with('>')
        || trimmed.starts_with("<!--")
        || list_marker(line).is_some()
        || is_rule(line)
        || (line.contains('|') && next.is_some_and(is_delimiter_row))
}

fn parse_blocks(lines: &[&str]) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            i += 1;
        } else if line.trim_start().starts_with("<!--") {
            while i < lines.len() && !lines[i].contains("-->") {
                i += 1;
            }
            i += 1;
        } else if let Some((marker, count, info)) = fence_of(line) {
            i = parse_fence(lines, i, marker, count, info, &mut blocks);
        } else if let Some((level, text)) = heading_of(line) {
            blocks.push(Block::Heading {
                level,
                content: parse_inlines(text),
            });
            i += 1;
        } else if is_rule(line) && list_marker(line).is_none() {
            blocks.push(Block::Rule);
            i += 1;
        } else if line.trim_start().starts_with('>') {
            let start = i;
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                i += 1;
            }
            let inner: Vec<&str> = lines[start..i]
                .iter()
                .map(|quoted| {
                    let rest = quoted.trim_start().strip_prefix('>').unwrap_or(quoted);
                    rest.strip_prefix(' ').unwrap_or(rest)
                })
                .collect();
            blocks.push(Block::Quote(parse_blocks(&inner)));
        } else if line.contains('|') && lines.get(i + 1).copied().is_some_and(is_delimiter_row) {
            i = parse_table(lines, i, &mut blocks);
        } else if list_marker(line).is_some() {
            i = parse_list(lines, i, &mut blocks);
        } else {
            let start = i;
            i += 1;
            while i < lines.len()
                && !lines[i].trim().is_empty()
                && !starts_block(lines[i], lines.get(i + 1).copied())
            {
                i += 1;
            }
            let text = lines[start..i]
                .iter()
                .map(|l| l.trim())
                .collect::<Vec<_>>()
                .join("\n");
            blocks.push(Block::Paragraph(parse_inlines(&text)));
        }
    }
    blocks
}

fn parse_fence(
    lines: &[&str],
    start: usize,
    marker: char,
    count: usize,
    info: &str,
    blocks: &mut Vec<Block>,
) -> usize {
    let open_indent = indent_of(lines[start]);
    let mut text: Vec<&str> = Vec::new();
    let mut i = start + 1;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        let run = trimmed.chars().take_while(|c| *c == marker).count();
        if run >= count && run == trimmed.chars().count() {
            i += 1;
            break;
        }
        let strip = indent_of(line).min(open_indent);
        text.push(&line[strip..]);
        i += 1;
    }
    blocks.push(Block::Code {
        info: info.to_string(),
        text: text.join("\n"),
    });
    i
}

fn parse_table(lines: &[&str], start: usize, blocks: &mut Vec<Block>) -> usize {
    let header = split_cells(lines[start])
        .iter()
        .map(|cell| parse_inlines(cell))
        .collect();
    let mut rows = Vec::new();
    let mut i = start + 2;
    while i < lines.len() && !lines[i].trim().is_empty() && lines[i].contains('|') {
        rows.push(
            split_cells(lines[i])
                .iter()
                .map(|cell| parse_inlines(cell))
                .collect(),
        );
        i += 1;
    }
    blocks.push(Block::Table { header, rows });
    i
}

fn parse_list(lines: &[&str], start: usize, blocks: &mut Vec<Block>) -> usize {
    let Some(first) = list_marker(lines[start]) else {
        return start + 1;
    };
    let base_indent = first.indent;
    let ordered = first.ordered;
    let mut items: Vec<Vec<Block>> = Vec::new();
    let mut i = start;
    while i < lines.len() {
        let Some(marker) = list_marker(lines[i]) else {
            break;
        };
        if marker.indent != base_indent || marker.ordered != ordered {
            break;
        }
        let mut item_lines: Vec<String> = vec![marker.rest.clone()];
        i += 1;
        while i < lines.len() {
            let line = lines[i];
            if line.trim().is_empty() {
                let next = lines[i + 1..].iter().find(|l| !l.trim().is_empty());
                match next {
                    Some(next_line) if indent_of(next_line) >= marker.content_indent => {
                        item_lines.push(String::new());
                        i += 1;
                    }
                    _ => break,
                }
            } else if indent_of(line) >= marker.content_indent {
                item_lines.push(line[marker.content_indent..].to_string());
                i += 1;
            } else if list_marker(line).is_none()
                && !starts_block(line, lines.get(i + 1).copied())
                && item_lines.last().is_some_and(|prev| !prev.is_empty())
            {
                item_lines.push(line.trim().to_string());
                i += 1;
            } else {
                break;
            }
        }
        let refs: Vec<&str> = item_lines.iter().map(String::as_str).collect();
        items.push(parse_blocks(&refs));
        if i < lines.len() && lines[i].trim().is_empty() {
            let next = lines[i..].iter().find(|l| !l.trim().is_empty());
            if next
                .and_then(|l| list_marker(l))
                .is_some_and(|m| m.indent == base_indent && m.ordered == ordered)
            {
                while i < lines.len() && lines[i].trim().is_empty() {
                    i += 1;
                }
            }
        }
    }
    blocks.push(Block::List { ordered, items });
    i
}

/// Parses inline Markdown.
#[must_use]
pub fn parse_inlines(text: &str) -> Vec<Inline> {
    let chars: Vec<char> = text.chars().collect();
    parse_inline_range(&chars)
}

fn flush(buffer: &mut String, out: &mut Vec<Inline>) {
    if !buffer.is_empty() {
        out.push(Inline::Text(std::mem::take(buffer)));
    }
}

fn find_run(chars: &[char], from: usize, marker: char, count: usize) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == '`' && marker != '`' {
            let run = chars[i..].iter().take_while(|c| **c == '`').count();
            match find_run(chars, i + run, '`', run) {
                Some(end) => {
                    i = end + run;
                    continue;
                }
                None => {
                    i += run;
                    continue;
                }
            }
        }
        if chars[i] == marker {
            let run = chars[i..].iter().take_while(|c| **c == marker).count();
            if run == count {
                return Some(i);
            }
            i += run;
            continue;
        }
        i += 1;
    }
    None
}

fn parse_inline_range(chars: &[char]) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut buffer = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' if i + 1 < chars.len() && chars[i + 1].is_ascii_punctuation() => {
                buffer.push(chars[i + 1]);
                i += 2;
            }
            '\n' => {
                flush(&mut buffer, &mut out);
                out.push(Inline::SoftBreak);
                i += 1;
            }
            '`' => {
                let run = chars[i..].iter().take_while(|x| **x == '`').count();
                if let Some(end) = find_run(chars, i + run, '`', run) {
                    flush(&mut buffer, &mut out);
                    let raw: String = chars[i + run..end].iter().collect();
                    let raw = raw.replace('\n', " ");
                    let code = if raw.starts_with(' ') && raw.ends_with(' ') && raw.trim() != "" {
                        raw[1..raw.len() - 1].to_string()
                    } else {
                        raw
                    };
                    out.push(Inline::Code(code));
                    i = end + run;
                } else {
                    buffer.extend(std::iter::repeat_n('`', run));
                    i += run;
                }
            }
            '*' | '_' => {
                let run = chars[i..].iter().take_while(|x| **x == c).count();
                let before_word = i > 0 && chars[i - 1].is_alphanumeric();
                let usable = run <= 2 && !(c == '_' && before_word);
                let closing = if usable {
                    find_run(chars, i + run, c, run).filter(|end| {
                        *end > i + run
                            && !chars[end - 1].is_whitespace()
                            && !chars.get(i + run).is_some_and(|n| n.is_whitespace())
                            && !(c == '_'
                                && chars.get(end + run).is_some_and(|n| n.is_alphanumeric()))
                    })
                } else {
                    None
                };
                if let Some(end) = closing {
                    flush(&mut buffer, &mut out);
                    let inner = parse_inline_range(&chars[i + run..end]);
                    out.push(if run == 2 {
                        Inline::Strong(inner)
                    } else {
                        Inline::Emphasis(inner)
                    });
                    i = end + run;
                } else {
                    buffer.extend(std::iter::repeat_n(c, run));
                    i += run;
                }
            }
            '[' => {
                if let Some((text_end, url_end)) = link_extent(chars, i) {
                    flush(&mut buffer, &mut out);
                    let inner = parse_inline_range(&chars[i + 1..text_end]);
                    let target: String = chars[text_end + 2..url_end].iter().collect();
                    let url = target.split_whitespace().next().unwrap_or("").to_string();
                    out.push(Inline::Link { text: inner, url });
                    i = url_end + 1;
                } else {
                    buffer.push(c);
                    i += 1;
                }
            }
            '<' => {
                let rest: String = chars[i + 1..].iter().take_while(|x| **x != '>').collect();
                let closed = chars.get(i + 1 + rest.chars().count()) == Some(&'>');
                if closed && (rest.starts_with("http://") || rest.starts_with("https://")) {
                    flush(&mut buffer, &mut out);
                    out.push(Inline::Link {
                        text: vec![Inline::Text(rest.clone())],
                        url: rest.clone(),
                    });
                    i += rest.chars().count() + 2;
                } else {
                    buffer.push(c);
                    i += 1;
                }
            }
            _ => {
                buffer.push(c);
                i += 1;
            }
        }
    }
    flush(&mut buffer, &mut out);
    out
}

fn link_extent(chars: &[char], open: usize) -> Option<(usize, usize)> {
    let mut depth = 0usize;
    let mut i = open;
    let mut text_end = None;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '`' => {
                let run = chars[i..].iter().take_while(|c| **c == '`').count();
                match find_run(chars, i + run, '`', run) {
                    Some(end) => i = end + run - 1,
                    None => i += run - 1,
                }
            }
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    text_end = Some(i);
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let text_end = text_end?;
    if chars.get(text_end + 1) != Some(&'(') {
        return None;
    }
    let mut paren = 0usize;
    let mut j = text_end + 1;
    while j < chars.len() {
        match chars[j] {
            '(' => paren += 1,
            ')' => {
                paren -= 1;
                if paren == 0 {
                    return Some((text_end, j));
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_inlines(source: &str) -> Vec<Inline> {
        match parse(source).blocks.into_iter().next() {
            Some(Block::Paragraph(inlines)) => inlines,
            other => panic!("expected a paragraph, found {other:?}"),
        }
    }

    #[test]
    fn splits_front_matter_from_body() {
        let (front, body) = split_front_matter("---\na: 1\nb: 2\n---\n# T\n");
        assert_eq!(front, Some("a: 1\nb: 2"));
        assert_eq!(body, "# T\n");
    }

    #[test]
    fn keeps_text_without_front_matter() {
        assert_eq!(split_front_matter("# T\n"), (None, "# T\n"));
        assert_eq!(split_front_matter("---\nopen\n"), (None, "---\nopen\n"));
    }

    #[test]
    fn parses_headings_with_levels() {
        let doc = parse("# One\n\n### Three ##\n");
        assert_eq!(
            doc.blocks,
            vec![
                Block::Heading {
                    level: 1,
                    content: vec![Inline::Text("One".into())]
                },
                Block::Heading {
                    level: 3,
                    content: vec![Inline::Text("Three".into())]
                },
            ]
        );
    }

    #[test]
    fn joins_paragraph_lines_with_soft_breaks() {
        let inlines = first_inlines("first\nsecond\n\nthird");
        assert_eq!(
            inlines,
            vec![
                Inline::Text("first".into()),
                Inline::SoftBreak,
                Inline::Text("second".into())
            ]
        );
    }

    #[test]
    fn parses_code_spans_with_special_characters() {
        let inlines = first_inlines("use `a | b` and ``x ` y``");
        assert_eq!(
            inlines,
            vec![
                Inline::Text("use ".into()),
                Inline::Code("a | b".into()),
                Inline::Text(" and ".into()),
                Inline::Code("x ` y".into()),
            ]
        );
    }

    #[test]
    fn parses_emphasis_strong_and_links() {
        let inlines = first_inlines("*a* **b** [c](d.md \"t\") <https://example.com>");
        assert_eq!(
            inlines,
            vec![
                Inline::Emphasis(vec![Inline::Text("a".into())]),
                Inline::Text(" ".into()),
                Inline::Strong(vec![Inline::Text("b".into())]),
                Inline::Text(" ".into()),
                Inline::Link {
                    text: vec![Inline::Text("c".into())],
                    url: "d.md".into()
                },
                Inline::Text(" ".into()),
                Inline::Link {
                    text: vec![Inline::Text("https://example.com".into())],
                    url: "https://example.com".into()
                },
            ]
        );
    }

    #[test]
    fn keeps_underscores_inside_words() {
        let inlines = first_inlines("on_missing and case_sensitive");
        assert_eq!(
            inlines,
            vec![Inline::Text("on_missing and case_sensitive".into())]
        );
    }

    #[test]
    fn unescapes_backslash_punctuation() {
        let inlines = first_inlines(r"a \* b \[c\]");
        assert_eq!(inlines, vec![Inline::Text("a * b [c]".into())]);
    }

    #[test]
    fn parses_fenced_code_with_info_string() {
        let doc = parse("```nurt example\nfind \"/a\"\n|> count\n```\n");
        assert_eq!(
            doc.blocks,
            vec![Block::Code {
                info: "nurt example".into(),
                text: "find \"/a\"\n|> count".into()
            }]
        );
    }

    #[test]
    fn longer_fence_may_contain_shorter_fence() {
        let doc = parse("````markdown\n```text\nx\n```\n````\n");
        assert_eq!(
            doc.blocks,
            vec![Block::Code {
                info: "markdown".into(),
                text: "```text\nx\n```".into()
            }]
        );
    }

    #[test]
    fn parses_tables_with_escaped_pipes() {
        let doc = parse("| A | B |\n|---|:-:|\n| `x \\| y` | z |\n");
        let Some(Block::Table { header, rows }) = doc.blocks.first() else {
            panic!("expected a table");
        };
        assert_eq!(header.len(), 2);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0], vec![Inline::Code("x | y".into())]);
    }

    #[test]
    fn parses_nested_lists() {
        let doc = parse("- a\n  - b\n  - c\n- d\n");
        let Some(Block::List { ordered, items }) = doc.blocks.first() else {
            panic!("expected a list");
        };
        assert!(!ordered);
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0][1], Block::List { .. }));
    }

    #[test]
    fn parses_numbered_lists_and_lazy_continuation() {
        let doc = parse("1. first\ncontinued\n2. second\n");
        let Some(Block::List { ordered, items }) = doc.blocks.first() else {
            panic!("expected a list");
        };
        assert!(ordered);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn drops_html_comments() {
        let doc = parse("<!-- one\ntwo -->\ntext\n");
        assert_eq!(doc.blocks.len(), 1);
    }

    #[test]
    fn parses_quotes_and_rules() {
        let doc = parse("> quoted\n\n---\n");
        assert!(matches!(doc.blocks[0], Block::Quote(_)));
        assert_eq!(doc.blocks[1], Block::Rule);
    }

    #[test]
    fn extracts_plain_text_of_blocks() {
        let doc = parse("# T\n\nA `b` **c**\n");
        assert_eq!(blocks_text(&doc.blocks), "T A b c");
    }
}
