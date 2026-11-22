[
  (program)
  (class_declaration)
  (interface_declaration)
  (trait_declaration)
  (function_definition)
  (method_declaration)
  (anonymous_function)
  (arrow_function)
] @local.scope

(simple_parameter name: (variable_name (name) @local.definition))
(variadic_parameter name: (variable_name (name) @local.definition))
(assignment_expression left: (variable_name (name) @local.definition))
(foreach_statement (_) (variable_name (name) @local.definition))
(foreach_statement (_) (pair (variable_name (name) @local.definition)))

(namespace_use_clause alias: (name) @local.import)
(namespace_use_clause (qualified_name (name) @local.import))
(namespace_use_clause (name) @local.import)
