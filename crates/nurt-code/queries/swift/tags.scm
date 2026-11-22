(class_declaration declaration_kind: "class" name: (type_identifier) @name) @definition.class
(class_declaration declaration_kind: "actor" name: (type_identifier) @name) @definition.class
(class_declaration declaration_kind: "struct" name: (type_identifier) @name) @definition.struct
(class_declaration declaration_kind: "enum" name: (type_identifier) @name) @definition.enum
(class_declaration declaration_kind: "extension" name: (user_type (type_identifier) @name)) @scope
(protocol_declaration name: (type_identifier) @name) @definition.interface
(function_declaration name: (simple_identifier) @name) @definition.function
(protocol_function_declaration name: (simple_identifier) @name) @definition.method
(init_declaration "init" @name) @definition.method
(typealias_declaration name: (type_identifier) @name) @definition.type
(enum_entry name: (simple_identifier) @name) @definition.constant
(property_declaration
  (value_binding_pattern mutability: "let")
  name: (pattern bound_identifier: (simple_identifier) @name)) @definition.constant
(property_declaration name: (pattern bound_identifier: (simple_identifier) @name)) @definition.variable
(protocol_property_declaration name: (pattern bound_identifier: (simple_identifier) @name)) @definition.field
