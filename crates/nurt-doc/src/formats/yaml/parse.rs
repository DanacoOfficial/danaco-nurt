// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Construction of a [`YamlModel`] from source text.
//!
//! The `saphyr-parser` event parser validates the stream and reports the range of every
//! node. Indicators (`?`, `:`, `-`, `,`, brackets), properties, directives,
//! document markers and comments are then read from the source between those
//! ranges with a cursor that moves forward through the text.

use std::collections::HashMap;
use std::ops::Range;

use nurt_contracts::error::{Error, ErrorCode, Result};

use super::backend_saphyr::{self, RawEvent, RawKind};

use super::model::{
    BlockHeader, Chomping, CollectionStyle, Directive, DirectiveKind, FlowDelimiters, MapEntry,
    MappingNode, NodeId, NodeKind, Property, ScalarNode, ScalarStyle, SeqItem, SequenceNode,
    TagProperty, YamlDocument, YamlModel, YamlNode, YamlVersion,
};

/// Byte order mark.
const BYTE_ORDER_MARK: &str = "\u{FEFF}";

/// Prefix of the tags of the YAML core schema.
pub(crate) const CORE_TAG_PREFIX: &str = "tag:yaml.org,2002:";

/// Characters that end an anchor name or a tag shorthand.
const FLOW_INDICATORS: &[u8] = b",[]{}";

/// Length of the document markers `---` and `...`.
const MARKER_LEN: usize = 3;

/// Length of a URI percent escape (`%` and two hexadecimal digits).
const PERCENT_ESCAPE_LEN: usize = 3;

/// Radix of the digits of a percent escape.
const HEX_RADIX: u32 = 16;

impl YamlModel {
    /// Model of the YAML stream `text`, typed by the YAML 1.2 rules unless a
    /// document declares version 1.1.
    ///
    /// # Errors
    ///
    /// `E_DOC_PARSE` when the text is not a valid YAML stream or when it
    /// contains template syntax (`{{ }}`, `{% %}`).
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_doc::formats::yaml::YamlModel;
    ///
    /// let model = YamlModel::parse("name: demo # app\nreplicas: 2\n")?;
    /// let root = model.documents()[0].root;
    /// let replicas = model.map_get(root, "replicas").unwrap();
    /// assert_eq!(model.text(&model.node(replicas).span), "2");
    /// assert_eq!(model.comments().len(), 1);
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn parse(text: &str) -> Result<Self> {
        Self::parse_with(text, YamlVersion::V1_2)
    }

    /// Model of the YAML stream `text`, typed by the given rules unless a
    /// document declares its version.
    ///
    /// # Errors
    ///
    /// As for [`YamlModel::parse`].
    pub fn parse_with(text: &str, default_version: YamlVersion) -> Result<Self> {
        let raw = match backend_saphyr::events(text) {
            Ok(raw) => raw,
            Err(error) if has_template(text) => return Err(template_error(text, error.offset)),
            Err(error) => return Err(parse_error(text, error.offset, &error.message)),
        };
        let model = Builder::new(text, default_version).build(raw);
        if let Some(offset) = template_in_flow(&model) {
            return Err(template_error(text, offset));
        }
        Ok(model)
    }
}

fn has_template(text: &str) -> bool {
    text.contains("{{") || text.contains("{%")
}

/// Offset of a flow mapping that opens with `{{`, the start of a template
/// expression that happens to parse as nested flow mappings.
fn template_in_flow(model: &YamlModel) -> Option<usize> {
    model.nodes.iter().find_map(|node| {
        let delimiters = node.as_mapping()?.delimiters.as_ref()?;
        (model.source.as_bytes().get(delimiters.open.end) == Some(&b'{'))
            .then_some(delimiters.open.start)
    })
}

fn position(text: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let before = text.get(..offset).unwrap_or(text);
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit('\n')
        .next()
        .map_or(0, |tail| tail.chars().count())
        + 1;
    (line, column)
}

fn located(error: Error, text: &str, offset: usize) -> Error {
    let (line, column) = position(text, offset);
    error
        .with_detail("format", "yaml".into())
        .with_detail("line", line.into())
        .with_detail("column", column.into())
        .with_detail("byte", offset.into())
}

fn parse_error(text: &str, offset: usize, message: &str) -> Error {
    let (line, column) = position(text, offset);
    let error = Error::new(
        ErrorCode::DocParse,
        format!("invalid YAML document at line {line}, column {column}: {message}"),
    );
    located(error, text, offset)
}

fn template_error(text: &str, offset: usize) -> Error {
    let (line, column) = position(text, offset);
    let error = Error::new(
        ErrorCode::DocParse,
        format!(
            "template syntax at line {line}, column {column} is not YAML; change the file as text"
        ),
    )
    .with_problem("the document is a template, not a YAML document")
    .with_fix("replace")
    .with_next_step("edit");
    located(error, text, offset)
}

/// Position of a node in its parent, which decides where an empty node lies
/// and which indicators precede the node.
#[derive(Debug, Clone, Copy)]
enum Slot {
    Root,
    Key,
    Value { colon_end: Option<usize> },
    Item,
}

/// Properties read at the start of a node range.
#[derive(Debug, Default)]
struct Props {
    anchor: Option<Property>,
    tag: Option<(String, Range<usize>)>,
    end: usize,
    same_line: bool,
}

impl Props {
    const fn is_empty(&self) -> bool {
        self.anchor.is_none() && self.tag.is_none()
    }
}

