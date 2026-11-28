; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; https://github.com/DanacoOfficial | support@danaco-group.pl
;
; Module:           crates/nurt-code/queries/java/locals.scm
; Responsibility:   Query of the scopes, definitions and references of local names in Java sources.
; Dependencies:     none
; Author:           Danaco Official
; Version:          0.1.1, 2026-11-27
; License:          see the LICENSE file or https://www.danaco-group.pl

[
  (program)
  (class_body)
  (interface_body)
  (enum_body)
  (method_declaration)
  (constructor_declaration)
  (lambda_expression)
  (block)
  (for_statement)
  (enhanced_for_statement)
  (catch_clause)
  (try_with_resources_statement)
] @local.scope

(formal_parameter name: (identifier) @local.definition)
(spread_parameter (variable_declarator name: (identifier) @local.definition))
(local_variable_declaration declarator: (variable_declarator name: (identifier) @local.definition))
(enhanced_for_statement name: (identifier) @local.definition)
(catch_formal_parameter name: (identifier) @local.definition)
(lambda_expression parameters: (identifier) @local.definition)
(inferred_parameters (identifier) @local.definition)
(resource name: (identifier) @local.definition)

(import_declaration (scoped_identifier name: (identifier) @local.import))

; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; Support: support@danaco-group.pl
