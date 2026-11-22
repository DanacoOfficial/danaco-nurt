// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
/**
 * Tree-sitter grammar for Nurt program text, syntax variant A.
 *
 * The grammar describes the surface syntax only. Argument positions, types and
 * defaults belong to the binding stage, so every stage is a command name
 * followed by positional and named arguments.
 *
 * Newlines are significant. A newline ends a statement unless the next
 * non-empty line starts with `|>`; the `pipe` token therefore absorbs the
 * newlines and comment lines that precede it. Inside brackets a newline is
 * accepted after the opening bracket, after each element and after each comma.
 *
 * Lexical notes:
 * - A label is a lower-case name immediately followed by a colon.
 * - `/` starts a regular expression only when it is followed by a character
 *   that is not white space; a `/` followed by white space is division.
 * - A sign directly followed by a digit belongs to the numeric literal, so
 *   `first 20 -1` has two arguments while `size - 100` is a subtraction.
 * - Raw strings support up to three `#` characters in the delimiters.
 */

const PREC = {
  or: 1,
  and: 2,
  not: 3,
  compare: 4,
  additive: 5,
  multiplicative: 6,
  field: 7,
};

const IDENTIFIER = /[a-z][a-z0-9_]*/;
const MAX_RAW_STRING_HASHES = 3;

function rawString(hashes) {
  const delimiter = '#'.repeat(hashes);
  if (hashes === 0) {
    return /r"[^"]*"/;
  }
  const guards = [];
  for (let run = 0; run < hashes; run += 1) {
    guards.push(`"+${'#'.repeat(run)}[^"#]`);
  }
  return new RegExp(`r${delimiter}"(?:[^"]|${guards.join('|')})*"+${delimiter}`);
}

function delimited($, open, item, close) {
  const element = seq(item, optional($._newlines));
  return seq(
    open,
    optional($._newlines),
    optional(
      seq(
        element,
        repeat(seq(',', optional($._newlines), element)),
        optional(seq(',', optional($._newlines))),
      ),
    ),
    close,
  );
}