/// Lexical items that delimit documents.
#[derive(Debug)]
enum Marker {
    Directive(Directive),
    Start(Range<usize>),
    End(Range<usize>),
    Content,
}

#[derive(Debug, Default)]
struct DocDraft {
    directives: Vec<Directive>,
    start_marker: Option<Range<usize>>,
    end_marker: Option<Range<usize>>,
}

struct Builder<'a> {
    text: &'a str,
    bytes: &'a [u8],
    model: YamlModel,
    default_version: YamlVersion,
    events: Vec<RawEvent>,
    ends: Vec<usize>,
    next: usize,
    cursor: usize,
    document: usize,
    handles: Vec<(String, String)>,
    anchors: HashMap<String, NodeId>,
}

impl<'a> Builder<'a> {
    fn new(text: &'a str, default_version: YamlVersion) -> Self {
        let bytes = text.as_bytes();
        let mut line_starts = vec![0];
        for (index, &byte) in bytes.iter().enumerate() {
            let breaks = byte == b'\n' || (byte == b'\r' && bytes.get(index + 1) != Some(&b'\n'));
            if breaks {
                line_starts.push(index + 1);
            }
        }
        let bom = text.starts_with(BYTE_ORDER_MARK);
        Self {
            text,
            bytes,
            model: YamlModel {
                source: text.to_owned(),
                bom,
                documents: Vec::new(),
                nodes: Vec::new(),
                comments: Vec::new(),
                line_starts,
                alias_targets: HashMap::new(),
            },
            default_version,
            events: Vec::new(),
            ends: Vec::new(),
            next: 0,
            cursor: if bom { BYTE_ORDER_MARK.len() } else { 0 },
            document: 0,
            handles: Vec::new(),
            anchors: HashMap::new(),
        }
    }

    fn build(mut self, raw: Vec<Vec<RawEvent>>) -> YamlModel {
        let mut scalars: Vec<Range<usize>> = raw
            .iter()
            .flatten()
            .filter(|event| matches!(event.kind, RawKind::Scalar(_) | RawKind::Alias))
            .map(|event| event.span.start..raw_scalar_end(self.bytes, &event.span))
            .filter(|span| !span.is_empty())
            .collect();
        scalars.sort_by_key(|span| span.start);
        let drafts = self.split_documents(&raw, &scalars);
        let mut drafts = drafts.into_iter();
        for (index, events) in raw.into_iter().enumerate() {
            let draft = drafts.next().unwrap_or_default();
            self.document = index;
            self.build_document(events, draft);
        }
        self.model.comments = scan_comments(self.bytes, &scalars);
        self.model
    }

    fn build_document(&mut self, events: Vec<RawEvent>, draft: DocDraft) {
        let version = draft
            .directives
            .iter()
            .find_map(|directive| match &directive.kind {
                DirectiveKind::Yaml { version } if version == "1.1" => Some(YamlVersion::V1_1),
                DirectiveKind::Yaml { .. } => Some(YamlVersion::V1_2),
                _ => None,
            })
            .unwrap_or(self.default_version);
        self.handles = draft
            .directives
            .iter()
            .filter_map(|directive| match &directive.kind {
                DirectiveKind::Tag { handle, prefix } => Some((handle.clone(), prefix.clone())),
                _ => None,
            })
            .collect();
        self.anchors.clear();
        if let Some(marker) = &draft.start_marker {
            self.cursor = self.cursor.max(marker.end);
        }
        self.ends = matching_ends(&events);
        self.events = events;
        self.next = 0;
        let root = if self.events.is_empty() {
            self.empty_node(self.cursor, None, false)
        } else {
            self.node(None, Slot::Root, false)
        };
        let root_span = self.model.node(root).full_span();
        let mut start = root_span.start;
        if let Some(first) = draft.directives.first() {
            start = start.min(first.span.start);
        }
        if let Some(marker) = &draft.start_marker {
            start = start.min(marker.start);
        }
        let mut end = root_span.end;
        if let Some(marker) = &draft.start_marker {
            end = end.max(marker.end);
        }
        if let Some(marker) = &draft.end_marker {
            end = end.max(marker.end);
            self.cursor = self.cursor.max(marker.end);
        }
        self.model.documents.push(YamlDocument {
            span: start..end.max(start),
            directives: draft.directives,
            start_marker: draft.start_marker,
            end_marker: draft.end_marker,
            root,
            version,
        });
    }

    /// Splits the stream into documents by directives, `---` and `...` at
    /// the start of lines outside scalars.
    fn split_documents(&self, raw: &[Vec<RawEvent>], scalars: &[Range<usize>]) -> Vec<DocDraft> {
        let mut items: Vec<(usize, Marker)> = Vec::new();
        for &line in &self.model.line_starts {
            if inside(scalars, line) {
                continue;
            }
            let start = if line == 0 && self.model.bom {
                BYTE_ORDER_MARK.len()
            } else {
                line
            };
            if let Some(item) = self.marker_at(start) {
                items.push((start, item));
            }
        }
        for events in raw {
            if let Some(first) = events.first() {
                let empty = matches!(first.kind, RawKind::Scalar(_)) && first.span.is_empty();
                if !empty {
                    items.push((first.span.start, Marker::Content));
                }
            }
        }
        items.sort_by_key(|(offset, _)| *offset);
        let mut drafts = Vec::new();
        let mut current: Option<DocDraft> = None;
        let mut pending = Vec::new();
        for (_, item) in items {
            match item {
                Marker::Directive(directive) => {
                    drafts.extend(current.take());
                    pending.push(directive);
                }
                Marker::Start(span) => {
                    drafts.extend(current.take());
                    current = Some(DocDraft {
                        directives: std::mem::take(&mut pending),
                        start_marker: Some(span),
                        end_marker: None,
                    });
                }
                Marker::End(span) => {
                    if let Some(mut draft) = current.take() {
                        draft.end_marker = Some(span);
                        drafts.push(draft);
                    }
                }
                Marker::Content => {
                    if current.is_none() {
                        current = Some(DocDraft {
                            directives: std::mem::take(&mut pending),
                            ..DocDraft::default()
                        });
                    }
                }
            }
        }
        drafts.extend(current);
        drafts
    }

