// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Control channel of the service: a Unix socket `service.sock` in the
//! state directory, or the named pipe `\\.\pipe\nurt-service-<SID>` on
//! Windows, both reachable only by the account of the service.
//!
//! The accepting thread and the handling of `ping` and `status` never wait
//! for a module, so `ping` answers within its time limit even when a module
//! is blocked. Every connection has its own thread.

pub mod client;
pub mod protocol;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

use std::io::{self, BufReader, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use nurt_contracts::error::Error;
use nurt_contracts::stage::CancelToken;
use serde_json::{Value, json};

use crate::hub::{Closer, Hub, SharedWriter, write_line};
use crate::module::ModuleRequest;
use crate::paths::ServicePaths;
use protocol::{LineRead, Request};

const ACCEPT_RETRY: Duration = Duration::from_millis(100);
const REGISTER_ACTION_OP: &str = "schedule/register_action";

/// Reading half of a connection with an adjustable idle limit.
pub(crate) trait TimedRead: Read + Send {
    /// Sets the longest wait for incoming data; `None` waits without limit.
    fn set_idle(&mut self, limit: Option<Duration>) -> io::Result<()>;
}

/// Connection accepted by a listener.
pub(crate) struct Accepted {
    pub(crate) reader: Box<dyn TimedRead>,
    pub(crate) writer: Box<dyn Write + Send>,
    pub(crate) closer: Closer,
}

/// Listening endpoint of the control channel.
pub(crate) trait Listener: Send + Sync {
    /// Waits for the next connection.
    fn accept(&self) -> io::Result<Accepted>;

    /// Makes a blocked [`accept`](Self::accept) return.
    fn wake(&self);

    /// Removes the endpoint from the file system, where it has a path.
    fn remove(&self);
}

/// Why a request for a module was not handled by it.
#[derive(Debug)]
pub(crate) enum RouteError {
    /// The module is not hosted or not running.
    Unavailable(String),
    /// The module reported an error.
    Module(Error),
}

/// Operations of the host invoked by the control channel.
pub(crate) trait Control: Send + Sync {
    fn ping(&self) -> Value;
    fn status(&self) -> Value;
    fn reload(&self) -> Result<Value, Error>;
    fn hosts(&self, module: &str) -> bool;
    fn module_request(&self, module: &str, request: ModuleRequest) -> Result<Value, RouteError>;
    fn client_closed(&self, client: &str);
}

/// Binds the control channel for the state directory.
///
/// Must be called only while the instance lock is held: on Unix an existing
/// socket file is a leftover of a previous service and is removed.
pub(crate) fn bind(paths: &ServicePaths) -> io::Result<Arc<dyn Listener>> {
    #[cfg(unix)]
    {
        unix::SocketListener::bind(&paths.socket_file()).map(|l| Arc::new(l) as Arc<dyn Listener>)
    }
    #[cfg(windows)]
    {
        let _ = paths;
        windows::PipeListener::bind().map(|l| Arc::new(l) as Arc<dyn Listener>)
    }
}

/// Opens a client connection to the control channel.
pub(crate) fn connect(
    paths: &ServicePaths,
) -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    #[cfg(unix)]
    {
        unix::connect(&paths.socket_file())
    }
    #[cfg(windows)]
    {
        let _ = paths;
        windows::connect()
    }
}

/// Accepting side of the control channel.
pub(crate) struct Server {
    listener: Arc<dyn Listener>,
    hub: Arc<Hub>,
    stop: CancelToken,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    /// Starts the accepting thread.
    pub(crate) fn start(
        listener: Arc<dyn Listener>,
        hub: Arc<Hub>,
        control: Arc<dyn Control>,
    ) -> io::Result<Self> {
        let stop = CancelToken::new();
        let thread = {
            let listener = Arc::clone(&listener);
            let hub = Arc::clone(&hub);
            let stop = stop.clone();
            thread::Builder::new()
                .name("nurt-service-accept".to_owned())
                .spawn(move || accept_loop(&*listener, &hub, &control, &stop))?
        };
        Ok(Self {
            listener,
            hub,
            stop,
            thread: Some(thread),
        })
    }

    /// Stops accepting, closes every connection and removes the endpoint.
    pub(crate) fn shutdown(mut self) {
        self.stop
            .cancel(nurt_contracts::stage::CancelReason::Shutdown);
        self.listener.wake();
        if let Some(thread) = self.thread.take() {
            let _ended = thread.join();
        }
        self.hub.close_all();
        self.listener.remove();
    }
}

