// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Settings of the language server client and selection of servers.
//!
//! Settings come from the `[code]` section of the configuration
//! ([`CodeConfig`]): the server command and initialization options of each
//! language (`code.servers.<language>.command`, `.init`), the allowance
//! (`code.servers_allow`), the idle time, the retry period, the pool limit,
//! request time limits, the diagnostics quiet period and the largest file
//! read.
//!
//! On Windows `npm` installs language servers as batch wrappers (`.cmd`).
//! Arguments of batch files are not passed through the command interpreter,
//! so a wrapper is resolved to a direct call of the interpreter with the
//! script the wrapper names.
//!
//! # Examples
//!
//! ```
//! use std::path::Path;
//! use nurt_lspclient::config::{language_for_path, language_id};
//!
//! assert_eq!(language_for_path(Path::new("web/app.tsx")), Some("typescript"));
//! assert_eq!(language_id(Path::new("web/app.tsx"), "typescript"), "typescriptreact");
//! ```

use std::path::{MAIN_SEPARATOR_STR, Path, PathBuf};
use std::time::Duration;

use nurt_contracts::config::{CodeConfig, ServerAllow, ServerAllowMode};
use nurt_contracts::services::ProcessHost;
use serde_json::{Value, json};

use crate::server::LspError;

/// Shortest diagnostics quiet period.
pub const DIAGNOSTICS_QUIET_MIN: Duration = Duration::from_millis(50);

/// Files that mark the root of a project, nearest first.
pub const ROOT_MARKERS: &[&str] = &[
    "pyproject.toml",
    "setup.py",
    "package.json",
    "tsconfig.json",
    "go.mod",
    "Cargo.toml",
    "compile_commands.json",
    ".git",
];

/// Library of a project-local TypeScript installation.
const TSSERVER_LIBRARY: &str = "node_modules/typescript/lib/tsserver.js";

/// Program name of the TypeScript language server.
const TYPESCRIPT_SERVER: &str = "typescript-language-server";

/// Languages by file extension; the first extension of a language is its
/// usual one.
const EXTENSIONS: &[(&str, &[&str])] = &[
    ("python", &["py", "pyi"]),
    ("typescript", &["ts", "tsx", "mts", "cts"]),
    ("javascript", &["js", "jsx", "mjs", "cjs"]),
    ("go", &["go"]),
    ("rust", &["rs"]),
    ("c", &["c", "h"]),
    ("cpp", &["cc", "cpp", "cxx", "hpp", "hh", "hxx"]),
    ("java", &["java"]),
    ("csharp", &["cs"]),
    ("php", &["php"]),
    ("kotlin", &["kt", "kts"]),
    ("swift", &["swift"]),
    ("ruby", &["rb"]),
];

/// Language identifiers of `didOpen` that differ from the language name.
const LANGUAGE_IDS: &[(&str, &str)] = &[
    ("tsx", "typescriptreact"),
    ("jsx", "javascriptreact"),
    ("hh", "cpp"),
    ("hxx", "cpp"),
];

/// Other names of languages accepted in parameters and from syntax analysis.
const ALIASES: &[(&str, &str)] = &[
    ("tsx", "typescript"),
    ("jsx", "javascript"),
    ("py", "python"),
    ("c++", "cpp"),
    ("cxx", "cpp"),
    ("c_sharp", "csharp"),
    ("c-sharp", "csharp"),
    ("cs", "csharp"),
    ("golang", "go"),
];

/// How to install the default servers, by program name.
const INSTALL_HINTS: &[(&str, &str)] = &[
    ("pyright-langserver", "npm install --global pyright"),
    (
        TYPESCRIPT_SERVER,
        "npm install --global typescript-language-server typescript",
    ),
    ("gopls", "go install golang.org/x/tools/gopls@latest"),
    ("rust-analyzer", "rustup component add rust-analyzer"),
    ("clangd", "install clangd from the LLVM packages of the system"),
];

