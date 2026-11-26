// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Protocol handler of one connection.

// Logging requests and notifications are deprecated in protocol revisions
// newer than the ones this server negotiates; the revisions it serves still
// define them.
#![allow(deprecated)]

use std::borrow::Cow;
use std::sync::Arc;

use nurt_contracts::envelope::Response;
use nurt_contracts::error::AbiStatus;
use nurt_contracts::stage::{CancelReason, CancelToken};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorCode,
    Implementation, JsonObject, ListResourceTemplatesResult, ListResourcesResult, ListToolsResult,
    PaginatedRequestParams, ProtocolVersion, ReadResourceRequestParams, ReadResourceResponse,
    ReadResourceResult, ResourceContents, ServerCapabilities, ServerConfig, SetLevelRequestParams,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::json;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::backend::{Backend, BackendInfo, CallError, CheckOutcome, DescribeSubject, HelpQuery};
use crate::events::{ChannelSink, Forwarder, LogThreshold};
use crate::resources::{NurtResource, fixed_resources, resource_templates};
use crate::tool_schema::{
    CHECK_TOOL, HELP_TOOL, RUN_TOOL, check_request_from_arguments, check_tool,
    help_query_from_arguments, help_tool, request_from_arguments, run_tool,
};

/// Name of the server in the `initialize` result.
pub(crate) const SERVER_NAME: &str = "nurt";

/// Title of the server in the `initialize` result.
pub(crate) const SERVER_TITLE: &str = "Danaco Nurt";

/// Newest protocol revision served. Later revisions replace the
/// `initialize` handshake with metadata in every request and are not
/// negotiated.
pub(crate) const MAX_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::V_2025_11_25;

/// Error code returned when the limit of calls in progress is reached; the
/// range from -32000 to -32099 is reserved by JSON-RPC 2.0 for server errors.
pub(crate) const BUSY_CODE: ErrorCode = ErrorCode(-32000);

const INSTRUCTIONS: &str = "Run programs of the Nurt language with the `nurt` tool. Built-in \
help is available through the `nurt_help` tool and the `nurt://help/{topic}` resources.";

/// State shared by every connection of one server: the engine and the
/// limit of calls in progress.
pub(crate) struct Shared<B> {
    backend: Arc<B>,
    info: BackendInfo,
    inflight: Arc<Semaphore>,
}

impl<B: Backend> Shared<B> {
    /// Creates the shared state with the limit reported by the engine.
    pub(crate) fn new(backend: Arc<B>) -> Arc<Self> {
        let info = backend.info();
        let permits = info.max_inflight.clamp(1, Semaphore::MAX_PERMITS);
        Arc::new(Self {
            backend,
            info,
            inflight: Arc::new(Semaphore::new(permits)),
        })
    }

    /// Reserves a place for one call, or reports that the limit is reached.
    fn permit(&self) -> Result<OwnedSemaphorePermit, ErrorData> {
        Arc::clone(&self.inflight).try_acquire_owned().map_err(|_| {
            ErrorData::new(
                BUSY_CODE,
                format!(
                    "too many calls in progress (limit {}); retry when a call completes",
                    self.info.max_inflight.max(1)
                ),
                None,
            )
        })
    }

    /// Runs a blocking engine call on a blocking thread within the limit.
    async fn call<T, F>(&self, operation: F) -> Result<T, ErrorData>
    where
        T: Send + 'static,
        F: FnOnce(&B) -> T + Send + 'static,
    {
        let permit = self.permit()?;
        let backend = Arc::clone(&self.backend);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation(&backend)
        })
        .await
        .map_err(|error| ErrorData::internal_error(format!("engine call failed: {error}"), None))
    }
}

/// Handler of one connection: one stdio stream or one HTTP session.
pub(crate) struct NurtServer<B> {
    shared: Arc<Shared<B>>,
    session: Option<String>,
    threshold: LogThreshold,
}

impl<B: Backend> NurtServer<B> {
    /// Creates a handler; `session` is the engine session used by calls
    /// that do not name one.
    pub(crate) fn new(shared: Arc<Shared<B>>, session: Option<String>) -> Self {
        Self {
            shared,
            session,
            threshold: LogThreshold::new(),
        }
    }

