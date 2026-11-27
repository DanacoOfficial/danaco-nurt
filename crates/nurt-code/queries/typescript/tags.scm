; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; https://github.com/DanacoOfficial | support@danaco-group.pl
;
; Module:           crates/nurt-code/queries/typescript/tags.scm
; Responsibility:   Query of the symbols defined in TypeScript sources.
; Dependencies:     none
; Author:           Danaco Official
; Version:          0.1.1, 2026-11-27
; License:          see the LICENSE file or https://www.danaco-group.pl

(function_declaration name: (identifier) @name) @definition.function
(generator_function_declaration name: (identifier) @name) @definition.function
(function_signature name: (identifier) @name) @definition.function
(class_declaration name: (type_identifier) @name) @definition.class
(abstract_class_declaration name: (type_identifier) @name) @definition.class
(interface_declaration name: (type_identifier) @name) @definition.interface
(enum_declaration name: (identifier) @name) @definition.enum
(enum_body [(property_identifier) @name (enum_assignment name: (property_identifier) @name)] @definition.constant)
(type_alias_declaration name: (type_identifier) @name) @definition.type
(internal_module name: [(identifier) (nested_identifier)] @name) @definition.module
(method_definition name: [(property_identifier) (private_property_identifier)] @name) @definition.method
(method_signature name: (property_identifier) @name) @definition.method
(abstract_method_signature name: (property_identifier) @name) @definition.method
(public_field_definition name: [(property_identifier) (private_property_identifier)] @name) @definition.field
(property_signature name: (property_identifier) @name) @definition.field

(program
  (lexical_declaration
    (variable_declarator
      name: (identifier) @name
      value: [(arrow_function) (function_expression)]) @definition.function))
(program
  (export_statement
    (lexical_declaration
      (variable_declarator
        name: (identifier) @name
        value: [(arrow_function) (function_expression)]) @definition.function)))
(program (lexical_declaration (variable_declarator name: (identifier) @name) @definition.variable))
(program (variable_declaration (variable_declarator name: (identifier) @name) @definition.variable))
(program (export_statement (lexical_declaration (variable_declarator name: (identifier) @name) @definition.variable)))
(program (export_statement (variable_declaration (variable_declarator name: (identifier) @name) @definition.variable)))

; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; Support: support@danaco-group.pl
