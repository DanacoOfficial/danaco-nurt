// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Remote command channel bridged to blocking reads and writes.
//!
//! A task on the runtime thread moves data between the `exec` channel and
//! two bounded queues. [`ChannelReader`] and [`ChannelWriter`] block on
//! those queues, so `gix-protocol` reads and writes the remote command like
//! the pipes of a local process. Standard error, the exit status and an exit
//! signal of the remote command are kept in a [`Status`] shared with the
//! caller. Cancellation and the deadline close the channel, which ends
//! blocked reads and writes with an error.

use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex, MutexGuard};

use nurt_contracts::error::{Error, ErrorCode};
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf};
use tokio::sync::mpsc;

use super::session::{Limits, SessionHandle};

/// Chunks queued in each direction before the producer waits.
const QUEUE_DEPTH: usize = 16;

/// Extended data type of standard error (RFC 4254, section 5.2).
const STDERR: u32 = 1;

/// Bytes of standard error kept for messages.
const MAX_STDERR: usize = 64 * 1024;

/// What is known about the remote command.
#[derive(Debug, Default)]
pub(crate) struct RemoteStatus {
    pub(crate) exit_status: Option<u32>,
    pub(crate) signal: Option<String>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) refused: bool,
    pub(crate) aborted: Option<Error>,
}

impl RemoteStatus {
    fn push_stderr(&mut self, data: &[u8]) {
        let room = MAX_STDERR.saturating_sub(self.stderr.len());
        self.stderr.extend_from_slice(&data[..data.len().min(room)]);
    }

    /// Last non-empty line of standard error.
    pub(crate) fn last_stderr_line(&self) -> Option<String> {
        String::from_utf8_lossy(&self.stderr)
            .lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .map(str::to_owned)
    }

    /// Error to report when the command ended abnormally.
    pub(crate) fn failure(&self, command: &str) -> Option<Error> {
        if let Some(error) = &self.aborted {
            return Some(error.clone());
        }
        let detail = self
            .last_stderr_line()
            .map(|line| format!(": {line}"))
            .unwrap_or_default();
        let message = if self.refused {
            format!("server refused to run `{command}`{detail}")
        } else if let Some(signal) = &self.signal {
            format!("`{command}` ended on the server with signal {signal}{detail}")
        } else {
            match self.exit_status {
                Some(code) if code != 0 => {
                    format!("`{command}` failed on the server with exit status {code}{detail}")
                }
                _ => return None,
            }
        };
        Some(classify(message, &String::from_utf8_lossy(&self.stderr)))
    }
}

fn classify(message: String, stderr: &str) -> Error {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("permission denied") || lower.contains("access denied") {
        Error::new(ErrorCode::GitAuth, message).with_problem("rejected")
    } else if lower.contains("not found")
        || lower.contains("does not appear to be a git repository")
        || lower.contains("does not exist")
    {
        Error::new(ErrorCode::GitProtocol, message).with_problem("repository_not_found")
    } else {
        Error::new(ErrorCode::GitProtocol, message).with_problem("remote_command_failed")
    }
}

/// Shared [`RemoteStatus`] of one channel.
#[derive(Debug, Clone, Default)]
pub(crate) struct Status(Arc<Mutex<RemoteStatus>>);

impl Status {
    /// Locks the status; a poisoned lock still yields the data.
    pub(crate) fn lock(&self) -> MutexGuard<'_, RemoteStatus> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Blocking reader of the standard output of the remote command.
pub(crate) struct ChannelReader {
    queue: mpsc::Receiver<Vec<u8>>,
    chunk: Vec<u8>,
    position: usize,
    status: Status,
    command: String,
}

impl Read for ChannelReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        while self.position >= self.chunk.len() {
            match self.queue.blocking_recv() {
                Some(chunk) => {
                    self.chunk = chunk;
                    self.position = 0;
                }
                None => return self.end(),
            }
        }
        let available = &self.chunk[self.position..];
        let count = available.len().min(buf.len());
        buf[..count].copy_from_slice(&available[..count]);
        self.position += count;
        Ok(count)
    }
}

