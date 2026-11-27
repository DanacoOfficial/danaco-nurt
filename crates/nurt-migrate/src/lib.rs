// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-migrate
// Responsibility:   Translation of shell scripts into Danaco Nurt programs.
// Dependencies:     language, migrate, nurt_bind, nurt_correct, report, translate
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Translation of shell scripts into Danaco Nurt programs.
//!
//! [`migrate`] reads a bash or PowerShell script with its tree-sitter
//! grammar, submits each statement to a [`TableTranslator`] that applies the
//! bash and PowerShell habit tables, and returns a `.nurt` file with the
//! translated statements and a [`MigrationReport`]. Each translated
//! statement is preceded by the comment `# translated:` with the rules
//! applied and, for probable rules, `# assumption:` lines. A statement
//! without an equivalent is kept as the comment `# untranslated:` with its
//! error code and the proposed fix. When any statement is not translated,
//! the file header and the report propose running the whole script
//! unchanged with `run ["bash", "s.sh"]` or `run ["pwsh", "s.ps1"]`.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod language;
mod migrate;
mod output;
mod report;
mod segment;
mod translate;

pub use language::ScriptLanguage;
pub use migrate::{MigrateOptions, Migration, migrate, migrate_with};
pub use nurt_bind::Catalog;
pub use nurt_correct::bash::FunctionSignature;
pub use report::{Item, MigrationReport, REPORT_FORMAT, Reason, Status, Summary};
pub use translate::{LetBinding, TableTranslator, TranslateRequest, Translation, Translator};

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
