// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
'use strict';

/**
 * Shared definition of the Nurt tree-sitter grammars.
 *
 * Variants differ only in stage arguments: `a` uses positional arguments and
 * `name: value` separated by white space, `b` uses
 * `command(positional, name: value)`, `c` uses only `name=value`. All
 * variants produce the same node types, so one set of queries serves all.
 * Argument positions, types and defaults belong to the binding stage.
 *
 * A newline ends a statement unless the next non-empty line starts with
 * `|>`; the `pipe` token absorbs the newlines and comment lines before it.
 * Inside brackets a newline may follow the opening bracket, each element
 * and each comma.
 *
 * Lexical rules, with the token boundaries of the lexer in `nurt-syntax`:
 * - A label is a name directly followed by `:` (variant C arguments: `=`).
 *   The name and the separator are separate leaves.
 * - A call is a name directly followed by `(`.
 * - A numeric literal has no sign. `-` is the unary negation or the binary
 *   subtraction, so `-1` is a `-` leaf and an `integer` leaf.
 * - In variant A a `-` that follows white space and precedes a
 *   non-blank character starts a new argument (`first 20 -1` has two
 *   arguments); the external scanner reports it. Elsewhere `-` after an
 *   operand is the subtraction (`a-1`, `a - 1`).
 * - `/` directly after an operand is the division (`a/b`). It starts a
 *   regular expression where an operand is expected and the next character
 *   is not white space.
 * - A keyword directly followed by the label separator is a name, as in
 *   `in: [path]`; the external scanner reads it.
 * - `|>` is a leaf of its own; the line breaks before it are separate.
 */

const PREC = {
  or: 1,
  and: 2,
  not: 3,
  compare: 4,
  additive: 5,
  multiplicative: 6,
  unary: 7,
  range: 8,
  field: 9,
};

// Lexical precedence of tokens that must win over a longer token starting
// at the same character, such as `/` in `a/b` over a regular expression.
const GLUED = 1;

const IDENTIFIER = /[a-z][a-z0-9_]*/;
const MAX_RAW_STRING_HASHES = 3;
const VARIANTS = ['a', 'b', 'c'];

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

function stageRules(variant) {
  if (variant === 'b') {
    return {
      stage: ($) =>
        seq(
          field('command', $.command_name),
          delimited($, '(', field('argument', $._argument), ')'),
        ),

      _argument: ($) => choice($.named_argument, $._argument_value),

      named_argument: ($) =>
        seq(
          field('name', $.argument_name),
          field('value', $._argument_value),
        ),

      _argument_value: ($) => choice($.subpipeline, $._expression),

      // A command call in argument position is a sub-pipeline only when a
      // `|>` follows it or when parentheses enclose it; otherwise it is a
      // function call.
      subpipeline: ($) =>
        prec.dynamic(
          1,
          choice(
            alias($._argument_pipeline, $.pipeline),
            seq(
              '(',
              optional($._newlines),
              alias($._stage_pipeline, $.pipeline),
              optional($._newlines),
              ')',
            ),
          ),
        ),

      _argument_pipeline: ($) => seq($.stage, repeat1($._continuation)),

      _stage_pipeline: ($) => seq($.stage, repeat($._continuation)),

      _call_open: (_) => '(',
    };
  }
  return {
    stage: ($) =>
      seq(
        field('command', $.command_name),
        repeat(field('argument', $._argument)),
      ),

    _argument: ($) =>
      variant === 'c' ? $.named_argument : choice($.named_argument, $._expression),

    named_argument: ($) =>
      seq(
        field('name', $.argument_name),
        field('value', $._expression),
      ),

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

    _stage_pipeline: ($) => seq($.stage, repeat($._continuation)),

    _call_open: (_) => token.immediate(prec(GLUED, '(')),
  };
}

function expressionChoices($, variant) {
  const choices = [
    $.binary_expression,
    $.not_expression,
    $.unary_expression,
    $.field_expression,
    $.call_expression,
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
  ];
  if (variant !== 'b') {
    choices.splice(5, 0, $.subpipeline);
  }
  return choices;
}