/// Settings of the client, derived from the `[code]` configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct ClientSettings {
    servers: Vec<(String, Vec<String>, Option<Value>)>,
    allow: ServerAllow,
    /// An unused server stops after this time.
    pub idle: Duration,
    /// A server that failed to start is not started again for this time.
    pub retry: Duration,
    /// Largest number of running servers.
    pub max_servers: usize,
    /// Default time limit of a command.
    pub timeout: Duration,
    /// Largest time limit a command may request.
    pub timeout_max: Duration,
    /// Silence after which the last published diagnostics apply.
    pub diagnostics_quiet: Duration,
    /// Largest file read, in bytes.
    pub file_bytes_max: u64,
    /// Largest number of files examined when searching the project.
    pub index_max_files: usize,
}

impl Default for ClientSettings {
    fn default() -> Self {
        Self::from_config(&CodeConfig::default())
    }
}

impl ClientSettings {
    /// Settings from the `[code]` section.
    #[must_use]
    pub fn from_config(code: &CodeConfig) -> Self {
        let servers = code
            .servers
            .iter()
            .map(|(language, server)| {
                (
                    normalize_language(language),
                    server.command.clone(),
                    server.init.clone(),
                )
            })
            .collect();
        Self {
            servers,
            allow: code.servers_allow.clone(),
            idle: code.server_idle.as_std(),
            retry: code.server_retry.as_std(),
            max_servers: usize::try_from(code.max_servers.max(1)).unwrap_or(usize::MAX),
            timeout: code.timeout.as_std(),
            timeout_max: code.timeout_max.as_std(),
            diagnostics_quiet: code.diagnostics_quiet.as_std().max(DIAGNOSTICS_QUIET_MIN),
            file_bytes_max: code.file_bytes_max.bytes(),
            index_max_files: usize::try_from(code.index_max_files).unwrap_or(usize::MAX),
        }
    }

    /// Time limit of a command: the requested limit or the default, never
    /// above the largest allowed limit.
    #[must_use]
    pub fn request_timeout(&self, requested: Option<Duration>) -> Duration {
        requested.unwrap_or(self.timeout).min(self.timeout_max)
    }

    /// Languages with a configured server command, in configuration order.
    #[must_use]
    pub fn languages(&self) -> Vec<&str> {
        self.servers
            .iter()
            .filter(|(_, command, _)| !command.is_empty())
            .map(|(language, _, _)| language.as_str())
            .collect()
    }

    /// Configured command of the server of `language`, program first.
    #[must_use]
    pub fn command(&self, language: &str) -> Option<&[String]> {
        let language = normalize_language(language);
        self.servers
            .iter()
            .find(|(name, command, _)| *name == language && !command.is_empty())
            .map(|(_, command, _)| command.as_slice())
    }

    /// Configured initialization options of the server of `language`.
    #[must_use]
    pub fn init_options(&self, language: &str) -> Option<&Value> {
        let language = normalize_language(language);
        self.servers
            .iter()
            .find(|(name, _, _)| *name == language)
            .and_then(|(_, _, init)| init.as_ref())
    }

    /// Returns `true` when the allowance permits the server of `language`
    /// with program `program` (a language name or a program name in the
    /// list).
    #[must_use]
    pub fn allows(&self, language: &str, program: &str) -> bool {
        match &self.allow {
            ServerAllow::Mode(ServerAllowMode::All) => true,
            ServerAllow::Mode(ServerAllowMode::Off) => false,
            ServerAllow::Languages(list) => {
                let language = normalize_language(language);
                let program = program_name(program);
                list.iter()
                    .any(|entry| normalize_language(entry) == language || *entry == program)
            }
        }
    }
}

/// Canonical name of a language: lower case with aliases resolved.
#[must_use]
pub fn normalize_language(name: &str) -> String {
    let lower = name.trim().to_ascii_lowercase();
    ALIASES
        .iter()
        .find(|(alias, _)| *alias == lower)
        .map_or(lower, |(_, canonical)| (*canonical).to_owned())
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
}