    fn marker_at(&self, start: usize) -> Option<Marker> {
        let rest = self.bytes.get(start..)?;
        let separated = |length: usize| {
            rest.get(length)
                .is_none_or(|&byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
        };
        if rest.starts_with(b"---") && separated(MARKER_LEN) {
            return Some(Marker::Start(start..start + MARKER_LEN));
        }
        if rest.starts_with(b"...") && separated(MARKER_LEN) {
            return Some(Marker::End(start..start + MARKER_LEN));
        }
        if rest.first() == Some(&b'%') {
            return Some(Marker::Directive(self.directive_at(start)));
        }
        None
    }

    fn directive_at(&self, start: usize) -> Directive {
        let line_end = self.model.line_end(start);
        let mut end = line_end;
        let line = &self.bytes[start..line_end];
        if let Some(hash) = line
            .windows(2)
            .position(|pair| matches!(pair[0], b' ' | b'\t') && pair[1] == b'#')
        {
            end = start + hash;
        }
        while end > start && matches!(self.bytes[end - 1], b' ' | b'\t') {
            end -= 1;
        }
        let words: Vec<&str> = self.text[start + 1..end].split_whitespace().collect();
        let kind = match words.as_slice() {
            ["YAML", version, ..] => DirectiveKind::Yaml {
                version: (*version).to_owned(),
            },
            ["TAG", handle, prefix, ..] => DirectiveKind::Tag {
                handle: (*handle).to_owned(),
                prefix: (*prefix).to_owned(),
            },
            [name, ..] => DirectiveKind::Reserved {
                name: (*name).to_owned(),
            },
            [] => DirectiveKind::Reserved {
                name: String::new(),
            },
        };
        Directive {
            span: start..end,
            kind,
        }
    }

    fn take_event(&mut self) -> Option<(usize, RawEvent)> {
        let index = self.next;
        let event = self.events.get(index)?.clone();
        self.next += 1;
        Some((index, event))
    }

    fn peek_is_end(&self) -> bool {
        self.events
            .get(self.next)
            .is_none_or(|event| matches!(event.kind, RawKind::MappingEnd | RawKind::SequenceEnd))
    }

    fn node(&mut self, parent: Option<NodeId>, slot: Slot, in_flow: bool) -> NodeId {
        let Some((index, event)) = self.take_event() else {
            return self.empty_node(self.cursor, parent, in_flow);
        };
        match event.kind {
            RawKind::Scalar(raw) => self.scalar(&event.span, raw, parent, slot, in_flow),
            RawKind::Alias => self.alias(&event.span, parent, in_flow),
            RawKind::MappingStart => self.mapping(index, &event.span, parent, in_flow),
            RawKind::SequenceStart => self.sequence(index, &event.span, parent, in_flow),
            RawKind::MappingEnd | RawKind::SequenceEnd => {
                self.next -= 1;
                self.empty_node(self.cursor, parent, in_flow)
            }
        }
    }

    fn base(&self, span: Range<usize>, parent: Option<NodeId>, in_flow: bool) -> YamlNode {
        YamlNode {
            kind: NodeKind::Scalar(ScalarNode {
                style: ScalarStyle::Plain,
                value: String::new(),
                block: None,
            }),
            span,
            anchor: None,
            tag: None,
            parent,
            document: self.document,
            in_flow,
        }
    }

    fn empty_node(&mut self, at: usize, parent: Option<NodeId>, in_flow: bool) -> NodeId {
        let node = self.base(at..at, parent, in_flow);
        self.model.push_node(node)
    }

    fn register_anchor(&mut self, id: NodeId) {
        if let Some(anchor) = &self.model.node(id).anchor {
            self.anchors.insert(anchor.name.clone(), id);
        }
    }

    fn alias(&mut self, span: &Range<usize>, parent: Option<NodeId>, in_flow: bool) -> NodeId {
        let end = token_end(self.bytes, span.start + 1, span.end);
        let span = &(span.start..end);
        let name = self.text[span.start + 1..end].to_owned();
        let mut node = self.base(span.clone(), parent, in_flow);
        node.kind = NodeKind::Alias(Property {
            name: name.clone(),
            span: span.clone(),
        });
        let id = self.model.push_node(node);
        if let Some(&target) = self.anchors.get(&name) {
            self.model.alias_targets.insert(id, target);
        }
        self.cursor = self.cursor.max(span.end);
        id
    }

    fn scalar(
        &mut self,
        span: &Range<usize>,
        raw: String,
        parent: Option<NodeId>,
        slot: Slot,
        in_flow: bool,
    ) -> NodeId {
        let props = self.read_props(span.start, span.end);
        let after_props = if props.is_empty() {
            span.start
        } else {
            props.end
        };
        let start = skip_space_and_comments(self.bytes, after_props, span.end);
        let mut content = start.min(span.end)..span.end;
        if !content.is_empty() {
            content.end = scalar_end(self.bytes, content.start, content.end);
        }
        // The parser reports an empty node with the range of the next token
        // (`?`, `,`, `---`).
        let misplaced = raw.is_empty()
            && !matches!(
                self.bytes.get(content.start),
                Some(b'\'' | b'"' | b'|' | b'>')
            );
        if span.is_empty() || content.is_empty() || misplaced {
            let at = match slot {
                _ if !props.is_empty() => props.end,
                Slot::Value {
                    colon_end: Some(colon_end),
                } => colon_end,
                _ => self.cursor.max(span.start.min(self.cursor)),
            };
            content = at..at;
        }
        let style = if content.is_empty() {
            ScalarStyle::Plain
        } else {
            match self.bytes[content.start] {
                b'\'' => ScalarStyle::SingleQuoted,
                b'"' => ScalarStyle::DoubleQuoted,
                b'|' => ScalarStyle::Literal,
                b'>' => ScalarStyle::Folded,
                _ => ScalarStyle::Plain,
            }
        };
        let value = raw;
        let block = style
            .is_block()
            .then(|| self.block_header(content.start, span.end, &value));
        if let Some(header) = &block {
            content.end = self
                .block_end(span.end, header.chomping)
                .max(header.span.end);
        }
        let mut node = self.base(content, parent, in_flow);
        node.kind = NodeKind::Scalar(ScalarNode {
            style,
            value,
            block,
        });
        node.anchor = props.anchor;
        node.tag = props.tag.map(|(text, span)| self.tag(text, span));
        let id = self.model.push_node(node);
        self.register_anchor(id);
        let consumed = if style.is_block() {
            span.end
        } else {
            self.model.node(id).span.end
        };
        self.cursor = self.cursor.max(consumed);
        id
    }

    fn block_header(&self, start: usize, limit: usize, value: &str) -> BlockHeader {
        let mut end = start + 1;
        let mut chomping = Chomping::Clip;
        let mut indicator = None;
        while end < limit {
            match self.bytes[end] {
                b'-' => chomping = Chomping::Strip,
                b'+' => chomping = Chomping::Keep,
                digit @ b'1'..=b'9' => indicator = Some(digit - b'0'),
                _ => break,
            }
            end += 1;
        }
        let first_line = self.model.next_line_start(end);
        let mut line = first_line;
        let mut content_indent = None;
        while line < limit {
            let line_end = self.model.line_end(line);
            let spaces = self.bytes[line..line_end]
                .iter()
                .take_while(|&&byte| byte == b' ')
                .count();
            if line + spaces < line_end {
                let leading = value.chars().take_while(|&c| c == ' ').count();
                let leading = if indicator.is_some() { leading } else { 0 };
                content_indent = Some(spaces.saturating_sub(leading));
                break;
            }
            line = self.model.next_line_start(line);
        }
        BlockHeader {
            span: start..end,
            chomping,
            indent_indicator: indicator,
            content_indent: content_indent.unwrap_or_else(|| self.model.line_indent(start) + 2),
        }
    }

    /// End of a block scalar: before the final line break for `+`, at the
    /// end of the last non-blank line otherwise.
    fn block_end(&self, raw_end: usize, chomping: Chomping) -> usize {
        let bytes = self.bytes;
        let mut end = raw_end.min(bytes.len());
        if chomping == Chomping::Keep {
            if end > 0 && bytes[end - 1] == b'\n' {
                end -= 1;
            }
            if end > 0 && bytes[end - 1] == b'\r' {
                end -= 1;
            }
            return end;
        }
        loop {
            while end > 0 && matches!(bytes[end - 1], b'\n' | b'\r') {
                end -= 1;
            }
            let start = self.model.line_start(end.saturating_sub(1));
            let blank = bytes[start..end]
                .iter()
                .all(|&byte| byte == b' ' || byte == b'\t');
            if blank && start > 0 && start < end {
                end = start;
            } else if blank && start == end && end > 0 {
                continue;
            } else {
                return end;
            }
        }
    }

    fn tag(&self, text: String, span: Range<usize>) -> TagProperty {
        let resolved = resolve_tag(&text, &self.handles);
        TagProperty {
            text,
            resolved,
            span,
        }
    }

    /// Reads anchors and tags at `start`, not beyond `limit`.
    fn read_props(&self, start: usize, limit: usize) -> Props {
        let bytes = self.bytes;
        let mut props = Props {
            end: start,
            ..Props::default()
        };
        let mut position = start;
        while position < limit {
            match bytes[position] {
                b'&' if props.anchor.is_none() => {
                    let end = token_end(bytes, position + 1, limit);
                    props.anchor = Some(Property {
                        name: self.text[position + 1..end].to_owned(),
                        span: position..end,
                    });
                    position = end;
                }
                b'!' if props.tag.is_none() => {
                    let end = if bytes.get(position + 1) == Some(&b'<') {
                        bytes[position..limit]
                            .iter()
                            .position(|&byte| byte == b'>')
                            .map_or(limit, |offset| position + offset + 1)
                    } else {
                        token_end(bytes, position + 1, limit)
                    };
                    props.tag = Some((self.text[position..end].to_owned(), position..end));
                    position = end;
                }
                _ => break,
            }
            props.end = position;
            while position < limit && matches!(bytes[position], b' ' | b'\t') {
                position += 1;
            }
        }
        props.same_line = position < bytes.len()
            && position < limit
            && !matches!(bytes[position], b'\n' | b'\r' | b'#');
        props
    }

    fn mapping(
        &mut self,
        index: usize,
        span: &Range<usize>,
        parent: Option<NodeId>,
        in_flow: bool,
    ) -> NodeId {
        let end_position = self.end_position(index);
        let flow_close = closes_with(self.bytes, end_position, b'}');
        let mut props = self.read_props(span.start, self.limit(index));
        if !flow_close && props.same_line && !in_flow {
            props = Props {
                end: span.start,
                ..Props::default()
            };
        }
        if !props.is_empty() {
            self.cursor = self.cursor.max(props.end);
        }
        let content_start = skip_space_and_comments(self.bytes, self.cursor, self.bytes.len());
        let flow = flow_close && self.bytes.get(content_start) == Some(&b'{');
        let style = if flow {
            CollectionStyle::Flow
        } else if in_flow {
            CollectionStyle::FlowPair
        } else {
            CollectionStyle::Block
        };
        let id = self.placeholder(content_start, parent, in_flow, props);
        if flow {
            self.cursor = content_start + 1;
        }
        let child_flow = in_flow || flow;
        let mut entries = Vec::new();
        while !self.peek_is_end() {
            entries.push(self.entry(id, style, child_flow));
        }
        self.take_event();
        let delimiters = flow.then(|| {
            let close = self.close_bracket(end_position, b'}');
            FlowDelimiters {
                open: content_start..content_start + 1,
                close: close..close + 1,
            }
        });
        let start = entries.first().map_or(content_start, |entry| {
            if flow {
                content_start
            } else {
                entry.span.start
            }
        });
        let end = match &delimiters {
            Some(delimiters) => delimiters.close.end,
            None => entries.last().map_or(start, |entry| {
                self.model.node(entry.value).span.end.max(
                    entry
                        .colon
                        .as_ref()
                        .map_or(entry.span.start, |colon| colon.end),
                )
            }),
        };
        let indent = self.model.column(start);
        let node = self.model.node_mut(id);
        node.span = start..end.max(start);
        node.kind = NodeKind::Mapping(MappingNode {
            style,
            delimiters,
            indent,
            entries,
        });
        self.register_anchor(id);
        id
    }

    fn entry(&mut self, map: NodeId, style: CollectionStyle, in_flow: bool) -> MapEntry {
        let start = skip_space_and_comments(self.bytes, self.cursor, self.bytes.len());
        let question = (self.bytes.get(start) == Some(&b'?')
            && separated_at(self.bytes, start + 1))
        .then(|| start..start + 1);
        if let Some(question) = &question {
            self.cursor = question.end;
        }
        let key = self.node(Some(map), Slot::Key, in_flow);
        let key_span = self.model.node(key).full_span();
        let after_key = skip_space_and_comments(self.bytes, self.cursor, self.bytes.len());
        let colon = (self.bytes.get(after_key) == Some(&b':')).then(|| after_key..after_key + 1);
        if let Some(colon) = &colon {
            self.cursor = colon.end;
        }
        let value = if self.peek_is_end() && colon.is_none() {
            self.empty_node(key_span.end, Some(map), in_flow)
        } else {
            self.node(
                Some(map),
                Slot::Value {
                    colon_end: colon.as_ref().map(|colon| colon.end),
                },
                in_flow,
            )
        };
        let comma = if style == CollectionStyle::Flow {
            self.comma()
        } else {
            None
        };
        let merge = self
            .model
            .node(key)
            .as_scalar()
            .is_some_and(|scalar| scalar.style == ScalarStyle::Plain && scalar.value == "<<");
        let mut entry_start = key_span.start;
        if let Some(question) = &question {
            entry_start = entry_start.min(question.start);
        }
        if let Some(colon) = &colon {
            entry_start = entry_start.min(colon.start);
        }
        let value_end = self
            .model
            .node(value)
            .span
            .end
            .max(colon.as_ref().map_or(key_span.end, |colon| colon.end));
        let end = if in_flow {
            value_end
        } else {
            self.model.line_end(value_end)
        };
        MapEntry {
            key,
            value,
            question,
            colon,
            comma,
            merge,
            span: entry_start..end,
        }
    }

    fn sequence(
        &mut self,
        index: usize,
        span: &Range<usize>,
        parent: Option<NodeId>,
        in_flow: bool,
    ) -> NodeId {
        let end_position = self.end_position(index);
        let lead = skip_space_and_comments(self.bytes, self.cursor, self.bytes.len());
        let props = self.read_props(lead.min(span.start), self.limit(index));
        if !props.is_empty() {
            self.cursor = self.cursor.max(props.end);
        }
        let content_start = skip_space_and_comments(self.bytes, self.cursor, self.bytes.len());
        let flow = closes_with(self.bytes, end_position, b']')
            && self.bytes.get(content_start) == Some(&b'[');
        let id = self.placeholder(content_start, parent, in_flow, props);
        if flow {
            self.cursor = content_start + 1;
        }
        let child_flow = in_flow || flow;
        let mut items = Vec::new();
        while !self.peek_is_end() {
            items.push(self.item(id, flow, child_flow));
        }
        self.take_event();
        let delimiters = flow.then(|| {
            let close = self.close_bracket(end_position, b']');
            FlowDelimiters {
                open: content_start..content_start + 1,
                close: close..close + 1,
            }
        });
        let start = if flow {
            content_start
        } else {
            items.first().map_or(content_start, |item| item.span.start)
        };
        let end = match &delimiters {
            Some(delimiters) => delimiters.close.end,
            None => items.last().map_or(start, |item| {
                self.model.node(item.node).span.end.max(item.span.start + 1)
            }),
        };
        let indent = self.model.column(start);
        let node = self.model.node_mut(id);
        node.span = start..end.max(start);
        node.kind = NodeKind::Sequence(SequenceNode {
            style: if flow {
                CollectionStyle::Flow
            } else {
                CollectionStyle::Block
            },
            delimiters,
            indent,
            items,
        });
        self.register_anchor(id);
        id
    }

    fn item(&mut self, sequence: NodeId, flow: bool, in_flow: bool) -> SeqItem {
        let mut dash = None;
        if !flow {
            let start = skip_space_and_comments(self.bytes, self.cursor, self.bytes.len());
            if self.bytes.get(start) == Some(&b'-') && separated_at(self.bytes, start + 1) {
                dash = Some(start..start + 1);
                self.cursor = start + 1;
            }
        }
        let node = self.node(Some(sequence), Slot::Item, in_flow);
        if let Some(dash) = &dash
            && self.model.node(node).is_empty_node()
            && self.model.node(node).span.start < dash.end
        {
            self.model.node_mut(node).span = dash.end..dash.end;
        }
        let comma = if flow { self.comma() } else { None };
        let node_span = self.model.node(node).full_span();
        let start = dash.as_ref().map_or(node_span.start, |dash| dash.start);
        let end = if in_flow {
            node_span.end
        } else {
            self.model.line_end(node_span.end.max(start + 1))
        };
        SeqItem {
            node,
            dash,
            comma,
            span: start..end,
        }
    }

    fn comma(&mut self) -> Option<Range<usize>> {
        let position = skip_space_and_comments(self.bytes, self.cursor, self.bytes.len());
        (self.bytes.get(position) == Some(&b',')).then(|| {
            self.cursor = position + 1;
            position..position + 1
        })
    }

    fn placeholder(
        &mut self,
        start: usize,
        parent: Option<NodeId>,
        in_flow: bool,
        props: Props,
    ) -> NodeId {
        let mut node = self.base(start..start, parent, in_flow);
        node.anchor = props.anchor;
        node.tag = props.tag.map(|(text, span)| self.tag(text, span));
        self.model.push_node(node)
    }

    fn close_bracket(&mut self, end_position: usize, bracket: u8) -> usize {
        let position = if self.bytes.get(end_position) == Some(&bracket) {
            end_position
        } else {
            let start = skip_space_and_comments(self.bytes, self.cursor, self.bytes.len());
            self.bytes[start..]
                .iter()
                .position(|&byte| byte == bracket)
                .map_or(start, |offset| start + offset)
        };
        self.cursor = self.cursor.max(position + 1);
        position
    }

    fn end_position(&self, start_index: usize) -> usize {
        self.ends
            .get(start_index)
            .and_then(|&end| self.events.get(end))
            .map_or(self.bytes.len(), |event| event.span.start)
    }

    /// Start of the first child of a collection, bounding the properties of
    /// the collection.
    fn limit(&self, start_index: usize) -> usize {
        self.events
            .get(start_index + 1)
            .map_or(self.bytes.len(), |event| event.span.start)
            .max(self.events[start_index].span.start)
    }
}

/// Index of the matching end event of every start event.
fn matching_ends(events: &[RawEvent]) -> Vec<usize> {
    let mut ends = vec![usize::MAX; events.len()];
    let mut stack = Vec::new();
    for (index, event) in events.iter().enumerate() {
        match event.kind {
            RawKind::MappingStart | RawKind::SequenceStart => stack.push(index),
            RawKind::MappingEnd | RawKind::SequenceEnd => {
                if let Some(start) = stack.pop() {
                    ends[start] = index;
                }
            }
            _ => {}
        }
    }
    ends
}

const fn is_blank(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

/// Returns `true` when the collection end reported at `position` is the
/// bracket `bracket`, possibly after blanks and a trailing comma.
fn closes_with(bytes: &[u8], position: usize, bracket: u8) -> bool {
    if bytes.get(position) == Some(&bracket) {
        return true;
    }
    let mut forward = position;
    while bytes
        .get(forward)
        .is_some_and(|&byte| is_blank(byte) || byte == b',')
    {
        forward += 1;
    }
    if bytes.get(forward) == Some(&bracket) {
        return true;
    }
    let mut back = position.min(bytes.len());
    while back > 0 && is_blank(bytes[back - 1]) {
        back -= 1;
    }
    back > 0 && bytes[back - 1] == bracket
}

/// End of the content of a scalar starting at `start`: the loader reports
/// the range up to the next token, which includes blanks and comments after
/// a quoted or plain scalar.
fn scalar_end(bytes: &[u8], start: usize, end: usize) -> usize {
    match bytes.get(start) {
        Some(b'"') => quoted_end(bytes, start, end, b'"'),
        Some(b'\'') => quoted_end(bytes, start, end, b'\''),
        Some(b'|' | b'>') => end,
        _ => plain_end(bytes, start, end),
    }
}

fn quoted_end(bytes: &[u8], start: usize, end: usize, quote: u8) -> usize {
    let mut position = start + 1;
    while position < end {
        match bytes[position] {
            b'\\' if quote == b'"' => position += 2,
            b'\'' if quote == b'\'' && bytes.get(position + 1) == Some(&b'\'') => position += 2,
            byte if byte == quote => return position + 1,
            _ => position += 1,
        }
    }
    end
}

fn plain_end(bytes: &[u8], start: usize, end: usize) -> usize {
    let mut last = start;
    for position in start..end.min(bytes.len()) {
        let byte = bytes[position];
        if byte == b'#' && position > start && is_blank(bytes[position - 1]) {
            break;
        }
        if !is_blank(byte) {
            last = position + 1;
        }
    }
    last
}

/// End of the content of a raw scalar or alias range, which may start with
/// an anchor and a tag.
fn raw_scalar_end(bytes: &[u8], span: &Range<usize>) -> usize {
    let mut start = span.start;
    while matches!(bytes.get(start), Some(b'&' | b'!')) {
        start = token_end(bytes, start + 1, span.end);
        while start < span.end && is_blank(bytes[start]) {
            start += 1;
        }
    }
    if start >= span.end {
        return span.end;
    }
    scalar_end(bytes, start, span.end)
}

fn separated_at(bytes: &[u8], position: usize) -> bool {
    bytes
        .get(position)
        .is_none_or(|&byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
}

fn token_end(bytes: &[u8], start: usize, limit: usize) -> usize {
    let mut end = start;
    while end < limit
        && !matches!(bytes[end], b' ' | b'\t' | b'\r' | b'\n')
        && !FLOW_INDICATORS.contains(&bytes[end])
    {
        end += 1;
    }
    end
}

/// First position at or after `start` that is not a blank, a line break or
/// part of a comment.
pub(crate) fn skip_space_and_comments(bytes: &[u8], start: usize, limit: usize) -> usize {
    let mut position = start;
    let mut after_blank = position == 0
        || bytes
            .get(position.wrapping_sub(1))
            .is_some_and(|&byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'));
    while position < limit.min(bytes.len()) {
        match bytes[position] {
            b' ' | b'\t' | b'\r' | b'\n' => {
                after_blank = true;
                position += 1;
            }
            b'#' if after_blank => {
                while position < bytes.len() && !matches!(bytes[position], b'\n' | b'\r') {
                    position += 1;
                }
            }
            _ => break,
        }
    }
    position
}

/// Whether `offset` lies strictly inside one of the sorted ranges.
fn inside(ranges: &[Range<usize>], offset: usize) -> bool {
    let index = ranges.partition_point(|range| range.start < offset);
    index
        .checked_sub(1)
        .is_some_and(|last| ranges[last].end > offset)
}

fn scan_comments(bytes: &[u8], scalars: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut comments = Vec::new();
    let mut position = 0;
    let mut covered = 0;
    while position < bytes.len() {
        if bytes[position] != b'#' {
            position += 1;
            continue;
        }
        while covered < scalars.len() && scalars[covered].end <= position {
            covered += 1;
        }
        let in_scalar = scalars
            .get(covered)
            .is_some_and(|range| range.start <= position && position < range.end);
        let after_blank = position == 0
            || matches!(bytes[position - 1], b' ' | b'\t' | b'\r' | b'\n')
            || (position == BYTE_ORDER_MARK.len() && bytes.starts_with(BYTE_ORDER_MARK.as_bytes()));
        if in_scalar || !after_blank {
            position += 1;
            continue;
        }
        let start = position;
        while position < bytes.len() && !matches!(bytes[position], b'\n' | b'\r') {
            position += 1;
        }
        comments.push(start..position);
    }
    comments
}

/// Full form of a tag: verbatim tags unchanged, shorthands with their handle
/// replaced by its prefix and percent escapes decoded.
pub(crate) fn resolve_tag(text: &str, handles: &[(String, String)]) -> String {
    if let Some(verbatim) = text
        .strip_prefix("!<")
        .and_then(|rest| rest.strip_suffix('>'))
    {
        return verbatim.to_owned();
    }
    if text == "!" {
        return text.to_owned();
    }
    let (handle, suffix) = if let Some(suffix) = text.strip_prefix("!!") {
        ("!!", suffix)
    } else if let Some(second) = text[1..].find('!') {
        text.split_at(second + 2)
    } else {
        ("!", &text[1..])
    };
    let prefix = handles
        .iter()
        .rev()
        .find(|(name, _)| name == handle)
        .map(|(_, prefix)| prefix.as_str())
        .unwrap_or(match handle {
            "!!" => CORE_TAG_PREFIX,
            other => other,
        });
    format!("{prefix}{}", percent_decode(suffix))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escape = (bytes[index] == b'%')
            .then(|| text.get(index + 1..index + PERCENT_ESCAPE_LEN))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, HEX_RADIX).ok());
        if let Some(byte) = escape {
            decoded.push(byte);
            index += PERCENT_ESCAPE_LEN;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> YamlModel {
        YamlModel::parse(text).unwrap()
    }

    #[test]
    fn records_ranges_of_keys_values_and_indicators() {
        let model = parse("a: 1\nb:   'x' # note\n");
        let root = model.documents()[0].root;
        let mapping = model.node(root).as_mapping().unwrap();
        let entry = &mapping.entries[1];
        assert_eq!(model.text(&model.node(entry.key).span), "b");
        assert_eq!(model.text(&model.node(entry.value).span), "'x'");
        assert_eq!(model.text(entry.colon.as_ref().unwrap()), ":");
        assert_eq!(model.text(&entry.span), "b:   'x' # note");
        assert_eq!(model.comments().len(), 1);
        assert_eq!(model.comments()[0], 14..20);
    }

    #[test]
    fn separates_properties_from_content() {
        let model = parse("k: &anc !!str v\nr: *anc\n");
        let root = model.documents()[0].root;
        let value = model.map_get(root, "k").unwrap();
        let node = model.node(value);
        assert_eq!(model.text(&node.span), "v");
        assert_eq!(node.anchor.as_ref().unwrap().name, "anc");
        assert_eq!(node.tag.as_ref().unwrap().resolved, "tag:yaml.org,2002:str");
        let alias = model.map_get(root, "r").unwrap();
        assert_eq!(model.anchor_target(alias), Some(value));
    }

    #[test]
    fn places_empty_values_after_the_colon() {
        let model = parse("e:\nf: ~\n");
        let root = model.documents()[0].root;
        let empty = model.map_get(root, "e").unwrap();
        assert_eq!(model.node(empty).span, 2..2);
        assert!(model.node(empty).is_empty_node());
    }

    #[test]
    fn ends_block_scalars_at_their_last_content_line() {
        let model = parse("lit: |+\n  one\n\nfold: >-\n  a\n  b\n\n# c\nz: 1\n");
        let root = model.documents()[0].root;
        let keep = model.map_get(root, "lit").unwrap();
        assert_eq!(model.text(&model.node(keep).span), "|+\n  one\n");
        let fold = model.map_get(root, "fold").unwrap();
        let node = model.node(fold);
        assert_eq!(model.text(&node.span), ">-\n  a\n  b");
        let header = node.as_scalar().unwrap().block.as_ref().unwrap();
        assert_eq!(header.chomping, Chomping::Strip);
        assert_eq!(header.content_indent, 2);
    }

    #[test]
    fn describes_flow_and_block_collections() {
        let model = parse("m:\n  x: [1, 2]\nn: !t { p: q }\nl:\n- a\n");
        let root = model.documents()[0].root;
        let n = model.node(model.map_get(root, "n").unwrap());
        let mapping = n.as_mapping().unwrap();
        assert_eq!(mapping.style, CollectionStyle::Flow);
        assert_eq!(model.text(&n.span), "{ p: q }");
        assert_eq!(n.tag.as_ref().unwrap().text, "!t");
        let l = model.node(model.map_get(root, "l").unwrap());
        let sequence = l.as_sequence().unwrap();
        assert_eq!(sequence.indent, 0);
        assert_eq!(model.text(sequence.items[0].dash.as_ref().unwrap()), "-");
        let m = model.node(model.map_get(root, "m").unwrap());
        assert_eq!(m.as_mapping().unwrap().indent, 2);
    }

    #[test]
    fn splits_documents_with_directives_and_markers() {
        let model = parse("%YAML 1.1\n---\na: yes\n...\n--- b\n");
        assert_eq!(model.documents().len(), 2);
        let first = &model.documents()[0];
        assert_eq!(first.version, YamlVersion::V1_1);
        assert_eq!(first.directives.len(), 1);
        assert!(first.end_marker.is_some());
        assert_eq!(model.documents()[1].version, YamlVersion::V1_2);
    }

    #[test]
    fn rejects_templates_with_a_text_fix() {
        let error = YamlModel::parse("a: {{ .Values.x }}\n").unwrap_err();
        assert_eq!(error.code(), ErrorCode::DocParse);
        assert_eq!(error.fix.first().map(String::as_str), Some("replace"));
        let error = YamlModel::parse("{% if x %}\na: 1\n").unwrap_err();
        assert_eq!(error.code(), ErrorCode::DocParse);
    }

    #[test]
    fn accepts_expressions_inside_scalars() {
        assert!(YamlModel::parse("run: echo ${{ secrets.TOKEN }}\n").is_ok());
    }

    #[test]
    fn reports_position_of_syntax_errors() {
        let error = YamlModel::parse("a: [1\nb: 2\n").unwrap_err();
        assert_eq!(error.code(), ErrorCode::DocParse);
        assert!(error.details.contains_key("line"));
    }

    #[test]
    fn resolves_tag_handles() {
        let handles = vec![("!e!".to_owned(), "tag:example.com,2000:".to_owned())];
        assert_eq!(
            resolve_tag("!e!foo%21", &handles),
            "tag:example.com,2000:foo!"
        );
        assert_eq!(resolve_tag("!<!bar>", &handles), "!bar");
        assert_eq!(resolve_tag("!local", &handles), "!local");
        assert_eq!(resolve_tag("!", &handles), "!");
    }

    #[test]
    fn keeps_byte_order_mark_and_crlf() {
        let model = parse("\u{FEFF}a: 1\r\nb: 2\r\n");
        assert!(model.has_bom());
        assert_eq!(model.line_ending(), super::super::model::LineEnding::CrLf);
        let root = model.documents()[0].root;
        let b = model.map_get(root, "b").unwrap();
        assert_eq!(model.text(&model.node(b).span), "2");
    }
}
