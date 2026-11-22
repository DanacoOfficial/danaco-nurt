// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Processor use of processes.
//!
//! The `cpu` field is the processor use in percent of one core. Measuring the
//! current use needs two readings of the processor time some time apart, so
//! the second reading and the wait before it happen only when the rest of the
//! pipeline names the field. Without a list of used fields the value is the
//! average over the lifetime of the process, from one reading.

use std::collections::HashMap;
use std::time::Duration;

use nurt_contracts::error::Result;
use nurt_contracts::stage::{ExecCtx, FieldDemand};

use super::model::{ProcessInfo, ProcessReader};

/// Name of the field filled by the measurement.
pub(crate) const FIELD: &str = "cpu";

/// Time between the two readings of a measurement.
pub(crate) const SAMPLE_INTERVAL: Duration = Duration::from_millis(200);

const PERCENT: f64 = 100.0;

/// Values are rounded to this many parts of one percent.
const PRECISION: f64 = 10.0;

/// How the `cpu` field is computed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CpuMode {
    /// The field is not used later and is left out.
    Omitted,
    /// Average over the lifetime of the process, from one reading.
    Lifetime,
    /// Use during [`SAMPLE_INTERVAL`], from two readings.
    Sampled,
}

impl CpuMode {
    /// Mode for the fields the rest of the pipeline uses.
    pub(crate) fn for_demand(fields: &FieldDemand) -> Self {
        match fields {
            FieldDemand::All => Self::Lifetime,
            FieldDemand::Only(_) if fields.wants(FIELD) => Self::Sampled,
            FieldDemand::Only(_) => Self::Omitted,
        }
    }
}

/// Computes the `cpu` field of processes read during one execution.
#[derive(Debug)]
pub(crate) struct CpuMeter {
    mode: CpuMode,
    baseline: HashMap<u64, Duration>,
    baseline_at: Duration,
}

impl CpuMeter {
    /// Prepares the meter; in [`CpuMode::Sampled`] takes the first reading of
    /// `pids` and waits [`SAMPLE_INTERVAL`] on the context clock.
    ///
    /// # Errors
    ///
    /// Fails with `E_CANCELLED`, `E_TIMEOUT` or `E_LIMIT` when the execution
    /// ends during the wait.
    pub(crate) fn start(
        mode: CpuMode,
        reader: &mut dyn ProcessReader,
        pids: &[u64],
        ctx: &ExecCtx,
    ) -> Result<Self> {
        let mut baseline = HashMap::new();
        if mode == CpuMode::Sampled {
            for &pid in pids {
                ctx.checkpoint()?;
                if let Some(time) = reader.cpu_time(pid) {
                    baseline.insert(pid, time);
                }
            }
        }
        let baseline_at = ctx.clock().monotonic();
        if mode == CpuMode::Sampled {
            ctx.sleep(SAMPLE_INTERVAL)?;
        }
        Ok(Self {
            mode,
            baseline,
            baseline_at,
        })
    }

    /// Value of the `cpu` field of `process`, read now.
    pub(crate) fn cpu(&self, process: &ProcessInfo, ctx: &ExecCtx) -> Option<f64> {
        let time = process.cpu_time?;
        match self.mode {
            CpuMode::Omitted => None,
            CpuMode::Lifetime => {
                let started = process.started?.as_unix_nanos();
                let now = ctx.clock().now().unix_nanos();
                let elapsed = u64::try_from(now.saturating_sub(i128::from(started))).ok()?;
                percent(time, Duration::from_nanos(elapsed))
            }
            CpuMode::Sampled => {
                let before = self.baseline.get(&process.pid)?;
                let elapsed = ctx.clock().monotonic().saturating_sub(self.baseline_at);
                percent(time.saturating_sub(*before), elapsed)
            }
        }
    }
}

/// Processor time `used` during `elapsed` wall time, in percent of one core.
pub(crate) fn percent(used: Duration, elapsed: Duration) -> Option<f64> {
    if elapsed.is_zero() {
        return None;
    }
    let value = used.as_secs_f64() / elapsed.as_secs_f64() * PERCENT;
    Some((value * PRECISION).round() / PRECISION)
}