impl ChannelReader {
    fn end(&self) -> io::Result<usize> {
        match self.status.lock().failure(&self.command) {
            Some(error) => Err(io_error(&error)),
            None => Ok(0),
        }
    }
}

/// Blocking writer to the standard input of the remote command.
///
/// Dropping the writer sends end of file to the remote command.
pub(crate) struct ChannelWriter {
    queue: mpsc::Sender<Vec<u8>>,
    status: Status,
}

impl Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        self.queue.blocking_send(buf.to_vec()).map_err(|_| {
            let status = self.status.lock();
            match &status.aborted {
                Some(error) => io_error(error),
                None => io::Error::new(io::ErrorKind::BrokenPipe, "ssh channel is closed"),
            }
        })?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn io_error(error: &Error) -> io::Error {
    let kind = match error.code() {
        ErrorCode::Cancelled => io::ErrorKind::Interrupted,
        ErrorCode::Timeout => io::ErrorKind::TimedOut,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, error.to_string())
}

/// Opens a session channel, sets the variable `GIT_PROTOCOL` when a value
/// is given, runs the command and starts the task that moves its data.
/// Fails with `E_NETWORK` when the channel cannot be opened or the request
/// cannot be sent.
pub(crate) async fn open(
    session: Arc<SessionHandle>,
    command: String,
    protocol: Option<String>,
    status: Status,
    limits: Limits,
) -> nurt_contracts::error::Result<(ChannelReader, ChannelWriter)> {
    let failed = |err: russh::Error| {
        Error::new(
            ErrorCode::Network,
            format!("cannot start `{command}` over ssh: {err}"),
        )
        .with_problem("ssh_channel")
    };
    let channel = session.channel_open_session().await.map_err(failed)?;
    if let Some(value) = protocol {
        channel
            .set_env(false, "GIT_PROTOCOL", value)
            .await
            .map_err(failed)?;
    }
    channel
        .exec(true, command.as_bytes())
        .await
        .map_err(failed)?;
    let (read_half, write_half) = channel.split();
    let (incoming_tx, incoming_rx) = mpsc::channel(QUEUE_DEPTH);
    let (outgoing_tx, outgoing_rx) = mpsc::channel(QUEUE_DEPTH);
    tokio::spawn(pump(
        read_half,
        write_half,
        incoming_tx,
        outgoing_rx,
        status.clone(),
        limits,
    ));
    Ok((
        ChannelReader {
            queue: incoming_rx,
            chunk: Vec::new(),
            position: 0,
            status: status.clone(),
            command,
        },
        ChannelWriter {
            queue: outgoing_tx,
            status,
        },
    ))
}

async fn pump(
    mut read_half: ChannelReadHalf,
    write_half: ChannelWriteHalf<russh::client::Msg>,
    incoming: mpsc::Sender<Vec<u8>>,
    mut outgoing: mpsc::Receiver<Vec<u8>>,
    status: Status,
    limits: Limits,
) {
    let aborted = tokio::select! {
        () = receive(&mut read_half, &incoming, &status) => None,
        () = send(&write_half, &mut outgoing) => None,
        error = limits.watch() => Some(error),
    };
    if let Some(error) = aborted {
        status.lock().aborted = Some(error);
        let _ = write_half.close().await;
    }
}

async fn receive(
    read_half: &mut ChannelReadHalf,
    incoming: &mpsc::Sender<Vec<u8>>,
    status: &Status,
) {
    while let Some(message) = read_half.wait().await {
        match message {
            ChannelMsg::Data { data } => {
                if incoming.send(data.to_vec()).await.is_err() {
                    return;
                }
            }
            ChannelMsg::ExtendedData { data, ext: STDERR } => status.lock().push_stderr(&data),
            ChannelMsg::ExitStatus { exit_status } => status.lock().exit_status = Some(exit_status),
            ChannelMsg::ExitSignal {
                signal_name,
                error_message,
                ..
            } => {
                status.lock().signal =
                    Some(format!("{signal_name:?} {error_message}").trim().to_owned())
            }
            ChannelMsg::Failure => {
                status.lock().refused = true;
                return;
            }
            ChannelMsg::Close => return,
            _ => {}
        }
    }
}

