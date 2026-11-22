(function_declaration name: (identifier) @name) @definition.function

(method_declaration
  receiver: (parameter_list
    (parameter_declaration
      type: [
        (type_identifier) @container.name
        (pointer_type (type_identifier) @container.name)
        (generic_type type: (type_identifier) @container.name)
        (pointer_type (generic_type type: (type_identifier) @container.name))
      ]))
  name: (field_identifier) @name) @definition.method

(method_declaration name: (field_identifier) @name) @definition.method

(type_spec name: (type_identifier) @name type: (struct_type)) @definition.struct
(type_spec name: (type_identifier) @name type: (interface_type)) @definition.interface
(type_spec name: (type_identifier) @name) @definition.type
(type_alias name: (type_identifier) @name) @definition.type

(method_elem name: (field_identifier) @name) @definition.method
(field_declaration name: (field_identifier) @name) @definition.field

(source_file (const_declaration (const_spec name: (identifier) @name) @definition.constant))
(source_file (var_declaration (var_spec name: (identifier) @name) @definition.variable))
(source_file (var_declaration (var_spec_list (var_spec name: (identifier) @name) @definition.variable)))
