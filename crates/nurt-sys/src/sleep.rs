// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Waiting for a duration.
//!
//! The `sleep` command blocks the program for the given time. The wait is
//! performed on the clock of the execution context and is interrupted by
//! cancellation and by the time limit of the program, so it never holds a
//! cancelled program for longer than the clock needs to observe the signal.
//!
//! The command has no effect on the world: it opens no journal operation and
//! reports no changes. The time spent waiting is reported as a progress event.

use std::time::Duration;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::journal::MutationTx;
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, EffectOutcome, EffectPlan, EffectStage, ExecCtx, InputKinds, ReadStage,
    StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Duration as ValueDuration, Value};

/// Name of the command in the catalog.
pub const COMMAND: &str = "sleep";

/// Name of the only parameter of the command.
const PARAM_DURATION: &str = "duration";

/// Key of the static information that holds the requested wait.
///
/// The executor raises the time limit of a program that does not state one to
/// the longest wait of its stages, so a long `sleep` is not cut short by the
/// default limit.
pub const DETAIL_WAIT: &str = "wait";

/// Phase name reported in the progress event of a finished wait.
const PROGRESS_PHASE: &str = "sleep";

/// Implementation of the `sleep` command.
///
/// The catalog declares the command in the category `run` with the effect
/// `read`; the implementation therefore provides both the read and the effect
/// interface, and the one the binder selects performs the same wait. The
/// planning step of the effect interface does not wait, so a preview returns
/// at once.
///
/// ```
/// use nurt_contracts::stage::StageImpl;
/// use nurt_sys::sleep::Sleep;
///
/// assert_eq!(Sleep.descriptor().command, "sleep");
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct Sleep;

impl Sleep {
    /// Creates the command implementation.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// Waits for `duration` on the clock of `ctx`.
///
/// Returns the time that elapsed on the clock. A zero duration returns at
/// once without touching the clock.
///
/// # Errors
///
/// * `E_INVALID_VALUE` when `duration` is negative or its end is beyond the
///   range of the clock.
/// * `E_CANCELLED` when the execution is cancelled before or during the wait.
/// * `E_TIMEOUT` when the time limit of the program ends the wait; the error
///   carries the requested time as `expected` and the elapsed time as
///   `actual`, both in milliseconds.
///
/// # Examples
///
/// ```
/// use nurt_contracts::stage::ExecCtx;
/// use nurt_contracts::value::Duration;
///
/// fn pause(ctx: &ExecCtx) -> nurt_contracts::error::Result<()> {
///     nurt_sys::sleep::wait(ctx, Duration::from_millis(500)).map(|_| ())
/// }
/// ```
pub fn wait(ctx: &ExecCtx, duration: ValueDuration) -> Result<Duration> {
    let requested = to_std(duration, ctx)?;
    let started = ctx.clock().monotonic();
    if requested.is_zero() {
        ctx.checkpoint()?;
        return Ok(Duration::ZERO);
    }
    let outcome = ctx.sleep(requested);
    let elapsed = ctx.clock().monotonic().saturating_sub(started);
    match outcome {
        Ok(()) => Ok(elapsed),
        Err(error) if error.code() == ErrorCode::Timeout => Err(error
            .with_expected(millis_json(requested))
            .with_actual(millis_json(elapsed))),
        Err(error) => Err(error),
    }
}

fn to_std(duration: ValueDuration, ctx: &ExecCtx) -> Result<Duration> {
    let Some(requested) = duration.to_std() else {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "duration must not be negative; give a value such as 500ms or 2s",
        )
        .with_found(format!("{}ms", duration.as_millis())));
    };
    if ctx.clock().monotonic().checked_add(requested).is_none() {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "duration exceeds the range of the clock; give a shorter wait",
        )
        .with_found(format!("{}ms", duration.as_millis())));
    }
    Ok(requested)
}

fn millis_json(duration: Duration) -> serde_json::Value {
    serde_json::Value::from(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
}

fn requested(args: &BoundArgs) -> Result<ValueDuration> {
    match args.value(PARAM_DURATION) {
        Some(Value::Duration(duration)) => Ok(*duration),
        _ => Err(Error::new(
            ErrorCode::InvalidValue,
            "duration must be a time value such as 500ms or 2s",
        )),
    }
}

fn report_elapsed(ctx: &ExecCtx, requested: ValueDuration, elapsed: Duration) {
    let done = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    let total = u64::try_from(requested.as_millis()).ok();
    ctx.progress(PROGRESS_PHASE, done, total);
}

impl StageImpl for Sleep {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        let duration = requested(args)?;
        if duration.is_negative() {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                "duration must not be negative; give a value such as 500ms or 2s",
            )
            .with_found(format!("{}ms", duration.as_millis())));
        }
        let mut info = StaticInfo {
            waits: true,
            ..StaticInfo::default()
        };
        info.details
            .insert(DETAIL_WAIT.to_owned(), Value::Duration(duration));
        Ok(info)
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }

    fn as_effect(&self) -> Option<&dyn EffectStage> {
        Some(self)
    }
}

