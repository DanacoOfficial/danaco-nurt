// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Event source over the Windows Event Log API.
//!
//! Every handle of the API is owned by an [`EventHandle`] and closed with
//! `EvtClose` when it is dropped. Calls that return text are made twice: the
//! first call reports the size of the buffer, the second fills it.

#![allow(
    unsafe_code,
    reason = "the Windows Event Log API is a foreign interface of the operating system"
)]

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_EVT_CHANNEL_NOT_FOUND, ERROR_EVT_INVALID_QUERY,
    ERROR_EVT_UNRESOLVED_VALUE_INSERT, ERROR_INSUFFICIENT_BUFFER, ERROR_NO_MORE_ITEMS, WIN32_ERROR,
};
use windows::Win32::System::EventLog::{
    EVT_HANDLE, EvtClose, EvtFormatMessage, EvtFormatMessageEvent, EvtFormatMessageTask, EvtNext,
    EvtNextChannelPath, EvtOpenChannelEnum, EvtOpenPublisherMetadata, EvtQuery,
    EvtQueryChannelPath, EvtQueryReverseDirection, EvtRender, EvtRenderEventXml,
};
use windows::core::{HRESULT, PCWSTR};

use super::source::{EventSource, MessagePart, SourceError};

/// Number of event handles requested from `EvtNext` at once.
const BATCH: usize = 64;

/// `INFINITE` timeout of `EvtNext`.
const INFINITE: u32 = u32::MAX;

/// Win32 error code of an RPC server that is not available; the event log
/// service reports it when it is stopped.
const RPC_S_SERVER_UNAVAILABLE: u32 = 1722;

/// Initial capacity, in UTF-16 units, of the buffer for channel names.
const CHANNEL_NAME_CAPACITY: usize = 256;

/// Size in bytes of one UTF-16 unit.
const UTF16_UNIT: usize = 2;

/// Handle of the Windows Event Log API, closed when dropped.
#[derive(Debug)]
pub struct EventHandle(EVT_HANDLE);

impl Drop for EventHandle {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by the API, is owned by this value
        // only and is closed exactly once. A failure to close cannot be
        // handled during drop.
        let _ = unsafe { EvtClose(self.0) };
    }
}

/// Event source of the local event log.
///
/// Metadata of providers is opened once per provider and kept for the
/// lifetime of the source, including the absence of metadata.
#[derive(Debug, Default)]
pub struct WevtSource {
    publishers: Mutex<HashMap<String, Option<EventHandle>>>,
}

impl WevtSource {
    /// A source of the event log of the local system.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Events of one channel returned by [`WevtSource`], newest first.
#[derive(Debug)]
pub struct WevtEvents {
    query: EventHandle,
    pending: VecDeque<EventHandle>,
    finished: bool,
}

impl WevtEvents {
    fn fetch(&mut self) -> Result<(), SourceError> {
        let mut handles = [0_isize; BATCH];
        let mut returned = 0_u32;
        // SAFETY: `handles` is a writable array whose length is passed with
        // it, and `returned` is a valid location for the count.
        let result = unsafe { EvtNext(self.query.0, &mut handles, INFINITE, 0, &mut returned) };
        match result {
            Ok(()) => {
                let count = usize::try_from(returned).map_or(BATCH, |count| count.min(BATCH));
                self.pending.extend(
                    handles
                        .iter()
                        .take(count)
                        .map(|&raw| EventHandle(EVT_HANDLE(raw))),
                );
                self.finished = count == 0;
                Ok(())
            }
            Err(error) if is(&error, ERROR_NO_MORE_ITEMS) => {
                self.finished = true;
                Ok(())
            }
            Err(error) => {
                self.finished = true;
                Err(source_error(&error))
            }
        }
    }
}

impl Iterator for WevtEvents {
    type Item = Result<EventHandle, SourceError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pending.is_empty() && !self.finished {
            if let Err(error) = self.fetch() {
                return Some(Err(error));
            }
        }
        self.pending.pop_front().map(Ok)
    }
}

impl EventSource for WevtSource {
    type Event = EventHandle;
    type Events<'a> = WevtEvents;

    fn channels(&self) -> Result<Vec<String>, SourceError> {
        // SAFETY: no session handle selects the local computer; no flags.
        let enumerator = unsafe { EvtOpenChannelEnum(None, 0) }.map_err(|e| source_error(&e))?;
        let enumerator = EventHandle(enumerator);
        let mut names = Vec::new();
        let mut buffer = vec![0_u16; CHANNEL_NAME_CAPACITY];
        loop {
            let mut used = 0_u32;
            // SAFETY: `buffer` is a writable slice whose length is passed with
            // it, and `used` is a valid location for the size.
            let result = unsafe { EvtNextChannelPath(enumerator.0, Some(&mut buffer), &mut used) };
            match result {
                Ok(()) => names.push(from_wide(&buffer, used)),
                Err(error) if is(&error, ERROR_NO_MORE_ITEMS) => return Ok(names),
                Err(error) if is(&error, ERROR_INSUFFICIENT_BUFFER) => {
                    buffer.resize(units(used), 0);
                }
                Err(error) => return Err(source_error(&error)),
            }
        }
    }

