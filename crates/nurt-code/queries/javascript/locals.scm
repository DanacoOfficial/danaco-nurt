; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; https://github.com/DanacoOfficial | support@danaco-group.pl
;
; Module:           crates/nurt-code/queries/javascript/locals.scm
; Responsibility:   Query of the scopes, definitions and references of local names in JavaScript sources.
; Dependencies:     none
; Author:           Danaco Official
; Version:          0.1.1, 2026-11-27
; License:          see the LICENSE file or https://www.danaco-group.pl

[
  (program)
  (statement_block)
  (function_declaration)
  (generator_function_declaration)
  (function_expression)
  (arrow_function)
  (method_definition)
  (class_declaration)
  (for_statement)
  (for_in_statement)
  (catch_clause)
] @local.scope

(formal_parameters (identifier) @local.definition)
(formal_parameters (assignment_pattern left: (identifier) @local.definition))
(formal_parameters (rest_pattern (identifier) @local.definition))
(arrow_function parameter: (identifier) @local.definition)
(variable_declarator name: (identifier) @local.definition)
(variable_declarator name: (object_pattern (shorthand_property_identifier_pattern) @local.definition))
(variable_declarator name: (array_pattern (identifier) @local.definition))
(for_in_statement left: (identifier) @local.definition)
(catch_clause parameter: (identifier) @local.definition)

(import_specifier alias: (identifier) @local.import)
(import_specifier !alias name: (identifier) @local.import)
(import_clause (identifier) @local.import)
(namespace_import (identifier) @local.import)

; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; Support: support@danaco-group.pl
