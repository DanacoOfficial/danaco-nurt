; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; https://github.com/DanacoOfficial | support@danaco-group.pl
;
; Module:           crates/nurt-code/queries/ruby/tags.scm
; Responsibility:   Query of the symbols defined in Ruby sources.
; Dependencies:     none
; Author:           Danaco Official
; Version:          0.1.1, 2026-11-27
; License:          see the LICENSE file or https://www.danaco-group.pl

(class name: [(constant) @name (scope_resolution name: (constant) @name)]) @definition.class
(module name: [(constant) @name (scope_resolution name: (constant) @name)]) @definition.module
(method name: (_) @name) @definition.method
(singleton_method name: (_) @name) @definition.method
(assignment left: (constant) @name) @definition.constant

; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; Support: support@danaco-group.pl
