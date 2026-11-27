// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-node::addon
// Responsibility:   Exports of the addon to JavaScript.
// Dependencies:     napi, napi_derive, nurt_core
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Exports of the addon to JavaScript.

use std::sync::Arc;

use napi::bindgen_prelude::{AsyncTask, Task};
use napi::threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode};
use napi::{Env, Error, Status};
use napi_derive::napi;
use nurt_core::ExecHandle;

use crate::dispatch::{Failure, Message, Native, Sink};

/// Item of the ordered stream of a request.
///
/// `kind` is `event` (`text` is an event as JSON text), `response` (`text` is
/// the response as JSON text) or `failure` (`text` is `NURT_E_NAME: message`).
#[napi(object)]
pub struct Delivery {
    /// Kind of the item.
    pub kind: String,
    /// Content of the item.
    pub text: String,
}

/// Receiver of the stream of a request. One function carries events and the
/// response, so the response cannot overtake an event.
type DeliveryCallback = ThreadsafeFunction<Delivery, (), Delivery, Status, false>;

fn thrown(failure: &Failure) -> Error {
    Error::new(Status::GenericFailure, failure.to_string())
}

fn delivery(message: Message) -> Delivery {
    match message {
        Message::Event(text) => Delivery {
            kind: "event".to_owned(),
            text,
        },
        Message::Response(Ok(text)) => Delivery {
            kind: "response".to_owned(),
            text,
        },
        Message::Response(Err(failure)) => Delivery {
            kind: "failure".to_owned(),
            text: failure.to_string(),
        },
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

    /// Starts a request. `on_message` receives every event and, after the
    /// last event, the response.
    #[napi]
    pub fn start(
        &self,
        request: String,
        on_message: DeliveryCallback,
    ) -> napi::Result<NativeExecution> {
        let sink: Sink = Arc::new(move |message| {
            on_message.call(delivery(message), ThreadsafeFunctionCallMode::NonBlocking);
        });
        let handle = self
            .native
            .start_ordered(&request, sink)
            .map_err(|failure| thrown(&failure))?;
        Ok(NativeExecution { handle })
    }
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

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