function conflictsOf(variant) {
  if (variant === 'b') {
    return ($) => [
      [$.command_name, $._expression],
      [$.command_name, $.call_expression],
      [$.pipeline],
      [$._stage_pipeline],
      [$._argument_pipeline],
    ];
  }
  return ($) => [[$.command_name, $._expression], [$.pipeline], [$._stage_pipeline]];
}

module.exports = function defineGrammar(variant) {
  if (!VARIANTS.includes(variant)) {
    throw new Error(`unknown syntax variant ${variant}; expected one of ${VARIANTS.join(', ')}`);
  }

  const variantRules = stageRules(variant);
  const labelSeparator = variant === 'c' ? '=' : ':';

  return grammar({
    name: variant === 'a' ? 'nurt' : `nurt_${variant}`,

    extras: ($) => [/[ \t]+/, /\uFEFF/, $.comment],

    externals: ($) => (variant === 'a' ? [$._keyword_name, $._argument_minus] : [$._keyword_name]),

    conflicts: conflictsOf(variant),

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
        seq(choice($.stage, $._reference_head), repeat($._continuation)),

      _reference_head: ($) =>
        seq($.reference, repeat(seq(token.immediate('.'), $._field_name))),

      _continuation: ($) =>
        seq(optional($._newlines), $.pipe, optional($._newlines), $.stage),

      pipe: (_) => '|>',

      ...variantRules,

      command_name: ($) =>
        seq($.identifier, repeat(seq(token.immediate('.'), $._field_name))),

      _expression: ($) => choice(...expressionChoices($, variant)),

      binary_expression: ($) => {
        const table = [
          [PREC.or, 'or'],
          [PREC.and, 'and'],
          [PREC.compare, choice('==', '!=', '<=', '>=', '<', '>', '!~', '~', 'in')],
          [PREC.additive, choice('+', '-')],
          [PREC.multiplicative, choice('*', '/', alias($._glued_slash, '/'))],
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

      _glued_slash: (_) => token.immediate(prec(GLUED, '/')),

      not_expression: ($) => prec(PREC.not, seq('not', $._expression)),

      unary_expression: ($) =>
        prec(
          PREC.unary,
          seq(
            field(
              'operator',
              variant === 'a' ? choice('-', alias($._argument_minus, '-')) : '-',
            ),
            field('operand', $._expression),
          ),
        ),

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
        seq(
          field('function', alias($.identifier, $.function_name)),
          delimited($, $._call_open, $._expression, ')'),
        ),

      parenthesized_expression: ($) =>
        seq('(', optional($._newlines), $._expression, optional($._newlines), ')'),

      list: ($) =>
        delimited($, '[', choice($._expression, $.spread_element), ']'),

      spread_element: ($) => seq('...', $._expression),

      map: ($) => delimited($, '{', $.map_entry, '}'),

      map_key: ($) => seq($.identifier, token.immediate(':')),

      map_entry: ($) =>
        choice(
          seq(field('key', $.map_key), field('value', $._expression)),
          seq(field('key', $.string), ':', field('value', $._expression)),
        ),

      range: ($) =>
        prec.right(
          PREC.range,
          choice(seq($.integer, '..', optional($.integer)), seq('..', $.integer)),
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

      regex_flags: (_) => token.immediate(prec(GLUED, /[imsx]+/)),

      integer: (_) => token(/(?:0x[0-9A-Fa-f_]+|[0-9][0-9_]*)/),

      number: (_) => token(/[0-9][0-9_]*\.[0-9][0-9_]*/),

      size: (_) =>
        token(/[0-9][0-9_]*(?:\.[0-9][0-9_]*)?(?:kib|mib|gib|tib|kb|mb|gb|tb|b)/),

      duration: (_) => token(/(?:[0-9][0-9_]*(?:ms|s|m|h|d|w))+/),

      boolean: (_) => choice('true', 'false'),

      null: (_) => 'null',

      reference: (_) => token(seq('@', IDENTIFIER)),

      identifier: (_) => IDENTIFIER,

      _field_name: ($) => alias($._immediate_identifier, $.identifier),

      _immediate_identifier: (_) => token.immediate(IDENTIFIER),

      argument_name: ($) =>
        seq(
          choice($.identifier, alias($._keyword_name, $.identifier)),
          token.immediate(labelSeparator),
        ),
    },
  });
};
