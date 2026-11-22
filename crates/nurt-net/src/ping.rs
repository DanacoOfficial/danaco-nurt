// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! ICMP echo requests.
//!
//! `network.ping` sends echo requests to a host and reports how many replies
//! arrived and their round-trip times. No privilege is needed: Linux and
//! macOS use ICMP datagram sockets (`SOCK_DGRAM` with `IPPROTO_ICMP` or
//! `IPPROTO_ICMPV6`), Windows uses `IcmpSendEcho` and `Icmp6SendEcho2`. Raw
//! sockets are never opened. On Linux an ICMP datagram socket is allowed
//! only for the groups in `net.ipv4.ping_group_range`; for other accounts the
//! command fails with `E_PRIVILEGE` and a fix that widens the range.
//!
//! Requests are sent one second apart, as by the `ping` program. A host name
//! is resolved by the resolver of the operating system
//! ([`crate::dns::resolve_host`]) and the first address is used.

use std::net::IpAddr;
use std::time::{Duration, Instant};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::record::PingResult;
use nurt_contracts::stage::{
    BoundArgs, CheckCtx, ExecCtx, InputKinds, NetworkTarget, ReadStage, RecordStream,
    StageDescriptor, StageImpl, StageInput, StageOutput, StaticInfo,
};
use nurt_contracts::value::{Record, Value};

use crate::dns::resolve_host;
use crate::interfaces::SysRoot;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

/// Name of the command in the catalog.
pub const COMMAND: &str = "network.ping";

const PARAM_HOST: &str = "host";
const PARAM_COUNT: &str = "count";
const PARAM_TIMEOUT: &str = "timeout";

/// Requests sent when `count` is not given.
const DEFAULT_COUNT: u64 = 4;
/// Time to wait for each reply when `timeout` is not given.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);
/// Time between the starts of two requests.
const INTERVAL: Duration = Duration::from_secs(1);
/// Phase name of the progress events.
const PROGRESS_PHASE: &str = "echo";
/// Percent of a whole, for the loss.
const PERCENT: f64 = 100.0;
/// Milliseconds per second, for round-trip times.
const MILLIS_PER_SECOND: f64 = 1000.0;
/// Length of the payload of a request: an identifying token.
pub(crate) const PAYLOAD_LEN: usize = 8;
/// Longest wait between two checks of cancellation.
#[cfg(unix)]
const POLL_SLICE: Duration = Duration::from_millis(100);

