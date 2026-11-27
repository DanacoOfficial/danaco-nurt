; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; https://github.com/DanacoOfficial | support@danaco-group.pl
;
; Module:           crates/nurt-code/queries/go/locals.scm
; Responsibility:   Query of the scopes, definitions and references of local names in Go sources.
; Dependencies:     none
; Author:           Danaco Official
; Version:          0.1.1, 2026-11-27
; License:          see the LICENSE file or https://www.danaco-group.pl

[
  (source_file)
  (function_declaration)
  (method_declaration)
  (func_literal)
  (block)
  (if_statement)
  (for_statement)
  (expression_switch_statement)
  (type_switch_statement)
  (select_statement)
  (expression_case)
  (type_case)
  (communication_case)
] @local.scope

(parameter_declaration name: (identifier) @local.definition)
(variadic_parameter_declaration name: (identifier) @local.definition)
(short_var_declaration left: (expression_list (identifier) @local.definition))
(var_spec name: (identifier) @local.definition)
(const_spec name: (identifier) @local.definition)
(range_clause left: (expression_list (identifier) @local.definition))
(type_switch_statement alias: (expression_list (identifier) @local.definition))
(receive_statement left: (expression_list (identifier) @local.definition))

(import_spec name: (package_identifier) @local.import)

; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; Support: support@danaco-group.pl
