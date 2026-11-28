// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-langserver::server
// Responsibility:   Session of the language server: initialization, message loop and dispatch.
// Dependencies:     lsp_server, lsp_types, nurt_bind, nurt_contracts, nurt_correct, nurt_help, nurt_syntax, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Session of the language server: initialization, message loop and
//! dispatch.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use lsp_server::{Connection, ErrorCode, Message, Notification, ProtocolError, Request, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Exit, LogMessage,
    Notification as _, PublishDiagnostics, ShowMessage,
};
use lsp_types::request::{
    CodeActionRequest, Completion, Formatting, HoverRequest, Request as _,
    SemanticTokensFullRequest, SignatureHelpRequest,
};
use lsp_types::{
    CodeActionKind, CodeActionOptions, CodeActionProviderCapability, CompletionOptions,
    CompletionResponse, HoverProviderCapability, InitializeParams, InitializeResult,
    LogMessageParams, MessageType, OneOf, PublishDiagnosticsParams, SemanticTokens,
    SemanticTokensFullOptions, SemanticTokensOptions, SemanticTokensResult,
    SemanticTokensServerCapabilities, ServerCapabilities, ServerInfo, ShowMessageParams,
    SignatureHelpOptions, TextDocumentSyncCapability, TextDocumentSyncKind, Uri,
};
use nurt_bind::{BindContext, Catalog};
use nurt_contracts::config::Config;
use nurt_correct::tables::{Tables, load_embedded};
use nurt_help::catalog::Catalog as HelpCatalog;
use nurt_syntax::parse::Variant;
use serde_json::Value;

use crate::analysis::{self, Analysis, Engine};
use crate::assist;
use crate::catalog::Commands;
use crate::document::Documents;
use crate::options::{self, Options};
use crate::position::Encoding;
use crate::semantic;
use crate::signature;

/// Name of the server reported at initialization.
const SERVER_NAME: &str = "nurt";

/// Characters after which the client asks for completion: labels, `@`
/// references, the pipe operator, argument separators and call parentheses.
const TRIGGER_CHARACTERS: [&str; 8] = [":", "=", "@", "|", ">", " ", "(", ","];

/// Characters after which the client asks for the signature: the start of
/// an argument and the end of a label.
const SIGNATURE_TRIGGER_CHARACTERS: [&str; 5] = [" ", "(", ",", ":", "="];

/// Error that ends a language server session.
#[derive(Debug)]
#[non_exhaustive]
pub enum ServerError {
    /// The client broke the protocol, for example with a request before
    /// `initialize` or a message other than `exit` after `shutdown`.
    Protocol(ProtocolError),
    /// The `initialize` request carried parameters that are not valid.
    Initialize(String),
    /// A response could not be encoded.
    Encode(serde_json::Error),
    /// The standard streams failed.
    Io(std::io::Error),
    /// The client closed the connection before `shutdown`.
    Disconnected,
    /// The client sent `exit` without a preceding `shutdown`.
    ExitWithoutShutdown,
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(error) => write!(f, "language server protocol error: {error}"),
            Self::Initialize(message) => write!(f, "invalid initialize parameters: {message}"),
            Self::Encode(error) => write!(f, "cannot encode a message: {error}"),
            Self::Io(error) => write!(f, "standard stream error: {error}"),
            Self::Disconnected => f.write_str("the client closed the connection before shutdown"),
            Self::ExitWithoutShutdown => f.write_str("the client sent exit before shutdown"),
        }
    }
}