    /// Engine session of a call that names none: the HTTP session of the
    /// request, or the session the handler was created with.
    fn engine_session(&self, context: &RequestContext<RoleServer>) -> Option<String> {
        transport_session(context).or_else(|| self.session.clone())
    }

    async fn run_program(
        &self,
        arguments: Option<JsonObject>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let session = self.engine_session(&context);
        let request = match request_from_arguments(arguments, session.as_deref()) {
            Ok(request) => request,
            Err(message) => return Ok(error_text(message)),
        };
        let permit = self.shared.permit()?;
        let backend = Arc::clone(&self.shared.backend);
        let (sink, mut events) = ChannelSink::channel();
        let cancel = CancelToken::new();
        let engine_cancel = cancel.clone();
        let mut run = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            backend.run(&request, sink, engine_cancel)
        });

        let progress_token = context.meta.get_progress_token();
        let mut forwarder = Forwarder::new(&context.peer, progress_token, &self.threshold);
        let mut events_open = true;
        let joined = loop {
            tokio::select! {
                biased;
                event = events.recv(), if events_open => match event {
                    Some(event) => forwarder.forward(event).await,
                    None => events_open = false,
                },
                joined = &mut run => break joined,
                () = context.ct.cancelled() => {
                    cancel.cancel(CancelReason::Requested);
                    return Ok(error_text("the call was cancelled".to_owned()));
                }
            }
        };
        while let Ok(event) = events.try_recv() {
            forwarder.forward(event).await;
        }
        run_result(joined)
    }

    async fn check_program(
        &self,
        arguments: Option<JsonObject>,
        context: &RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let session = self.engine_session(context);
        let request = match check_request_from_arguments(arguments, session.as_deref()) {
            Ok(request) => request,
            Err(message) => return Ok(error_text(message)),
        };
        match self
            .shared
            .call(move |backend| backend.check(&request))
            .await?
        {
            Ok(outcome) => check_result(outcome),
            Err(error) => Ok(call_error_text(&error)),
        }
    }

    async fn read_help(&self, arguments: Option<JsonObject>) -> Result<CallToolResult, ErrorData> {
        let query = match help_query_from_arguments(arguments) {
            Ok(query) => query,
            Err(message) => return Ok(error_text(message)),
        };
        match self
            .shared
            .call(move |backend| backend.help(&query))
            .await?
        {
            Ok(text) => Ok(CallToolResult::success(vec![ContentBlock::text(text)])),
            Err(error) => Ok(call_error_text(&error)),
        }
    }
}

#[cfg(feature = "http")]
use crate::http::session_of as transport_session;

/// Session of the transport a request arrived on; the stdio transport has
/// none.
#[cfg(not(feature = "http"))]
fn transport_session(_context: &RequestContext<RoleServer>) -> Option<String> {
    None
}

/// Tool result of a finished engine run.
fn run_result(
    joined: Result<Result<Response, CallError>, tokio::task::JoinError>,
) -> Result<CallToolResult, ErrorData> {
    match joined {
        Ok(Ok(response)) => tool_result(response),
        Ok(Err(error)) => Ok(call_error_text(&error)),
        Err(error) => Err(ErrorData::internal_error(
            format!("engine call failed: {error}"),
            None,
        )),
    }
}

/// Tool result with the rendered text and the report as structured content.
fn tool_result(response: Response) -> Result<CallToolResult, ErrorData> {
    let failed = response.report.error.is_some();
    let report = serde_json::to_value(&response.report).map_err(|error| {
        ErrorData::internal_error(format!("report cannot be serialized: {error}"), None)
    })?;
    let text = match response.rendered {
        Some(rendered) => rendered.text,
        None => report.to_string(),
    };
    let content = vec![ContentBlock::text(text)];
    let mut result = if failed {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    };
    result.structured_content = Some(report);
    Ok(result)
}

/// Tool result of a check: the check document as text and structured content,
/// or the language error that stopped the check.
fn check_result(outcome: CheckOutcome) -> Result<CallToolResult, ErrorData> {
    match outcome {
        CheckOutcome::Report(document) => {
            let mut result =
                CallToolResult::success(vec![ContentBlock::text(document.to_string())]);
            result.structured_content = Some(document);
            Ok(result)
        }
        CheckOutcome::Failed(error) => {
            let text = format!("error {}: {}", error.code, error.message);
            let document = serde_json::to_value(&error).map_err(|failure| {
                ErrorData::internal_error(format!("error cannot be serialized: {failure}"), None)
            })?;
            let mut result = CallToolResult::error(vec![ContentBlock::text(text)]);
            result.structured_content = Some(json!({ "error": document }));
            Ok(result)
        }
    }
}

