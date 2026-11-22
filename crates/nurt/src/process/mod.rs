// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Client of the `nurt serve` process.
//!
//! The client starts `nurt serve --stdio` as a child process, or uses a
//! stream given by the application (for example a socket of a long-running
//! server), and speaks the protocol `nurt-rpc/1`: JSON-RPC 2.0 with one
//! message per line. When the process ends, requests in flight fail with
//! [`ErrorKind::Disconnected`](crate::ErrorKind::Disconnected) and the next
//! request starts a new process.

mod connection;
mod locate;

use std::fmt;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::Value;

pub(crate) use self::connection::{Call, Connection, PROTOCOL, Reply, receive, receive_timeout};
pub(crate) use self::locate::locate;
use crate::error::{Error, ErrorKind};
use crate::response::Event;

/// Time limit of `initialize` when no request timeout is configured.
const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) type Listener = Arc<dyn Fn(Event) + Send + Sync>;

pub(crate) type Connect = Box<dyn Fn() -> io::Result<Transport> + Send + Sync>;

/// Locks a mutex; the data guarded here stays consistent when a thread
/// panics while holding the lock, so a poisoned lock is used as it is.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Byte streams to a `nurt serve` server: the output of the server and its
/// input.
///
/// # Examples
///
/// ```no_run
/// # #[cfg(unix)]
/// # fn connect() -> Result<nurt::Nurt, nurt::Error> {
/// use std::os::unix::net::UnixStream;
///
/// use nurt::{Nurt, Transport};
///
/// Nurt::builder().connect_with(|| {
///     let stream = UnixStream::connect("/home/user/.local/state/nurt/serve.sock")?;
///     Ok(Transport::new(stream.try_clone()?, stream))
/// })
/// # }
/// ```
pub struct Transport {
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    child: Option<Child>,
}

impl Transport {
    /// Transport over a stream that carries the messages of the server
    /// (`reader`) and a stream that carries the messages to it (`writer`).
    #[must_use]
    pub fn new(reader: impl Read + Send + 'static, writer: impl Write + Send + 'static) -> Self {
        Self {
            reader: Box::new(reader),
            writer: Box::new(writer),
            child: None,
        }
    }

    /// Starts `nurt serve --stdio`; the standard error of the server is
    /// inherited.
    pub(crate) fn spawn(executable: &Path) -> io::Result<Self> {
        let mut child = Command::new(executable)
            .args(["serve", "--stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let streams = child.stdout.take().zip(child.stdin.take());
        let Some((reader, writer)) = streams else {
            let _killed = child.kill();
            let _status = child.wait();
            return Err(io::Error::other(
                "standard streams of nurt serve are not available",
            ));
        };
        Ok(Self {
            reader: Box::new(reader),
            writer: Box::new(writer),
            child: Some(child),
        })
    }
}

impl fmt::Debug for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Transport")
            .field("child", &self.child.as_ref().map(Child::id))
            .finish_non_exhaustive()
    }
}

/// Holds the current connection and replaces it after a failure.
pub(crate) struct Supervisor {
    connect: Connect,
    initialize: Value,
    listener: Option<Listener>,
    timeout: Option<Duration>,
    current: Mutex<Option<Arc<Connection>>>,
}

impl Supervisor {
    /// Connects and initializes the first connection.
    pub(crate) fn start(
        connect: Connect,
        initialize: Value,
        listener: Option<Listener>,
        timeout: Option<Duration>,
    ) -> Result<Self, Error> {
        let supervisor = Self {
            connect,
            initialize,
            listener,
            timeout,
            current: Mutex::new(None),
        };
        supervisor.connection()?;
        Ok(supervisor)
    }

    /// The open connection; a closed one is replaced by a new connection.
    pub(crate) fn connection(&self) -> Result<Arc<Connection>, Error> {
        let mut current = lock(&self.current);
        if let Some(connection) = current.as_ref()
            && connection.is_open()
        {
            return Ok(Arc::clone(connection));
        }
        if let Some(stale) = current.take() {
            let _ended = stale.shutdown(Duration::ZERO);
        }
        let transport = (self.connect)().map_err(|error| {
            let kind = if error.kind() == io::ErrorKind::NotFound {
                ErrorKind::NotFound
            } else {
                ErrorKind::Io
            };
            Error::new(kind, format!("cannot start nurt serve: {error}"))
        })?;
        let connection = Arc::new(Connection::open(transport, self.listener.clone())?);
        if let Err(error) = self.initialize(&connection) {
            let _ended = connection.shutdown(Duration::ZERO);
            return Err(error);
        }
        *current = Some(Arc::clone(&connection));
        Ok(connection)
    }

    fn initialize(&self, connection: &Connection) -> Result<(), Error> {
        let result = connection.call(
            "initialize",
            self.initialize.clone(),
            Some(self.timeout.unwrap_or(INITIALIZE_TIMEOUT)),
        )?;
        let protocol = result
            .get("protocol")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if connection::is_compatible(protocol) {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::Incompatible,
                format!(
                    "nurt serve uses protocol `{protocol}`, this client needs `{PROTOCOL}`; \
                     install a matching version of nurt"
                ),
            ))
        }
    }

    /// Sends a request and waits for its response.
    pub(crate) fn call(&self, method: &str, params: Value) -> Result<Value, Error> {
        self.connection()?.call(method, params, self.timeout)
    }

    /// Ends the current connection.
    pub(crate) fn shutdown(&self, grace: Duration) -> Result<(), Error> {
        let current = lock(&self.current).take();
        current.map_or(Ok(()), |connection| connection.shutdown(grace))
    }
}
