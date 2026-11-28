; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; https://github.com/DanacoOfficial | support@danaco-group.pl
;
; Module:           crates/nurt-code/queries/cpp/locals.scm
; Responsibility:   Query of the scopes, definitions and references of local names in C++ sources.
; Dependencies:     none
; Author:           Danaco Official
; Version:          0.1.1, 2026-11-27
; License:          see the LICENSE file or https://www.danaco-group.pl

[
  (translation_unit)
  (namespace_definition)
  (class_specifier)
  (function_definition)
  (lambda_expression)
  (compound_statement)
  (for_statement)
  (for_range_loop)
  (while_statement)
  (if_statement)
] @local.scope

(parameter_declaration declarator: (identifier) @local.definition)
(parameter_declaration declarator: (pointer_declarator declarator: (identifier) @local.definition))
(parameter_declaration declarator: (reference_declarator (identifier) @local.definition))
(optional_parameter_declaration declarator: (identifier) @local.definition)
(declaration declarator: (identifier) @local.definition)
(declaration declarator: (pointer_declarator declarator: (identifier) @local.definition))
(declaration declarator: (reference_declarator (identifier) @local.definition))
(declaration declarator: (init_declarator declarator: (identifier) @local.definition))
(declaration declarator: (init_declarator declarator: (pointer_declarator declarator: (identifier) @local.definition)))
(declaration declarator: (init_declarator declarator: (reference_declarator (identifier) @local.definition)))
(for_range_loop declarator: (identifier) @local.definition)
(for_range_loop declarator: (reference_declarator (identifier) @local.definition))

(using_declaration (identifier) @local.import)
(using_declaration (qualified_identifier name: (identifier) @local.import))

; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; Support: support@danaco-group.pl
