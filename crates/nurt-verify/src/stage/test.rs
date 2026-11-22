// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `test.run`, `test.failed`, `test.show` and `test.list` commands.

use std::sync::Arc;

use nurt_contracts::stage::StageImpl;
use nurt_proc::JobFinisher;

mod args;
mod failed;
mod finish;
mod launch;
mod list;
mod output;
mod run;
mod show;
#[cfg(test)]
pub(crate) mod support;
#[cfg(test)]
mod tests;

pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![
        Arc::new(run::TestRun),
        Arc::new(failed::TestFailed),
        Arc::new(show::TestShow),
        Arc::new(list::TestList),
    ]
}

/// Finisher of test runs that continue in the background.
pub(crate) fn job_finisher() -> Arc<dyn JobFinisher> {
    Arc::new(finish::TestFinisher)
}
