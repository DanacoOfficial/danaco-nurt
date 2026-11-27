; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; https://github.com/DanacoOfficial | support@danaco-group.pl
;
; Module:           crates/nurt-code/queries/python/tags.scm
; Responsibility:   Query of the symbols defined in Python sources.
; Dependencies:     none
; Author:           Danaco Official
; Version:          0.1.1, 2026-11-27
; License:          see the LICENSE file or https://www.danaco-group.pl

(class_definition name: (identifier) @name) @definition.class
(function_definition name: (identifier) @name) @definition.function
(type_alias_statement left: (type (identifier) @name)) @definition.type

(module (expression_statement (assignment left: (identifier) @name) @definition.variable))
(class_definition
  body: (block (expression_statement (assignment left: (identifier) @name) @definition.field)))

; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; Support: support@danaco-group.pl
