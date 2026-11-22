(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier scope: (_) @container.name name: (identifier) @name))) @definition.method
(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier scope: (_) @container.name name: (destructor_name) @name))) @definition.method
(function_definition declarator: (function_declarator declarator: (field_identifier) @name)) @definition.method
(function_definition declarator: (function_declarator declarator: (destructor_name) @name)) @definition.method
(function_definition declarator: (function_declarator declarator: (identifier) @name)) @definition.function
(function_definition
  declarator: (pointer_declarator declarator: (function_declarator declarator: (identifier) @name))) @definition.function
(function_definition
  declarator: (reference_declarator (function_declarator declarator: (identifier) @name))) @definition.function
(declaration declarator: (function_declarator declarator: (identifier) @name)) @definition.function
(field_declaration declarator: (function_declarator declarator: (field_identifier) @name)) @definition.method

(class_specifier name: (type_identifier) @name body: (_)) @definition.class
(struct_specifier name: (type_identifier) @name body: (_)) @definition.struct
(union_specifier name: (type_identifier) @name body: (_)) @definition.struct
(enum_specifier name: (type_identifier) @name body: (_)) @definition.enum
(enumerator name: (identifier) @name) @definition.constant
(namespace_definition name: [(namespace_identifier) (nested_namespace_specifier)] @name) @definition.module
(alias_declaration name: (type_identifier) @name) @definition.type
(type_definition declarator: (type_identifier) @name) @definition.type
(preproc_def name: (identifier) @name) @definition.macro
(preproc_function_def name: (identifier) @name) @definition.macro

(field_declaration declarator: (field_identifier) @name) @definition.field
(field_declaration declarator: (pointer_declarator declarator: (field_identifier) @name)) @definition.field
(field_declaration declarator: (array_declarator declarator: (field_identifier) @name)) @definition.field

(translation_unit
  (declaration
    declarator: [
      (identifier) @name
      (pointer_declarator declarator: (identifier) @name)
      (init_declarator declarator: (identifier) @name)
      (init_declarator declarator: (pointer_declarator declarator: (identifier) @name))
    ]) @definition.variable)