impl std::error::Error for ServerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Encode(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ProtocolError> for ServerError {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}

/// Runs the language server on the standard input and output until the
/// client ends the session.
///
/// This is the entry point of `nurt lsp`.
///
/// # Errors
///
/// Returns the [`ServerError`] that ended the session, including a failure of
/// the standard streams.
pub fn run_stdio(options: &Options) -> Result<(), ServerError> {
    let (connection, io_threads) = Connection::stdio();
    serve(connection, options)?;
    io_threads.join().map_err(ServerError::Io)
}

/// Runs a language server session on `connection` until `shutdown` and
/// `exit`.
///
/// The position encoding is UTF-32 when the client offers it and UTF-16
/// otherwise. Every change of a document publishes its diagnostics. A
/// warning about an initialization option of the wrong form, or about
/// correction tables that cannot be loaded, is shown to the client.
///
/// # Errors
///
/// Returns the [`ServerError`] that ended the session before `shutdown`.
///
/// # Examples
///
/// ```
/// use lsp_server::{Connection, Message, Notification, Request, RequestId};
/// use serde_json::json;
///
/// let (server, client) = Connection::memory();
/// let session = std::thread::spawn(move || nurt_langserver::serve(server, &Default::default()));
/// let send = |message: Message| client.sender.send(message);
/// let params = json!({"capabilities": {"general": {"positionEncodings": ["utf-32"]}}});
/// send(Request::new(RequestId::from(1), "initialize".to_owned(), params).into())?;
/// let Message::Response(response) = client.receiver.recv()? else { panic!() };
/// let result = response.response_result.map_err(|error| error.message)?;
/// assert_eq!(result["capabilities"]["positionEncoding"], "utf-32");
/// send(Notification::new("initialized".to_owned(), json!({})).into())?;
/// send(Request::new(RequestId::from(2), "shutdown".to_owned(), ()).into())?;
/// client.receiver.recv()?;
/// send(Notification::new("exit".to_owned(), ()).into())?;
/// assert!(session.join().is_ok_and(|ended| ended.is_ok()));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn serve(connection: Connection, options: &Options) -> Result<(), ServerError> {
    let (id, params) = connection.initialize_start()?;
    let params: InitializeParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(error) => {
            let message = error.to_string();
            let response = Response::new_err(id, ErrorCode::InvalidParams as i32, message.clone());
            send(&connection, response.into())?;
            return Err(ServerError::Initialize(message));
        }
    };
    let offered = params
        .capabilities
        .general
        .as_ref()
        .and_then(|general| general.position_encodings.as_deref());
    let encoding = Encoding::negotiate(offered);
    let (settings, mut warnings) =
        options::settings(params.initialization_options.as_ref(), options);
    let result = serde_json::to_value(initialize_result(encoding)).map_err(ServerError::Encode)?;
    connection.initialize_finish(id, result)?;
    let mut config = options.config.clone();
    config.syntax.variant = settings.variant;
    config.syntax.tolerant = settings.tolerant;
    config.correction.enabled = settings.correction;
    let tables = match load_embedded() {
        Ok(tables) => Some(tables),
        Err(error) => {
            warnings.push(format!(
                "correction tables cannot be loaded: {error}; only errors of the parser are reported"
            ));
            None
        }
    };
    let mut session = Session {
        connection: &connection,
        commands: Commands::new(options.commands.clone()),
        variant: settings.variant,
        encoding,
        documents: Documents::default(),
        config,
        tables,
        catalog: options.catalog.clone(),
        help: options.help.clone(),
        context: BindContext::for_host(),
        analyses: BTreeMap::new(),
    };
    for warning in warnings {
        session.notify_client::<ShowMessage>(ShowMessageParams {
            typ: MessageType::WARNING,
            message: warning,
        })?;
    }
    session.run()
}

fn initialize_result(encoding: Encoding) -> InitializeResult {
    let completion = CompletionOptions {
        trigger_characters: Some(strings(&TRIGGER_CHARACTERS)),
        ..CompletionOptions::default()
    };
    let signature_help = SignatureHelpOptions {
        trigger_characters: Some(strings(&SIGNATURE_TRIGGER_CHARACTERS)),
        ..SignatureHelpOptions::default()
    };
    let semantic_tokens = SemanticTokensOptions {
        legend: semantic::legend(),
        full: Some(SemanticTokensFullOptions::Bool(true)),
        ..SemanticTokensOptions::default()
    };
    let code_actions = CodeActionOptions {
        code_action_kinds: Some(vec![CodeActionKind::QUICKFIX]),
        ..CodeActionOptions::default()
    };
    InitializeResult {
        capabilities: ServerCapabilities {
            position_encoding: Some(encoding.kind()),
            text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
            completion_provider: Some(completion),
            hover_provider: Some(HoverProviderCapability::Simple(true)),
            signature_help_provider: Some(signature_help),
            document_formatting_provider: Some(OneOf::Left(true)),
            code_action_provider: Some(CodeActionProviderCapability::Options(code_actions)),
            semantic_tokens_provider: Some(
                SemanticTokensServerCapabilities::SemanticTokensOptions(semantic_tokens),
            ),
            ..ServerCapabilities::default()
        },
        server_info: Some(ServerInfo {
            name: SERVER_NAME.to_owned(),
            version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        }),
    }
}

