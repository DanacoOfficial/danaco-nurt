// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Output of `gcc` and `clang`: the lines `file:line:col: error: message`.

use super::Tally;

/// Takes one line; returns whether it belongs to the format.
pub(super) fn observe(_tally: &mut Tally, _line: &str) -> bool {
    false
}