/// Language of a file from its extension.
#[must_use]
pub fn language_for_path(path: &Path) -> Option<&'static str> {
    let extension = extension(path)?;
    EXTENSIONS
        .iter()
        .find(|(_, extensions)| extensions.contains(&extension.as_str()))
        .map(|(language, _)| *language)
}

/// Language identifier sent with `textDocument/didOpen`.
#[must_use]
pub fn language_id(path: &Path, language: &str) -> String {
    extension(path)
        .and_then(|extension| {
            LANGUAGE_IDS
                .iter()
                .find(|(known, _)| *known == extension)
                .map(|(_, id)| (*id).to_owned())
        })
        .unwrap_or_else(|| normalize_language(language))
}

/// File name of a program without the directory and a Windows executable
/// extension.
#[must_use]
pub fn program_name(program: &str) -> String {
    let name = program
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(program)
        .to_owned();
    let lower = name.to_ascii_lowercase();
    for suffix in [".exe", ".cmd", ".bat"] {
        if lower.ends_with(suffix) {
            return name[..name.len() - suffix.len()].to_owned();
        }
    }
    name
}

/// How to install a default server, for the `fix` of `E_EXTERNAL_MISSING`.
#[must_use]
pub fn install_hint(program: &str) -> Option<&'static str> {
    let name = program_name(program);
    INSTALL_HINTS
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, hint)| *hint)
}

/// Project root of `target`: the nearest directory with one of the
/// [`ROOT_MARKERS`], or the directory of the file when there is none.
#[must_use]
pub fn project_root(target: &Path) -> PathBuf {
    let start = if target.is_dir() {
        target.to_path_buf()
    } else {
        target.parent().map_or_else(|| target.to_path_buf(), Path::to_path_buf)
    };
    start
        .ancestors()
        .find(|directory| ROOT_MARKERS.iter().any(|marker| directory.join(marker).exists()))
        .map_or_else(|| start.clone(), Path::to_path_buf)
}

/// Program and arguments of a server after resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCommand {
    /// File name of the configured program, shown in results.
    pub name: String,
    /// Program to start.
    pub program: String,
    /// Arguments.
    pub args: Vec<String>,
}

/// Resolves a configured server command through the process host.
///
/// A Windows batch wrapper written by `npm` is replaced by its interpreter
/// and script.
///
/// # Errors
///
/// Fails with [`LspError::Missing`] when the command is empty or the program
/// (or the `node` interpreter of a wrapper) is not found, and with
/// [`LspError::Unavailable`] when the program is a batch file that is not an
/// `npm` wrapper.
pub fn resolve_command(
    host: &dyn ProcessHost,
    command: &[String],
    cwd: &Path,
) -> Result<ResolvedCommand, LspError> {
    let Some((program, args)) = command.split_first() else {
        return Err(LspError::Missing {
            server: String::new(),
            message: "the language server command is empty".to_owned(),
        });
    };
    let name = program_name(program);
    let missing = |message: String| LspError::Missing {
        server: name.clone(),
        message,
    };
    let unavailable = |message: String| LspError::Unavailable {
        server: name.clone(),
        message,
    };
    let path = host
        .resolve_program(program, cwd)
        .map_err(|error| missing(error.data().message.clone()))?;
    if !is_batch_file(&path) {
        return Ok(ResolvedCommand {
            name: name.clone(),
            program: path.to_string_lossy().into_owned(),
            args: args.to_vec(),
        });
    }
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let shim = npm_shim(&path, &text, Path::exists).ok_or_else(|| {
        unavailable(format!(
            "{} is a batch file that cannot receive arguments safely; set code.servers.<language>.command to the interpreter and script",
            path.display()
        ))
    })?;
    let interpreter = match shim.interpreter {
        Some(interpreter) => interpreter,
        None => host
            .resolve_program("node", cwd)
            .map_err(|error| missing(error.data().message.clone()))?,
    };
    let mut resolved_args = vec![shim.script.to_string_lossy().into_owned()];
    resolved_args.extend(args.iter().cloned());
    Ok(ResolvedCommand {
        name: name.clone(),
        program: interpreter.to_string_lossy().into_owned(),
        args: resolved_args,
    })
}