fn accept_loop(
    listener: &dyn Listener,
    hub: &Arc<Hub>,
    control: &Arc<dyn Control>,
    stop: &CancelToken,
) {
    let sequence = AtomicU64::new(1);
    while !stop.is_cancelled() {
        match listener.accept() {
            Ok(accepted) => {
                if stop.is_cancelled() {
                    (accepted.closer)();
                    break;
                }
                let client = format!("c{}", sequence.fetch_add(1, Ordering::Relaxed));
                let hub = Arc::clone(hub);
                let control = Arc::clone(control);
                let spawned = thread::Builder::new()
                    .name(format!("nurt-service-{client}"))
                    .spawn(move || serve_connection(&client, accepted, &hub, &*control));
                if spawned.is_err() {
                    let _ = stop.wait_timeout(ACCEPT_RETRY);
                }
            }
            Err(_) => {
                let _ = stop.wait_timeout(ACCEPT_RETRY);
            }
        }
    }
}

fn serve_connection(client: &str, accepted: Accepted, hub: &Hub, control: &dyn Control) {
    let Accepted {
        mut reader,
        writer,
        closer,
    } = accepted;
    let writer: SharedWriter = Arc::new(Mutex::new(writer));
    hub.attach(client, Arc::clone(&writer), closer);
    let mut idle = Some(protocol::IDLE_TIMEOUT);
    if reader.set_idle(idle).is_ok() {
        let mut reader = BufReader::new(reader);
        loop {
            let line = match protocol::read_line(&mut reader, protocol::MAX_LINE) {
                Ok(LineRead::Line(line)) => line,
                Ok(LineRead::TooLong) => {
                    let error = protocol::error_object(
                        protocol::ERR_LINE_TOO_LONG,
                        "the request exceeds 65536 bytes; the connection is closed",
                    );
                    let _ = write_line(&writer, &protocol::failure_line(&Value::Null, &error));
                    break;
                }
                Ok(LineRead::Eof) | Err(_) => break,
            };
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let response = handle_line(client, &line, hub, control);
            if write_line(&writer, &response).is_err() {
                break;
            }
            if idle.is_some() && hub.is_listening(client) {
                idle = None;
                if reader.get_mut().set_idle(None).is_err() {
                    break;
                }
            }
        }
    }
    hub.detach(client);
    control.client_closed(client);
}

/// Handles one request line and returns the response line.
pub(crate) fn handle_line(client: &str, line: &[u8], hub: &Hub, control: &dyn Control) -> Vec<u8> {
    let request = match protocol::parse_request(line) {
        Ok(request) => request,
        Err(error) => return protocol::failure_line(&error.id, &error.error),
    };
    match dispatch(client, &request, hub, control) {
        Ok(result) => protocol::success_line(&request.id, &result),
        Err(error) => protocol::failure_line(&request.id, &error),
    }
}

fn dispatch(
    client: &str,
    request: &Request,
    hub: &Hub,
    control: &dyn Control,
) -> Result<Value, Value> {
    let op = request.op.as_str();
    match op {
        protocol::OP_PING => Ok(control.ping()),
        protocol::OP_STATUS => Ok(control.status()),
        protocol::OP_RELOAD => control
            .reload()
            .map_err(|error| protocol::language_error(&error)),
        protocol::OP_SUBSCRIBE => {
            hub.subscribe(client);
            Ok(json!({ "subscribed": true }))
        }
        _ if protocol::module_of(op).is_some_and(|module| control.hosts(module)) => {
            let module = protocol::module_of(op).unwrap_or_default();
            let module_request = ModuleRequest::new(op, request.body.clone()).from_client(client);
            let result =
                control
                    .module_request(module, module_request)
                    .map_err(|error| match error {
                        RouteError::Unavailable(message) => {
                            protocol::error_object(protocol::ERR_MODULE_UNAVAILABLE, &message)
                        }
                        RouteError::Module(error) => protocol::language_error(&error),
                    })?;
            if op == REGISTER_ACTION_OP {
                hub.mark_listening(client);
            }
            Ok(result)
        }
        _ => Err(protocol::with_field(
            protocol::error_object(
                protocol::ERR_UNKNOWN_OP,
                &format!("unknown operation `{op}`; see `ops` for the operations of the service"),
            ),
            "ops",
            json!(protocol::HOST_OPS),
        )),
    }
}
