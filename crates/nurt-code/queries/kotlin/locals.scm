[
  (source_file)
  (class_declaration)
  (object_declaration)
  (function_declaration)
  (function_body)
  (lambda_literal)
  (for_statement)
] @local.scope

(parameter (identifier) @local.definition)
(class_parameter (identifier) @local.definition)
(variable_declaration (identifier) @local.definition)

(import (identifier) @local.import)
(import (qualified_identifier (identifier) @local.import .))
