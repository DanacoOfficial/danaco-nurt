(class_definition name: (identifier) @name) @definition.class
(function_definition name: (identifier) @name) @definition.function
(type_alias_statement left: (type (identifier) @name)) @definition.type

(module (expression_statement (assignment left: (identifier) @name) @definition.variable))
(class_definition
  body: (block (expression_statement (assignment left: (identifier) @name) @definition.field)))
