; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; https://github.com/DanacoOfficial | support@danaco-group.pl
;
; Module:           crates/nurt-code/queries/swift/locals.scm
; Responsibility:   Query of the scopes, definitions and references of local names in Swift sources.
; Dependencies:     none
; Author:           Danaco Official
; Version:          0.1.1, 2026-11-27
; License:          see the LICENSE file or https://www.danaco-group.pl

[
  (source_file)
  (class_declaration)
  (protocol_declaration)
  (function_declaration)
  (init_declaration)
  (function_body)
  (lambda_literal)
  (for_statement)
] @local.scope

(parameter name: (simple_identifier) @local.definition)
(property_declaration name: (pattern bound_identifier: (simple_identifier) @local.definition))
(for_statement (pattern bound_identifier: (simple_identifier) @local.definition))

(import_declaration (identifier (simple_identifier) @local.import .))

; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; Support: support@danaco-group.pl
