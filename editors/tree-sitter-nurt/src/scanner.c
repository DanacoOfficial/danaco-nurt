// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           editors/tree-sitter-nurt/src/scanner.c
// Responsibility:   Source file editors/tree-sitter-nurt/src/scanner.c.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

#include "tree_sitter/parser.h"

#include <string.h>

/*
 * External scanner of the Nurt grammar, variant A.
 *
 * It reads a keyword used as an argument name, as in `replace "a" "b" in: [f]`,
 * so that the name is not taken for an operator. A keyword name is a keyword
 * that the separator `:` follows directly.
 * A `-` that follows white space and precedes a non-blank character starts a
 * new argument, as in `first 20 -1`; it is read here as well.
 */

enum TokenType {
  KEYWORD_NAME,
  ARGUMENT_MINUS,
};

#define SEPARATOR ':'
#define LONGEST_KEYWORD 5

static const char *const KEYWORDS[] = {"let", "and", "or", "not", "in", "true", "false", "null"};

void *tree_sitter_nurt_external_scanner_create(void) { return NULL; }

void tree_sitter_nurt_external_scanner_destroy(void *payload) { (void)payload; }

unsigned tree_sitter_nurt_external_scanner_serialize(void *payload, char *buffer) {
  (void)payload;
  (void)buffer;
  return 0;
}

void tree_sitter_nurt_external_scanner_deserialize(void *payload, const char *buffer, unsigned length) {
  (void)payload;
  (void)buffer;
  (void)length;
}

static bool is_keyword(const char *word) {
  for (unsigned index = 0; index < sizeof KEYWORDS / sizeof KEYWORDS[0]; index += 1) {
    if (strcmp(word, KEYWORDS[index]) == 0) {
      return true;
    }
  }
  return false;
}

static bool is_name_character(int32_t character) {
  return (character >= 'a' && character <= 'z') || (character >= '0' && character <= '9') ||
         character == '_';
}

static bool scan_keyword_name(TSLexer *lexer) {
  char word[LONGEST_KEYWORD + 2] = {0};
  unsigned length = 0;
  if (lexer->lookahead < 'a' || lexer->lookahead > 'z') {
    return false;
  }
  while (is_name_character(lexer->lookahead)) {
    if (length > LONGEST_KEYWORD) {
      return false;
    }
    word[length] = (char)lexer->lookahead;
    length += 1;
    lexer->advance(lexer, false);
  }
  lexer->mark_end(lexer);
  if (lexer->lookahead != SEPARATOR || !is_keyword(word)) {
    return false;
  }
  lexer->result_symbol = KEYWORD_NAME;
  return true;
}

static bool is_blank(int32_t character) {
  return character == ' ' || character == '\t' || character == '\r' || character == '\n' ||
         character == '#' || character == 0;
}

static bool scan_argument_minus(TSLexer *lexer) {
  lexer->advance(lexer, false);
  lexer->mark_end(lexer);
  if (is_blank(lexer->lookahead)) {
    return false;
  }
  lexer->result_symbol = ARGUMENT_MINUS;
  return true;
}

bool tree_sitter_nurt_external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid_symbols) {
  (void)payload;
  bool after_space = false;
  if (!valid_symbols[KEYWORD_NAME] && !valid_symbols[ARGUMENT_MINUS]) {
    return false;
  }
  while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
    lexer->advance(lexer, true);
    after_space = true;
  }
  if (valid_symbols[ARGUMENT_MINUS] && after_space && lexer->lookahead == '-') {
    return scan_argument_minus(lexer);
  }
  return valid_symbols[KEYWORD_NAME] && scan_keyword_name(lexer);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
