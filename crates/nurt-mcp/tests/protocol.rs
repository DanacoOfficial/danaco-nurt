// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Protocol behavior over a byte stream, against the mock engine.

// Helpers abort the test on a broken expectation, as test code does.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{CORE_DESCRIPTION, Client, MockBackend};
use nurt_mcp::McpOptions;
use nurt_tooldef::{RenderOptions, ToolFormat};
use serde_json::{Value, json};

fn text(result: &Value) -> &str {
    result["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
}

#[tokio::test]
async fn initialize_reports_server_identity_and_capabilities() {
    let mut client = Client::start(MockBackend::new());
    let response = client.initialize().await;
    let result = &response["result"];
    assert_eq!(result["protocolVersion"], "2025-06-18");
    assert_eq!(
        result["serverInfo"],
        json!({"name": "nurt", "title": "Danaco Nurt", "version": "1.4.0"})
    );
    for capability in ["tools", "resources", "logging"] {
        assert!(
            result["capabilities"][capability].is_object(),
            "{capability}"
        );
    }
    let (result, rest) = client.close().await;
    assert!(result.is_ok());
    assert!(rest.is_empty());
}

#[tokio::test]
async fn every_supported_version_is_accepted_and_newer_ones_fall_back() {
    for (requested, negotiated) in [
        ("2024-11-05", "2024-11-05"),
        ("2025-03-26", "2025-03-26"),
        ("2025-06-18", "2025-06-18"),
        ("2025-11-25", "2025-11-25"),
        ("2026-07-28", "2025-11-25"),
        ("2099-01-01", "2025-11-25"),
    ] {
        let mut client = Client::start(MockBackend::new());
        let (response, _) = client
            .call(
                1,
                "initialize",
                json!({
                    "protocolVersion": requested,
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "1.0.0"}
                }),
            )
            .await;
        assert_eq!(
            response["result"]["protocolVersion"], negotiated,
            "{requested}"
        );
        let (result, _) = client.close().await;
        assert!(result.is_ok());
    }
}

#[tokio::test]
async fn first_message_other_than_initialize_is_an_initialization_error() {
    let mut client = Client::start(MockBackend::new());
    client
        .send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .await;
    let (result, _) = client.close().await;
    assert!(
        matches!(result, Err(nurt_mcp::McpError::Initialize(_))),
        "{result:?}"
    );
}

#[tokio::test]
async fn closing_before_initialize_ends_without_error() {
    let client = Client::start(MockBackend::new());
    let (result, rest) = client.close().await;
    assert!(result.is_ok());
    assert!(rest.is_empty());
}

#[tokio::test]
async fn tools_list_carries_the_mcp_tool_definition() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    let (response, _) = client.call(1, "tools/list", json!({})).await;
    let tools = response["result"]["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert_eq!(names, ["nurt", "nurt_help", "nurt_check"]);

    let expected =
        nurt_tooldef::definition(ToolFormat::Mcp, &RenderOptions::new(CORE_DESCRIPTION)).unwrap();
    assert_eq!(tools[0]["inputSchema"], expected["inputSchema"]);
    assert_eq!(tools[0]["description"], expected["description"]);
    assert_eq!(tools[0]["annotations"]["destructiveHint"], true);
    assert_eq!(tools[1]["annotations"]["readOnlyHint"], true);
}

#[tokio::test]
async fn successful_call_returns_text_and_report() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    let (response, _) = client.tool(1, "nurt", json!({"program": "ok"})).await;
    assert_eq!(text(&response), "ok");
    assert_eq!(response["result"]["isError"], false);
    assert_eq!(response["result"]["structuredContent"]["status"], "ok");

    let (response, _) = client
        .tool(2, "nurt", json!({"program": "ok", "format": "json"}))
        .await;
    let rendered: Value = serde_json::from_str(text(&response)).unwrap();
    assert_eq!(rendered, response["result"]["structuredContent"]);
}

