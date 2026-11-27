// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-proc::track::tests::operations
// Responsibility:   Journal operation of the kept previous content and of the detected.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Journal operation of the kept previous content and of the detected
//! changes.

use super::support::{Harness, RecordingTx};
use crate::iface::TrackMode;

#[test]
fn declared_indexes_require_the_same_operation() {
    let h = Harness::new();
    h.write("a.txt", "alpha");
    let mut first = RecordingTx::new();
    let prepared = h.prepare(&h.spec(TrackMode::Scan), &mut first);
    h.write("a.txt", "alpha changed");
    let mut other = RecordingTx::other_operation();
    prepared.finish(&h.ctx, &mut other).unwrap();
    assert_eq!(other.declared_indexes(), vec![None]);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
