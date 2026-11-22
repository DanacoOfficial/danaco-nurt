// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Record stages of the core command set.
//!
//! The stages `filter`, `select`, `sort`, `first`, `last`, `skip`, `count`,
//! `assert`, `verify`, `group`, `summarize`, `distinct`, `parse` and `split`
//! select, order, aggregate, split and check streams of records. Each stage
//! implements `StageImpl` from `nurt_contracts::stage`. The executor applies
//! the rollback semantics of `verify`.
