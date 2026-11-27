// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-service::signals
// Responsibility:   Termination and reload signals on Unix.
// Dependencies:     nix
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Termination and reload signals on Unix.
//!
//! `SIGTERM`, `SIGINT` and `SIGHUP` are blocked in the thread that runs the
//! service before any other service thread starts, so every service thread
//! inherits the mask, and one thread receives them with `sigwait`. No
//! asynchronous signal handler is installed.

use std::io;
use std::os::unix::thread::JoinHandleExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

use nix::sys::pthread::pthread_kill;
use nix::sys::signal::{SigSet, SigmaskHow, Signal, pthread_sigmask};

/// Signals blocked for the lifetime of the service; the previous mask of the
/// calling thread is restored on drop.
pub(crate) struct SignalGuard {
    set: SigSet,
    previous: SigSet,
}

impl SignalGuard {
    /// Blocks the signals in the calling thread.
    pub(crate) fn block() -> io::Result<Self> {
        let mut set = SigSet::empty();
        for signal in [Signal::SIGTERM, Signal::SIGINT, Signal::SIGHUP] {
            set.add(signal);
        }
        let mut previous = SigSet::empty();
        pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&set), Some(&mut previous))
            .map_err(io::Error::from)?;
        Ok(Self { set, previous })
    }

    /// Starts the thread that waits for the signals: `SIGTERM` and `SIGINT`
    /// call `on_stop`, `SIGHUP` calls `on_reload`.
    pub(crate) fn spawn<S, R>(self, on_stop: S, on_reload: R) -> io::Result<SignalThread>
    where
        S: Fn() + Send + 'static,
        R: Fn() + Send + 'static,
    {
        let finished = Arc::new(AtomicBool::new(false));
        let set = self.set;
        let done = Arc::clone(&finished);
        let thread = thread::Builder::new()
            .name("nurt-service-signals".to_owned())
            .spawn(move || {
                while let Ok(signal) = set.wait() {
                    if done.load(Ordering::Acquire) {
                        break;
                    }
                    match signal {
                        Signal::SIGHUP => on_reload(),
                        _ => on_stop(),
                    }
                }
            })?;
        Ok(SignalThread {
            thread,
            finished,
            _guard: self,
        })
    }
}

impl Drop for SignalGuard {
    fn drop(&mut self) {
        let _ = self.previous.thread_set_mask();
    }
}

/// Thread receiving the signals.
pub(crate) struct SignalThread {
    thread: JoinHandle<()>,
    finished: Arc<AtomicBool>,
    _guard: SignalGuard,
}

impl SignalThread {
    /// Ends the thread and restores the signal mask of the calling thread.
    pub(crate) fn finish(self) {
        self.finished.store(true, Ordering::Release);
        // The handle is an integer in `std` and a pointer in the type of `nix`
        // on musl, so the cast infers its target type.
        if pthread_kill(self.thread.as_pthread_t() as _, Signal::SIGHUP).is_ok() {
            let _ = self.thread.join();
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