fn strings(characters: &[&str]) -> Vec<String> {
    characters
        .iter()
        .map(|character| (*character).to_owned())
        .collect()
}

fn send(connection: &Connection, message: Message) -> Result<(), ServerError> {
    connection
        .sender
        .send(message)
        .map_err(|_| ServerError::Disconnected)
}

/// Failure of a request, answered with an error response.
#[derive(Debug)]
struct Failure {
    code: ErrorCode,
    message: String,
}

impl Failure {
    fn new(code: ErrorCode, message: String) -> Self {
        Self { code, message }
    }
}

/// State of an initialized session.
struct Session<'a> {
    connection: &'a Connection,
    commands: Commands,
    variant: Variant,
    encoding: Encoding,
    documents: Documents,
    config: Config,
    tables: Option<Tables>,
    catalog: Option<Arc<Catalog>>,
    help: Option<Arc<HelpCatalog>>,
    context: BindContext,
    /// Diagnostics and quick fixes of the open documents.
    analyses: BTreeMap<Uri, Analysis>,
}

impl Session<'_> {
    fn run(&mut self) -> Result<(), ServerError> {
        let connection = self.connection;
        for message in &connection.receiver {
            match message {
                Message::Request(request) => {
                    if connection.handle_shutdown(&request)? {
                        return Ok(());
                    }
                    let response = self.respond(request);
                    send(connection, response.into())?;
                }
                Message::Notification(notification) if notification.method == Exit::METHOD => {
                    return Err(ServerError::ExitWithoutShutdown);
                }
                Message::Notification(notification) => self.handle(notification)?,
                Message::Response(_) => {}
            }
        }
        Err(ServerError::Disconnected)
    }

    fn respond(&self, request: Request) -> Response {
        let Request { id, method, params } = request;
        let result = match method.as_str() {
            Completion::METHOD => request_params::<Completion>(params).and_then(|params| {
                let position = params.text_document_position;
                let items = self.assist(&position.text_document.uri).map(|request| {
                    CompletionResponse::Array(assist::completion(request, position.position))
                });
                encode(serde_json::to_value(items))
            }),
            HoverRequest::METHOD => request_params::<HoverRequest>(params).and_then(|params| {
                let position = params.text_document_position_params;
                let hover = self
                    .assist(&position.text_document.uri)
                    .and_then(|request| assist::hover(request, position.position));
                encode(serde_json::to_value(hover))
            }),
            SignatureHelpRequest::METHOD => request_params::<SignatureHelpRequest>(params)
                .and_then(|params| {
                    let position = params.text_document_position_params;
                    let help = self
                        .assist(&position.text_document.uri)
                        .and_then(|request| signature::help(request, position.position));
                    encode(serde_json::to_value(help))
                }),
            SemanticTokensFullRequest::METHOD => {
                request_params::<SemanticTokensFullRequest>(params).and_then(|params| {
                    let tokens = self.assist(&params.text_document.uri).map(|request| {
                        SemanticTokensResult::Tokens(SemanticTokens {
                            result_id: None,
                            data: semantic::tokens(
                                request.text,
                                request.variant,
                                request.encoding,
                                request.commands,
                            ),
                        })
                    });
                    encode(serde_json::to_value(tokens))
                })
            }
            CodeActionRequest::METHOD => {
                request_params::<CodeActionRequest>(params).and_then(|params| {
                    let uri = params.text_document.uri;
                    let actions = self
                        .analyses
                        .get(&uri)
                        .map(|analysis| analysis.actions(&uri, params.range));
                    encode(serde_json::to_value(actions))
                })
            }
            Formatting::METHOD => request_params::<Formatting>(params)
                .and_then(|params| self.formatting(&params.text_document.uri)),
            _ => Err(Failure::new(
                ErrorCode::MethodNotFound,
                format!("unsupported method {method}"),
            )),
        };
        match result {
            Ok(value) => Response::new_ok(id, value),
            Err(failure) => Response::new_err(id, failure.code as i32, failure.message),
        }
    }

    fn assist(&self, uri: &Uri) -> Option<assist::Request<'_>> {
        let document = self.documents.get(uri)?;
        let diagnostics = self
            .analyses
            .get(uri)
            .map_or(&[][..], |analysis| analysis.diagnostics.as_slice());
        Some(assist::Request {
            text: &document.text,
            variant: self.variant,
            encoding: self.encoding,
            commands: &self.commands,
            help: self.help.as_deref(),
            diagnostics,
        })
    }

    fn formatting(&self, uri: &Uri) -> Result<Value, Failure> {
        let Some(document) = self.documents.get(uri) else {
            return Ok(Value::Null);
        };
        let edits = analysis::formatting(&document.text, self.variant, self.encoding)
            .map_err(|message| Failure::new(ErrorCode::RequestFailed, message))?;
        encode(serde_json::to_value(edits))
    }

    fn handle(&mut self, notification: Notification) -> Result<(), ServerError> {
        let Notification { method, params } = notification;
        match method.as_str() {
            DidOpenTextDocument::METHOD => {
                let Some(params) = self.notification_params::<DidOpenTextDocument>(params)? else {
                    return Ok(());
                };
                let document = params.text_document;
                self.documents
                    .open(document.uri.clone(), document.text, document.version);
                self.publish(document.uri)
            }
            DidChangeTextDocument::METHOD => {
                let Some(params) = self.notification_params::<DidChangeTextDocument>(params)?
                else {
                    return Ok(());
                };
                let document = params.text_document;
                let changed = self.documents.change(
                    &document.uri,
                    document.version,
                    params.content_changes,
                    self.encoding,
                );
                if changed {
                    self.publish(document.uri)
                } else {
                    Ok(())
                }
            }
            DidCloseTextDocument::METHOD => {
                let Some(params) = self.notification_params::<DidCloseTextDocument>(params)? else {
                    return Ok(());
                };
                let uri = params.text_document.uri;
                self.documents.close(&uri);
                self.analyses.remove(&uri);
                self.notify_client::<PublishDiagnostics>(PublishDiagnosticsParams::new(
                    uri,
                    Vec::new(),
                    None,
                ))
            }
            _ => Ok(()),
        }
    }

    /// Analyzes an open document, keeps its quick fixes and publishes its
    /// diagnostics.
    fn publish(&mut self, uri: Uri) -> Result<(), ServerError> {
        let Some(document) = self.documents.get(&uri) else {
            return Ok(());
        };
        let engine = Engine {
            variant: self.variant,
            encoding: self.encoding,
            config: &self.config,
            tables: self.tables.as_ref(),
            catalog: self.catalog.as_deref(),
            commands: &self.commands,
            context: &self.context,
        };
        let analysis = engine.analyze(&document.text);
        let params = PublishDiagnosticsParams::new(
            uri.clone(),
            analysis.diagnostics.clone(),
            Some(document.version),
        );
        self.analyses.insert(uri, analysis);
        self.notify_client::<PublishDiagnostics>(params)
    }

    /// Decodes the parameters of a notification; invalid parameters are
    /// reported to the client log and the notification is ignored.
    fn notification_params<N>(&self, params: Value) -> Result<Option<N::Params>, ServerError>
    where
        N: lsp_types::notification::Notification,
    {
        match serde_json::from_value(params) {
            Ok(params) => Ok(Some(params)),
            Err(error) => {
                let message = format!("invalid parameters of {}: {error}", N::METHOD);
                self.notify_client::<LogMessage>(LogMessageParams {
                    typ: MessageType::ERROR,
                    message,
                })?;
                Ok(None)
            }
        }
    }

    fn notify_client<N>(&self, params: N::Params) -> Result<(), ServerError>
    where
        N: lsp_types::notification::Notification,
    {
        send(
            self.connection,
            Notification::new(N::METHOD.to_owned(), params).into(),
        )
    }
}