module.exports = grammar({
  name: 'nurt',

  extras: ($) => [/[ \t]+/, /\uFEFF/, $.comment],

  conflicts: ($) => [[$.command_name, $._expression]],

  rules: {
    source_file: ($) =>
      seq(
        optional($._newlines),
        optional(
          seq(
            $._statement,
            repeat(seq($._newlines, $._statement)),
            optional($._newlines),
          ),
        ),
      ),

    _newlines: ($) => repeat1($._newline),

    _newline: (_) => /\r?\n/,

    comment: (_) => token(seq('#', /[^\r\n]*/)),

    _statement: ($) => choice($.let_statement, $.pipeline),

    let_statement: ($) =>
      seq(
        'let',
        field('name', $.identifier),
        '=',
        optional($._newlines),
        field('value', $.pipeline),
      ),

    pipeline: ($) =>
      seq(
        choice($.stage, $._reference_head),
        repeat($._continuation),
      ),

    _stage_pipeline: ($) => seq($.stage, repeat($._continuation)),

    _reference_head: ($) =>
      seq($.reference, repeat(seq(token.immediate('.'), $._field_name))),

    _continuation: ($) =>
      seq($.pipe, optional($._newlines), $.stage),

    pipe: (_) => token(/(?:\r?\n[ \t]*(?:#[^\r\n]*)?)*\|>/),

    stage: ($) =>
      seq(
        field('command', $.command_name),
        repeat(field('argument', $._argument)),
      ),

    command_name: ($) =>
      seq($.identifier, repeat(seq(token.immediate('.'), $._field_name))),

    _argument: ($) => choice($.named_argument, $._expression),

    named_argument: ($) =>
      seq(
        field('name', alias($._label, $.argument_name)),
        field('value', $._expression),
      ),

    _expression: ($) =>
      choice(
        $.binary_expression,
        $.not_expression,
        $.field_expression,
        $.call_expression,
        $.subpipeline,
        $.parenthesized_expression,
        $.list,
        $.map,
        $.range,
        $.string,
        $.raw_string,
        $.regex,
        $.integer,
        $.number,
        $.size,
        $.duration,
        $.boolean,
        $.null,
        $.reference,
        $.identifier,
      ),

    binary_expression: ($) => {
      const table = [
        [PREC.or, 'or'],
        [PREC.and, 'and'],
        [PREC.compare, choice('==', '!=', '<=', '>=', '<', '>', '!~', '~', 'in')],
        [PREC.additive, choice('+', '-')],
        [PREC.multiplicative, choice('*', '/')],
      ];
      return choice(
        ...table.map(([precedence, operator]) =>
          prec.left(
            precedence,
            seq(
              field('left', $._expression),
              field('operator', operator),
              field('right', $._expression),
            ),
          ),
        ),
      );
    },

    not_expression: ($) => prec(PREC.not, seq('not', $._expression)),

    field_expression: ($) =>
      prec.left(
        PREC.field,
        seq(
          field('object', $._expression),
          token.immediate('.'),
          field('field', $._field_name),
        ),
      ),

    call_expression: ($) =>
      delimited($, field('function', $.function_name), $._expression, ')'),

    function_name: (_) => token(/[a-z][a-z0-9_]*\(/),

    subpipeline: ($) =>
      prec.dynamic(
        1,
        seq(
          '(',
          optional($._newlines),
          alias($._stage_pipeline, $.pipeline),
          optional($._newlines),
          ')',
        ),
      ),

    parenthesized_expression: ($) =>
      seq('(', optional($._newlines), $._expression, optional($._newlines), ')'),

    list: ($) =>
      delimited($, '[', choice($._expression, $.spread_element), ']'),

    spread_element: ($) => seq('...', $._expression),

    map: ($) => delimited($, '{', $.map_entry, '}'),

    map_entry: ($) =>
      choice(
        seq(
          field('key', alias($._label, $.map_key)),
          field('value', $._expression),
        ),
        seq(field('key', $.string), ':', field('value', $._expression)),
      ),

    range: ($) =>
      choice(
        seq($.integer, '..', optional($.integer)),
        seq('..', $.integer),
      ),

    string: (_) => token(/"(?:[^"\\]|\\[\s\S])*"/),

    raw_string: (_) =>
      token(
        choice(
          ...Array.from({ length: MAX_RAW_STRING_HASHES + 1 }, (_value, hashes) =>
            rawString(hashes),
          ),
        ),
      ),

    regex: ($) => seq($.regex_pattern, optional($.regex_flags)),

    regex_pattern: (_) =>
      token(/\/(?:[^\s\/\\]|\\[^\r\n])(?:[^\r\n\/\\]|\\[^\r\n])*\//),

    regex_flags: (_) => token.immediate(/[imsx]+/),

    integer: (_) => token(/-?(?:0x[0-9A-Fa-f_]+|[0-9][0-9_]*)/),

    number: (_) => token(/-?[0-9][0-9_]*\.[0-9][0-9_]*/),

    size: (_) =>
      token(
        /[0-9][0-9_]*(?:\.[0-9][0-9_]*)?(?:kib|mib|gib|tib|kb|mb|gb|tb|b)/,
      ),

    duration: (_) => token(/(?:[0-9][0-9_]*(?:ms|s|m|h|d|w))+/),

    boolean: (_) => choice('true', 'false'),

    null: (_) => 'null',

    reference: (_) => token(seq('@', IDENTIFIER)),

    identifier: (_) => IDENTIFIER,

    _field_name: ($) => alias($._immediate_identifier, $.identifier),

    _immediate_identifier: (_) => token.immediate(IDENTIFIER),

    _label: (_) => token(seq(IDENTIFIER, ':')),
  },
});
