// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
#include "tree_sitter/parser.h"

#include <string.h>

/*
 * External scanner of the Nurt grammar, variant C.
 *
 * It reads a keyword used as an argument name, as in `replace "a" "b" in: [f]`,
 * so that the name is not taken for an operator. A keyword name is a keyword
 * that the separator `=` follows directly.
 */

enum TokenType {
  KEYWORD_NAME,
};

#define SEPARATOR '='
#define LONGEST_KEYWORD 5

static const char *const KEYWORDS[] = {"let", "and", "or", "not", "in", "true", "false", "null"};

void *tree_sitter_nurt_c_external_scanner_create(void) { return NULL; }

void tree_sitter_nurt_c_external_scanner_destroy(void *payload) { (void)payload; }

unsigned tree_sitter_nurt_c_external_scanner_serialize(void *payload, char *buffer) {
  (void)payload;
  (void)buffer;
  return 0;
}

void tree_sitter_nurt_c_external_scanner_deserialize(void *payload, const char *buffer, unsigned length) {
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
  if (SEPARATOR == '=') {
    lexer->advance(lexer, false);
    if (lexer->lookahead == '=') {
      return false;
    }
  }
  lexer->result_symbol = KEYWORD_NAME;
  return true;
}

bool tree_sitter_nurt_c_external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid_symbols) {
  (void)payload;
  if (!valid_symbols[KEYWORD_NAME]) {
    return false;
  }
  while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
    lexer->advance(lexer, true);
  }
  return scan_keyword_name(lexer);
}