/// Returns `true` for `.cmd` and `.bat` files.
#[must_use]
pub fn is_batch_file(path: &Path) -> bool {
    extension(path).is_some_and(|extension| extension == "cmd" || extension == "bat")
}

/// Interpreter and script named by an `npm` batch wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpmShim {
    /// `node.exe` next to the wrapper, when present; otherwise `node` is
    /// looked up in `PATH`.
    pub interpreter: Option<PathBuf>,
    /// Script started by the wrapper.
    pub script: PathBuf,
}

/// Reads the script of an `npm` wrapper at `shim` with content `text`.
///
/// The wrapper starts `"%dp0%\<script>" %*` (or `"%~dp0\<script>" %*`)
/// through `node`; `exists` tells whether `node.exe` lies next to the
/// wrapper. `None` when the text is not such a wrapper.
#[must_use]
pub fn npm_shim(shim: &Path, text: &str, exists: impl Fn(&Path) -> bool) -> Option<NpmShim> {
    let directory = shim.parent()?;
    let relative = text
        .lines()
        .filter(|line| line.contains("%*"))
        .filter_map(script_of_line)
        .next_back()?;
    let script = join_windows_relative(directory, &relative);
    let local_node = directory.join("node.exe");
    Some(NpmShim {
        interpreter: exists(&local_node).then_some(local_node),
        script,
    })
}

/// Last quoted path relative to the wrapper directory on a line, other than
/// `node.exe`.
fn script_of_line(line: &str) -> Option<String> {
    line.split('"')
        .skip(1)
        .step_by(2)
        .filter_map(|quoted| {
            ["%dp0%", "%~dp0"]
                .iter()
                .find_map(|prefix| quoted.strip_prefix(prefix))
        })
        .filter(|relative| !relative.to_ascii_lowercase().ends_with("node.exe"))
        .map(str::to_owned)
        .next_back()
}

fn join_windows_relative(directory: &Path, relative: &str) -> PathBuf {
    let parts: Vec<&str> = relative
        .split(['\\', '/'])
        .filter(|part| !part.is_empty())
        .collect();
    let mut path = directory.to_path_buf();
    for part in parts {
        if part == ".." {
            path.pop();
        } else if part != "." {
            path.push(part);
        }
    }
    let text = path.to_string_lossy().replace(['\\', '/'], MAIN_SEPARATOR_STR);
    PathBuf::from(text)
}

/// Initialization options of a server: the configured options, or for
/// `typescript-language-server` in a project without its own TypeScript the
/// `tsserver` library of the host.
#[must_use]
pub fn init_options(
    settings: &ClientSettings,
    language: &str,
    server: &str,
    root: &Path,
    host: &dyn ProcessHost,
) -> Option<Value> {
    if let Some(configured) = settings.init_options(language) {
        return Some(configured.clone());
    }
    if program_name(server) != TYPESCRIPT_SERVER || has_own_typescript(root) {
        return None;
    }
    let found = host.resolve_program("tsserver", root).ok()?;
    host_tsserver_library(&found).map(|library| json!({"tsserver": {"path": library}}))
}

fn has_own_typescript(root: &Path) -> bool {
    root.ancestors()
        .any(|directory| directory.join(TSSERVER_LIBRARY).exists())
}

/// `lib/tsserver.js` of the TypeScript installation that holds the
/// `tsserver` program (`<install>/bin/tsserver`).
#[must_use]
pub fn host_tsserver_library(tsserver: &Path) -> Option<String> {
    let real = std::fs::canonicalize(tsserver).ok()?;
    let library = real.parent()?.parent()?.join("lib").join("tsserver.js");
    library
        .exists()
        .then(|| library.to_string_lossy().into_owned())
}
