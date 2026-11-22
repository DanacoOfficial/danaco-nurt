//! Node events of a YAML stream read with the `saphyr-parser` event parser.
//!
//! The parser validates the whole stream against YAML 1.2 and reports every
//! event with its range in characters. The events below carry byte ranges;
//! anchors, tags, indicators and comments are recovered from the source by
//! the model builder. Repeated keys are valid YAML and are kept; the model
//! reads the last of them.

use std::ops::Range;

use saphyr_parser::{Event, Parser};

/// Maximum nesting of collections accepted in a stream.
pub(crate) const MAX_DEPTH: usize = 512;

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
    /// Scalar with its value after unescaping and folding.
    Scalar(String),
    /// Alias.
    Alias,
}

/// Node event with its byte range.
///
/// The range of a scalar includes its anchor and tag and may reach the next
/// token. The range of a collection start or end is the position of the
/// token that starts or ends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawEvent {
    /// Kind of the event.
    pub(crate) kind: RawKind,
    /// Byte range.
    pub(crate) span: Range<usize>,
}

/// Failure of the parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawError {
    /// Byte offset of the error.
    pub(crate) offset: usize,
    /// Message of the parser.
    pub(crate) message: String,
}

/// Events of every document of `text`, in stream order.
pub(crate) fn events(text: &str) -> Result<Vec<Vec<RawEvent>>, RawError> {
    let map = CharMap::new(text);
    let mut documents = Vec::new();
    let mut current: Vec<RawEvent> = Vec::new();
    let mut depth = 0usize;
    for item in Parser::new_from_str(text) {
        let (event, span) = item.map_err(|error| RawError {
            offset: map.byte(error.marker().index()),
            message: error.info().to_owned(),
        })?;
        let range = map.byte(span.start.index())..map.byte(span.end.index());
        let kind = match event {
            Event::DocumentStart(_) => {
                current.clear();
                continue;
            }
            Event::DocumentEnd => {
                documents.push(std::mem::take(&mut current));
                continue;
            }
            Event::Scalar(value, ..) => {
                current.push(RawEvent {
                    kind: RawKind::Scalar(value.into_owned()),
                    span: range,
                });
                continue;
            }
            Event::Alias(_) => {
                current.push(RawEvent {
                    kind: RawKind::Alias,
                    span: range,
                });
                continue;
            }
            Event::MappingStart(..) => RawKind::MappingStart,
            Event::SequenceStart(..) => RawKind::SequenceStart,
            Event::MappingEnd => RawKind::MappingEnd,
            Event::SequenceEnd => RawKind::SequenceEnd,
            Event::StreamStart | Event::StreamEnd | Event::Nothing => continue,
        };
        depth = nest(depth, &kind, range.start)?;
        current.push(RawEvent {
            kind,
            span: range.start..range.start,
        });
    }
    Ok(documents)
}

/// Nesting depth after a collection event.
fn nest(depth: usize, kind: &RawKind, offset: usize) -> Result<usize, RawError> {
    match kind {
        RawKind::MappingStart | RawKind::SequenceStart if depth >= MAX_DEPTH => Err(RawError {
            offset,
            message: format!("collections are nested deeper than {MAX_DEPTH} levels"),
        }),
        RawKind::MappingStart | RawKind::SequenceStart => Ok(depth + 1),
        _ => Ok(depth.saturating_sub(1)),
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
        let extra = position.checked_sub(1).map_or(0, |last| self.extra[last].1);
        (index + extra).min(self.len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar_starts(events: &[RawEvent]) -> Vec<usize> {
        events
            .iter()
            .filter(|event| matches!(event.kind, RawKind::Scalar(_)))
            .map(|event| event.span.start)
            .collect()
    }

    #[test]
    fn maps_character_indices_to_bytes() {
        let map = CharMap::new("aé b");
        assert_eq!(map.byte(0), 0);
        assert_eq!(map.byte(1), 1);
        assert_eq!(map.byte(2), 3);
        assert_eq!(map.byte(4), 5);
        assert_eq!(map.byte(9), 5);
    }

    #[test]
    fn reports_byte_ranges_of_scalars_after_multibyte_text() {
        let events = events("zaéüλö: €é\n").unwrap();
        assert_eq!(scalar_starts(&events[0]), vec![0, 12]);
    }

    #[test]
    fn reports_aliases_and_repeated_keys() {
        let events = events("a: &x 1\nb: *x\na: 2\n").unwrap();
        assert!(events[0].iter().any(|event| event.kind == RawKind::Alias));
        assert_eq!(scalar_starts(&events[0]).len(), 5);
    }

    #[test]
    fn splits_documents() {
        let events = events("a: 1\n--- b\n...\n").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1][0].kind, RawKind::Scalar("b".to_owned()));
        assert!(super::events("").unwrap().is_empty());
    }

    #[test]
    fn reports_error_offset_in_bytes() {
        let error = events("é: [1\n").unwrap_err();
        assert!(error.offset >= 2);
        assert!(events("a: *missing\n").is_err());
    }

    #[test]
    fn rejects_nesting_beyond_the_limit() {
        let text = format!("{}{}", "[".repeat(MAX_DEPTH + 2), "]".repeat(MAX_DEPTH + 2));
        assert!(events(&text).is_err());
    }
}
