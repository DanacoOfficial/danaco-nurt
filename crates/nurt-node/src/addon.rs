// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Exports of the addon to JavaScript.

use std::sync::Arc;

use napi::bindgen_prelude::{AsyncTask, Task};
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi::{Env, Error, Status};
use napi_derive::napi;
use nurt_core::{DeliveredEvent, EventListener, ExecHandle};

use crate::dispatch::{Failure, Native, encode_event, finish};

/// Receiver of the events of an execution, called with the event as JSON text.
type EventCallback = ThreadsafeFunction<String, (), String, Status, false>;

fn thrown(failure: &Failure) -> Error {
    Error::new(Status::GenericFailure, failure.to_string())
}

struct Forward(EventCallback);

impl EventListener for Forward {
    fn deliver(&self, event: DeliveredEvent) {
        if let Ok(text) = encode_event(&event) {
            self.0.call(text, ThreadsafeFunctionCallMode::NonBlocking);
        }
    }
}

/// Engine of the library.
#[napi]
pub struct NativeEngine {
    native: Native,
}

#[napi]
impl NativeEngine {
    /// Opens an engine. `options` is absent or a JSON object with the members
    /// `session` and `state_dir`.
    #[napi(constructor)]
    pub fn new(options: Option<String>) -> napi::Result<Self> {
        let native = Native::open(options.as_deref()).map_err(|failure| thrown(&failure))?;
        Ok(Self { native })
    }

    /// Description of the engine as JSON text.
    #[napi]
    pub fn info(&self) -> napi::Result<String> {
        self.native.info().map_err(|failure| thrown(&failure))
    }

    /// Runs an operation other than `run` on a worker thread; resolves with
    /// the result as JSON text.
    #[napi]
    pub fn call(&self, operation: String, input: String) -> AsyncTask<CallTask> {
        AsyncTask::new(CallTask {
            native: self.native.clone(),
            operation,
            input,
        })
    }

    /// Starts a request. `on_event` receives every event as JSON text.
    #[napi]
    pub fn start(
        &self,
        request: String,
        on_event: Option<EventCallback>,
    ) -> napi::Result<NativeExecution> {
        let listener: Arc<dyn EventListener> = match on_event {
            Some(callback) => Arc::new(Forward(callback)),
            None => Arc::new(Discard),
        };
        let handle = self
            .native
            .start(&request, listener)
            .map_err(|failure| thrown(&failure))?;
        Ok(NativeExecution { handle })
    }
}

struct Discard;

impl EventListener for Discard {
    fn deliver(&self, _event: DeliveredEvent) {}
}

/// Request started with `NativeEngine.start`.
#[napi]
pub struct NativeExecution {
    handle: ExecHandle,
}

#[napi]
impl NativeExecution {
    /// Identifier of the request, as carried by its events.
    #[napi(getter)]
    pub fn request_id(&self) -> String {
        self.handle.request_id().to_owned()
    }

    /// Requests cancellation.
    #[napi]
    pub fn cancel(&self) {
        self.handle.cancel();
    }

    /// Whether the response is available.
    #[napi]
    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }

    /// Resolves with the response as JSON text when the request has finished.
    #[napi]
    pub fn wait(&self) -> AsyncTask<WaitTask> {
        AsyncTask::new(WaitTask {
            handle: self.handle.clone(),
        })
    }
}

/// Operation running on a worker thread.
pub struct CallTask {
    native: Native,
    operation: String,
    input: String,
}

impl Task for CallTask {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> napi::Result<String> {
        self.native
            .call(&self.operation, &self.input)
            .map_err(|failure| thrown(&failure))
    }

    fn resolve(&mut self, _env: Env, output: String) -> napi::Result<String> {
        Ok(output)
    }
}

/// Wait for a response on a worker thread.
pub struct WaitTask {
    handle: ExecHandle,
}

impl Task for WaitTask {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> napi::Result<String> {
        finish(&self.handle, None).map_err(|failure| thrown(&failure))
    }

    fn resolve(&mut self, _env: Env, output: String) -> napi::Result<String> {
        Ok(output)
    }
}
