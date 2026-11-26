// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Process trees of Unix systems: a new session and process group per run.
//!
//! The main process becomes the leader of a new session before `exec`, so
//! it has no controlling terminal and every process it starts belongs to
//! the process group with its identifier. The leader is observed with
//! `waitid(WNOWAIT)` and collected only after the rest of the group was
//! ended, so the group identifier cannot be reused while signals are sent
//! to it.

use std::fs::File;
use std::io::{self, PipeReader};
use std::os::fd::RawFd;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use libc::{c_int, rlim_t};
use nurt_contracts::error::Result;
use nurt_contracts::record::{LimitHit, Signal};

use super::{Exit, Finished, Streams, ceil_seconds, io_error, spawn_error, unsupported_signal};
use crate::iface::{Priority, ProcessLimits, ResourceUsage, SpawnRequest, StdinSource};

#[cfg(target_os = "linux")]
use super::linux::Membership;
#[cfg(target_os = "macos")]
use super::macos::Membership;

/// Niceness of [`Priority::Low`].
const LOW_NICENESS: c_int = 10;
/// Niceness of [`Priority::High`]; applied only with the privilege.
const HIGH_NICENESS: c_int = -5;
/// Highest niceness accepted by [`Priority::Nice`].
const MAX_NICENESS: u8 = 19;
/// Signals 1 to 31 have dispositions that survive `exec` when ignored.
const STANDARD_SIGNALS: c_int = 32;
/// First descriptor that is not standard input, output or error.
const FIRST_EXTRA_DESCRIPTOR: c_int = 3;
/// Upper bound of the descriptor scan when `close_range` is not available.
const DESCRIPTOR_SCAN_LIMIT: c_int = 65_536;
/// The address space limit is this multiple of the memory limit. Programs
/// reserve more address space than they use; the measured resident memory
/// of the tree is the limit, the address space limit only bounds a sudden
/// allocation between two measurements.
const ADDRESS_SPACE_FACTOR: u64 = 4;

/// Bytes in one kibibyte.
const KIBIBYTE: u64 = 1024;

#[cfg(all(target_os = "linux", target_env = "gnu"))]
type Resource = libc::__rlimit_resource_t;
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
type Resource = c_int;

/// Signals with names common to all systems.
const SIGNALS: [(Signal, c_int); 11] = [
    (Signal::Terminate, libc::SIGTERM),
    (Signal::Kill, libc::SIGKILL),
    (Signal::Interrupt, libc::SIGINT),
    (Signal::Hangup, libc::SIGHUP),
    (Signal::Quit, libc::SIGQUIT),
    (Signal::Abort, libc::SIGABRT),
    (Signal::Segv, libc::SIGSEGV),
    (Signal::Bus, libc::SIGBUS),
    (Signal::Fpe, libc::SIGFPE),
    (Signal::Ill, libc::SIGILL),
    (Signal::Pipe, libc::SIGPIPE),
];

/// Name of a signal number; [`Signal::Other`] for signals without a common
/// name.
pub(crate) fn signal_name(number: c_int) -> Signal {
    SIGNALS
        .iter()
        .find(|(_, value)| *value == number)
        .map_or(Signal::Other, |(signal, _)| *signal)
}

/// Number of a named signal.
pub(super) fn signal_number(signal: Signal) -> Result<c_int> {
    SIGNALS
        .iter()
        .find(|(name, _)| *name == signal)
        .map(|(_, number)| *number)
        .ok_or_else(|| unsupported_signal(signal))
}

/// Settings applied in the child between `fork` and `exec`. Holds only
/// plain values, so applying it does not allocate.
#[derive(Debug, Clone, Copy)]
struct ChildSetup {
    niceness: Option<c_int>,
    cpu_seconds: Option<(rlim_t, rlim_t)>,
    address_space: Option<rlim_t>,
    cgroup_procs: Option<RawFd>,
}

impl ChildSetup {
    fn new(limits: &ProcessLimits, membership: &Membership) -> Self {
        let niceness = match limits.priority {
            Priority::Low => Some(LOW_NICENESS),
            Priority::Normal => None,
            Priority::High => Some(HIGH_NICENESS),
            Priority::Nice(value) => Some(c_int::from(value.min(MAX_NICENESS))),
        };
        let cpu_seconds = limits.max_cpu.map(|limit| {
            let soft = ceil_seconds(limit).max(1);
            let hard = soft.saturating_add(ceil_seconds(limits.kill_grace).max(1));
            (to_rlim(soft), to_rlim(hard))
        });
        let address_space = match (limits.max_memory, membership.enforces_memory()) {
            (Some(bytes), false) => Some(to_rlim(bytes.saturating_mul(ADDRESS_SPACE_FACTOR))),
            _ => None,
        };
        Self {
            niceness,
            cpu_seconds,
            address_space,
            cgroup_procs: membership.join_descriptor(),
        }
    }

