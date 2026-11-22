// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Classification of the tokens of a document for highlighting.
//!
//! Command names, parameter labels, `@` references and names bound by
//! `let`, keywords, texts, regular expressions, numbers with their units,
//! operators and comments are classified with the standard token types of
//! the protocol. A token that spans lines is sent as one token per line.

use std::ops::Range;

use lsp_types::{SemanticToken, SemanticTokenModifier, SemanticTokenType, SemanticTokensLegend};
use nurt_syntax::lex::{Token, TokenKind};
use nurt_syntax::parse::Variant;

use crate::catalog::Commands;
use crate::context::{self, adjacent, binds_name, chain_end, is_label};
use crate::position::{Encoding, Lines};

/// Token types of the legend, in the order of their indexes.
const TOKEN_TYPES: [SemanticTokenType; 9] = [
    SemanticTokenType::FUNCTION,
    SemanticTokenType::PARAMETER,
    SemanticTokenType::VARIABLE,
    SemanticTokenType::KEYWORD,
    SemanticTokenType::STRING,
    SemanticTokenType::REGEXP,
    SemanticTokenType::NUMBER,
    SemanticTokenType::OPERATOR,
    SemanticTokenType::COMMENT,
];

const FUNCTION: u32 = 0;
const PARAMETER: u32 = 1;
const VARIABLE: u32 = 2;
const KEYWORD: u32 = 3;
const STRING: u32 = 4;
const REGEXP: u32 = 5;
const NUMBER: u32 = 6;
const OPERATOR: u32 = 7;
const COMMENT: u32 = 8;

/// Bit of the `declaration` modifier, the only modifier of the legend.
const DECLARATION: u32 = 1;

/// Returns the legend of the token types and modifiers.
pub(crate) fn legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: TOKEN_TYPES.to_vec(),
        token_modifiers: vec![SemanticTokenModifier::DECLARATION],
    }
}

/// A classified byte range of the text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Piece {
    range: Range<usize>,
    kind: u32,
    modifiers: u32,
}

impl Piece {
    fn new(range: Range<usize>, kind: u32) -> Self {
        Self {
            range,
            kind,
            modifiers: 0,
        }
    }
}

/// Returns the classified tokens of a document, relatively encoded.
pub(crate) fn tokens(
    text: &str,
    variant: Variant,
    encoding: Encoding,
    commands: &Commands,
) -> Vec<SemanticToken> {
    let lexed = context::tokens(text, variant);
    let mut pieces = classify(text, &lexed, commands);
    pieces.extend(comments(text, &lexed));
    pieces.sort_by_key(|piece| piece.range.start);
    encode(text, &pieces, encoding)
}

/// Classifies the tokens of a text.
///
/// The first name of a stage is a command; inside parentheses only a name
/// of the catalog is, because a parenthesis also groups an expression.
fn classify(text: &str, tokens: &[Token], commands: &Commands) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        let previous = index.checked_sub(1).map(|before| &tokens[before]);
        match token.kind {
            TokenKind::Ident if is_label(token) => {
                pieces.push(Piece::new(token.range.clone(), PARAMETER));
            }
            TokenKind::Ident => {
                let last = chain_end(tokens, index, tokens.len());
                let range = token.range.start..tokens[last].range.end;
                let name = text.get(range.clone()).unwrap_or_default();
                let after_paren = previous.is_some_and(|before| before.kind == TokenKind::LParen);
                if starts_stage(tokens, index) && (!after_paren || commands.get(name).is_some()) {
                    pieces.push(Piece::new(range, FUNCTION));
                    index = last;
                } else if previous.is_some_and(|before| before.kind == TokenKind::Let) {
                    pieces.push(Piece {
                        range: token.range.clone(),
                        kind: VARIABLE,
                        modifiers: DECLARATION,
                    });
                }
            }
            TokenKind::At => {
                let name = tokens
                    .get(index + 1)
                    .filter(|next| next.kind == TokenKind::Ident && adjacent(token, next));
                let end = name.map_or(token.range.end, |next| next.range.end);
                pieces.push(Piece::new(token.range.start..end, VARIABLE));
                if name.is_some() {
                    index += 1;
                }
            }
            ref kind => {
                if let Some(class) = class_of(kind) {
                    pieces.push(Piece::new(token.range.clone(), class));
                }
            }
        }
        index += 1;
    }
    pieces
}

