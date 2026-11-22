(namespace_definition name: (namespace_name) @name) @definition.module
(class_declaration name: (name) @name) @definition.class
(interface_declaration name: (name) @name) @definition.interface
(trait_declaration name: (name) @name) @definition.interface
(enum_declaration name: (name) @name) @definition.enum
(enum_case name: (name) @name) @definition.constant
(function_definition name: (name) @name) @definition.function
(method_declaration name: (name) @name) @definition.method
(property_declaration (property_element name: (variable_name (name) @name))) @definition.field
(const_declaration (const_element (name) @name)) @definition.constant
