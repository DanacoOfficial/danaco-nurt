// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Test files by the conventions of each language.
//!
//! | Language | Test file |
//! |---|---|
//! | all | a directory named `test`, `tests`, `__tests__`, `spec`, `specs` or `testing` on the path |
//! | Go | `*_test.go` |
//! | Python | `test_*.py`, `*_test.py`, `conftest.py` |
//! | JavaScript, TypeScript | `*.test.*`, `*.spec.*` |
//! | Rust | `tests.rs` |
//! | Java, Kotlin, C#, Swift, PHP | a file name ending in `Test`, `Tests` or `IT` (Java, Kotlin); a directory ending in `.Tests` or `.Test` (C#) |
//! | Ruby | `*_spec.rb`, `*_test.rb`, `test_*.rb` |
//! | C, C++ | `test_*`, `*_test`, `*_unittest` |
//!
//! Without a language every rule applies.

use std::path::{Component, Path};

use nurt_contracts::services::LanguageId;

/// Directory names that hold tests in every language.
const TEST_DIRECTORIES: &[&str] = &["test", "tests", "__tests__", "spec", "specs", "testing"];

/// Directory name suffixes of .NET test projects.
const TEST_PROJECT_SUFFIXES: &[&str] = &[".Tests", ".Test", ".UnitTests", ".IntegrationTests"];

/// Returns `true` when `path` is a test file of `language` by the
/// conventions of the module documentation.
///
/// ```
/// use std::path::Path;
/// use nurt_code::local::is_test_path;
/// use nurt_contracts::services::LanguageId;
///
/// let go = LanguageId::new("go")?;
/// assert!(is_test_path(Path::new("/src/server_test.go"), Some(&go)));
/// assert!(!is_test_path(Path::new("/src/server.go"), Some(&go)));
/// assert!(is_test_path(Path::new("/app/tests/test_api.py"), None));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
pub fn is_test_path(path: &Path, language: Option<&LanguageId>) -> bool {
    if in_test_directory(path) {
        return true;
    }
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let stem = file_name.split('.').next().unwrap_or(file_name);
    match language.map(LanguageId::as_str) {
        Some(id) => language_rule(id, file_name, stem),
        None => ALL_LANGUAGES
            .iter()
            .any(|id| language_rule(id, file_name, stem)),
    }
}

const ALL_LANGUAGES: &[&str] = &[
    "go",
    "python",
    "javascript",
    "rust",
    "java",
    "csharp",
    "ruby",
    "c",
];

fn in_test_directory(path: &Path) -> bool {
    let parent = path.parent().unwrap_or(path);
    parent.components().any(|component| match component {
        Component::Normal(name) => name.to_str().is_some_and(|name| {
            TEST_DIRECTORIES.contains(&name)
                || TEST_PROJECT_SUFFIXES
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
        }),
        _ => false,
    })
}

fn language_rule(id: &str, file_name: &str, stem: &str) -> bool {
    match id {
        "go" => file_name.ends_with("_test.go"),
        "python" => {
            stem.starts_with("test_") || stem.ends_with("_test") || file_name == "conftest.py"
        }
        "javascript" | "typescript" | "tsx" => {
            file_name.contains(".test.") || file_name.contains(".spec.")
        }
        "rust" => file_name == "tests.rs",
        "java" | "kotlin" => ends_with_any(stem, &["Test", "Tests", "IT"]),
        "csharp" | "swift" | "php" => ends_with_any(stem, &["Test", "Tests"]),
        "ruby" => stem.ends_with("_spec") || stem.ends_with("_test") || stem.starts_with("test_"),
        "c" | "cpp" => {
            stem.starts_with("test_") || stem.ends_with("_test") || stem.ends_with("_unittest")
        }
        _ => false,
    }
}

fn ends_with_any(stem: &str, suffixes: &[&str]) -> bool {
    suffixes
        .iter()
        .any(|suffix| stem.len() > suffix.len() && stem.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_file(path: &str, language: &str) -> bool {
        is_test_path(Path::new(path), Some(&LanguageId::new(language).unwrap()))
    }

    #[test]
    fn language_conventions_are_recognized() {
        assert!(test_file("/p/test_models.py", "python"));
        assert!(test_file("/p/models_test.py", "python"));
        assert!(!test_file("/p/models.py", "python"));
        assert!(test_file("/p/app.spec.ts", "typescript"));
        assert!(test_file("/p/app.test.jsx", "javascript"));
        assert!(test_file("/p/src/UserServiceTest.java", "java"));
        assert!(test_file("/p/src/UserServiceIT.java", "java"));
        assert!(!test_file("/p/src/Test.java", "java"));
        assert!(test_file("/p/Shop.Tests/CartTests.cs", "csharp"));
        assert!(test_file("/p/lib/cart_spec.rb", "ruby"));
        assert!(test_file("/p/src/parser_unittest.cc", "cpp"));
        assert!(test_file("/p/src/tests.rs", "rust"));
    }

    #[test]
    fn test_directories_apply_to_every_language() {
        assert!(test_file("/p/tests/common.rs", "rust"));
        assert!(test_file("/p/__tests__/util.js", "javascript"));
        assert!(!test_file("/p/src/testing_tools.rs", "rust"));
    }

    #[test]
    fn without_language_every_rule_applies() {
        assert!(is_test_path(Path::new("/p/handler_test.go"), None));
        assert!(is_test_path(Path::new("/p/OrderTests.cs"), None));
        assert!(!is_test_path(Path::new("/p/order.cs"), None));
    }
}
