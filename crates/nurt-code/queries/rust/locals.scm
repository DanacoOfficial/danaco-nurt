; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; https://github.com/DanacoOfficial | support@danaco-group.pl
;
; Module:           crates/nurt-code/queries/rust/locals.scm
; Responsibility:   Query of the scopes, definitions and references of local names in Rust sources.
; Dependencies:     none
; Author:           Danaco Official
; Version:          0.1.1, 2026-11-27
; License:          see the LICENSE file or https://www.danaco-group.pl

[
  (source_file)
  (function_item)
  (closure_expression)
  (block)
  (impl_item)
  (trait_item)
  (mod_item)
  (for_expression)
  (while_expression)
  (if_expression)
  (match_arm)
] @local.scope

(parameter pattern: (identifier) @local.definition)
(parameter pattern: (mut_pattern (identifier) @local.definition))
(let_declaration pattern: (identifier) @local.definition)
(let_declaration pattern: (mut_pattern (identifier) @local.definition))
(let_declaration pattern: (tuple_pattern (identifier) @local.definition))
(let_condition pattern: (identifier) @local.definition)
(let_condition pattern: (tuple_struct_pattern type: (_) (identifier) @local.definition))
(closure_parameters (identifier) @local.definition)
(for_expression pattern: (identifier) @local.definition)
(for_expression pattern: (tuple_pattern (identifier) @local.definition))
(match_pattern (identifier) @local.definition)
(match_pattern (tuple_struct_pattern type: (_) (identifier) @local.definition))

(use_declaration argument: (identifier) @local.import)
(use_declaration argument: (scoped_identifier name: (identifier) @local.import))
(use_as_clause alias: (identifier) @local.import)
(use_list (identifier) @local.import)
(use_list (scoped_identifier name: (identifier) @local.import))

; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; Support: support@danaco-group.pl
