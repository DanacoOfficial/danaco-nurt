(class_declaration name: (identifier) @name) @definition.class
(record_declaration name: (identifier) @name) @definition.class
(struct_declaration name: (identifier) @name) @definition.struct
(interface_declaration name: (identifier) @name) @definition.interface
(enum_declaration name: (identifier) @name) @definition.enum
(enum_member_declaration name: (identifier) @name) @definition.constant
(delegate_declaration name: (identifier) @name) @definition.type
(method_declaration name: (identifier) @name) @definition.method
(constructor_declaration name: (identifier) @name) @definition.method
(local_function_statement name: (identifier) @name) @definition.function
(property_declaration name: (identifier) @name) @definition.field
(namespace_declaration name: (_) @name) @definition.module
(file_scoped_namespace_declaration name: (_) @name) @definition.module

((field_declaration
   (modifier) @_modifier
   (variable_declaration (variable_declarator name: (identifier) @name))) @definition.constant
  (#eq? @_modifier "const"))
(field_declaration (variable_declaration (variable_declarator name: (identifier) @name))) @definition.field
(event_field_declaration (variable_declaration (variable_declarator name: (identifier) @name))) @definition.field