/// Forwards written data; after the writer is dropped it sends end of file
/// and stays pending, so that reading continues until the server closes
/// the channel.
async fn send(
    write_half: &ChannelWriteHalf<russh::client::Msg>,
    outgoing: &mut mpsc::Receiver<Vec<u8>>,
) {
    while let Some(data) = outgoing.recv().await {
        if write_half.data_bytes(data).await.is_err() {
            return;
        }
    }
    let _ = write_half.eof().await;
    std::future::pending::<()>().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_command_has_no_failure() {
        let status = RemoteStatus {
            exit_status: Some(0),
            ..RemoteStatus::default()
        };
        assert!(status.failure("git-upload-pack 'repo'").is_none());
        assert!(RemoteStatus::default().failure("x").is_none());
    }

    #[test]
    fn missing_repository_is_classified() {
        let mut status = RemoteStatus {
            exit_status: Some(128),
            ..RemoteStatus::default()
        };
        status.push_stderr(b"fatal: '/srv/none.git' does not appear to be a git repository\n");
        let error = status.failure("git-upload-pack '/srv/none.git'").unwrap();
        assert_eq!(error.code(), ErrorCode::GitProtocol);
        assert_eq!(
            error.data().problem.as_deref(),
            Some("repository_not_found")
        );
        assert!(error.to_string().contains("exit status 128"));
        assert!(error.to_string().contains("does not appear"));
    }

    #[test]
    fn permission_denied_is_an_auth_error() {
        let mut status = RemoteStatus {
            refused: true,
            ..RemoteStatus::default()
        };
        status.push_stderr(b"Permission denied to user.\n");
        assert_eq!(
            status.failure("git-receive-pack 'r'").unwrap().code(),
            ErrorCode::GitAuth
        );
    }

    #[test]
    fn signal_and_abort_are_reported() {
        let status = RemoteStatus {
            signal: Some("KILL".into()),
            ..RemoteStatus::default()
        };
        assert!(
            status
                .failure("c")
                .unwrap()
                .to_string()
                .contains("signal KILL")
        );
        let status = RemoteStatus {
            aborted: Some(Error::new(ErrorCode::Timeout, "late")),
            ..RemoteStatus::default()
        };
        assert_eq!(status.failure("c").unwrap().code(), ErrorCode::Timeout);
        assert_eq!(
            io_error(&Error::new(ErrorCode::Cancelled, "x")).kind(),
            io::ErrorKind::Interrupted
        );
    }

    #[test]
    fn stderr_is_bounded() {
        let mut status = RemoteStatus::default();
        status.push_stderr(&vec![b'x'; MAX_STDERR + 10]);
        status.push_stderr(b"more");
        assert_eq!(status.stderr.len(), MAX_STDERR);
    }

    #[test]
    fn reader_returns_queued_data_then_end_of_file() {
        let (tx, rx) = mpsc::channel(4);
        let mut reader = ChannelReader {
            queue: rx,
            chunk: Vec::new(),
            position: 0,
            status: Status::default(),
            command: "c".into(),
        };
        tx.try_send(b"hello ".to_vec()).unwrap();
        tx.try_send(b"world".to_vec()).unwrap();
        drop(tx);
        let mut text = String::new();
        reader.read_to_string(&mut text).unwrap();
        assert_eq!(text, "hello world");
    }

    #[test]
    fn reader_reports_remote_failure_at_end() {
        let (tx, rx) = mpsc::channel(1);
        drop(tx);
        let status = Status::default();
        status.lock().exit_status = Some(1);
        let mut reader = ChannelReader {
            queue: rx,
            chunk: Vec::new(),
            position: 0,
            status,
            command: "c".into(),
        };
        assert!(reader.read(&mut [0; 4]).is_err());
    }

    #[test]
    fn writer_fails_after_channel_closed() {
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let mut writer = ChannelWriter {
            queue: tx,
            status: Status::default(),
        };
        assert_eq!(
            writer.write(b"x").unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(writer.write(b"").unwrap(), 0);
        writer.flush().unwrap();
    }
}
