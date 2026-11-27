; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; https://github.com/DanacoOfficial | support@danaco-group.pl
;
; Module:           crates/nurt-code/queries/yaml/tags.scm
; Responsibility:   Query of the symbols defined in YAML sources.
; Dependencies:     none
; Author:           Danaco Official
; Version:          0.1.1, 2026-11-27
; License:          see the LICENSE file or https://www.danaco-group.pl

(block_mapping_pair
  key: (flow_node [(plain_scalar (string_scalar) @name) (double_quote_scalar) @name (single_quote_scalar) @name])) @definition.field
(flow_pair
  key: (flow_node [(plain_scalar (string_scalar) @name) (double_quote_scalar) @name (single_quote_scalar) @name])) @definition.field

; Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
; Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
; Support: support@danaco-group.pl