/// Whether the token at `index` is the first of a stage.
fn starts_stage(tokens: &[Token], index: usize) -> bool {
    let Some(before) = index.checked_sub(1) else {
        return true;
    };
    match tokens[before].kind {
        TokenKind::PipeGt | TokenKind::Nl | TokenKind::LParen => true,
        TokenKind::Eq => binds_name(tokens, before),
        _ => false,
    }
}

/// Token type of a keyword, a literal or an operator.
fn class_of(kind: &TokenKind) -> Option<u32> {
    match kind {
        TokenKind::Let
        | TokenKind::True
        | TokenKind::False
        | TokenKind::Null
        | TokenKind::And
        | TokenKind::Or
        | TokenKind::Not
        | TokenKind::In => Some(KEYWORD),
        TokenKind::Text | TokenKind::RawText => Some(STRING),
        TokenKind::Regex => Some(REGEXP),
        TokenKind::Int | TokenKind::Number | TokenKind::Size | TokenKind::Duration => Some(NUMBER),
        TokenKind::PipeGt
        | TokenKind::EqEq
        | TokenKind::NotEq
        | TokenKind::Lt
        | TokenKind::Le
        | TokenKind::Gt
        | TokenKind::Ge
        | TokenKind::Tilde
        | TokenKind::NotTilde
        | TokenKind::Plus
        | TokenKind::Minus
        | TokenKind::Star
        | TokenKind::Slash
        | TokenKind::Eq
        | TokenKind::DotDot
        | TokenKind::Ellipsis => Some(OPERATOR),
        _ => None,
    }
}

/// Finds the comments in the gaps between tokens. A `#` after a quotation
/// mark in the same gap belongs to text the lexer rejected, not to a
/// comment.
fn comments(text: &str, tokens: &[Token]) -> Vec<Piece> {
    let mut starts = vec![0];
    starts.extend(tokens.iter().map(|token| token.range.end));
    let mut ends: Vec<usize> = tokens.iter().map(|token| token.range.start).collect();
    ends.push(text.len());
    let mut pieces = Vec::new();
    for (start, end) in starts.into_iter().zip(ends) {
        let mut position = start;
        while let Some(gap) = text.get(position..end) {
            let Some(hash) = gap.find(['#', '"', '\'']) else {
                break;
            };
            if gap[hash..].starts_with(['"', '\'']) {
                break;
            }
            let comment_start = position + hash;
            let comment_end = text[comment_start..end]
                .find('\n')
                .map_or(end, |line_end| comment_start + line_end);
            pieces.push(Piece::new(comment_start..comment_end, COMMENT));
            position = comment_end;
        }
    }
    pieces
}

/// Encodes the pieces in the relative form of the protocol, one token per
/// line of a piece.
fn encode(text: &str, pieces: &[Piece], encoding: Encoding) -> Vec<SemanticToken> {
    let lines = Lines::new(text);
    let mut encoded = Vec::with_capacity(pieces.len());
    let (mut line, mut character) = (0, 0);
    for piece in pieces {
        for part in line_parts(text, piece.range.clone()) {
            let length = encoding.measure(text.get(part.clone()).unwrap_or_default());
            if length == 0 {
                continue;
            }
            let start = lines.position(part.start, encoding);
            let delta_line = start.line - line;
            let delta_start = if delta_line == 0 {
                start.character - character
            } else {
                start.character
            };
            encoded.push(SemanticToken {
                delta_line,
                delta_start,
                length,
                token_type: piece.kind,
                token_modifiers_bitset: piece.modifiers,
            });
            line = start.line;
            character = start.character;
        }
    }
    encoded
}

