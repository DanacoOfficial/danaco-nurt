[
  (translation_unit)
  (function_definition)
  (compound_statement)
  (for_statement)
  (while_statement)
  (if_statement)
] @local.scope

(parameter_declaration declarator: (identifier) @local.definition)
(parameter_declaration declarator: (pointer_declarator declarator: (identifier) @local.definition))
(parameter_declaration declarator: (array_declarator declarator: (identifier) @local.definition))
(declaration declarator: (identifier) @local.definition)
(declaration declarator: (pointer_declarator declarator: (identifier) @local.definition))
(declaration declarator: (array_declarator declarator: (identifier) @local.definition))
(declaration declarator: (init_declarator declarator: (identifier) @local.definition))
(declaration declarator: (init_declarator declarator: (pointer_declarator declarator: (identifier) @local.definition)))
(declaration declarator: (init_declarator declarator: (array_declarator declarator: (identifier) @local.definition)))
