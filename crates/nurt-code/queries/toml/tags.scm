; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; https://github.com/DanacoOfficial | support@danaco-group.pl
;
; Module:           crates/nurt-code/queries/toml/tags.scm
; Responsibility:   Query of the symbols defined in TOML sources.
; Dependencies:     none
; Author:           Danaco Official
; Version:          0.1.1, 2026-11-27
; License:          see the LICENSE file or https://www.danaco-group.pl

(table [(bare_key) (dotted_key) (quoted_key)] @name) @definition.module
(table_array_element [(bare_key) (dotted_key) (quoted_key)] @name) @definition.module
(pair [(bare_key) (dotted_key) (quoted_key)] @name) @definition.field

; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; Support: support@danaco-group.pl