#[tokio::test]
async fn language_error_sets_is_error() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    let (response, _) = client.tool(1, "nurt", json!({"program": "fail"})).await;
    assert_eq!(response["result"]["isError"], true);
    assert_eq!(text(&response), "error E_SYNTAX: unexpected token");
    assert_eq!(
        response["result"]["structuredContent"]["error"]["code"],
        "E_SYNTAX"
    );
}

#[tokio::test]
async fn call_error_and_invalid_arguments_set_is_error() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    let (response, _) = client
        .tool(1, "nurt", json!({"program": "call-error"}))
        .await;
    assert_eq!(response["result"]["isError"], true);
    assert_eq!(
        text(&response),
        "call failed: NURT_E_IO: journal is not writable"
    );

    let (response, _) = client
        .tool(2, "nurt", json!({"program": "ok", "colour": "red"}))
        .await;
    assert_eq!(response["result"]["isError"], true);
    assert!(text(&response).contains("colour"), "{response}");
}

#[tokio::test]
async fn unknown_tool_is_a_protocol_error() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    let (response, _) = client.tool(1, "bash", json!({"command": "ls"})).await;
    assert_eq!(response["error"]["code"], -32602);
    assert_eq!(
        response["error"]["message"],
        "unknown tool `bash`; available tools: nurt, nurt_help, nurt_check"
    );
}

#[tokio::test]
async fn check_tool_returns_the_check_document() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    let (response, _) = client.tool(1, "nurt_check", json!({"program": "ok"})).await;
    let result = &response["result"];
    assert_eq!(result["isError"], false);
    assert_eq!(result["structuredContent"]["format"], "nurt-check/1");
    assert_eq!(result["structuredContent"]["program"], "ok");
    let rendered: Value = serde_json::from_str(text(&response)).unwrap();
    assert_eq!(rendered, result["structuredContent"]);
}

#[tokio::test]
async fn check_tool_reports_a_rejected_program_as_an_error_result() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    let (response, _) = client
        .tool(1, "nurt_check", json!({"program": "fail"}))
        .await;
    assert_eq!(response["result"]["isError"], true);
    assert_eq!(text(&response), "error E_SYNTAX: unexpected token");
    assert_eq!(
        response["result"]["structuredContent"]["error"]["code"],
        "E_SYNTAX"
    );
}

#[tokio::test]
async fn check_tool_reports_call_and_argument_errors_as_text() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    let (failed, _) = client
        .tool(1, "nurt_check", json!({"program": "call-error"}))
        .await;
    assert_eq!(failed["result"]["isError"], true);
    assert_eq!(
        text(&failed),
        "call failed: NURT_E_IO: journal is not writable"
    );
    let (invalid, _) = client.tool(2, "nurt_check", json!({})).await;
    assert_eq!(invalid["result"]["isError"], true);
    assert!(
        text(&invalid).contains("exactly one of"),
        "{}",
        text(&invalid)
    );
}

#[tokio::test]
async fn help_tool_passes_the_query() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    let (response, _) = client
        .tool(
            1,
            "nurt_help",
            json!({"topic": "replace", "kind": "command", "examples": 6, "language": "pl"}),
        )
        .await;
    assert_eq!(
        text(&response),
        "help topic=replace kind=command examples=6 language=pl"
    );
    assert_eq!(response["result"]["isError"], false);

    let (response, _) = client.tool(2, "nurt_help", json!({"depth": 1})).await;
    assert_eq!(response["result"]["isError"], true);
}

