// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-migrate::language
// Responsibility:   Script languages accepted by the migration and their detection.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Script languages accepted by the migration and their detection.

use std::fmt;
use std::path::Path;

use nurt_contracts::error::{Error, ErrorCode};

/// File extension of program files, rejected as migration input.
const PROGRAM_EXTENSION: &str = "nurt";

/// Language of a script to migrate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ScriptLanguage {
    /// POSIX shell script read with the bash grammar.
    Bash,
    /// PowerShell script.
    PowerShell,
}

impl ScriptLanguage {
    /// Returns the name used in reports: `bash` or `powershell`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::PowerShell => "powershell",
        }
    }

    /// Returns the program that runs a script of this language unchanged, `bash` or `pwsh`.
    #[must_use]
    pub const fn interpreter(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::PowerShell => "pwsh",
        }
    }

    /// Determines the language of a script from its file name or its `#!` line.
    ///
    /// The extensions `.sh` and `.bash` select bash, and `.ps1` selects PowerShell.
    /// Letter case of the extension is ignored.
    /// Without a known extension, the `#!` line decides, also through `/usr/bin/env`.
    /// The programs `sh`, `bash` and `dash` select bash.
    /// The programs `pwsh` and `powershell` select PowerShell.
    ///
    /// # Errors
    ///
    /// Returns `E_ARG` when the file is a program file (`.nurt`) or when
    /// neither the extension nor the `#!` line names a supported language.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_migrate::ScriptLanguage;
    ///
    /// assert_eq!(ScriptLanguage::detect("deploy.sh", "")?, ScriptLanguage::Bash);
    /// assert_eq!(
    ///     ScriptLanguage::detect("deploy", "#!/usr/bin/env pwsh\n")?,
    ///     ScriptLanguage::PowerShell,
    /// );
    /// assert!(ScriptLanguage::detect("deploy.nurt", "").is_err());
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn detect(path: &str, source: &str) -> Result<Self, Error> {
        let extension = Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        match extension.as_deref() {
            Some("sh" | "bash") => return Ok(Self::Bash),
            Some("ps1") => return Ok(Self::PowerShell),
            Some(PROGRAM_EXTENSION) => {
                return Err(Error::new(
                    ErrorCode::Arg,
                    "the file is already a program; migrate a .sh or .ps1 script",
                ));
            }
            _ => {}
        }
        Self::from_shebang(source).ok_or_else(|| {
            Error::new(
                ErrorCode::Arg,
                "cannot determine the script language; use the extension .sh or .ps1 or a #! line",
            )
        })
    }

    fn from_shebang(source: &str) -> Option<Self> {
        let line = source
            .strip_prefix('\u{feff}')
            .unwrap_or(source)
            .lines()
            .next()?;
        let mut words = line.strip_prefix("#!")?.split_whitespace();
        let mut program = program_name(words.next()?);
        if program == "env" {
            program = program_name(words.find(|word| !word.starts_with('-'))?);
        }
        match program {
            "sh" | "bash" | "dash" => Some(Self::Bash),
            "pwsh" | "powershell" => Some(Self::PowerShell),
            _ => None,
        }
    }
}

impl fmt::Display for ScriptLanguage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

fn program_name(word: &str) -> &str {
    let name = word.rsplit(['/', '\\']).next().unwrap_or(word);
    name.strip_suffix(".exe").unwrap_or(name)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