fn error_text(message: String) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message)])
}

fn call_error_text(error: &CallError) -> CallToolResult {
    error_text(format!("call failed: {error}"))
}

fn resource_error(uri: &str, error: &CallError) -> ErrorData {
    let data = Some(json!({ "uri": uri, "status": error.status.name() }));
    if error.status == AbiStatus::Arg {
        ErrorData::resource_not_found(
            format!("resource `{uri}` not found: {}", error.message),
            data,
        )
    } else {
        ErrorData::internal_error(format!("resource `{uri}` cannot be read: {error}"), data)
    }
}

impl<B: Backend> ServerHandler for NurtServer<B> {
    fn get_info(&self) -> ServerConfig {
        let mut implementation = Implementation::new(SERVER_NAME, self.shared.info.version.clone());
        implementation.title = Some(SERVER_TITLE.to_owned());
        let capabilities = ServerCapabilities::builder()
            .enable_logging()
            .enable_resources()
            .enable_tools()
            .build();
        ServerConfig::new(capabilities)
            .with_server_info(implementation)
            .with_instructions(INSTRUCTIONS)
            .with_protocol_version(MAX_PROTOCOL_VERSION)
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(ProtocolVersion::known_up_to(&MAX_PROTOCOL_VERSION))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let core = self
            .shared
            .call(|backend| backend.describe(&DescribeSubject::Core))
            .await?
            .map_err(|error| {
                ErrorData::internal_error(format!("core description unavailable: {error}"), None)
            })?;
        let run = run_tool(&core).map_err(|message| ErrorData::internal_error(message, None))?;
        Ok(ListToolsResult::with_all_items(vec![
            run,
            help_tool(),
            check_tool(),
        ]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let result = match request.name.as_ref() {
            RUN_TOOL => self.run_program(request.arguments, context).await?,
            HELP_TOOL => self.read_help(request.arguments).await?,
            CHECK_TOOL => self.check_program(request.arguments, &context).await?,
            other => {
                return Err(ErrorData::invalid_params(
                    format!(
                        "unknown tool `{other}`; available tools: {RUN_TOOL}, {HELP_TOOL}, {CHECK_TOOL}"
                    ),
                    None,
                ));
            }
        };
        Ok(result.into())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(fixed_resources()))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        Ok(ListResourceTemplatesResult::with_all_items(
            resource_templates(),
        ))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let uri = request.uri;
        let Some(resource) = NurtResource::parse(&uri) else {
            return Err(ErrorData::resource_not_found(
                format!(
                    "unknown resource `{uri}`; see resources/list and resources/templates/list"
                ),
                Some(json!({ "uri": uri })),
            ));
        };
        let mime_type = resource.mime_type();
        let text = self
            .shared
            .call(move |backend| match resource {
                NurtResource::Help(topic) => backend.help(&HelpQuery {
                    topic: Some(topic),
                    ..HelpQuery::default()
                }),
                NurtResource::Schema(name) => backend.schema(name),
                NurtResource::Describe(subject) => backend.describe(&subject),
            })
            .await?
            .map_err(|error| resource_error(&uri, &error))?;
        let contents = ResourceContents::text(text, uri).with_mime_type(mime_type);
        Ok(ReadResourceResult::new(vec![contents]).into())
    }

    async fn set_level(
        &self,
        request: SetLevelRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        self.threshold.set(request.level);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::envelope::{OutputFormat, Rendered, Response};
    use nurt_contracts::error::{AbiStatus, Error, ErrorCode};
    use nurt_contracts::report::{Report, ReportStatus};

    use serde_json::json;

    use super::{
        call_error_text, check_result, error_text, resource_error, run_result, tool_result,
    };
    use crate::backend::{CallError, CheckOutcome};

    fn as_json(result: rmcp::model::CallToolResult) -> serde_json::Value {
        serde_json::to_value(result).unwrap()
    }

    #[test]
    fn a_check_document_is_the_text_and_the_structured_content() {
        let document = json!({"format": "nurt-check/1", "effect": "read"});
        let result = as_json(check_result(CheckOutcome::Report(document.clone())).unwrap());
        assert_eq!(result["isError"], false);
        assert_eq!(result["structuredContent"], document);
        assert_eq!(result["content"][0]["text"], document.to_string());
    }

    #[test]
    fn a_rejected_check_is_an_error_with_the_language_error_as_structured_content() {
        let error = Error::new(ErrorCode::Syntax, "unexpected token");
        let result = as_json(check_result(CheckOutcome::Failed(error)).unwrap());
        assert_eq!(result["isError"], true);
        assert_eq!(
            result["content"][0]["text"],
            "error E_SYNTAX: unexpected token"
        );
        assert_eq!(result["structuredContent"]["error"]["code"], "E_SYNTAX");
        assert_eq!(
            result["structuredContent"]["error"]["message"],
            "unexpected token"
        );
    }

    #[test]
    fn a_failed_call_is_an_error_result_with_the_status_name() {
        let result = as_json(
            run_result(Ok(Err(CallError::new(
                AbiStatus::Io,
                "journal is not writable",
            ))))
            .unwrap(),
        );
        assert_eq!(result["isError"], true);
        assert_eq!(
            result["content"][0]["text"],
            "call failed: NURT_E_IO: journal is not writable"
        );
        assert!(result["structuredContent"].is_null());
    }

    #[test]
    fn a_finished_run_is_the_result_of_its_response() {
        let response = Response::new(Report::new(ReportStatus::Ok, "find"));
        let result = as_json(run_result(Ok(Ok(response))).unwrap());
        assert_eq!(result["isError"], false);
        assert_eq!(result["structuredContent"]["kind"], "find");
    }

    #[test]
    fn text_errors_carry_only_their_message() {
        let result = as_json(error_text("bad \u{142}\u{f3}d\u{17a}".to_owned()));
        assert_eq!(result["isError"], true);
        assert_eq!(result["content"][0]["text"], "bad \u{142}\u{f3}d\u{17a}");
        let call = as_json(call_error_text(&CallError::new(AbiStatus::Arg, "x")));
        assert_eq!(call["content"][0]["text"], "call failed: NURT_E_ARG: x");
    }

    #[test]
    fn a_report_that_failed_without_an_error_is_not_an_error_result() {
        let result =
            as_json(tool_result(Response::new(Report::new(ReportStatus::Failed, "find"))).unwrap());
        assert_eq!(result["isError"], false);
    }

    #[test]
    fn other_call_failures_of_a_resource_are_internal_errors() {
        for status in [
            AbiStatus::Timeout,
            AbiStatus::Cancelled,
            AbiStatus::Internal,
        ] {
            let error = resource_error("nurt://help/x", &CallError::new(status, "x"));
            assert_eq!(error.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
        }
    }

    #[test]
    fn rendered_text_is_the_first_content() {
        let mut response = Response::new(Report::new(ReportStatus::Ok, "find"));
        response.rendered = Some(Rendered {
            format: OutputFormat::Text,
            text: "3 files".to_owned(),
        });
        let result = serde_json::to_value(tool_result(response).unwrap()).unwrap();
        assert_eq!(result["content"][0]["text"], "3 files");
        assert_eq!(result["isError"], false);
        assert_eq!(result["structuredContent"]["status"], "ok");
    }

    #[test]
    fn report_is_the_text_without_rendering() {
        let mut report = Report::new(ReportStatus::Failed, "find");
        report.error = Some(Error::new(ErrorCode::Syntax, "unexpected token"));
        let result = serde_json::to_value(tool_result(Response::new(report)).unwrap()).unwrap();
        let text: serde_json::Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text, result["structuredContent"]);
        assert_eq!(result["isError"], true);
    }

    #[test]
    fn unknown_group_is_a_missing_resource() {
        let missing = resource_error(
            "nurt://describe/group/x",
            &CallError::new(AbiStatus::Arg, "unknown group `x`"),
        );
        assert_eq!(missing.code, rmcp::model::ErrorCode::RESOURCE_NOT_FOUND);
        let failed = resource_error(
            "nurt://describe/core",
            &CallError::new(AbiStatus::Io, "catalog unreadable"),
        );
        assert_eq!(failed.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
        assert_eq!(
            failed.data,
            Some(serde_json::json!({"uri": "nurt://describe/core", "status": "NURT_E_IO"}))
        );
    }
}