#[tokio::test]
async fn resources_are_read_by_uri() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    let cases = [
        (
            "nurt://help/E_SYNTAX",
            "text/plain",
            "help topic=E_SYNTAX kind=any examples=3 language=en",
        ),
        ("nurt://describe/core", "text/plain", CORE_DESCRIPTION),
        (
            "nurt://describe/group/git",
            "text/plain",
            "Group git: repository commands.",
        ),
        (
            "nurt://schema/tool",
            "application/schema+json",
            r#"{"title":"nurt-tool/1"}"#,
        ),
    ];
    for (id, (uri, mime, expected)) in (1_u64..).zip(cases) {
        let (response, _) = client.call(id, "resources/read", json!({"uri": uri})).await;
        let contents = &response["result"]["contents"][0];
        assert_eq!(contents["uri"], uri);
        assert_eq!(contents["mimeType"], mime);
        assert_eq!(contents["text"], expected, "{uri}");
    }
    let (response, _) = client
        .call(9, "resources/read", json!({"uri": "nurt://schema/report"}))
        .await;
    let schema: Value =
        serde_json::from_str(response["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(schema["title"], "nurt-report/1");
}

#[tokio::test]
async fn missing_and_unreadable_resources_are_protocol_errors() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    let (response, _) = client
        .call(
            1,
            "resources/read",
            json!({"uri": "nurt://describe/group/none"}),
        )
        .await;
    assert_eq!(response["error"]["code"], -32002);
    let (response, _) = client
        .call(2, "resources/read", json!({"uri": "file:///etc/hosts"}))
        .await;
    assert_eq!(response["error"]["code"], -32002);
    assert_eq!(response["error"]["data"]["uri"], "file:///etc/hosts");
    let (response, _) = client
        .call(3, "resources/read", json!({"uri": "nurt://schema/config"}))
        .await;
    assert_eq!(response["error"]["code"], -32603);
}

#[tokio::test]
async fn resource_lists_name_every_resource() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    let (response, _) = client.call(1, "resources/list", json!({})).await;
    let uris: Vec<&str> = response["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|resource| resource["uri"].as_str())
        .collect();
    assert_eq!(
        uris,
        [
            "nurt://schema/ast",
            "nurt://schema/bound",
            "nurt://schema/report",
            "nurt://schema/request",
            "nurt://schema/config",
            "nurt://schema/tool",
            "nurt://describe/core",
        ]
    );
    let (response, _) = client.call(2, "resources/templates/list", json!({})).await;
    let templates: Vec<&str> = response["result"]["resourceTemplates"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|template| template["uriTemplate"].as_str())
        .collect();
    assert_eq!(
        templates,
        [
            "nurt://help/{topic}",
            "nurt://schema/{name}",
            "nurt://describe/group/{name}"
        ]
    );
}