/// Socket able to send one echo request and wait for its reply.
trait Echo {
    /// Sends request `sequence` and waits up to `timeout` for the reply.
    /// Returns the round-trip time, or `None` when no reply arrived in time.
    /// `check` is called while waiting and ends the wait with its error.
    fn echo(
        &mut self,
        sequence: u16,
        timeout: Duration,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Option<Duration>>;
}

#[cfg(unix)]
fn open(address: IpAddr, root: &SysRoot) -> Result<Box<dyn Echo>> {
    Ok(Box::new(unix::DatagramEcho::open(address, root)?))
}

#[cfg(windows)]
fn open(address: IpAddr, _root: &SysRoot) -> Result<Box<dyn Echo>> {
    Ok(Box::new(windows::IcmpEcho::open(address)?))
}

#[cfg(not(any(unix, windows)))]
fn open(_address: IpAddr, _root: &SysRoot) -> Result<Box<dyn Echo>> {
    Err(Error::new(
        ErrorCode::OsOnly,
        "network.ping is available on Linux, macOS and Windows",
    ))
}

/// Statistics of a run of echo requests.
///
/// `rtts` holds the round-trip times of the replies in milliseconds. The
/// standard deviation is the population deviation, as reported by `ping`.
///
/// ```
/// use nurt_net::ping::statistics;
///
/// let result = statistics("host", "192.0.2.1".parse().unwrap(), 4, &[10.0, 20.0, 30.0]);
/// assert_eq!(result.received, 3);
/// assert_eq!(result.loss, 25.0);
/// assert_eq!(result.rtt_avg, Some(20.0));
/// ```
#[must_use]
pub fn statistics(host: &str, address: IpAddr, sent: u64, rtts: &[f64]) -> PingResult {
    let received = u64::try_from(rtts.len()).unwrap_or(u64::MAX);
    let loss = if sent == 0 {
        0.0
    } else {
        (sent.saturating_sub(received)) as f64 * PERCENT / sent as f64
    };
    let mut result = PingResult {
        host: host.to_owned(),
        address: address.to_string(),
        sent,
        received,
        loss,
        ..PingResult::default()
    };
    if !rtts.is_empty() {
        let count = rtts.len() as f64;
        let average = rtts.iter().sum::<f64>() / count;
        let variance = rtts.iter().map(|rtt| (rtt - average).powi(2)).sum::<f64>() / count;
        result.rtt_min = rtts.iter().copied().reduce(f64::min);
        result.rtt_max = rtts.iter().copied().reduce(f64::max);
        result.rtt_avg = Some(average);
        result.rtt_stddev = Some(variance.sqrt());
    }
    result
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * MILLIS_PER_SECOND
}

/// Statistics of a run of echo requests to an address.
///
/// The `count` requests are sent one second apart. System files
/// (`/proc/sys/net/ipv4/ping_group_range` for the error of a refused socket)
/// are read below `root`. Progress events report the requests sent.
///
/// # Errors
///
/// The function fails with `E_PRIVILEGE` when the system refuses an ICMP
/// socket to the account, with `E_NETWORK` (problem `icmp`) when a request
/// cannot be sent, and with `E_CANCELLED` or `E_TIMEOUT` when the execution
/// is cancelled or reaches its time limit.
///
/// # Examples
///
/// ```no_run
/// use std::time::Duration;
/// use nurt_net::interfaces::SysRoot;
/// use nurt_net::ping::ping;
///
/// # fn run(ctx: &nurt_contracts::stage::ExecCtx) -> nurt_contracts::error::Result<()> {
/// let address = "127.0.0.1".parse().unwrap();
/// let result = ping("localhost", address, 2, Duration::from_secs(1), &SysRoot::host(), ctx)?;
/// assert_eq!(result.sent, 2);
/// # Ok(())
/// # }
/// ```
pub fn ping(
    host: &str,
    address: IpAddr,
    count: u64,
    timeout: Duration,
    root: &SysRoot,
    ctx: &ExecCtx,
) -> Result<PingResult> {
    ctx.checkpoint()?;
    let mut socket = open(address, root)?;
    series(socket.as_mut(), host, address, count, timeout, ctx)
}

/// Returns the statistics of `count` requests sent through `socket` one
/// interval apart.
fn series(
    socket: &mut dyn Echo,
    host: &str,
    address: IpAddr,
    count: u64,
    timeout: Duration,
    ctx: &ExecCtx,
) -> Result<PingResult> {
    let mut rtts = Vec::new();
    let mut sent = 0;
    let mut previous: Option<Instant> = None;
    let check = || ctx.checkpoint();
    for index in 0..count {
        if let Some(started) = previous {
            let wait = INTERVAL.saturating_sub(started.elapsed());
            if !wait.is_zero() {
                ctx.sleep(wait)?;
            }
        }
        previous = Some(Instant::now());
        let sequence = u16::try_from((index + 1) % (u64::from(u16::MAX) + 1)).unwrap_or(0);
        sent += 1;
        if let Some(rtt) = socket.echo(sequence, timeout, &check)? {
            rtts.push(millis(rtt));
        }
        ctx.progress(PROGRESS_PHASE, sent, Some(count));
    }
    Ok(statistics(host, address, sent, &rtts))
}

fn count(args: &BoundArgs) -> Result<u64> {
    match args.int(PARAM_COUNT)? {
        None => Ok(DEFAULT_COUNT),
        Some(count) => u64::try_from(count)
            .ok()
            .filter(|count| *count > 0)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidValue,
                    "count must be at least 1; give a value such as count: 4",
                )
                .with_found(count.to_string())
            }),
    }
}

