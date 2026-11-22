// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Node events of a YAML stream read with the `saphyr` loader.
//!
//! The loader validates the whole stream against YAML 1.2 and reports every
//! node with its range in characters. The events below carry byte ranges;
//! anchors, tags, indicators and comments are recovered from the source by
//! the model builder. An alias appears in the loader output as a copy of
//! its target with the range of the alias, so an alias is recognised by the
//! `*` at the start of its range.

use std::ops::Range;

use saphyr::{LoadableYamlNode, MarkedYaml, Scalar, YamlData};

/// Maximum nesting of collections accepted in a stream.
pub(crate) const MAX_DEPTH: usize = 512;

/// Content of a scalar as reported by the loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RawScalar {
    /// Text after unescaping and folding.
    Text(String),
    /// Plain scalar that the loader resolved to a non-string type; its text
    /// equals its source text.
    Typed,
    /// Scalar without a loader value: an empty document or a value that does
    /// not match its core tag.
    Unresolved,
}

/// Kind of a [`RawEvent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RawKind {
    /// Start of a mapping.
    MappingStart,
    /// End of a mapping.
    MappingEnd,
    /// Start of a sequence.
    SequenceStart,
    /// End of a sequence.
    SequenceEnd,
    /// Scalar.
    Scalar(RawScalar),
    /// Alias.
    Alias,
}

/// Node event with its byte range.
///
/// The range of a node includes its anchor and tag. The range of a
/// collection end is the position of the token that ends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawEvent {
    /// Kind of the event.
    pub(crate) kind: RawKind,
    /// Byte range.
    pub(crate) span: Range<usize>,
}

/// Failure of the loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawError {
    /// Byte offset of the error.
    pub(crate) offset: usize,
    /// Message of the loader.
    pub(crate) message: String,
}

/// Events of every document of `text`, in stream order.
pub(crate) fn events(text: &str) -> Result<Vec<Vec<RawEvent>>, RawError> {
    let map = CharMap::new(text);
    let documents = MarkedYaml::load_from_str(text).map_err(|error| RawError {
        offset: map.byte(error.marker().index()),
        message: error.info().to_owned(),
    })?;
    let mut result = Vec::with_capacity(documents.len());
    for document in &documents {
        let mut out = Vec::new();
        walk(document, text, &map, 0, &mut out)?;
        result.push(out);
    }
    Ok(result)
}

fn walk(
    node: &MarkedYaml<'_>,
    text: &str,
    map: &CharMap,
    depth: usize,
    out: &mut Vec<RawEvent>,
) -> Result<(), RawError> {
    let span = map.byte(node.span.start.index())..map.byte(node.span.end.index());
    if depth > MAX_DEPTH {
        return Err(RawError {
            offset: span.start,
            message: format!("collections are nested deeper than {MAX_DEPTH} levels"),
        });
    }
    let alias = text.get(span.clone()).is_some_and(|slice| slice.starts_with('*'));
    if alias {
        out.push(RawEvent {
            kind: RawKind::Alias,
            span,
        });
        return Ok(());
    }
    match &node.data {
        YamlData::Tagged(_, inner) => walk(inner, text, map, depth, out),
        YamlData::Sequence(items) => {
            out.push(RawEvent {
                kind: RawKind::SequenceStart,
                span: span.start..span.start,
            });
            for item in items {
                walk(item, text, map, depth + 1, out)?;
            }
            out.push(RawEvent {
                kind: RawKind::SequenceEnd,
                span: span.end..span.end,
            });
            Ok(())
        }
        YamlData::Mapping(entries) => {
            out.push(RawEvent {
                kind: RawKind::MappingStart,
                span: span.start..span.start,
            });
            for (key, value) in entries {
                walk(key, text, map, depth + 1, out)?;
                walk(value, text, map, depth + 1, out)?;
            }
            out.push(RawEvent {
                kind: RawKind::MappingEnd,
                span: span.end..span.end,
            });
            Ok(())
        }
        YamlData::Value(Scalar::String(value)) | YamlData::Representation(value, _, _) => {
            out.push(RawEvent {
                kind: RawKind::Scalar(RawScalar::Text(value.to_string())),
                span,
            });
            Ok(())
        }
        YamlData::Value(_) => {
            out.push(RawEvent {
                kind: RawKind::Scalar(RawScalar::Typed),
                span,
            });
            Ok(())
        }
        YamlData::Alias(_) | YamlData::BadValue => {
            out.push(RawEvent {
                kind: RawKind::Scalar(RawScalar::Unresolved),
                span,
            });
            Ok(())
        }
    }
}

/// Conversion of character indices to byte offsets.
///
/// Only characters longer than one byte are recorded, so the map of an
/// ASCII text is empty.
struct CharMap {
    /// Pairs of the index of the character after a multi-byte character and
    /// the number of extra bytes before that index.
    extra: Vec<(usize, usize)>,
    len: usize,
}

impl CharMap {
    fn new(text: &str) -> Self {
        let mut extra = Vec::new();
        let mut total = 0;
        for (index, character) in text.chars().enumerate() {
            let width = character.len_utf8();
            if width > 1 {
                total += width - 1;
                extra.push((index + 1, total));
            }
        }
        Self {
            extra,
            len: text.len(),
        }
    }

    fn byte(&self, index: usize) -> usize {
        let position = self.extra.partition_point(|&(start, _)| start <= index);
        let extra = position
            .checked_sub(1)
            .map_or(0, |last| self.extra[last].1);
        (index + extra).min(self.len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_character_indices_to_bytes() {
        let map = CharMap::new("aż b");
        assert_eq!(map.byte(0), 0);
        assert_eq!(map.byte(1), 1);
        assert_eq!(map.byte(2), 3);
        assert_eq!(map.byte(4), 5);
        assert_eq!(map.byte(9), 5);
    }

    #[test]
    fn reports_byte_ranges_of_scalars_after_multibyte_text() {
        let events = events("zażółć: ąę\n").unwrap();
        let spans: Vec<_> = events[0].iter().map(|event| event.span.clone()).collect();
        assert_eq!(spans[1], 0..10);
        assert_eq!(spans[2], 12..16);
    }

    #[test]
    fn recognises_aliases_by_their_indicator() {
        let events = events("a: &x 1\nb: *x\n").unwrap();
        assert!(events[0].iter().any(|event| event.kind == RawKind::Alias));
    }

    #[test]
    fn reports_error_offset_in_bytes() {
        let error = events("ż: [1\n").unwrap_err();
        assert!(error.offset >= 2);
    }

    #[test]
    fn rejects_nesting_beyond_the_limit() {
        let text = format!("{}{}", "[".repeat(MAX_DEPTH + 2), "]".repeat(MAX_DEPTH + 2));
        assert!(events(&text).is_err());
    }
}
