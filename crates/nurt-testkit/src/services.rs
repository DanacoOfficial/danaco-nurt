// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Service doubles for contexts in which the providing command group is not
//! under test.
//!
//! Each double answers queries with empty results and fails every request
//! that needs a real implementation with the error code the service contract
//! prescribes for an unsupported input:
//!
//! | Double | Empty answers | Error |
//! |---|---|---|
//! | [`UnsupportedDocService`] | no formats, no detected format | `E_UNSUPPORTED` |
//! | [`UnsupportedCodeService`] | no languages, no detected language | `E_UNSUPPORTED_LANGUAGE` |
//! | [`UnavailableProcessHost`] | none | `E_EXTERNAL_MISSING` |
//! | [`EmptyFileList`] | every listing is empty | none |

use std::path::{Path, PathBuf};
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::services::{
    ClassifiedSpan, CodeService, DocFormatId, DocFormatInfo, DocItem, DocService, FileListService,
    FunctionMetrics, LanguageId, LanguageInfo, ProcessHost, ProgramOutcome, ProgramSpec,
    SerializeOptions, ServiceChild, ServiceSpawnSpec, Services, SymbolInfo, SymbolReference,
    SyntaxIssue, SyntaxMatch, SyntaxPattern, SyntaxToken, WalkIter, WalkSpec,
};
use nurt_contracts::stage::CancelToken;
use nurt_contracts::value::Record;

/// Document service that knows no format.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnsupportedDocService;

fn unsupported_format(format: &DocFormatId) -> Error {
    Error::new(
        ErrorCode::Unsupported,
        format!("document format `{format}` is not available in this context"),
    )
}

impl DocService for UnsupportedDocService {
    fn formats(&self) -> Vec<DocFormatInfo> {
        Vec::new()
    }

    fn detect_format(&self, _path: Option<&Path>, _bytes: &[u8]) -> Option<DocFormatId> {
        None
    }

    fn read_keys(
        &self,
        _bytes: &[u8],
        format: &DocFormatId,
        _key: &KeyPath,
        _depth: Option<u32>,
    ) -> Result<Vec<DocItem>> {
        Err(unsupported_format(format))
    }

    fn check_syntax(&self, _bytes: &[u8], format: &DocFormatId) -> Result<Vec<SyntaxIssue>> {
        Err(unsupported_format(format))
    }

    fn serialize_records(
        &self,
        _records: &[Record],
        format: &DocFormatId,
        _options: &SerializeOptions,
    ) -> Result<Vec<u8>> {
        Err(unsupported_format(format))
    }
}

/// Code service that has no grammar.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnsupportedCodeService;

fn unsupported_language(language: &str) -> Error {
    Error::new(
        ErrorCode::UnsupportedLanguage,
        format!("no grammar for `{language}` is available in this context"),
    )
}

impl CodeService for UnsupportedCodeService {
    fn languages(&self) -> Vec<LanguageInfo> {
        Vec::new()
    }

    fn language_of(&self, _path: Option<&Path>, _bytes: &[u8]) -> Option<LanguageId> {
        None
    }

    fn outline(
        &self,
        _bytes: &[u8],
        language: &LanguageId,
        _depth: Option<u32>,
    ) -> Result<Vec<SymbolInfo>> {
        Err(unsupported_language(language.as_str()))
    }

    fn find_symbol(
        &self,
        _bytes: &[u8],
        language: &LanguageId,
        _name: &str,
    ) -> Result<Vec<SymbolInfo>> {
        Err(unsupported_language(language.as_str()))
    }

    fn enclosing(
        &self,
        _bytes: &[u8],
        language: &LanguageId,
        _offset: u64,
    ) -> Result<Option<SymbolInfo>> {
        Err(unsupported_language(language.as_str()))
    }

    fn classify_spans(&self, _bytes: &[u8], language: &LanguageId) -> Result<Vec<ClassifiedSpan>> {
        Err(unsupported_language(language.as_str()))
    }

    fn check_syntax(&self, _bytes: &[u8], language: &LanguageId) -> Result<Vec<SyntaxIssue>> {
        Err(unsupported_language(language.as_str()))
    }