#[tokio::test]
async fn progress_token_turns_events_into_progress_notifications() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    let (response, notifications) = client
        .call(
            1,
            "tools/call",
            json!({"name": "nurt", "arguments": {"program": "progress"}, "_meta": {"progressToken": "p1"}}),
        )
        .await;
    assert_eq!(text(&response), "ok");
    let progress: Vec<(f64, &str)> = notifications
        .iter()
        .filter(|message| message["method"] == "notifications/progress")
        .map(|message| {
            assert_eq!(message["params"]["progressToken"], "p1");
            (
                message["params"]["progress"].as_f64().unwrap(),
                message["params"]["message"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        progress,
        [
            (1.0, "find scan: 1/2"),
            (2.0, "half done"),
            (3.0, "line one")
        ]
    );
    let logs: Vec<&Value> = notifications
        .iter()
        .filter(|message| message["method"] == "notifications/message")
        .collect();
    assert_eq!(logs.len(), 1);
    assert_eq!(
        logs[0]["params"],
        json!({"level": "warning", "logger": "space", "data": "slow volume"})
    );
}

#[tokio::test]
async fn without_progress_token_only_logs_are_sent() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    let (response, _) = client
        .call(1, "logging/setLevel", json!({"level": "debug"}))
        .await;
    assert_eq!(response["result"], json!({}));
    let (_, notifications) = client.tool(2, "nurt", json!({"program": "progress"})).await;
    let methods: Vec<&str> = notifications
        .iter()
        .filter_map(|message| message["method"].as_str())
        .collect();
    assert_eq!(methods, ["notifications/message", "notifications/message"]);
    assert_eq!(notifications[1]["params"]["logger"], "nurt");
    assert_eq!(notifications[1]["params"]["level"], "info");

    client
        .call(3, "logging/setLevel", json!({"level": "error"}))
        .await;
    let (_, notifications) = client.tool(4, "nurt", json!({"program": "progress"})).await;
    assert!(notifications.is_empty(), "{notifications:?}");
}

#[tokio::test]
async fn cancelled_notification_stops_the_program() {
    let backend = MockBackend::new();
    let mut client = Client::start(backend.clone());
    client.initialize().await;
    client
        .send(&json!({
            "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": {"name": "nurt", "arguments": {"program": "wait"}}
        }))
        .await;
    assert!(backend.wait_started(1).await);
    client.cancel(7).await;
    let (response, _) = client.call(8, "ping", json!({})).await;
    assert_eq!(response["result"], json!({}));
    assert!(backend.wait_cancelled(1).await);
    let (result, rest) = client.close().await;
    assert!(result.is_ok());
    assert!(
        rest.iter()
            .all(|message| message.get("id") != Some(&json!(7))),
        "{rest:?}"
    );
}

#[tokio::test]
async fn calls_run_concurrently() {
    let backend = MockBackend::with_limits(4, 3);
    let mut client = Client::start(backend);
    client.initialize().await;
    for id in 1..=3 {
        client
            .send(&json!({
                "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": {"name": "nurt", "arguments": {"program": "rendezvous"}}
            }))
            .await;
    }
    let mut answered = Vec::new();
    while answered.len() < 3 {
        let message = client.recv().await;
        assert_eq!(text(&message), "met", "{message}");
        answered.push(message["id"].as_u64().unwrap());
    }
    answered.sort_unstable();
    assert_eq!(answered, [1, 2, 3]);
}

#[tokio::test]
async fn exhausted_limit_is_a_protocol_error() {
    let backend = MockBackend::with_limits(1, 1);
    let mut client = Client::start(backend.clone());
    client.initialize().await;
    client
        .send(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "nurt", "arguments": {"program": "wait"}}
        }))
        .await;
    assert!(backend.wait_started(1).await);
    let (response, _) = client.tool(2, "nurt", json!({"program": "ok"})).await;
    assert_eq!(response["error"]["code"], -32000);
    assert_eq!(
        response["error"]["message"],
        "too many calls in progress (limit 1); retry when a call completes"
    );
    let (response, _) = client
        .call(3, "resources/read", json!({"uri": "nurt://describe/core"}))
        .await;
    assert_eq!(response["error"]["code"], -32000);
    client.cancel(1).await;
    assert!(backend.wait_cancelled(1).await);
    let (result, _) = client.close().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn default_session_applies_to_calls() {
    let mut client = Client::start_with(MockBackend::new(), McpOptions::new().with_session("s9"));
    client.initialize().await;
    let (response, _) = client.tool(1, "nurt", json!({"program": "session"})).await;
    assert_eq!(text(&response), "s9");
    let (response, _) = client
        .tool(
            2,
            "nurt",
            json!({"program": "session", "session": "agent-2"}),
        )
        .await;
    assert_eq!(text(&response), "agent-2");
}

#[tokio::test]
async fn malformed_json_is_ignored_and_unknown_methods_are_rejected() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    client.send_raw("{\"jsonrpc\": \"2.0\", \"id\": ").await;
    let (response, _) = client.call(2, "nurt/unknown", json!({})).await;
    assert_eq!(response["error"]["code"], -32601);
    let (response, _) = client.call(3, "ping", json!({})).await;
    assert_eq!(response["result"], json!({}));
}

#[tokio::test]
async fn help_without_arguments_asks_for_the_index_of_topics() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    let (response, _) = client
        .call(1, "tools/call", json!({"name": "nurt_help"}))
        .await;
    assert_eq!(
        text(&response),
        "help topic=index kind=any examples=3 language=en"
    );
    assert_eq!(response["result"]["isError"], false);
}

#[tokio::test]
async fn percent_encoded_topics_of_resources_are_decoded_and_the_uri_is_kept() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    let uri = "nurt://help/git%2Ecommit";
    let (response, _) = client.call(1, "resources/read", json!({"uri": uri})).await;
    let contents = &response["result"]["contents"][0];
    assert_eq!(contents["uri"], uri);
    assert_eq!(
        contents["text"],
        "help topic=git.commit kind=any examples=3 language=en"
    );
    let (response, _) = client
        .call(2, "resources/read", json!({"uri": "nurt://help/caf%C3%A9"}))
        .await;
    assert_eq!(
        response["result"]["contents"][0]["text"],
        "help topic=caf\u{e9} kind=any examples=3 language=en"
    );
}

