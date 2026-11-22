(struct_item name: (type_identifier) @name) @definition.struct
(union_item name: (type_identifier) @name) @definition.struct
(enum_item name: (type_identifier) @name) @definition.enum
(enum_variant name: (identifier) @name) @definition.constant
(type_item name: (type_identifier) @name) @definition.type
(associated_type name: (type_identifier) @name) @definition.type
(trait_item name: (type_identifier) @name) @definition.interface
(function_item name: (identifier) @name) @definition.function
(function_signature_item name: (identifier) @name) @definition.function
(mod_item name: (identifier) @name) @definition.module
(macro_definition name: (identifier) @name) @definition.macro
(const_item name: (identifier) @name) @definition.constant
(static_item name: (identifier) @name) @definition.variable
(field_declaration name: (field_identifier) @name) @definition.field

(impl_item
  type: [
    (type_identifier) @name
    (generic_type type: (type_identifier) @name)
    (scoped_type_identifier name: (type_identifier) @name)
  ]) @scope