    /// Runs in the child after `fork`. Only async-signal-safe calls.
    fn apply(self) -> io::Result<()> {
        // SAFETY: `setsid` takes no arguments and only changes the session of
        // the calling process.
        if unsafe { libc::setsid() } == -1 {
            return Err(io::Error::last_os_error());
        }
        reset_signals();
        if let Some(descriptor) = self.cgroup_procs {
            let request = b"0";
            // SAFETY: the descriptor is open in the child (inherited through
            // `fork`) and the buffer outlives the call.
            let written =
                unsafe { libc::write(descriptor, request.as_ptr().cast(), request.len()) };
            if written < 0 {
                return Err(io::Error::last_os_error());
            }
        }
        if let Some(niceness) = self.niceness {
            // SAFETY: plain system call on the calling process. Without the
            // privilege for a negative value the call fails and the program
            // keeps the inherited priority.
            unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, niceness) };
        }
        if let Some((soft, hard)) = self.cpu_seconds {
            lower_limit(libc::RLIMIT_CPU, soft, hard)?;
        }
        if let Some(bytes) = self.address_space {
            lower_limit(libc::RLIMIT_AS, bytes, bytes)?;
        }
        mark_descriptors_close_on_exec();
        Ok(())
    }
}

fn to_rlim(value: u64) -> rlim_t {
    rlim_t::try_from(value).unwrap_or(libc::RLIM_INFINITY)
}

/// Restores the default disposition of the standard signals and an empty
/// signal mask; ignored signals and the mask would otherwise survive `exec`.
fn reset_signals() {
    for number in 1..STANDARD_SIGNALS {
        if number == libc::SIGKILL || number == libc::SIGSTOP {
            continue;
        }
        // SAFETY: `SIG_DFL` is a valid disposition for every catchable
        // signal; invalid numbers make the call fail without effect.
        unsafe { libc::signal(number, libc::SIG_DFL) };
    }
    let mut empty = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
    // SAFETY: `sigemptyset` initializes the set before `sigprocmask` reads it.
    unsafe {
        libc::sigemptyset(empty.as_mut_ptr());
        libc::sigprocmask(libc::SIG_SETMASK, empty.as_ptr(), std::ptr::null_mut());
    }
}

