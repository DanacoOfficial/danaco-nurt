// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Doubles of the services that the commands reach through the context: the
//! code service, the file listing and the repository queries.

use std::path::{Path, PathBuf};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::services::{
    ByteRange, ClassifiedSpan, CodeService, FileHistoryStat, FileListService, FunctionMetrics,
    GitPathStatus, GitQueryService, LanguageId, LanguageInfo, SpanClass, SymbolInfo,
    SymbolReference, SyntaxIssue, SyntaxMatch, SyntaxPattern, SyntaxToken, TextPosition, TextSpan,
    WalkEntry, WalkIter, WalkSpec,
};

/// Word that adds one to the complexity of a function in the code double.
const DECISION: &str = "if ";

fn unsupported() -> Error {
    Error::new(
        ErrorCode::UnsupportedLanguage,
        "the double does not parse this",
    )
}

/// Code service that knows `rs` and `ts` files and measures one function per
/// file: its complexity is one plus the number of `if ` in the text.
pub(crate) struct FakeCode;

impl FakeCode {
    fn language(name: &str) -> LanguageId {
        LanguageId::new(name).unwrap()
    }
}

impl CodeService for FakeCode {
    fn languages(&self) -> Vec<LanguageInfo> {
        Vec::new()
    }

    fn language_of(&self, path: Option<&Path>, _bytes: &[u8]) -> Option<LanguageId> {
        let extension = path.and_then(Path::extension)?.to_str()?;
        match extension {
            "rs" => Some(Self::language("rust")),
            "ts" => Some(Self::language("typescript")),
            _ => None,
        }
    }

    fn outline(&self, _: &[u8], _: &LanguageId, _: Option<u32>) -> Result<Vec<SymbolInfo>> {
        Err(unsupported())
    }

    fn find_symbol(&self, _: &[u8], _: &LanguageId, _: &str) -> Result<Vec<SymbolInfo>> {
        Err(unsupported())
    }

    fn enclosing(&self, _: &[u8], _: &LanguageId, _: u64) -> Result<Option<SymbolInfo>> {
        Err(unsupported())
    }

    fn classify_spans(&self, _: &[u8], _: &LanguageId) -> Result<Vec<ClassifiedSpan>> {
        Err(unsupported())
    }

    fn check_syntax(&self, _: &[u8], _: &LanguageId) -> Result<Vec<SyntaxIssue>> {
        Err(unsupported())
    }

    fn compile_pattern(&self, _: &str, _: &LanguageId) -> Result<Box<dyn SyntaxPattern>> {
        Err(unsupported())
    }

    fn syntax_search(&self, _: &[u8], _: &dyn SyntaxPattern) -> Result<Vec<SyntaxMatch>> {
        Err(unsupported())
    }

    fn render_template(&self, _: &SyntaxMatch, _: &str) -> Result<String> {
        Err(unsupported())
    }

    fn references(&self, _: &[u8], _: &LanguageId, _: &str) -> Result<Vec<SymbolReference>> {
        Err(unsupported())
    }

    fn function_metrics(&self, bytes: &[u8], _: &LanguageId) -> Result<Vec<FunctionMetrics>> {
        let text = String::from_utf8_lossy(bytes);
        let lines = text.lines().count() as u32;
        let decisions = text.matches(DECISION).count() as u32;
        let position = |line| TextPosition { line, column: 1 };
        Ok(vec![FunctionMetrics {
            name: "main".to_owned(),
            container: Some("app".to_owned()),
            span: TextSpan {
                bytes: ByteRange::new(0, bytes.len() as u64),
                start: position(1),
                end: position(lines.max(1)),
            },
            lines,
            parameters: 0,
            cyclomatic: 1 + decisions,
            max_nesting: 0,
        }])
    }

    fn tokens(&self, bytes: &[u8], _: &LanguageId) -> Result<Vec<SyntaxToken>> {
        let mut tokens = Vec::new();
        let mut index = 0;
        while index < bytes.len() {
            let byte = bytes[index];
            let start = index;
            if byte.is_ascii_whitespace() {
                index += 1;
                continue;
            }
            let kind = if byte.is_ascii_alphanumeric() || byte == b'_' {
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
                {
                    index += 1;
                }
                "identifier"
            } else {
                index += 1;
                "punct"
            };
            tokens.push(SyntaxToken {
                bytes: ByteRange::new(start as u64, index as u64),
                kind: kind.to_owned(),
                class: SpanClass::Code,
            });
        }
        Ok(tokens)
    }
}

/// File listing that walks the directory and lists the files it finds, as
/// absolute paths.
pub(crate) struct DiskFiles;

fn collect(directory: &Path, found: &mut Vec<PathBuf>) {
    let mut children: Vec<PathBuf> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .collect();
    children.sort();
    for child in children {
        if child.is_dir() {
            collect(&child, found);
        } else {
            found.push(child);
        }
    }
}

impl FileListService for DiskFiles {
    fn walk<'a>(&'a self, spec: &WalkSpec) -> Result<WalkIter<'a>> {
        let mut files = Vec::new();
        collect(&spec.root, &mut files);
        let entries = files.into_iter().map(|path| {
            Ok(WalkEntry {
                path,
                depth: 1,
                is_dir: false,
            })
        });
        Ok(Box::new(entries))
    }
}

/// Repository queries with fixed answers.
pub(crate) struct FakeGit {
    /// Root of the working tree; `None` is a directory outside a repository.
    pub(crate) root: Option<PathBuf>,
    /// Paths tracked in the index, relative to the root.
    pub(crate) tracked: Vec<String>,
    /// Paths changed in the working tree, relative to the root.
    pub(crate) changed: Vec<String>,
}

impl GitQueryService for FakeGit {
    fn repo_root(&self, _: &Path) -> Result<Option<PathBuf>> {
        Ok(self.root.clone())
    }

    fn status(&self, _: &Path, _: &[String]) -> Result<Vec<GitPathStatus>> {
        Ok(Vec::new())
    }

    fn changed_files(&self, _: &Path, _: Option<&str>) -> Result<Vec<String>> {
        Ok(self.changed.clone())
    }

    fn tracked_files(&self, _: &Path) -> Result<Vec<String>> {
        Ok(self.tracked.clone())
    }

    fn is_ignored(&self, _: &Path, _: &str) -> Result<bool> {
        Ok(false)
    }

    fn file_history(&self, _: &Path, _: u32, _: u32) -> Result<Vec<FileHistoryStat>> {
        Ok(Vec::new())
    }
}