    fn compile_pattern(
        &self,
        _pattern: &str,
        language: &LanguageId,
    ) -> Result<Box<dyn SyntaxPattern>> {
        Err(unsupported_language(language.as_str()))
    }

    fn syntax_search(
        &self,
        _bytes: &[u8],
        pattern: &dyn SyntaxPattern,
    ) -> Result<Vec<SyntaxMatch>> {
        Err(unsupported_language(pattern.language().as_str()))
    }

    fn render_template(&self, _found: &SyntaxMatch, _template: &str) -> Result<String> {
        Err(Error::new(
            ErrorCode::UnsupportedLanguage,
            "structural templates are not available in this context",
        ))
    }

    fn references(
        &self,
        _bytes: &[u8],
        language: &LanguageId,
        _name: &str,
    ) -> Result<Vec<SymbolReference>> {
        Err(unsupported_language(language.as_str()))
    }

    fn function_metrics(
        &self,
        _bytes: &[u8],
        language: &LanguageId,
    ) -> Result<Vec<FunctionMetrics>> {
        Err(unsupported_language(language.as_str()))
    }

    fn tokens(&self, _bytes: &[u8], language: &LanguageId) -> Result<Vec<SyntaxToken>> {
        Err(unsupported_language(language.as_str()))
    }
}

/// Program host that finds no program.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnavailableProcessHost;

fn missing_program(program: &str) -> Error {
    Error::new(
        ErrorCode::ExternalMissing,
        format!("program `{program}` cannot be started in this context"),
    )
}

impl ProcessHost for UnavailableProcessHost {
    fn resolve_program(&self, program: &str, _cwd: &Path) -> Result<PathBuf> {
        Err(missing_program(program))
    }

    fn run_program(&self, spec: &ProgramSpec, _cancel: &CancelToken) -> Result<ProgramOutcome> {
        Err(missing_program(&spec.program))
    }

    fn spawn_service(&self, spec: &ServiceSpawnSpec) -> Result<Box<dyn ServiceChild>> {
        Err(missing_program(&spec.program))
    }
}

/// File listing that lists nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyFileList;

impl FileListService for EmptyFileList {
    fn walk<'a>(&'a self, _spec: &WalkSpec) -> Result<WalkIter<'a>> {
        Ok(Box::new(std::iter::empty()))
    }
}

/// Registry with the four doubles of this module.
///
/// ```
/// use nurt_testkit::services::test_services;
///
/// let services = test_services();
/// assert!(services.doc().is_some_and(|doc| doc.formats().is_empty()));
/// assert!(services.file_list().is_some());
/// ```
#[must_use]
pub fn test_services() -> Services {
    Services::new()
        .with_doc(Arc::new(UnsupportedDocService))
        .with_code(Arc::new(UnsupportedCodeService))
        .with_process_host(Arc::new(UnavailableProcessHost))
        .with_file_list(Arc::new(EmptyFileList))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use nurt_contracts::error::ErrorCode;
    use nurt_contracts::services::{
        CodeService, DocService, FileListService, ProcessHost, WalkSpec,
    };

    use super::{
        EmptyFileList, UnavailableProcessHost, UnsupportedCodeService, UnsupportedDocService,
    };

    #[test]
    fn document_service_detects_nothing() {
        assert!(UnsupportedDocService.formats().is_empty());
        assert_eq!(UnsupportedDocService.detect_format(None, b"{}"), None);
    }

    #[test]
    fn code_service_detects_nothing() {
        assert!(UnsupportedCodeService.languages().is_empty());
        assert_eq!(
            UnsupportedCodeService.language_of(Some(Path::new("main.rs")), b"fn main() {}"),
            None
        );
    }

    #[test]
    fn process_host_reports_missing_programs() {
        let error = UnavailableProcessHost
            .resolve_program("git", Path::new("/"))
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ExternalMissing);
    }

    #[test]
    fn file_list_is_empty() {
        let entries = EmptyFileList.walk(&WalkSpec::new("/")).unwrap();
        assert_eq!(entries.count(), 0);
    }
}
