; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; https://github.com/DanacoOfficial | support@danaco-group.pl
;
; Module:           crates/nurt-code/queries/swift/tags.scm
; Responsibility:   Query of the symbols defined in Swift sources.
; Dependencies:     none
; Author:           Danaco Official
; Version:          0.1.1, 2026-11-27
; License:          see the LICENSE file or https://www.danaco-group.pl

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

; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; Support: support@danaco-group.pl