/// Lowers a resource limit; never raises the inherited hard limit.
fn lower_limit(resource: Resource, soft: rlim_t, hard: rlim_t) -> io::Result<()> {
    let mut current = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `current` is a valid, writable `rlimit`.
    if unsafe { libc::getrlimit(resource, &raw mut current) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let hard = hard.min(current.rlim_max);
    let requested = libc::rlimit {
        rlim_cur: soft.min(hard),
        rlim_max: hard,
    };
    // SAFETY: `requested` is a valid `rlimit` that lives for the call.
    if unsafe { libc::setrlimit(resource, &raw const requested) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Keeps descriptors other than 0, 1 and 2 from reaching the program. The
/// descriptors are marked close-on-exec instead of closed, so the pipe that
/// reports a failed `exec` to the parent keeps working.
fn mark_descriptors_close_on_exec() {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: `close_range` with `CLOSE_RANGE_CLOEXEC` only changes
        // descriptor flags of the calling process.
        let marked = unsafe {
            libc::syscall(
                libc::SYS_close_range,
                FIRST_EXTRA_DESCRIPTOR.cast_unsigned(),
                c_int::MAX.cast_unsigned(),
                libc::CLOSE_RANGE_CLOEXEC,
            )
        };
        if marked == 0 {
            return;
        }
    }
    // SAFETY: `sysconf` has no preconditions.
    let open_max = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) };
    let last = c_int::try_from(open_max)
        .unwrap_or(DESCRIPTOR_SCAN_LIMIT)
        .clamp(FIRST_EXTRA_DESCRIPTOR, DESCRIPTOR_SCAN_LIMIT);
    for descriptor in FIRST_EXTRA_DESCRIPTOR..last {
        // SAFETY: `fcntl` with `F_GETFD` and `F_SETFD` only reads and sets
        // descriptor flags; closed descriptors make the call fail.
        unsafe {
            let flags = libc::fcntl(descriptor, libc::F_GETFD);
            if flags >= 0 && flags & libc::FD_CLOEXEC == 0 {
                libc::fcntl(descriptor, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }
}

/// Started program in its own session and process group.
pub(crate) struct Tree {
    pid: libc::pid_t,
    cpu_limit: Option<Duration>,
    membership: Membership,
    /// The leader was collected or its collection failed: the group
    /// identifier may name an unrelated group and is never signalled again.
    collected: AtomicBool,
}

impl Tree {
    /// Starts the program of `request` in a new session.
    pub(crate) fn spawn(request: &SpawnRequest) -> Result<(Self, Streams)> {
        if !request.cwd.is_dir() {
            return Err(spawn_error(
                &request.program,
                &io::Error::new(
                    io::ErrorKind::NotFound,
                    format!(
                        "the working directory {} does not exist",
                        request.cwd.display()
                    ),
                ),
            ));
        }
        let membership = Membership::prepare(&request.limits);
        let setup = ChildSetup::new(&request.limits, &membership);
        let mut command = Command::new(&request.program);
        command
            .args(&request.args)
            .current_dir(&request.cwd)
            .env_clear()
            .envs(request.env.iter().map(|(name, value)| (name, value)));
        let piped_stdin = matches!(request.stdin, StdinSource::Bytes(_));
        command.stdin(stdin(&request.stdin, &request.program)?);
        let (stdout, stderr) = output_pipes(&mut command, request.merge_stderr)
            .map_err(|error| spawn_error(&request.program, &error))?;
        // SAFETY: `ChildSetup::apply` makes only async-signal-safe calls and
        // does not allocate, as required between `fork` and `exec`.
        unsafe {
            command.pre_exec(move || setup.apply());
        }
        let mut child = command
            .spawn()
            .map_err(|error| spawn_error(&request.program, &error))?;
        // The command holds the write ends of the output pipes; the program
        // must be their only writer for the end of output to be seen.
        drop(command);
        let stdin = if piped_stdin {
            child
                .stdin
                .take()
                .map(|pipe| Box::new(pipe) as Box<dyn io::Write + Send>)
        } else {
            None
        };
        let pid = libc::pid_t::try_from(child.id()).unwrap_or(libc::pid_t::MAX);
        let tree = Self {
            pid,
            cpu_limit: request.limits.max_cpu,
            membership,
            collected: AtomicBool::new(false),
        };
        let streams = Streams {
            stdin,
            stdout: Box::new(stdout),
            stderr: stderr.map(|reader| Box::new(reader) as Box<dyn io::Read + Send>),
        };
        Ok((tree, streams))
    }

    /// Process identifier of the main process.
    pub(crate) fn pid(&self) -> u32 {
        self.pid.cast_unsigned()
    }

    /// Whether the group identifier is no longer signalled.
    #[cfg(test)]
    pub(crate) fn is_collected(&self) -> bool {
        self.collected.load(Ordering::Acquire)
    }

    /// Blocks until the main process ends, without collecting it.
    pub(crate) fn wait_exit(&self) -> Result<()> {
        loop {
            let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
            // SAFETY: `info` is a valid, writable `siginfo_t`; `WNOWAIT`
            // leaves the process waitable for `finish`.
            let status = unsafe {
                libc::waitid(
                    libc::P_PID,
                    self.pid.cast_unsigned(),
                    info.as_mut_ptr(),
                    libc::WEXITED | libc::WNOWAIT,
                )
            };
            if status == 0 {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(io_error("wait for the program", &error));
            }
        }
    }

    /// Sends `signal` to every process of the group.
    pub(crate) fn signal(&self, signal: Signal) -> Result<()> {
        self.send(signal_number(signal)?)
    }

    /// Ends every process of the group at once.
    pub(crate) fn kill(&self) -> Result<()> {
        self.membership.kill();
        self.send(libc::SIGKILL)
    }

    /// Stops every process of the group.
    pub(crate) fn suspend(&self) -> Result<()> {
        self.send(libc::SIGSTOP)
    }

    /// Continues every process of the group.
    pub(crate) fn resume(&self) -> Result<()> {
        self.send(libc::SIGCONT)
    }

    /// Current processor time and resident memory of the group.
    pub(crate) fn sample(&self) -> Result<ResourceUsage> {
        self.membership.sample(self.pid)
    }

    /// Limit enforced by the kernel since the start.
    pub(crate) fn limit_hit(&self) -> Option<LimitHit> {
        self.membership.limit_hit()
    }

    /// Ends the remaining members of the group, collects the main process
    /// and reports how it ended. Call once, after [`Tree::wait_exit`].
    pub(crate) fn finish(&self) -> Result<Finished> {
        // The leader is not collected yet, so the group identifier still
        // names this group and cannot reach an unrelated process. Members
        // that changed their credentials cannot be signalled; the leader is
        // collected regardless.
        self.membership.kill();
        self.send(libc::SIGKILL).ok();
        self.collected.store(true, Ordering::Release);
        let mut status: c_int = 0;
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        loop {
            // SAFETY: `status` and `usage` are valid, writable locations.
            let collected =
                unsafe { libc::wait4(self.pid, &raw mut status, 0, usage.as_mut_ptr()) };
            if collected == self.pid {
                break;
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(io_error("collect the program", &error));
            }
        }
        // SAFETY: `wait4` returned the process, so it filled `usage`.
        let usage = unsafe { usage.assume_init() };
        let exit = decode_status(status);
        let mut measured = ResourceUsage {
            cpu_time: timeval(usage.ru_utime).saturating_add(timeval(usage.ru_stime)),
            max_rss: max_rss_bytes(usage.ru_maxrss),
        };
        if let Some(cpu_time) = self.membership.cpu_time() {
            measured.cpu_time = measured.cpu_time.max(cpu_time);
        }
        let limit_hit = self
            .membership
            .limit_hit()
            .or_else(|| self.cpu_limit_signal(exit, measured.cpu_time));
        self.membership.release();
        Ok(Finished {
            exit,
            usage: measured,
            limit_hit,
        })
    }

    /// The processor time limit of the main process ends it with `SIGXCPU`,
    /// or with `SIGKILL` at the hard limit when `SIGXCPU` is handled.
    fn cpu_limit_signal(&self, exit: Exit, cpu_time: Duration) -> Option<LimitHit> {
        let limit = self.cpu_limit?;
        match exit {
            Exit::Signal {
                number: Some(libc::SIGXCPU),
                ..
            } => Some(LimitHit::Cpu),
            Exit::Signal {
                number: Some(libc::SIGKILL),
                ..
            } if cpu_time >= limit => Some(LimitHit::Cpu),
            _ => None,
        }
    }

    fn send(&self, number: c_int) -> Result<()> {
        if self.collected.load(Ordering::Acquire) {
            return Ok(());
        }
        // SAFETY: a negative identifier addresses the process group created
        // for this tree; the leader is not collected before `finish`.
        if unsafe { libc::kill(-self.pid, number) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        Err(io_error("signal the program", &error))
    }
}

fn stdin(source: &StdinSource, program: &std::path::Path) -> Result<Stdio> {
    match source {
        StdinSource::Null => Ok(Stdio::null()),
        StdinSource::Bytes(_) => Ok(Stdio::piped()),
        StdinSource::File(path) => File::open(path).map(Stdio::from).map_err(|error| {
            spawn_error(
                program,
                &io::Error::new(
                    error.kind(),
                    format!("cannot open the input file {}: {error}", path.display()),
                ),
            )
        }),
    }
}

fn output_pipes(
    command: &mut Command,
    merge_stderr: bool,
) -> io::Result<(PipeReader, Option<PipeReader>)> {
    let (stdout, stdout_writer) = io::pipe()?;
    let stderr = if merge_stderr {
        command.stderr(stdout_writer.try_clone()?);
        None
    } else {
        let (reader, writer) = io::pipe()?;
        command.stderr(writer);
        Some(reader)
    };
    command.stdout(stdout_writer);
    Ok((stdout, stderr))
}

fn decode_status(status: c_int) -> Exit {
    if libc::WIFSIGNALED(status) {
        let number = libc::WTERMSIG(status);
        Exit::Signal {
            signal: signal_name(number),
            number: Some(number),
        }
    } else {
        Exit::Code(libc::WEXITSTATUS(status))
    }
}

fn timeval(value: libc::timeval) -> Duration {
    let seconds = u64::try_from(value.tv_sec).unwrap_or(0);
    let micros = u64::try_from(value.tv_usec).unwrap_or(0);
    Duration::from_secs(seconds).saturating_add(Duration::from_micros(micros))
}

/// `ru_maxrss` is in kibibytes on Linux and in bytes on macOS.
fn max_rss_bytes(value: libc::c_long) -> u64 {
    let value = u64::try_from(value).unwrap_or(0);
    if cfg!(target_os = "macos") {
        value
    } else {
        value.saturating_mul(KIBIBYTE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_names_round_trip_through_numbers() {
        for (signal, number) in SIGNALS {
            assert_eq!(signal_name(number), signal);
            assert_eq!(signal_number(signal).ok(), Some(number));
        }
    }

    #[test]
    fn signals_without_common_name_are_other() {
        assert_eq!(signal_name(libc::SIGUSR1), Signal::Other);
        assert_eq!(signal_name(libc::SIGXCPU), Signal::Other);
    }

    #[test]
    fn signal_other_cannot_be_sent() {
        let error = signal_number(Signal::Other).err();
        assert_eq!(
            error.map(|error| error.code()),
            Some(nurt_contracts::error::ErrorCode::Unsupported)
        );
    }

    #[test]
    fn exit_status_decodes_code_and_signal() {
        assert_eq!(decode_status(3 << 8), Exit::Code(3));
        assert_eq!(
            decode_status(libc::SIGSEGV),
            Exit::Signal {
                signal: Signal::Segv,
                number: Some(libc::SIGSEGV)
            }
        );
    }

    #[test]
    fn priority_maps_to_niceness() {
        let limits = |priority| ProcessLimits {
            timeout: None,
            idle_timeout: None,
            kill_grace: Duration::from_secs(1),
            max_cpu: None,
            max_memory: None,
            priority,
        };
        let membership = Membership::prepare(&limits(Priority::Normal));
        let niceness = |priority| ChildSetup::new(&limits(priority), &membership).niceness;
        assert_eq!(niceness(Priority::Low), Some(LOW_NICENESS));
        assert_eq!(niceness(Priority::Normal), None);
        assert_eq!(niceness(Priority::High), Some(HIGH_NICENESS));
        assert_eq!(niceness(Priority::Nice(40)), Some(19));
    }

    #[test]
    fn processor_limit_rounds_up_and_adds_grace() {
        let limits = ProcessLimits {
            timeout: None,
            idle_timeout: None,
            kill_grace: Duration::from_millis(1500),
            max_cpu: Some(Duration::from_millis(200)),
            max_memory: None,
            priority: Priority::Normal,
        };
        let membership = Membership::prepare(&limits);
        let setup = ChildSetup::new(&limits, &membership);
        assert_eq!(setup.cpu_seconds, Some((1, 3)));
    }

    #[test]
    fn processor_limit_of_zero_still_allows_one_second_and_the_grace_time() {
        let limits = ProcessLimits {
            timeout: None,
            idle_timeout: None,
            kill_grace: Duration::ZERO,
            max_cpu: Some(Duration::ZERO),
            max_memory: None,
            priority: Priority::Normal,
        };
        let membership = Membership::prepare(&limits);
        let setup = ChildSetup::new(&limits, &membership);
        assert_eq!(setup.cpu_seconds, Some((1, 2)));
    }

    #[test]
    fn the_address_space_limit_saturates() {
        let limits = ProcessLimits {
            timeout: None,
            idle_timeout: None,
            kill_grace: Duration::from_secs(1),
            max_cpu: None,
            max_memory: Some(u64::MAX),
            priority: Priority::Normal,
        };
        let membership = Membership::prepare(&limits);
        let setup = ChildSetup::new(&limits, &membership);
        if membership.enforces_memory() {
            assert_eq!(setup.address_space, None);
        } else {
            assert_eq!(setup.address_space, Some(to_rlim(u64::MAX)));
        }
    }

    #[test]
    fn resource_values_of_the_system_are_converted_without_overflow() {
        let time = |tv_sec, tv_usec| timeval(libc::timeval { tv_sec, tv_usec });
        assert_eq!(time(3, 500_000), Duration::from_millis(3500));
        assert_eq!(time(0, 0), Duration::ZERO);
        assert_eq!(time(-1, -1), Duration::ZERO);
        assert_eq!(max_rss_bytes(-1), 0);
        assert_eq!(max_rss_bytes(0), 0);
        if cfg!(target_os = "macos") {
            assert_eq!(max_rss_bytes(4096), 4096);
        } else {
            assert_eq!(max_rss_bytes(4), 4096);
            assert_eq!(max_rss_bytes(libc::c_long::MAX), u64::MAX);
        }
    }

    #[test]
    fn exit_status_decodes_largest_codes_and_signals_without_a_common_name() {
        assert_eq!(decode_status(0), Exit::Code(0));
        assert_eq!(decode_status(255 << 8), Exit::Code(255));
        assert_eq!(
            decode_status(libc::SIGKILL),
            Exit::Signal {
                signal: Signal::Kill,
                number: Some(libc::SIGKILL)
            }
        );
        assert_eq!(
            decode_status(libc::SIGUSR1),
            Exit::Signal {
                signal: Signal::Other,
                number: Some(libc::SIGUSR1)
            }
        );
    }
}