#[tokio::test]
async fn uris_that_name_no_resource_are_not_found() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    for (id, uri) in (1_u64..).zip([
        "nurt://help/",
        "nurt://help/a%2",
        "nurt://help/%FF",
        "nurt://help/a/b",
        "nurt://schema/catalog",
        "nurt://describe/group/",
        "nurt://other/x",
    ]) {
        let (response, _) = client.call(id, "resources/read", json!({"uri": uri})).await;
        assert_eq!(response["error"]["code"], -32002, "{uri}");
        assert_eq!(response["error"]["data"]["uri"], uri, "{uri}");
    }
}

#[tokio::test]
async fn check_tool_takes_a_candidate_and_the_default_session() {
    let mut client = Client::start_with(MockBackend::new(), McpOptions::new().with_session("s9"));
    client.initialize().await;
    let (response, _) = client.tool(1, "nurt_check", json!({"accept": "c1"})).await;
    let document = &response["result"]["structuredContent"];
    assert_eq!(document["program"], "c1");
    assert_eq!(document["session"], "s9");
    let (response, _) = client
        .tool(2, "nurt_check", json!({"program": "ok", "session": "own"}))
        .await;
    assert_eq!(response["result"]["structuredContent"]["session"], "own");
}

#[tokio::test]
async fn invalid_arguments_of_the_check_and_help_tools_name_the_tool() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    let (response, _) = client
        .tool(1, "nurt_check", json!({"program": "ok", "accept": "c1"}))
        .await;
    assert_eq!(response["result"]["isError"], true);
    assert!(
        text(&response).contains("`nurt_check`"),
        "{}",
        text(&response)
    );
    let (response, _) = client.tool(2, "nurt_help", json!({"kind": "page"})).await;
    assert_eq!(response["result"]["isError"], true);
    assert!(
        text(&response).contains("`nurt_help`"),
        "{}",
        text(&response)
    );
}

#[tokio::test]
async fn tools_list_is_the_same_on_every_call_and_names_the_engine_version() {
    let mut client = Client::start(MockBackend::new());
    let initialized = client.initialize().await;
    assert_eq!(initialized["result"]["serverInfo"]["version"], "1.4.0");
    let (first, _) = client.call(1, "tools/list", json!({})).await;
    let (second, _) = client.call(2, "tools/list", json!({})).await;
    assert_eq!(first["result"], second["result"]);
}

#[tokio::test]
async fn tools_annotations_tell_which_tools_only_read() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    let (response, _) = client.call(1, "tools/list", json!({})).await;
    let tools = response["result"]["tools"].as_array().unwrap();
    let read_only: Vec<(&str, &Value)> = tools
        .iter()
        .map(|tool| {
            (
                tool["name"].as_str().unwrap(),
                &tool["annotations"]["readOnlyHint"],
            )
        })
        .collect();
    assert_eq!(
        read_only,
        [
            ("nurt", &json!(false)),
            ("nurt_help", &json!(true)),
            ("nurt_check", &json!(true))
        ]
    );
    assert_eq!(tools[2]["annotations"]["openWorldHint"], false);
    assert_eq!(tools[0]["annotations"]["openWorldHint"], true);
}

#[tokio::test]
async fn a_program_with_multi_byte_characters_reaches_the_engine_and_the_reply_is_valid() {
    let mut client = Client::start(MockBackend::new());
    client.initialize().await;
    let (response, _) = client
        .tool(
            1,
            "nurt",
            json!({"program": "\u{142}\u{f3}d\u{17a} \u{1f600}"}),
        )
        .await;
    assert_eq!(response["result"]["isError"], true);
    assert_eq!(
        response["result"]["structuredContent"]["error"]["message"],
        "unknown program \u{142}\u{f3}d\u{17a} \u{1f600}"
    );
}
