(function_statement (function_name) @name) @definition.function
(class_statement . (simple_name) @name) @definition.class
(class_method_definition (simple_name) @name) @definition.method
(class_property_definition (variable) @name) @definition.field
(enum_statement . (simple_name) @name) @definition.enum
(enum_member (simple_name) @name) @definition.constant
