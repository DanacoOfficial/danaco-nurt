; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; https://github.com/DanacoOfficial | support@danaco-group.pl
;
; Module:           crates/nurt-code/queries/kotlin/locals.scm
; Responsibility:   Query of the scopes, definitions and references of local names in Kotlin sources.
; Dependencies:     none
; Author:           Danaco Official
; Version:          0.1.1, 2026-11-27
; License:          see the LICENSE file or https://www.danaco-group.pl

[
  (source_file)
  (class_declaration)
  (object_declaration)
  (function_declaration)
  (function_body)
  (lambda_literal)
  (for_statement)
] @local.scope

(parameter (identifier) @local.definition)
(class_parameter (identifier) @local.definition)
(variable_declaration (identifier) @local.definition)

(import (identifier) @local.import)
(import (qualified_identifier (identifier) @local.import .))

; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; Support: support@danaco-group.pl