fn request_params<R>(params: Value) -> Result<R::Params, Failure>
where
    R: lsp_types::request::Request,
{
    serde_json::from_value(params).map_err(|error| {
        Failure::new(
            ErrorCode::InvalidParams,
            format!("invalid parameters of {}: {error}", R::METHOD),
        )
    })
}

fn encode(result: serde_json::Result<Value>) -> Result<Value, Failure> {
    result.map_err(|error| Failure::new(ErrorCode::InternalError, error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use lsp_types::PositionEncodingKind;
    use serde_json::json;

    use super::*;

    #[test]
    fn errors_name_their_cause() {
        let invalid = serde_json::from_str::<Value>("{").unwrap_err();
        assert!(
            ServerError::Encode(invalid)
                .to_string()
                .starts_with("cannot encode a message: ")
        );
        assert_eq!(
            ServerError::Io(std::io::Error::other("pipe closed")).to_string(),
            "standard stream error: pipe closed"
        );
        assert_eq!(
            ServerError::Initialize("no capabilities".to_owned()).to_string(),
            "invalid initialize parameters: no capabilities"
        );
        assert_eq!(
            ServerError::Disconnected.to_string(),
            "the client closed the connection before shutdown"
        );
        assert_eq!(
            ServerError::ExitWithoutShutdown.to_string(),
            "the client sent exit before shutdown"
        );
    }

    #[test]
    fn only_errors_that_wrap_another_error_have_a_source() {
        let invalid = serde_json::from_str::<Value>("{").unwrap_err();
        assert!(ServerError::Encode(invalid).source().is_some());
        assert!(
            ServerError::Io(std::io::Error::other("x"))
                .source()
                .is_some()
        );
        assert!(ServerError::Initialize("x".to_owned()).source().is_none());
        assert!(ServerError::Disconnected.source().is_none());
        assert!(ServerError::ExitWithoutShutdown.source().is_none());
    }

    #[test]
    fn the_capabilities_follow_the_negotiated_encoding() {
        for (encoding, kind) in [
            (Encoding::Utf16, PositionEncodingKind::UTF16),
            (Encoding::Utf32, PositionEncodingKind::UTF32),
        ] {
            let result = initialize_result(encoding);
            assert_eq!(result.capabilities.position_encoding, Some(kind));
        }
    }

    #[test]
    fn the_server_offers_the_documented_features() {
        let result = serde_json::to_value(initialize_result(Encoding::Utf16)).unwrap();
        let capabilities = &result["capabilities"];
        assert_eq!(result["serverInfo"]["name"], "nurt");
        assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(capabilities["textDocumentSync"], 1);
        assert_eq!(capabilities["hoverProvider"], true);
        assert_eq!(capabilities["documentFormattingProvider"], true);
        assert_eq!(
            capabilities["completionProvider"]["triggerCharacters"],
            json!([":", "=", "@", "|", ">", " ", "(", ","])
        );
        assert_eq!(
            capabilities["signatureHelpProvider"]["triggerCharacters"],
            json!([" ", "(", ",", ":", "="])
        );
        assert_eq!(
            capabilities["codeActionProvider"]["codeActionKinds"],
            json!(["quickfix"])
        );
        assert_eq!(capabilities["semanticTokensProvider"]["full"], true);
        assert_eq!(
            capabilities["semanticTokensProvider"]["legend"]["tokenModifiers"],
            json!(["declaration"])
        );
    }

    #[test]
    fn strings_copies_every_character() {
        assert_eq!(
            strings(&["a", "\u{142}"]),
            vec!["a".to_owned(), "\u{142}".to_owned()]
        );
        assert!(strings(&[]).is_empty());
    }

    #[test]
    fn invalid_request_parameters_are_an_invalid_params_failure() {
        let failure = request_params::<HoverRequest>(json!({"position": 1})).unwrap_err();
        assert_eq!(failure.code as i32, -32602);
        assert!(
            failure
                .message
                .starts_with("invalid parameters of textDocument/hover: "),
            "{}",
            failure.message
        );
        assert!(
            request_params::<HoverRequest>(json!({
                "textDocument": {"uri": "file:///work/a.nurt"},
                "position": {"line": 0, "character": 0}
            }))
            .is_ok()
        );
    }

    #[test]
    fn an_encoding_error_is_an_internal_error() {
        let invalid = serde_json::from_str::<Value>("{").unwrap_err();
        let failure = encode(Err(invalid)).unwrap_err();
        assert_eq!(failure.code as i32, -32603);
        assert_eq!(encode(Ok(json!(1))).unwrap(), json!(1));
    }

    #[test]
    fn sending_to_a_closed_connection_is_a_disconnection() {
        let (server, client) = Connection::memory();
        drop(client);
        let message = Notification::new("window/logMessage".to_owned(), json!({})).into();
        assert!(matches!(
            send(&server, message),
            Err(ServerError::Disconnected)
        ));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
