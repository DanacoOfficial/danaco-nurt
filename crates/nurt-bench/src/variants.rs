// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Syntax variants B and C of the reference programs, generated from the
//! syntax tree of variant A by `nurt serve` (`parse` and `print`).
//!
//! Each variant is parsed back; the variant is accepted only when its tree
//! equals the tree of variant A.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{BenchError, Result};
use crate::rpc::{CALL_TIMEOUT, RpcClient};
use crate::task::TaskFile;

/// Format identifier of a variant record.
pub const VARIANTS_FORMAT: &str = "nurt-bench-variants/1";

/// Variants of the reference program of one task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariantRecord {
    /// Format identifier, [`VARIANTS_FORMAT`].
    pub format: String,
    /// Task identifier.
    pub id: String,
    /// Program in variant A, as in the task.
    pub a: String,
    /// Program in variant B.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b: Option<String>,
    /// Program in variant C.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub c: Option<String>,
    /// Why the variants could not be generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn language_error(result: &Value) -> Option<String> {
    let error = result.get("error")?;
    Some(format!(
        "{}: {}",
        error["code"].as_str().unwrap_or("error"),
        error["message"].as_str().unwrap_or_default()
    ))
}

fn parse(
    client: &mut RpcClient,
    program: &str,
    variant: &str,
) -> Result<std::result::Result<Value, String>> {
    let result = client.call(
        "parse",
        json!({"program": program, "variant": variant}),
        CALL_TIMEOUT,
    )?;
    if let Some(message) = language_error(&result) {
        return Ok(Err(message));
    }
    Ok(result
        .get("ast")
        .cloned()
        .ok_or_else(|| "parse returned no tree".to_owned()))
}

fn print(
    client: &mut RpcClient,
    ast: &Value,
    variant: &str,
) -> Result<std::result::Result<String, String>> {
    let result = client.call(
        "print",
        json!({"ast": ast, "variant": variant}),
        CALL_TIMEOUT,
    )?;
    if let Some(message) = language_error(&result) {
        return Ok(Err(message));
    }
    Ok(result
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "print returned no text".to_owned()))
}

fn variant(
    client: &mut RpcClient,
    ast: &Value,
    name: &str,
) -> Result<std::result::Result<String, String>> {
    let text = match print(client, ast, name)? {
        Ok(text) => text,
        Err(message) => return Ok(Err(message)),
    };
    match parse(client, &text, name)? {
        Ok(back) if &back == ast => Ok(Ok(text)),
        Ok(_) => Ok(Err(format!("variant {name} parses to a different tree"))),
        Err(message) => Ok(Err(format!("variant {name} does not parse: {message}"))),
    }
}

/// Generates the variants of the reference program of one task.
///
/// # Errors
///
/// Returns an error when the server fails; language errors are recorded in
/// the record.
pub fn generate(client: &mut RpcClient, task: &TaskFile) -> Result<VariantRecord> {
    let mut record = VariantRecord {
        format: VARIANTS_FORMAT.to_owned(),
        id: task.task.id.clone(),
        a: task.task.oracle_program.clone(),
        b: None,
        c: None,
        error: None,
    };
    let ast = match parse(client, &record.a, "a")? {
        Ok(ast) => ast,
        Err(message) => {
            record.error = Some(format!("variant a does not parse: {message}"));
            return Ok(record);
        }
    };
    match (variant(client, &ast, "b")?, variant(client, &ast, "c")?) {
        (Ok(b), Ok(c)) => {
            record.b = Some(b);
            record.c = Some(c);
        }
        (Err(message), _) | (_, Err(message)) => record.error = Some(message),
    }
    Ok(record)
}

/// Generates the variants of every task and writes them as JSON Lines.
/// Returns the number of tasks with both variants and the number of tasks
/// with an error.
///
/// # Errors
///
/// Returns an error when the server fails or the file cannot be written.
pub fn generate_file(
    client: &mut RpcClient,
    tasks: &[TaskFile],
    out: &Path,
) -> Result<(usize, usize)> {
    let mut lines = String::new();
    let mut generated = 0;
    let mut failed = 0;
    for task in tasks {
        let record = generate(client, task)?;
        if record.error.is_some() {
            failed += 1;
        } else {
            generated += 1;
        }
        let line = serde_json::to_string(&record)
            .map_err(|error| BenchError::format(out, "variants", error.to_string()))?;
        lines.push_str(&line);
        lines.push('\n');
    }
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|error| BenchError::io(parent, error))?;
    }
    fs::write(out, lines).map_err(|error| BenchError::io(out, error))?;
    Ok((generated, failed))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use super::*;
    use crate::rpc::testing::{Handler, connect};
    use crate::task::tests::task_document;

    /// Server whose tree is the list of words of the program; variant B
    /// prints the words joined by commas, variant C by semicolons.
    pub(crate) fn variant_server() -> Arc<Handler> {
        Arc::new(|method: &str, params: &Value| {
            let separator = |variant: &str| match variant {
                "b" => ",",
                "c" => ";",
                _ => " ",
            };
            match method {
                "parse" => {
                    let program = params["program"].as_str().unwrap_or_default();
                    if program.starts_with("bad") {
                        return Ok(
                            json!({"error": {"code": "E_PARSE", "message": "unexpected word"}}),
                        );
                    }
                    let sep = separator(params["variant"].as_str().unwrap_or("a"));
                    let words: Vec<&str> = program.split(sep).collect();
                    Ok(json!({"ast": words}))
                }
                "print" => {
                    let words: Vec<String> =
                        serde_json::from_value(params["ast"].clone()).unwrap_or_default();
                    if words.first().is_some_and(|word| word == "lossy") {
                        return Ok(json!({"text": "other"}));
                    }
                    Ok(
                        json!({"text": words.join(separator(params["variant"].as_str().unwrap_or("a")))}),
                    )
                }
                "check" => {
                    let program = params["program"].as_str().unwrap_or_default();
                    if program.starts_with("bad") {
                        Ok(json!({"error": {"code": "E_UNKNOWN_COMMAND", "message": "unknown"}}))
                    } else {
                        Ok(json!({"bound": {}, "ast_hash": "sha256:0"}))
                    }
                }
                _ => Err((-32601, "unknown".to_owned())),
            }
        })
    }

    fn task(program: &str) -> TaskFile {
        let mut document = task_document("edit-001");
        document["oracle_program"] = json!(program);
        TaskFile::from_document(&PathBuf::from("t.json"), document).unwrap()
    }

    #[test]
    fn generates_variants_that_parse_back_to_the_same_tree() {
        let mut client = connect(variant_server());
        let record = generate(&mut client, &task("write out.txt hello")).unwrap();
        assert_eq!(record.b.as_deref(), Some("write,out.txt,hello"));
        assert_eq!(record.c.as_deref(), Some("write;out.txt;hello"));
        assert!(record.error.is_none());
    }

    #[test]
    fn records_language_errors_and_lossy_printing() {
        let mut client = connect(variant_server());
        let bad = generate(&mut client, &task("bad program")).unwrap();
        assert!(bad.error.unwrap().contains("E_PARSE"));
        let lossy = generate(&mut client, &task("lossy x")).unwrap();
        assert!(lossy.error.unwrap().contains("different tree"));
    }

    #[test]
    fn writes_one_record_per_task() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("v/variants.jsonl");
        let mut client = connect(variant_server());
        let counts = generate_file(&mut client, &[task("write a b"), task("bad")], &out).unwrap();
        assert_eq!(counts, (1, 1));
        assert_eq!(fs::read_to_string(out).unwrap().lines().count(), 2);
    }
}
