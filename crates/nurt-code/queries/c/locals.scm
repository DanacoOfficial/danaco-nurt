; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; https://github.com/DanacoOfficial | support@danaco-group.pl
;
; Module:           crates/nurt-code/queries/c/locals.scm
; Responsibility:   Query of the scopes, definitions and references of local names in C sources.
; Dependencies:     none
; Author:           Danaco Official
; Version:          0.1.1, 2026-11-27
; License:          see the LICENSE file or https://www.danaco-group.pl

[
  (translation_unit)
  (function_definition)
  (compound_statement)
  (for_statement)
  (while_statement)
  (if_statement)
] @local.scope

(parameter_declaration declarator: (identifier) @local.definition)
(parameter_declaration declarator: (pointer_declarator declarator: (identifier) @local.definition))
(parameter_declaration declarator: (array_declarator declarator: (identifier) @local.definition))
(declaration declarator: (identifier) @local.definition)
(declaration declarator: (pointer_declarator declarator: (identifier) @local.definition))
(declaration declarator: (array_declarator declarator: (identifier) @local.definition))
(declaration declarator: (init_declarator declarator: (identifier) @local.definition))
(declaration declarator: (init_declarator declarator: (pointer_declarator declarator: (identifier) @local.definition)))
(declaration declarator: (init_declarator declarator: (array_declarator declarator: (identifier) @local.definition)))

; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; Support: support@danaco-group.pl
