; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; https://github.com/DanacoOfficial | support@danaco-group.pl
;
; Module:           crates/nurt-code/queries/c/tags.scm
; Responsibility:   Query of the symbols defined in C sources.
; Dependencies:     none
; Author:           Danaco Official
; Version:          0.1.1, 2026-11-27
; License:          see the LICENSE file or https://www.danaco-group.pl

(function_definition declarator: (function_declarator declarator: (identifier) @name)) @definition.function
(function_definition
  declarator: (pointer_declarator declarator: (function_declarator declarator: (identifier) @name))) @definition.function
(declaration declarator: (function_declarator declarator: (identifier) @name)) @definition.function
(declaration
  declarator: (pointer_declarator declarator: (function_declarator declarator: (identifier) @name))) @definition.function

(struct_specifier name: (type_identifier) @name body: (_)) @definition.struct
(union_specifier name: (type_identifier) @name body: (_)) @definition.struct
(enum_specifier name: (type_identifier) @name body: (_)) @definition.enum
(enumerator name: (identifier) @name) @definition.constant
(type_definition declarator: (type_identifier) @name) @definition.type
(type_definition declarator: (pointer_declarator declarator: (type_identifier) @name)) @definition.type
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
      (array_declarator declarator: (identifier) @name)
      (init_declarator declarator: (identifier) @name)
      (init_declarator declarator: (pointer_declarator declarator: (identifier) @name))
      (init_declarator declarator: (array_declarator declarator: (identifier) @name))
    ]) @definition.variable)

; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; Support: support@danaco-group.pl