fn timeout(args: &BoundArgs) -> Result<Duration> {
    match args.value(PARAM_TIMEOUT) {
        None | Some(Value::Null) => Ok(DEFAULT_TIMEOUT),
        Some(Value::Duration(duration)) => duration
            .to_std()
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidValue,
                    "timeout must be longer than zero; give a value such as timeout: 2s",
                )
                .with_found(format!("{}ms", duration.as_millis()))
            }),
        Some(other) => Err(Error::new(
            ErrorCode::Type,
            format!(
                "`{COMMAND}` expects a duration for `{PARAM_TIMEOUT}`, not {}",
                other.type_name()
            ),
        )),
    }
}

fn host(args: &BoundArgs) -> Result<&str> {
    args.require(PARAM_HOST)?;
    args.text(PARAM_HOST)?
        .filter(|host| !host.trim().is_empty())
        .ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidValue,
                "host must be a host name or an address such as 192.0.2.1",
            )
        })
}

/// Implementation of the `network.ping` command.
///
/// ```
/// use nurt_contracts::stage::StageImpl;
/// use nurt_net::ping::Ping;
///
/// assert_eq!(Ping::new().descriptor().command, "network.ping");
/// ```
#[derive(Debug, Clone, Default)]
pub struct Ping {
    root: SysRoot,
}

impl Ping {
    /// The command reading system files of the host.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The command reading system files below `root`.
    #[must_use]
    pub fn with_root(root: SysRoot) -> Self {
        Self { root }
    }
}

