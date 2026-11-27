; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; https://github.com/DanacoOfficial | support@danaco-group.pl
;
; Module:           crates/nurt-code/queries/csharp/locals.scm
; Responsibility:   Query of the scopes, definitions and references of local names in C# sources.
; Dependencies:     none
; Author:           Danaco Official
; Version:          0.1.1, 2026-11-27
; License:          see the LICENSE file or https://www.danaco-group.pl

[
  (compilation_unit)
  (class_declaration)
  (struct_declaration)
  (record_declaration)
  (interface_declaration)
  (method_declaration)
  (constructor_declaration)
  (local_function_statement)
  (lambda_expression)
  (block)
  (for_statement)
  (foreach_statement)
  (catch_clause)
  (using_statement)
] @local.scope

(parameter name: (identifier) @local.definition)
(variable_declarator name: (identifier) @local.definition)
(foreach_statement left: (identifier) @local.definition)
(catch_declaration name: (identifier) @local.definition)

(using_directive name: (identifier) @local.import)
(using_directive (qualified_name name: (identifier) @local.import))
(using_directive (identifier) @local.import)

; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; Support: support@danaco-group.pl