    fn query<'a>(&'a self, channel: &str, xpath: &str) -> Result<Self::Events<'a>, SourceError> {
        let path = wide(channel);
        let query = wide(xpath);
        let flags = EvtQueryChannelPath.0 | EvtQueryReverseDirection.0;
        // SAFETY: both strings are terminated by NUL and outlive the call; no
        // session handle selects the local computer.
        let handle =
            unsafe { EvtQuery(None, PCWSTR(path.as_ptr()), PCWSTR(query.as_ptr()), flags) }
                .map_err(|error| source_error(&error))?;
        Ok(WevtEvents {
            query: EventHandle(handle),
            pending: VecDeque::new(),
            finished: false,
        })
    }

    fn render_xml(&self, event: &EventHandle) -> Result<String, SourceError> {
        let flags = EvtRenderEventXml.0;
        let mut used = 0_u32;
        let mut count = 0_u32;
        // SAFETY: a buffer of size zero without a pointer asks for the size;
        // `used` and `count` are valid locations.
        let size = unsafe { EvtRender(None, event.0, flags, 0, None, &mut used, &mut count) };
        match size {
            Err(error) if is(&error, ERROR_INSUFFICIENT_BUFFER) => {}
            Err(error) => return Err(source_error(&error)),
            Ok(()) => return Ok(String::new()),
        }
        let mut buffer = vec![0_u16; units(used).div_ceil(UTF16_UNIT)];
        let bytes = u32::try_from(buffer.len() * UTF16_UNIT).unwrap_or(u32::MAX);
        // SAFETY: `buffer` holds `bytes` writable bytes aligned for UTF-16,
        // which is what the XML rendering writes; `used` and `count` are valid
        // locations.
        unsafe {
            EvtRender(
                None,
                event.0,
                flags,
                bytes,
                Some(buffer.as_mut_ptr().cast()),
                &mut used,
                &mut count,
            )
        }
        .map_err(|error| source_error(&error))?;
        let written = u32::try_from(units(used) / UTF16_UNIT).unwrap_or(u32::MAX);
        Ok(from_wide(&buffer, written))
    }

    fn format(&self, event: &EventHandle, provider: &str, part: MessagePart) -> Option<String> {
        let mut publishers = self.publishers.lock().ok()?;
        let metadata = publishers
            .entry(provider.to_owned())
            .or_insert_with(|| open_publisher(provider))
            .as_ref()?;
        let flags = match part {
            MessagePart::Event => EvtFormatMessageEvent.0,
            MessagePart::Task => EvtFormatMessageTask.0,
        };
        format_message(metadata.0, event.0, flags)
    }
}

fn open_publisher(provider: &str) -> Option<EventHandle> {
    let name = wide(provider);
    // SAFETY: the name is terminated by NUL and outlives the call; a null log
    // file path selects the metadata installed on the computer; locale 0 is
    // the locale of the account.
    unsafe { EvtOpenPublisherMetadata(None, PCWSTR(name.as_ptr()), PCWSTR::null(), 0, 0) }
        .ok()
        .map(EventHandle)
}

/// Formats a part of an event. A message whose insertion strings cannot be
/// resolved (`ERROR_EVT_UNRESOLVED_VALUE_INSERT`) is still returned.
fn format_message(metadata: EVT_HANDLE, event: EVT_HANDLE, flags: u32) -> Option<String> {
    let mut used = 0_u32;
    // SAFETY: no buffer asks for the size; `used` is a valid location.
    let size =
        unsafe { EvtFormatMessage(Some(metadata), Some(event), 0, None, flags, None, &mut used) };
    match size {
        Err(error)
            if is(&error, ERROR_INSUFFICIENT_BUFFER)
                || is(&error, ERROR_EVT_UNRESOLVED_VALUE_INSERT) => {}
        Ok(()) | Err(_) => return None,
    }
    let mut buffer = vec![0_u16; units(used)];
    // SAFETY: `buffer` is a writable slice whose length is passed with it;
    // `used` is a valid location.
    let result = unsafe {
        EvtFormatMessage(
            Some(metadata),
            Some(event),
            0,
            None,
            flags,
            Some(&mut buffer),
            &mut used,
        )
    };
    let formatted = match result {
        Ok(()) => true,
        Err(error) => is(&error, ERROR_EVT_UNRESOLVED_VALUE_INSERT),
    };
    formatted.then(|| from_wide(&buffer, used).trim_end().to_owned())
}

fn is(error: &windows::core::Error, code: WIN32_ERROR) -> bool {
    error.code() == HRESULT::from_win32(code.0)
}

fn source_error(error: &windows::core::Error) -> SourceError {
    if is(error, ERROR_EVT_CHANNEL_NOT_FOUND) {
        SourceError::ChannelNotFound
    } else if is(error, ERROR_ACCESS_DENIED) {
        SourceError::AccessDenied
    } else if is(error, WIN32_ERROR(RPC_S_SERVER_UNAVAILABLE)) {
        SourceError::ServiceUnavailable
    } else if is(error, ERROR_EVT_INVALID_QUERY) {
        SourceError::InvalidQuery
    } else {
        SourceError::System {
            code: u32::from_ne_bytes(error.code().0.to_ne_bytes()),
            message: error.message(),
        }
    }
}

fn units(count: u32) -> usize {
    usize::try_from(count).unwrap_or(usize::MAX)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Text of the first `used` units of a buffer, up to the first NUL.
fn from_wide(buffer: &[u16], used: u32) -> String {
    let text = buffer
        .get(..units(used).min(buffer.len()))
        .unwrap_or_default();
    let end = text
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(text.len());
    String::from_utf16_lossy(text.get(..end).unwrap_or_default())
}
