; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; https://github.com/DanacoOfficial | support@danaco-group.pl
;
; Module:           crates/nurt-code/queries/php/locals.scm
; Responsibility:   Query of the scopes, definitions and references of local names in PHP sources.
; Dependencies:     none
; Author:           Danaco Official
; Version:          0.1.1, 2026-11-27
; License:          see the LICENSE file or https://www.danaco-group.pl

[
  (program)
  (class_declaration)
  (interface_declaration)
  (trait_declaration)
  (function_definition)
  (method_declaration)
  (anonymous_function)
  (arrow_function)
] @local.scope

(simple_parameter name: (variable_name (name) @local.definition))
(variadic_parameter name: (variable_name (name) @local.definition))
(assignment_expression left: (variable_name (name) @local.definition))
(foreach_statement (_) (variable_name (name) @local.definition))
(foreach_statement (_) (pair (variable_name (name) @local.definition)))

(namespace_use_clause alias: (name) @local.import)
(namespace_use_clause (qualified_name (name) @local.import))
(namespace_use_clause (name) @local.import)

; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; Support: support@danaco-group.pl
