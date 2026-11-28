; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; https://github.com/DanacoOfficial | support@danaco-group.pl
;
; Module:           crates/nurt-code/queries/python/locals.scm
; Responsibility:   Query of the scopes, definitions and references of local names in Python sources.
; Dependencies:     none
; Author:           Danaco Official
; Version:          0.1.1, 2026-11-27
; License:          see the LICENSE file or https://www.danaco-group.pl

[
  (module)
  (class_definition)
  (function_definition)
  (lambda)
  (list_comprehension)
  (dictionary_comprehension)
  (set_comprehension)
  (generator_expression)
] @local.scope

(parameters (identifier) @local.definition)
(default_parameter name: (identifier) @local.definition)
(typed_parameter (identifier) @local.definition)
(typed_default_parameter name: (identifier) @local.definition)
(lambda_parameters (identifier) @local.definition)
(list_splat_pattern (identifier) @local.definition)
(dictionary_splat_pattern (identifier) @local.definition)
(assignment left: (identifier) @local.definition)
(assignment left: (pattern_list (identifier) @local.definition))
(assignment left: (tuple_pattern (identifier) @local.definition))
(for_statement left: (identifier) @local.definition)
(for_statement left: (pattern_list (identifier) @local.definition))
(for_in_clause left: (identifier) @local.definition)
(as_pattern alias: (as_pattern_target (identifier) @local.definition))

(import_statement name: (dotted_name . (identifier) @local.import))
(import_statement name: (aliased_import alias: (identifier) @local.import))
(import_from_statement name: (dotted_name (identifier) @local.import))
(import_from_statement name: (aliased_import alias: (identifier) @local.import))

; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; Support: support@danaco-group.pl
