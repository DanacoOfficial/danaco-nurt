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
