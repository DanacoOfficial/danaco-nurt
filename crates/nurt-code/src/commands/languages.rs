// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The `code.languages` stage.
//!
//! Lists the languages with a built-in grammar and the languages with a
//! configured language server, one `language` record each. The state of the
//! servers comes from [`SemanticProvider::server_status`], which neither
//! starts servers nor waits for requests in progress, so the command answers
//! at once while other commands use the servers.

use std::collections::BTreeMap;
use std::sync::Arc;

use nurt_contracts::config::{CodeConfig, ServerAllow, ServerAllowMode};
use nurt_contracts::error::Result;
use nurt_contracts::record::Language;
use nurt_contracts::services::LanguageInfo;
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Path, Record};

use crate::semantic::{SemanticProvider, ServerStatus};

/// Command name in the catalog.
const COMMAND: &str = "code.languages";

/// Implementation of `code.languages`.
///
/// ```
/// use nurt_code::commands::languages::CodeLanguages;
/// use nurt_contracts::stage::StageImpl;
///
/// let stage = CodeLanguages::new(None);
/// assert_eq!(stage.descriptor().command, "code.languages");
/// assert!(stage.as_read().is_some());
/// ```
pub struct CodeLanguages {
    semantic: Option<Arc<dyn SemanticProvider>>,
}

impl CodeLanguages {
    /// Stage that reports server state through `semantic`; without a
    /// provider no server is used, so every record has `server_found: false`.
    #[must_use]
    pub fn new(semantic: Option<Arc<dyn SemanticProvider>>) -> Self {
        Self { semantic }
    }

    fn records(&self, ctx: &ExecCtx) -> Vec<Language> {
        let grammars = ctx.code().map(|code| code.languages()).unwrap_or_default();
        let statuses: BTreeMap<String, ServerStatus> = self
            .semantic
            .as_ref()
            .map(|semantic| semantic.server_status(ctx, None))
            .unwrap_or_default()
            .into_iter()
            .map(|status| (status.language.as_str().to_owned(), status))
            .collect();
        let config = &ctx.config().code;
        let mut records: Vec<Language> = grammars
            .iter()
            .map(|info| language_record(Some(info), info.id.as_str(), &statuses, config))
            .collect();
        let mut extra: Vec<&str> = config
            .servers
            .keys()
            .map(String::as_str)
            .chain(statuses.keys().map(String::as_str))
            .filter(|name| !grammars.iter().any(|info| info.id.as_str() == *name))
            .collect();
        extra.sort_unstable();
        extra.dedup();
        records.extend(
            extra
                .into_iter()
                .map(|name| language_record(None, name, &statuses, config)),
        );
        records
    }
}

impl StageImpl for CodeLanguages {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for CodeLanguages {
    fn execute<'a>(
        &'a self,
        _args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        ctx.checkpoint()?;
        let records = self.records(ctx).into_iter().map(Record::from).collect();
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

/// Record of one language: grammar facts from `info`, server facts from the
/// provider status, or the configured program when the provider has no
/// status for the language.
fn language_record(
    info: Option<&LanguageInfo>,
    name: &str,
    statuses: &BTreeMap<String, ServerStatus>,
    config: &CodeConfig,
) -> Language {
    let mut record = Language {
        language: name.to_owned(),
        extensions: info.map(|info| info.extensions.clone()).unwrap_or_default(),
        grammar_version: info.and_then(|info| info.grammar_version.clone()),
        ..Language::default()
    };
    match statuses.get(name) {
        Some(status) => {
            record.server.clone_from(&status.server);
            record.server_found = status.found && status.allowed;
            record.server_running = Some(status.running);
            record.server_pid = status.pid.map(u64::from);
            record.server_root = status
                .root
                .as_ref()
                .map(|root| Path::new(root.to_string_lossy()));
        }
        None => {
            record.server = configured_program(config, name);
            record.server_found = false;
        }
    }
    record
}

/// Program of the server configured for `language` when the configuration
/// allows it.
fn configured_program(config: &CodeConfig, language: &str) -> Option<String> {
    let allowed = match &config.servers_allow {
        ServerAllow::Mode(ServerAllowMode::Off) => false,
        ServerAllow::Mode(_) => true,
        ServerAllow::Languages(languages) => languages.iter().any(|item| item == language),
    };
    if !allowed {
        return None;
    }
    config.servers.get(language)?.command.first().cloned()
}
