// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! JSON over HTTP with repeated attempts for the provider adapters.

use std::thread;
use std::time::Duration;

use serde_json::Value;

use crate::error::{BenchError, Result};

/// HTTP status of a rate limit response.
const STATUS_TOO_MANY_REQUESTS: u16 = 429;

/// Lowest HTTP status of a server error.
const STATUS_SERVER_ERROR: u16 = 500;

/// Largest number of characters of a response body quoted in an error.
const ERROR_BODY_CHARS: usize = 500;

/// Settings of the HTTP client of one provider.
#[derive(Debug, Clone)]
pub(crate) struct Settings {
    /// Provider name for error messages.
    pub(crate) provider: &'static str,
    /// Time limit of one request.
    pub(crate) timeout: Duration,
    /// Number of repeated attempts after a retryable failure.
    pub(crate) retries: u32,
    /// Delay before the first repeated attempt.
    pub(crate) retry_delay: Duration,
}

/// HTTP client of one provider.
pub(crate) struct HttpClient {
    agent: ureq::Agent,
    settings: Settings,
}

impl HttpClient {
    pub(crate) fn new(settings: Settings) -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(settings.timeout))
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            settings,
        }
    }

    /// Sends a JSON body with `POST` and returns the JSON response, repeating
    /// the request after rate limits, server errors and connection failures.
    pub(crate) fn post(
        &self,
        url: &str,
        headers: &[(&str, String)],
        body: &Value,
    ) -> Result<Value> {
        let mut delay = self.settings.retry_delay;
        let mut attempt = 0;
        loop {
            match self.post_once(url, headers, body) {
                Err(error) if error.is_retryable() && attempt < self.settings.retries => {
                    attempt += 1;
                    thread::sleep(delay);
                    delay = delay.saturating_mul(2);
                }
                result => return result,
            }
        }
    }

    fn post_once(&self, url: &str, headers: &[(&str, String)], body: &Value) -> Result<Value> {
        let mut request = self.agent.post(url).content_type("application/json");
        for (name, value) in headers {
            request = request.header(*name, value.as_str());
        }
        let mut response = request
            .send(body.to_string().as_bytes())
            .map_err(|error| self.error(format!("request failed: {error}"), true))?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|error| self.error(format!("cannot read the response: {error}"), true))?;
        if !(200..300).contains(&status) {
            let retryable = status == STATUS_TOO_MANY_REQUESTS || status >= STATUS_SERVER_ERROR;
            let quoted: String = text.chars().take(ERROR_BODY_CHARS).collect();
            return Err(self.error(format!("HTTP status {status}: {quoted}"), retryable));
        }
        serde_json::from_str(&text)
            .map_err(|error| self.error(format!("the response is not JSON: {error}"), false))
    }

    /// Creates a provider error.
    pub(crate) fn error(&self, message: String, retryable: bool) -> BenchError {
        BenchError::Provider {
            provider: self.settings.provider.to_owned(),
            message,
            retryable,
        }
    }
}

#[cfg(test)]
pub(crate) mod testserver {
    //! Local HTTP server for adapter tests: answers each connection with the
    //! next prepared response and records the requests.

    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    /// Request received by the server.
    #[derive(Debug, Clone)]
    pub(crate) struct Received {
        pub(crate) path: String,
        pub(crate) headers: Vec<(String, String)>,
        pub(crate) body: serde_json::Value,
    }

    impl Received {
        pub(crate) fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        }
    }

    /// Starts a server that answers with `(status, body)` pairs in order and
    /// returns its base address and the log of requests.
    pub(crate) fn serve(responses: Vec<(u16, String)>) -> (String, Arc<Mutex<Vec<Received>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let log = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&log);
        thread::spawn(move || {
            for (status, body) in responses {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let path = line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_owned();
                let mut headers = Vec::new();
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    let header = header.trim_end();
                    if header.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':') {
                        if name.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse().unwrap();
                        }
                        headers.push((name.to_owned(), value.trim().to_owned()));
                    }
                }
                let mut content = vec![0; length];
                reader.read_exact(&mut content).unwrap();
                sink.lock().unwrap().push(Received {
                    path,
                    headers,
                    body: serde_json::from_slice(&content).unwrap_or_default(),
                });
                let mut stream = reader.into_inner();
                write!(
                    stream,
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        (address, log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> HttpClient {
        HttpClient::new(Settings {
            provider: "test",
            timeout: Duration::from_secs(10),
            retries: 2,
            retry_delay: Duration::ZERO,
        })
    }

    #[test]
    fn repeats_after_server_errors() {
        let (base, log) = testserver::serve(vec![
            (503, "busy".into()),
            (429, "slow down".into()),
            (200, r#"{"ok":true}"#.into()),
        ]);
        let value = client()
            .post(
                &format!("{base}/v1"),
                &[("x-key", "k".into())],
                &serde_json::json!({"a": 1}),
            )
            .unwrap();
        assert_eq!(value["ok"], true);
        let log = log.lock().unwrap();
        assert_eq!(log.len(), 3);
        assert_eq!(log[0].path, "/v1");
        assert_eq!(log[0].header("x-key"), Some("k"));
        assert_eq!(log[0].body["a"], 1);
    }

    #[test]
    fn does_not_repeat_client_errors() {
        let (base, log) = testserver::serve(vec![(400, "bad".into()), (200, "{}".into())]);
        let error = client()
            .post(&base, &[], &serde_json::json!({}))
            .unwrap_err();
        assert!(!error.is_retryable());
        assert!(error.to_string().contains("HTTP status 400: bad"));
        assert_eq!(log.lock().unwrap().len(), 1);
    }

    #[test]
    fn gives_up_after_the_last_attempt() {
        let (base, _) = testserver::serve(vec![
            (500, "a".into()),
            (500, "b".into()),
            (500, "c".into()),
        ]);
        let error = client()
            .post(&base, &[], &serde_json::json!({}))
            .unwrap_err();
        assert!(error.is_retryable());
    }

    #[test]
    fn rejects_a_body_that_is_not_json() {
        let (base, _) = testserver::serve(vec![(200, "<html>".into())]);
        let error = client()
            .post(&base, &[], &serde_json::json!({}))
            .unwrap_err();
        assert!(error.to_string().contains("not JSON"));
    }

    #[test]
    fn reports_a_connection_failure_as_retryable() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let mut settings = client().settings;
        settings.retries = 0;
        let error = HttpClient::new(settings)
            .post(&address, &[], &serde_json::json!({}))
            .unwrap_err();
        assert!(error.is_retryable());
    }
}
