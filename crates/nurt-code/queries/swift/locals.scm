[
  (source_file)
  (class_declaration)
  (protocol_declaration)
  (function_declaration)
  (init_declaration)
  (function_body)
  (lambda_literal)
  (for_statement)
] @local.scope

(parameter name: (simple_identifier) @local.definition)
(property_declaration name: (pattern bound_identifier: (simple_identifier) @local.definition))
(for_statement (pattern bound_identifier: (simple_identifier) @local.definition))

(import_declaration (identifier (simple_identifier) @local.import .))
