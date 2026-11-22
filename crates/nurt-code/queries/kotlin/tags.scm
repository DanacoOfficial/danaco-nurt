(class_declaration "interface" name: (identifier) @name) @definition.interface
((class_declaration (modifiers (class_modifier) @_modifier) name: (identifier) @name) @definition.enum
  (#eq? @_modifier "enum"))
(class_declaration name: (identifier) @name) @definition.class
(object_declaration name: (identifier) @name) @definition.class
(companion_object name: (identifier) @name) @definition.class
(function_declaration name: (identifier) @name) @definition.function
(property_declaration (variable_declaration (identifier) @name)) @definition.variable
(type_alias type: (identifier) @name) @definition.type
(enum_entry (identifier) @name) @definition.constant