/// Splits a byte range at its line breaks, without the breaks.
fn line_parts(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let mut parts = Vec::new();
    let mut start = range.start;
    let content = text.get(range.clone()).unwrap_or_default();
    for (offset, _) in content.match_indices('\n') {
        let end = range.start + offset;
        let end = if text[..end].ends_with('\r') {
            end - 1
        } else {
            end
        };
        parts.push(start..end.max(start));
        start = range.start + offset + 1;
    }
    parts.push(start..range.end);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::tests::sample;

    /// Absolute line, character, length, type and modifiers of each token.
    fn decoded(text: &str, variant: Variant, encoding: Encoding) -> Vec<(u32, u32, u32, u32, u32)> {
        let mut line = 0;
        let mut character = 0;
        tokens(text, variant, encoding, &sample())
            .into_iter()
            .map(|token| {
                if token.delta_line > 0 {
                    character = 0;
                }
                line += token.delta_line;
                character += token.delta_start;
                (
                    line,
                    character,
                    token.length,
                    token.token_type,
                    token.token_modifiers_bitset,
                )
            })
            .collect()
    }

    #[test]
    fn declares_a_legend_with_standard_types() {
        let legend = legend();
        assert_eq!(legend.token_types.len(), TOKEN_TYPES.len());
        assert_eq!(legend.token_types[FUNCTION as usize].as_str(), "function");
        assert_eq!(legend.token_types[COMMENT as usize].as_str(), "comment");
        assert_eq!(legend.token_modifiers[0].as_str(), "declaration");
    }

    #[test]
    fn classifies_commands_labels_literals_and_operators() {
        let text = "find \"/srv\" glob: \"*.go\" |> filter size > 1mb |> git.status";
        assert_eq!(
            decoded(text, Variant::A, Encoding::Utf16),
            [
                (0, 0, 4, FUNCTION, 0),
                (0, 5, 6, STRING, 0),
                (0, 12, 4, PARAMETER, 0),
                (0, 18, 6, STRING, 0),
                (0, 25, 2, OPERATOR, 0),
                (0, 28, 6, FUNCTION, 0),
                (0, 40, 1, OPERATOR, 0),
                (0, 42, 3, NUMBER, 0),
                (0, 46, 2, OPERATOR, 0),
                (0, 49, 10, FUNCTION, 0),
            ]
        );
    }

    #[test]
    fn classifies_references_bindings_keywords_and_comments() {
        let text = "let big = find \"/a\" # all\n@big |> filter not hidden\n";
        assert_eq!(
            decoded(text, Variant::A, Encoding::Utf16),
            [
                (0, 0, 3, KEYWORD, 0),
                (0, 4, 3, VARIABLE, DECLARATION),
                (0, 8, 1, OPERATOR, 0),
                (0, 10, 4, FUNCTION, 0),
                (0, 15, 4, STRING, 0),
                (0, 20, 5, COMMENT, 0),
                (1, 0, 4, VARIABLE, 0),
                (1, 5, 2, OPERATOR, 0),
                (1, 8, 6, FUNCTION, 0),
                (1, 15, 3, KEYWORD, 0),
            ]
        );
    }

    #[test]
    fn classifies_only_catalog_names_inside_parentheses() {
        let text = "verify (count) |> filter (size > 1)";
        let functions: Vec<(u32, u32)> = decoded(text, Variant::A, Encoding::Utf16)
            .into_iter()
            .filter(|token| token.3 == FUNCTION)
            .map(|token| (token.1, token.2))
            .collect();
        assert_eq!(functions, [(0, 6), (8, 5), (18, 6)]);
        let call = decoded("sort(size, order: desc)", Variant::B, Encoding::Utf16);
        assert_eq!(call[0], (0, 0, 4, FUNCTION, 0));
        assert_eq!(call[1], (0, 11, 5, PARAMETER, 0));
    }

    #[test]
    fn splits_tokens_that_span_lines_and_measures_in_the_encoding() {
        let text = "count r#\"a\r\nb\u{1f600}\"#";
        let utf16 = decoded(text, Variant::A, Encoding::Utf16);
        assert_eq!(utf16[1], (0, 6, 4, STRING, 0));
        assert_eq!(utf16[2], (1, 0, 5, STRING, 0));
        let utf32 = decoded(text, Variant::A, Encoding::Utf32);
        assert_eq!(utf32[2], (1, 0, 4, STRING, 0));
        assert_eq!(line_parts("a\n\nb", 0..4), [0..1, 2..2, 3..4]);
    }

    #[test]
    fn ignores_a_hash_in_rejected_text() {
        let text = "count \"a # b";
        let found = decoded(text, Variant::A, Encoding::Utf16);
        assert!(found.iter().all(|token| token.3 != COMMENT), "{found:?}");
        assert!(comments("# one\n# two", &[]).len() == 2);
    }
}
