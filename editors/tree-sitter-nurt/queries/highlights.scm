; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; https://github.com/DanacoOfficial | support@danaco-group.pl
;
; Module:           editors/tree-sitter-nurt/queries/highlights.scm
; Responsibility:   Highlight query of the Nurt grammar for tree-sitter editors.
; Dependencies:     none
; Author:           Danaco Official
; Version:          0.1.1, 2026-11-27
; License:          see the LICENSE file or https://www.danaco-group.pl

(comment) @comment

(identifier) @variable
(reference) @variable.special

(field_expression
  field: (identifier) @property)

(command_name
  (identifier) @function)

(function_name) @function.call

(argument_name
  (identifier) @variable.parameter)
(map_key
  (identifier) @property)

(let_statement
  name: (identifier) @variable)

[
  (string)
  (raw_string)
] @string

(regex_pattern) @string.regexp
(regex_flags) @string.special

[
  (integer)
  (number)
  (size)
  (duration)
] @number

(boolean) @boolean
(null) @constant.builtin

"let" @keyword

[
  "and"
  "or"
  "not"
  "in"
] @keyword.operator

[
  (pipe)
  "=="
  "!="
  "<"
  "<="
  ">"
  ">="
  "~"
  "!~"
  "+"
  "-"
  "*"
  "/"
  "="
  ".."
  "..."
] @operator

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

[
  ","
  ":"
] @punctuation.delimiter

; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; Support: support@danaco-group.pl