impl ReadStage for Sleep {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let duration = requested(args)?;
        let elapsed = wait(ctx, duration)?;
        report_elapsed(ctx, duration, elapsed);
        Ok(StageOutput::none())
    }
}

impl EffectStage for Sleep {
    fn plan<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        _ctx: &'a ExecCtx,
    ) -> Result<EffectPlan> {
        requested(args)?;
        Ok(EffectPlan::new())
    }

    fn apply<'a>(
        &'a self,
        args: &'a BoundArgs,
        _plan: EffectPlan,
        _tx: &mut dyn MutationTx,
        ctx: &'a ExecCtx,
    ) -> Result<EffectOutcome<'a>> {
        let duration = requested(args)?;
        let elapsed = wait(ctx, duration)?;
        report_elapsed(ctx, duration, elapsed);
        Ok(EffectOutcome::new(StageOutput::none()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use nurt_contracts::env::{BootId, Clock, ClockStamp, Host, HostInfo, Os, PathStyle, WallTime};
    use nurt_contracts::fs::WriteHandle;
    use nurt_contracts::journal::{
        Applied, ExemptWrite, ExternalChange, Mutation, OpHandle, OpId, Preserved, RollbackReason,
        RollbackReport, TargetIndex, TargetSpec,
    };
    use nurt_contracts::report::IrreversibleEntry;
    use nurt_contracts::spec::StageSpec;
    use nurt_contracts::stage::{
        CancelReason, CancelToken, Event, EventSink, ExecCtxBuilder, StageData,
    };

    use super::*;

    struct VirtualClock {
        monotonic: Mutex<Duration>,
    }

    impl VirtualClock {
        fn at(origin: Duration) -> Arc<Self> {
            Arc::new(Self {
                monotonic: Mutex::new(origin),
            })
        }
    }

    impl Clock for VirtualClock {
        fn now(&self) -> WallTime {
            WallTime::UNIX_EPOCH
        }

        fn monotonic(&self) -> Duration {
            *self.monotonic.lock().unwrap()
        }

        fn stamp(&self) -> ClockStamp {
            let monotonic = self.monotonic();
            ClockStamp {
                wall: self.now(),
                boot: BootId("boot".to_owned()),
                since_boot_with_suspend: monotonic,
                since_boot_active: monotonic,
            }
        }

        fn sleep(&self, duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason> {
            if let Some(reason) = cancel.reason() {
                return Err(reason);
            }
            *self.monotonic.lock().unwrap() += duration;
            Ok(())
        }
    }

    struct BlockingClock {
        origin: Instant,
    }

    impl Clock for BlockingClock {
        fn now(&self) -> WallTime {
            WallTime::UNIX_EPOCH
        }

        fn monotonic(&self) -> Duration {
            self.origin.elapsed()
        }

        fn stamp(&self) -> ClockStamp {
            ClockStamp {
                wall: self.now(),
                boot: BootId("boot".to_owned()),
                since_boot_with_suspend: self.monotonic(),
                since_boot_active: self.monotonic(),
            }
        }

        fn sleep(&self, duration: Duration, cancel: &CancelToken) -> Result<(), CancelReason> {
            match cancel.wait_timeout(duration) {
                Some(reason) => Err(reason),
                None => Ok(()),
            }
        }
    }

    struct TestHost(HostInfo);

    impl Host for TestHost {
        fn info(&self) -> &HostInfo {
            &self.0
        }

        fn env_var(&self, _name: &str) -> Option<std::ffi::OsString> {
            None
        }

        fn env_vars(&self) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
            Vec::new()
        }
    }

    fn host() -> Arc<TestHost> {
        Arc::new(TestHost(HostInfo {
            os: Os::Linux,
            os_version: "6.8.0".to_owned(),
            architecture: "x86_64".to_owned(),
            hostname: "host".to_owned(),
            user: "user".to_owned(),
            home: "/home/user".into(),
            temp: "/tmp".into(),
            path_style: PathStyle::Posix,
            timezone: "UTC".to_owned(),
            locale: None,
            cpus: 1,
        }))
    }

    fn builder(clock: Arc<dyn Clock>) -> ExecCtxBuilder {
        ExecCtx::builder(clock, host())
    }

    fn millis(value: i64) -> ValueDuration {
        ValueDuration::from_millis(value)
    }

    fn args(duration: ValueDuration) -> BoundArgs {
        BoundArgs::new(COMMAND).with(
            PARAM_DURATION,
            nurt_contracts::stage::ArgValue::Value(Value::Duration(duration)),
            false,
        )
    }

    #[derive(Default)]
    struct CountingTx {
        calls: AtomicUsize,
    }

    impl CountingTx {
        fn refuse<T>(&self) -> Result<T> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(Error::new(
                ErrorCode::StageFailed,
                "unexpected journal call",
            ))
        }
    }

    impl MutationTx for CountingTx {
        fn op(&self) -> Option<OpId> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            None
        }

        fn preserve(&mut self, _targets: &[TargetSpec]) -> Result<Vec<Preserved>> {
            self.refuse()
        }

        fn stage(&mut self, _target: TargetIndex) -> Result<Box<dyn WriteHandle>> {
            self.refuse()
        }

        fn apply(&mut self, _target: TargetIndex, _mutation: Mutation) -> Result<Applied> {
            self.refuse()
        }

        fn observe_external(&mut self, _changes: &[ExternalChange]) -> Result<Vec<Applied>> {
            self.refuse()
        }

        fn mark_irreversible(&mut self, _marker: IrreversibleEntry) {
            self.calls.fetch_add(1, Ordering::SeqCst);
        }

        fn record_exempt(&mut self, _write: ExemptWrite) {
            self.calls.fetch_add(1, Ordering::SeqCst);
        }

        fn commit(self: Box<Self>) -> Result<OpHandle> {
            self.refuse()
        }

        fn rollback(self: Box<Self>, _reason: RollbackReason) -> Result<RollbackReport> {
            self.refuse()
        }
    }

    #[derive(Default)]
    struct Recorder(Mutex<Vec<Event>>);

    impl EventSink for Recorder {
        fn emit(&self, event: Event) {
            self.0.lock().unwrap().push(event);
        }
    }

    #[test]
    fn sleep_zero_returns_immediately() {
        let clock = VirtualClock::at(Duration::ZERO);
        let ctx = builder(clock.clone()).build();
        let elapsed = wait(&ctx, ValueDuration::ZERO).unwrap();
        assert_eq!(elapsed, Duration::ZERO);
        assert_eq!(clock.monotonic(), Duration::ZERO);
    }

    #[test]
    fn sleep_waits_exact_duration_virtual_clock() {
        let clock = VirtualClock::at(Duration::from_secs(10));
        let ctx = builder(clock.clone()).build();
        let elapsed = wait(&ctx, millis(2_500)).unwrap();
        assert_eq!(elapsed, Duration::from_millis(2_500));
        assert_eq!(clock.monotonic(), Duration::from_millis(12_500));
    }

    #[test]
    fn sleep_cancel_returns_cancelled_within_100ms() {
        let cancel = CancelToken::new();
        let clock = Arc::new(BlockingClock {
            origin: Instant::now(),
        });
        let ctx = builder(clock).cancel(cancel.clone()).build();
        let signal = cancel.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            signal.cancel(CancelReason::Requested);
            Instant::now()
        });
        let error = wait(&ctx, millis(30_000)).unwrap_err();
        let returned = Instant::now();
        let signalled = thread.join().unwrap();
        assert_eq!(error.code(), ErrorCode::Cancelled);
        assert!(returned.saturating_duration_since(signalled) < Duration::from_millis(100));
    }

    #[test]
    fn sleep_cancel_before_start_returns_cancelled() {
        let cancel = CancelToken::new();
        cancel.cancel(CancelReason::Requested);
        let clock = VirtualClock::at(Duration::ZERO);
        let ctx = builder(clock.clone()).cancel(cancel).build();
        let error = wait(&ctx, millis(1_000)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Cancelled);
        assert_eq!(clock.monotonic(), Duration::ZERO);
    }

    #[test]
    fn sleep_zero_after_cancel_returns_cancelled() {
        let cancel = CancelToken::new();
        cancel.cancel(CancelReason::Requested);
        let ctx = builder(VirtualClock::at(Duration::ZERO))
            .cancel(cancel)
            .build();
        let error = wait(&ctx, ValueDuration::ZERO).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Cancelled);
    }

    #[test]
    fn sleep_longer_than_deadline_times_out_with_expected_actual() {
        let clock = VirtualClock::at(Duration::ZERO);
        let ctx = builder(clock.clone())
            .timeout(Duration::from_secs(2))
            .build();
        let error = wait(&ctx, millis(60_000)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Timeout);
        assert_eq!(error.data().expected, Some(serde_json::json!(60_000)));
        assert_eq!(error.data().actual, Some(serde_json::json!(2_000)));
        assert_eq!(clock.monotonic(), Duration::from_secs(2));
    }

    #[test]
    fn sleep_after_deadline_times_out_with_zero_actual() {
        let clock = VirtualClock::at(Duration::ZERO);
        let ctx = builder(clock.clone())
            .timeout(Duration::from_secs(1))
            .build();
        *clock.monotonic.lock().unwrap() = Duration::from_secs(5);
        let error = wait(&ctx, millis(10)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Timeout);
        assert_eq!(error.data().actual, Some(serde_json::json!(0)));
    }

    #[test]
    fn sleep_declares_wait_for_program_limit_rule() {
        let host = host();
        let config = nurt_contracts::config::Config::default();
        let spec = frozen_spec();
        let ctx = CheckCtx::new(&spec, host.as_ref(), &config);
        let info = Sleep.check(&args(millis(300_000)), &ctx).unwrap();
        assert!(info.waits);
        assert_eq!(
            info.details.get(DETAIL_WAIT),
            Some(&Value::Duration(millis(300_000)))
        );
    }

    #[test]
    fn sleep_check_rejects_negative_duration() {
        let host = host();
        let config = nurt_contracts::config::Config::default();
        let spec = frozen_spec();
        let ctx = CheckCtx::new(&spec, host.as_ref(), &config);
        let error = Sleep.check(&args(millis(-1)), &ctx).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn sleep_overflow_duration_invalid_value() {
        let clock = VirtualClock::at(Duration::MAX);
        let ctx = builder(clock).build();
        let error = wait(&ctx, millis(1)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn sleep_negative_duration_invalid_value() {
        let ctx = builder(VirtualClock::at(Duration::ZERO)).build();
        let error = wait(&ctx, millis(-5)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn sleep_rejects_missing_duration_argument() {
        let ctx = builder(VirtualClock::at(Duration::ZERO)).build();
        let error = Sleep
            .execute(&BoundArgs::new(COMMAND), StageInput::none(), &ctx)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }

    #[test]
    fn sleep_reports_elapsed() {
        let recorder = Arc::new(Recorder::default());
        let ctx = builder(VirtualClock::at(Duration::ZERO))
            .events(recorder.clone())
            .build();
        let args = args(millis(750));
        let output = Sleep.execute(&args, StageInput::none(), &ctx).unwrap();
        assert!(matches!(output.data, StageData::None));
        let events = recorder.0.lock().unwrap();
        let [Event::Progress(progress)] = events.as_slice() else {
            panic!("expected one progress event, got {events:?}");
        };
        assert_eq!(progress.done, 750);
        assert_eq!(progress.total, Some(750));
    }

    #[test]
    fn sleep_has_no_mutation_tx() {
        let clock = VirtualClock::at(Duration::ZERO);
        let ctx = builder(clock.clone()).build();
        let args = args(millis(100));
        let plan = Sleep.plan(&args, StageInput::none(), &ctx).unwrap();
        assert!(plan.changes.is_empty());
        assert_eq!(clock.monotonic(), Duration::ZERO);
        let mut tx = CountingTx::default();
        let outcome = Sleep.apply(&args, plan, &mut tx, &ctx).unwrap();
        assert_eq!(tx.calls.load(Ordering::SeqCst), 0);
        assert!(outcome.changes.is_empty());
        assert!(outcome.irreversible.is_empty());
        assert_eq!(clock.monotonic(), Duration::from_millis(100));
    }

    #[test]
    fn sleep_descriptor_is_a_source_without_input() {
        let descriptor = Sleep.descriptor();
        assert_eq!(descriptor.command, COMMAND);
        assert!(descriptor.source);
        assert_eq!(descriptor.input, InputKinds::None);
        assert!(Sleep.as_read().is_some() && Sleep.as_effect().is_some());
    }

    fn frozen_entry() -> toml::Value {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../catalog/system.toml");
        let text = std::fs::read_to_string(path).unwrap();
        let catalog: toml::Table = text.parse().unwrap();
        catalog["command"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| command["name"].as_str() == Some(COMMAND))
            .unwrap()
            .clone()
    }

    fn frozen_spec() -> StageSpec {
        frozen_entry().try_into().unwrap()
    }

    #[test]
    fn catalog_signature_sleep_matches_frozen() {
        let entry = frozen_entry();
        assert_eq!(entry["category"].as_str(), Some("run"));
        assert_eq!(entry["effect"].as_str(), Some("read"));
        assert_eq!(entry["waits"].as_bool(), Some(true));
        assert_eq!(entry["input"].as_array().unwrap().len(), 1);
        let params = entry["param"].as_array().unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0]["name"].as_str(), Some(PARAM_DURATION));
        assert_eq!(params[0]["type"].as_str(), Some("duration"));
        assert_eq!(params[0]["position"].as_integer(), Some(1));
        assert_eq!(params[0]["required"].as_bool(), Some(true));
        assert!(params[0].get("default").is_none());
    }
}