impl StageImpl for Ping {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn check(&self, args: &BoundArgs, _ctx: &CheckCtx<'_>) -> Result<StaticInfo> {
        count(args)?;
        timeout(args)?;
        let mut info = StaticInfo::default();
        if let Some(host) = args.text(PARAM_HOST)? {
            info.network.push(NetworkTarget {
                scheme: "icmp".to_owned(),
                host: host.to_owned(),
                port: None,
                url: None,
            });
        }
        Ok(info)
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Ping {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let host = host(args)?;
        let count = count(args)?;
        let timeout = timeout(args)?;
        let address = resolve_host(host)?.into_iter().next().ok_or_else(|| {
            Error::new(ErrorCode::Network, format!("host `{host}` has no address"))
                .with_problem("dns")
        })?;
        let result = ping(host, address, count, timeout, &self.root, ctx)?;
        Ok(StageOutput::records(RecordStream::once(Record::from(
            result,
        ))))
    }
}

/// Internet checksum of RFC 1071 over `bytes`.
#[cfg(any(unix, test))]
pub(crate) fn checksum(bytes: &[u8]) -> u16 {
    let mut sum: u32 = bytes
        .chunks(2)
        .map(|pair| match pair {
            [high, low] => u32::from(u16::from_be_bytes([*high, *low])),
            [high] => u32::from(u16::from_be_bytes([*high, 0])),
            _ => 0,
        })
        .sum();
    while sum > u32::from(u16::MAX) {
        sum = (sum & u32::from(u16::MAX)) + (sum >> u16::BITS);
    }
    !u16::try_from(sum).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use nurt_contracts::stage::ArgValue;
    use nurt_contracts::value::Duration as ValueDuration;

    use super::*;

    #[test]
    fn statistics_without_replies_reports_full_loss() {
        let result = statistics("host", "192.0.2.1".parse().unwrap(), 4, &[]);
        assert_eq!(result.loss, 100.0);
        assert_eq!(result.received, 0);
        assert_eq!(result.rtt_min, None);
        assert_eq!(result.rtt_stddev, None);
    }

    #[test]
    fn statistics_compute_population_deviation() {
        let result = statistics("host", "::1".parse().unwrap(), 2, &[1.0, 3.0]);
        assert_eq!(result.loss, 0.0);
        assert_eq!(result.rtt_min, Some(1.0));
        assert_eq!(result.rtt_max, Some(3.0));
        assert_eq!(result.rtt_avg, Some(2.0));
        assert_eq!(result.rtt_stddev, Some(1.0));
        assert_eq!(result.address, "::1");
    }

    #[test]
    fn checksum_matches_rfc_1071_example() {
        let bytes = [0x00, 0x01, 0xf2, 0x03, 0xf4, 0xf5, 0xf6, 0xf7];
        assert_eq!(checksum(&bytes), !0xddf2);
        let odd = [0x08, 0x00, 0x00, 0x00, 0x12, 0x34, 0x00, 0x01, 0xab];
        let sum = checksum(&odd);
        let mut with_sum = odd.to_vec();
        with_sum[2..4].copy_from_slice(&sum.to_be_bytes());
        assert_eq!(checksum(&with_sum), 0);
    }

    fn args(count: Option<i64>, timeout: Option<i64>) -> BoundArgs {
        let mut args = BoundArgs::new(COMMAND).with(
            PARAM_HOST,
            ArgValue::Value(Value::Text("127.0.0.1".to_owned())),
            false,
        );
        if let Some(count) = count {
            args.insert(PARAM_COUNT, ArgValue::Value(Value::Int(count)), false);
        }
        if let Some(millis) = timeout {
            let value = Value::Duration(ValueDuration::from_millis(millis));
            args.insert(PARAM_TIMEOUT, ArgValue::Value(value), false);
        }
        args
    }

    #[test]
    fn parameters_take_catalog_defaults() {
        let args = args(None, None);
        assert_eq!(count(&args).unwrap(), DEFAULT_COUNT);
        assert_eq!(timeout(&args).unwrap(), DEFAULT_TIMEOUT);
        assert_eq!(host(&args).unwrap(), "127.0.0.1");
    }

    /// Echo socket that answers from a list: a round-trip time, no reply, or
    /// an error of the check.
    struct ScriptedEcho {
        replies: Vec<Option<Duration>>,
        sequences: Vec<u16>,
    }

    impl Echo for ScriptedEcho {
        fn echo(
            &mut self,
            sequence: u16,
            _timeout: Duration,
            check: &dyn Fn() -> Result<()>,
        ) -> Result<Option<Duration>> {
            check()?;
            self.sequences.push(sequence);
            Ok(self.replies.remove(0))
        }
    }

    #[test]
    fn series_counts_replies_and_reports_progress() {
        let context = nurt_testkit::ctx::TestContext::new();
        let ctx = context.ctx();
        let mut socket = ScriptedEcho {
            replies: vec![
                Some(Duration::from_millis(10)),
                None,
                Some(Duration::from_millis(30)),
            ],
            sequences: Vec::new(),
        };
        let address = "192.0.2.1".parse().unwrap();
        let result = series(&mut socket, "host", address, 3, DEFAULT_TIMEOUT, &ctx).unwrap();
        assert_eq!(socket.sequences, [1, 2, 3]);
        assert_eq!(result.sent, 3);
        assert_eq!(result.received, 2);
        assert_eq!(result.rtt_avg, Some(20.0));
        let progress = context.events().events().len();
        assert_eq!(progress, 3);
    }

    #[test]
    fn series_stops_when_cancelled() {
        let context = nurt_testkit::ctx::TestContext::new();
        let cancel = nurt_contracts::stage::CancelToken::new();
        let ctx = context.builder().cancel(cancel.clone()).build();
        cancel.cancel(nurt_contracts::stage::CancelReason::Requested);
        let mut socket = ScriptedEcho {
            replies: vec![Some(Duration::from_millis(1))],
            sequences: Vec::new(),
        };
        let address = "192.0.2.1".parse().unwrap();
        let error = series(&mut socket, "host", address, 1, DEFAULT_TIMEOUT, &ctx).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Cancelled);
        assert!(socket.sequences.is_empty());
    }

    #[test]
    fn timeout_of_another_type_is_a_type_error() {
        let mut args = args(None, None);
        args.insert(PARAM_TIMEOUT, ArgValue::Value(Value::Int(2)), false);
        assert_eq!(timeout(&args).unwrap_err().code(), ErrorCode::Type);
    }

    #[test]
    fn zero_count_and_zero_timeout_are_invalid() {
        let error = count(&args(Some(0), None)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        let error = timeout(&args(None, Some(0))).unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
    }
}
