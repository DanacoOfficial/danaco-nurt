// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Build and test commands, entry points and manifests read from project
//! files.
//!
//! Every command and entry point carries its origin: the manifest and the key
//! or line it was read from. Commands are read, never run.

use nurt_contracts::services::DocFormatId;
use nurt_contracts::value::Value;

use super::model::{Entry, Section, file_name, join, parent};
use super::source::{Source, field, map_entries, text_field};

/// Manifests deeper than this many directories below the root are ignored.
pub(crate) const MANIFEST_DEPTH_MAX: usize = 4;

/// Kind of project manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Manifest {
    Npm,
    Cargo,
    Make,
    Just,
    Pyproject,
    SetupCfg,
    Tox,
    GoMod,
    Cmake,
    Gradle,
    Maven,
    Dotnet,
    Composer,
    Bundler,
    Deno,
}

impl Manifest {
    /// Kind of the manifest with this file name.
    pub(crate) fn of(name: &str) -> Option<Self> {
        let kind = match name {
            "package.json" => Self::Npm,
            "Cargo.toml" => Self::Cargo,
            "Makefile" | "makefile" | "GNUmakefile" => Self::Make,
            "justfile" | "Justfile" | ".justfile" => Self::Just,
            "pyproject.toml" => Self::Pyproject,
            "setup.cfg" => Self::SetupCfg,
            "tox.ini" => Self::Tox,
            "go.mod" => Self::GoMod,
            "CMakeLists.txt" => Self::Cmake,
            "build.gradle" | "build.gradle.kts" => Self::Gradle,
            "pom.xml" => Self::Maven,
            "Gemfile" => Self::Bundler,
            "deno.json" | "deno.jsonc" => Self::Deno,
            "composer.json" => Self::Composer,
            _ if name.ends_with(".csproj") => Self::Dotnet,
            _ => return None,
        };
        Some(kind)
    }

    /// Name of the tool the manifest belongs to.
    pub(crate) const fn tool(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Cargo => "cargo",
            Self::Make => "make",
            Self::Just => "just",
            Self::Pyproject | Self::SetupCfg => "python",
            Self::Tox => "tox",
            Self::GoMod => "go",
            Self::Cmake => "cmake",
            Self::Gradle => "gradle",
            Self::Maven => "maven",
            Self::Dotnet => "dotnet",
            Self::Composer => "composer",
            Self::Bundler => "bundler",
            Self::Deno => "deno",
        }
    }

    /// Manifests that describe one build of a whole tree: a nested manifest
    /// of the same kind is a member of the outer build.
    const fn nests(self) -> bool {
        matches!(self, Self::Cargo | Self::Cmake | Self::Gradle | Self::Maven)
    }

    /// Manifests that define a module of the project.
    pub(crate) const fn defines_module(self) -> bool {
        !matches!(self, Self::Make | Self::Just | Self::Tox | Self::SetupCfg)
    }
}

/// Manifest found in the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Found {
    /// Relative path of the manifest.
    pub(crate) path: String,
    /// Kind of the manifest.
    pub(crate) kind: Manifest,
}

/// Manifests of the tree, shallow ones first.
pub(crate) fn find(files: &[String]) -> Vec<Found> {
    let mut found: Vec<Found> = files
        .iter()
        .filter(|path| path.matches('/').count() <= MANIFEST_DEPTH_MAX)
        .filter_map(|path| {
            Manifest::of(file_name(path)).map(|kind| Found {
                path: path.clone(),
                kind,
            })
        })
        .collect();
    found.sort_by(|a, b| {
        let depth = |path: &str| path.matches('/').count();
        depth(&a.path)
            .cmp(&depth(&b.path))
            .then_with(|| a.path.cmp(&b.path))
    });
    found
}

/// Entries read from one manifest.
#[derive(Debug, Default)]
pub(crate) struct Extracted {
    /// Build, test and check commands.
    pub(crate) commands: Vec<Entry>,
    /// Entry points.
    pub(crate) entry_points: Vec<Entry>,
}

impl Extracted {
    fn command(&mut self, dir: &str, name: &str, command: String, origin: String) {
        let mut entry = Entry::new(Section::Commands, name)
            .command(command)
            .origin(origin);
        if !dir.is_empty() {
            entry = entry.path(dir);
        }
        self.commands.push(entry);
    }

    fn entry_point(&mut self, name: &str, path: Option<String>, origin: String) {
        let mut entry = Entry::new(Section::EntryPoints, name).origin(origin);
        if let Some(path) = path {
            entry = entry.path(path);
        }
        self.entry_points.push(entry);
    }
}

/// Commands and entry points of every manifest.
pub(crate) fn extract(source: &Source<'_>, manifests: &[Found]) -> Extracted {
    let mut out = Extracted::default();
    for manifest in manifests {
        if manifest.kind.nests() && has_outer(manifests, manifest) {
            continue;
        }
        let dir = parent(&manifest.path);
        let mut ctx = Ctx {
            source,
            dir,
            path: &manifest.path,
            out: &mut out,
        };
        match manifest.kind {
            Manifest::Npm => npm(&mut ctx),
            Manifest::Cargo => cargo(&mut ctx),
            Manifest::Make => make(&mut ctx),
            Manifest::Just => just(&mut ctx),
            Manifest::Pyproject => pyproject(&mut ctx),
            Manifest::SetupCfg => setup_cfg(&mut ctx),
            Manifest::Tox => tox(&mut ctx),
            Manifest::GoMod => go_mod(&mut ctx),
            Manifest::Cmake => cmake(&mut ctx),
            Manifest::Gradle => gradle(&mut ctx),
            Manifest::Maven => maven(&mut ctx),
            Manifest::Dotnet => dotnet(&mut ctx),
            Manifest::Composer => composer(&mut ctx),
            Manifest::Bundler => bundler(&mut ctx),
            Manifest::Deno => deno(&mut ctx),
        }
    }
    conventional_entry_points(source, &mut out);
    out
}

fn has_outer(manifests: &[Found], inner: &Found) -> bool {
    let dir = parent(&inner.path);
    manifests.iter().any(|outer| {
        let outer_dir = parent(&outer.path);
        outer.kind == inner.kind
            && outer.path != inner.path
            && (outer_dir.is_empty() || dir.starts_with(&format!("{outer_dir}/")))
    })
}

struct Ctx<'s, 'a> {
    source: &'s Source<'a>,
    dir: &'s str,
    path: &'s str,
    out: &'s mut Extracted,
}

impl Ctx<'_, '_> {
    fn sibling(&self, name: &str) -> String {
        join(self.dir, name)
    }

    fn has_sibling(&self, name: &str) -> bool {
        self.source.has(&self.sibling(name))
    }

    fn origin(&self, key: &str) -> String {
        if key.is_empty() {
            self.path.to_owned()
        } else {
            format!("{}:{key}", self.path)
        }
    }

    fn command(&mut self, name: &str, command: impl Into<String>, key: &str) {
        let origin = self.origin(key);
        self.out.command(self.dir, name, command.into(), origin);
    }

    fn entry_point(&mut self, name: &str, target: Option<&str>, key: &str) {
        let origin = self.origin(key);
        let path = target.map(|target| join(self.dir, target));
        self.out.entry_point(name, path, origin);
    }

    fn lines(&self) -> Vec<(usize, String)> {
        self.source
            .text(self.path)
            .map(|text| {
                text.lines()
                    .enumerate()
                    .map(|(index, line)| (index + 1, line.to_owned()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn package_manager(ctx: &Ctx<'_, '_>) -> &'static str {
    const LOCK_FILES: [(&str, &str); 5] = [
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lockb", "bun"),
        ("bun.lock", "bun"),
        ("package-lock.json", "npm"),
    ];
    LOCK_FILES
        .iter()
        .find(|(lock, _)| ctx.has_sibling(lock))
        .map_or("npm", |(_, manager)| manager)
}

fn npm(ctx: &mut Ctx<'_, '_>) {
    let Some(doc) = ctx.source.document(ctx.path, &DocFormatId::JSON) else {
        return;
    };
    let manager = package_manager(ctx);
    for (name, _) in map_entries(&doc, &["scripts"]) {
        ctx.command(
            name,
            format!("{manager} run {name}"),
            &format!("scripts.{name}"),
        );
    }
    for key in ["main", "module"] {
        if let Some(target) = text_field(&doc, &[key]) {
            ctx.entry_point(target, Some(target), key);
        }
    }
    bin_entries(ctx, &doc, "bin");
    match field(&doc, &["exports"]) {
        Some(Value::Text(target)) => ctx.entry_point(target, Some(target), "exports"),
        Some(exports @ Value::Map(_)) => {
            if let Some(target) = export_target(exports) {
                ctx.entry_point(target, Some(target), "exports.\".\"");
            }
        }
        _ => {}
    }
}

fn export_target(exports: &Value) -> Option<&str> {
    match field(exports, &["."])? {
        Value::Text(target) => Some(target),
        conditions => ["import", "require", "default"]
            .iter()
            .find_map(|key| text_field(conditions, &[key])),
    }
}

fn bin_entries(ctx: &mut Ctx<'_, '_>, doc: &Value, key: &str) {
    match field(doc, &[key]) {
        Some(Value::Text(target)) => ctx.entry_point(target, Some(target), key),
        Some(Value::Map(_)) => {
            for (name, target) in map_entries(doc, &[key]) {
                ctx.entry_point(name, target.as_str(), &format!("{key}.{name}"));
            }
        }
        Some(Value::List(targets)) => {
            for target in targets.iter().filter_map(Value::as_str) {
                ctx.entry_point(target, Some(target), key);
            }
        }
        _ => {}
    }
}

fn cargo(ctx: &mut Ctx<'_, '_>) {
    let doc = ctx.source.document(ctx.path, &DocFormatId::TOML);
    let workspace = doc
        .as_ref()
        .is_some_and(|doc| field(doc, &["workspace"]).is_some());
    let scope = if workspace { " --workspace" } else { "" };
    for (name, verb) in [("build", "build"), ("test", "test"), ("lint", "clippy")] {
        ctx.command(name, format!("cargo {verb}{scope}"), "");
    }
    let Some(doc) = doc else {
        return;
    };
    let package = text_field(&doc, &["package", "name"]).unwrap_or_default();
    if let Some(Value::List(bins)) = field(&doc, &["bin"]) {
        for (index, bin) in bins.iter().enumerate() {
            let name = text_field(bin, &["name"]).unwrap_or(package);
            let target = text_field(bin, &["path"]);
            ctx.entry_point(name, target, &format!("bin[{index}]"));
        }
    }
    if ctx.has_sibling("src/main.rs") && !package.is_empty() {
        ctx.entry_point(package, Some("src/main.rs"), "package.name");
    }
}

fn make(ctx: &mut Ctx<'_, '_>) {
    for (line_no, line) in ctx.lines() {
        for target in make_targets(&line) {
            ctx.command(target, format!("make {target}"), &line_no.to_string());
        }
    }
}

/// Explicit targets of a rule line; variable assignments, special targets
/// and pattern rules yield nothing.
fn make_targets(line: &str) -> Vec<&str> {
    if line.starts_with(|c: char| c.is_whitespace() || c == '#') {
        return Vec::new();
    }
    let Some((targets, rest)) = line.split_once(':') else {
        return Vec::new();
    };
    if rest.starts_with('=') || rest.starts_with(":=") || targets.contains('=') {
        return Vec::new();
    }
    targets
        .split_whitespace()
        .filter(|target| !target.starts_with('.') && !target.contains(['%', '$']))
        .collect()
}

fn just(ctx: &mut Ctx<'_, '_>) {
    for (line_no, line) in ctx.lines() {
        if let Some(recipe) = just_recipe(&line) {
            ctx.command(recipe, format!("just {recipe}"), &line_no.to_string());
        }
    }
}

/// Recipe name of a recipe header line.
fn just_recipe(line: &str) -> Option<&str> {
    if line.starts_with(|c: char| c.is_whitespace() || c == '#' || c == '[') {
        return None;
    }
    let (head, rest) = line.split_once(':')?;
    if rest.starts_with('=') {
        return None;
    }
    let name = head.split_whitespace().next()?.trim_start_matches('@');
    const KEYWORDS: [&str; 6] = ["set", "alias", "export", "import", "mod", "unexport"];
    let valid = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    (valid && !name.is_empty() && !KEYWORDS.contains(&name)).then_some(name)
}

fn pyproject(ctx: &mut Ctx<'_, '_>) {
    let Some(doc) = ctx.source.document(ctx.path, &DocFormatId::TOML) else {
        return;
    };
    for keys in [["project", "scripts"], ["tool", "poetry"]] {
        let base = keys.join(".");
        let entries = if keys[1] == "poetry" {
            map_entries(&doc, &["tool", "poetry", "scripts"])
        } else {
            map_entries(&doc, &keys)
        };
        for (name, target) in entries {
            let key = if keys[1] == "poetry" {
                format!("{base}.scripts.{name}")
            } else {
                format!("{base}.{name}")
            };
            let origin = ctx.origin(&key);
            let mut entry = Entry::new(Section::EntryPoints, name).origin(origin);
            if let Some(target) = target.as_str() {
                entry = entry.command(target);
            }
            ctx.out.entry_points.push(entry);
        }
    }
    python_tools(ctx, &doc);
}

fn python_tools(ctx: &mut Ctx<'_, '_>, doc: &Value) {
    let poetry = field(doc, &["tool", "poetry"]).is_some();
    let runner = if poetry { "poetry run " } else { "" };
    if poetry {
        ctx.command("install", "poetry install", "tool.poetry");
    }
    if field(doc, &["build-system"]).is_some() {
        ctx.command("build", "python -m build", "build-system");
    }
    const TOOLS: [(&str, &str, &str); 4] = [
        ("pytest", "test", "pytest"),
        ("ruff", "lint", "ruff check ."),
        ("mypy", "typecheck", "mypy ."),
        ("tox", "tox", "tox"),
    ];
    for (tool, name, command) in TOOLS {
        if field(doc, &["tool", tool]).is_some() {
            ctx.command(name, format!("{runner}{command}"), &format!("tool.{tool}"));
        }
    }
    for (name, _) in map_entries(doc, &["tool", "hatch", "envs", "default", "scripts"]) {
        ctx.command(
            name,
            format!("hatch run {name}"),
            &format!("tool.hatch.envs.default.scripts.{name}"),
        );
    }
}

fn setup_cfg(ctx: &mut Ctx<'_, '_>) {
    let mut section = String::new();
    for (line_no, line) in ctx.lines() {
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = name.trim().to_owned();
            if section == "tool:pytest" {
                ctx.command("test", "pytest", &line_no.to_string());
            }
            continue;
        }
        let continuation = line.starts_with(char::is_whitespace);
        if section == "options.entry_points" && continuation {
            if let Some((name, target)) = trimmed.split_once('=') {
                let origin = ctx.origin(&line_no.to_string());
                ctx.out.entry_points.push(
                    Entry::new(Section::EntryPoints, name.trim())
                        .command(target.trim())
                        .origin(origin),
                );
            }
        }
    }
}

fn tox(ctx: &mut Ctx<'_, '_>) {
    ctx.command("test", "tox", "");
    let mut in_tox = false;
    for (line_no, line) in ctx.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_tox = trimmed == "[tox]";
            continue;
        }
        let Some(list) = trimmed.strip_prefix("envlist") else {
            continue;
        };
        if !in_tox {
            continue;
        }
        let list = list.trim_start().trim_start_matches('=');
        for env in list.split(',').map(str::trim).filter(|env| !env.is_empty()) {
            ctx.command(env, format!("tox -e {env}"), &line_no.to_string());
        }
    }
}

fn go_mod(ctx: &mut Ctx<'_, '_>) {
    for (name, command) in [
        ("build", "go build ./..."),
        ("test", "go test ./..."),
        ("lint", "go vet ./..."),
    ] {
        ctx.command(name, command, "");
    }
    let module = ctx
        .lines()
        .into_iter()
        .find_map(|(_, line)| line.strip_prefix("module ").map(|m| m.trim().to_owned()));
    if ctx.has_sibling("main.go") {
        let name = module.as_deref().unwrap_or("main");
        ctx.entry_point(name, Some("main.go"), "module");
    }
    let cmd = ctx.sibling("cmd");
    let prefix = format!("{cmd}/");
    let commands: Vec<String> = ctx
        .source
        .files()
        .iter()
        .filter_map(|path| path.strip_prefix(&prefix))
        .filter_map(|rest| rest.strip_suffix("/main.go"))
        .filter(|name| !name.contains('/'))
        .map(str::to_owned)
        .collect();
    for name in commands {
        let target = format!("cmd/{name}/main.go");
        let origin = join(ctx.dir, "cmd/");
        let path = join(ctx.dir, &target);
        ctx.out.entry_point(&name, Some(path), origin);
    }
}

fn cmake(ctx: &mut Ctx<'_, '_>) {
    ctx.command("configure", "cmake -S . -B build", "");
    ctx.command("build", "cmake --build build", "");
    let mut testing = false;
    for (line_no, line) in ctx.lines() {
        let lower = line.trim_start().to_ascii_lowercase();
        testing |= lower.starts_with("enable_testing") || lower.starts_with("add_test");
        if let Some(rest) = lower.strip_prefix("add_executable") {
            let offset = line.len() - rest.len();
            let name = line[offset..]
                .trim_start()
                .trim_start_matches('(')
                .split(|c: char| c.is_whitespace() || c == ')')
                .find(|part| !part.is_empty());
            if let Some(name) = name {
                ctx.entry_point(name, None, &line_no.to_string());
            }
        }
    }
    if testing {
        ctx.command("test", "ctest --test-dir build", "");
    }
}

fn gradle(ctx: &mut Ctx<'_, '_>) {
    let runner = if ctx.has_sibling("gradlew") {
        "./gradlew"
    } else {
        "gradle"
    };
    ctx.command("build", format!("{runner} build"), "");
    ctx.command("test", format!("{runner} test"), "");
    for (line_no, line) in ctx.lines() {
        if let Some(class) = assigned_main_class(&line) {
            ctx.entry_point(&class, None, &line_no.to_string());
        }
    }
}

/// Class name of a `mainClass` or `mainClassName` setting of a build script.
fn assigned_main_class(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix("mainClass")?;
    let rest = rest.strip_prefix("Name").unwrap_or(rest);
    let value = rest.split_once(['"', '\''])?.1;
    let class = value.split(['"', '\'']).next()?;
    (!class.is_empty()).then(|| class.to_owned())
}

fn maven(ctx: &mut Ctx<'_, '_>) {
    let runner = if ctx.has_sibling("mvnw") {
        "./mvnw"
    } else {
        "mvn"
    };
    ctx.command("build", format!("{runner} package"), "");
    ctx.command("test", format!("{runner} test"), "");
    for (line_no, line) in ctx.lines() {
        if let Some(class) = element_text(&line, "mainClass") {
            ctx.entry_point(class, None, &line_no.to_string());
        }
    }
}

/// Text of a one-line XML element `<name>text</name>`.
fn element_text<'l>(line: &'l str, name: &str) -> Option<&'l str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = line.find(&open)? + open.len();
    let end = start + line[start..].find(&close)?;
    let text = line[start..end].trim();
    (!text.is_empty()).then_some(text)
}

fn dotnet(ctx: &mut Ctx<'_, '_>) {
    let project = file_name(ctx.path).to_owned();
    ctx.command("build", format!("dotnet build {project}"), "");
    let lines = ctx.lines();
    let tests = lines.iter().any(|(_, line)| {
        line.contains("Microsoft.NET.Test.Sdk")
            || element_text(line, "IsTestProject") == Some("true")
    });
    if tests {
        ctx.command("test", format!("dotnet test {project}"), "");
    }
    let executable = lines
        .iter()
        .find(|(_, line)| element_text(line, "OutputType").is_some_and(|t| t.eq_ignore_ascii_case("exe")));
    if let Some((line_no, _)) = executable {
        let name = project.trim_end_matches(".csproj").to_owned();
        let origin = ctx.origin(&line_no.to_string());
        ctx.out
            .entry_point(&name, Some(ctx.path.to_owned()), origin);
    }
}

fn composer(ctx: &mut Ctx<'_, '_>) {
    let Some(doc) = ctx.source.document(ctx.path, &DocFormatId::JSON) else {
        return;
    };
    for (name, _) in map_entries(&doc, &["scripts"]) {
        ctx.command(
            name,
            format!("composer run-script {name}"),
            &format!("scripts.{name}"),
        );
    }
    bin_entries(ctx, &doc, "bin");
}

fn bundler(ctx: &mut Ctx<'_, '_>) {
    ctx.command("install", "bundle install", "");
    if ctx.has_sibling("Rakefile") {
        let origin = ctx.sibling("Rakefile");
        ctx.out
            .command(ctx.dir, "rake", "bundle exec rake".to_owned(), origin);
    }
    let spec = ctx.sibling("spec");
    if ctx.source.has_under(&spec) {
        ctx.out
            .command(ctx.dir, "test", "bundle exec rspec".to_owned(), format!("{spec}/"));
    }
}

fn deno(ctx: &mut Ctx<'_, '_>) {
    let format = if ctx.path.ends_with(".jsonc") {
        DocFormatId::JSONC
    } else {
        DocFormatId::JSON
    };
    let Some(doc) = ctx.source.document(ctx.path, &format) else {
        return;
    };
    for (name, _) in map_entries(&doc, &["tasks"]) {
        ctx.command(name, format!("deno task {name}"), &format!("tasks.{name}"));
    }
    match field(&doc, &["exports"]) {
        Some(Value::Text(target)) => ctx.entry_point(target, Some(target), "exports"),
        Some(exports @ Value::Map(_)) => {
            if let Some(target) = export_target(exports) {
                ctx.entry_point(target, Some(target), "exports.\".\"");
            }
        }
        _ => {}
    }
}

/// File names that start a program by convention, at the root or in `src/`.
const CONVENTIONAL_ENTRY_POINTS: [&str; 12] = [
    "main.py",
    "__main__.py",
    "app.py",
    "manage.py",
    "main.go",
    "main.rs",
    "index.ts",
    "index.js",
    "main.ts",
    "main.js",
    "cli.ts",
    "cli.js",
];

fn conventional_entry_points(source: &Source<'_>, out: &mut Extracted) {
    let known: Vec<String> = out
        .entry_points
        .iter()
        .filter_map(|entry| entry.path.clone())
        .collect();
    for path in source.files() {
        let dir = parent(path);
        let name = file_name(path);
        if name == "__main__.py" {
            if !known.contains(path) {
                let module = dir.replace('/', ".");
                let module = module.strip_prefix("src.").unwrap_or(&module).to_owned();
                out.entry_points.push(
                    Entry::new(Section::EntryPoints, path.clone())
                        .path(path.clone())
                        .command(format!("python -m {module}"))
                        .origin("__main__.py"),
                );
            }
            continue;
        }
        let top = dir.is_empty() || dir == "src";
        if top && CONVENTIONAL_ENTRY_POINTS.contains(&name) && !known.contains(path) {
            out.entry_points.push(
                Entry::new(Section::EntryPoints, path.clone())
                    .path(path.clone())
                    .origin("file name"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_targets_skip_assignments_and_special_targets() {
        assert_eq!(make_targets("build test: deps"), ["build", "test"]);
        assert!(make_targets("CC := gcc").is_empty());
        assert!(make_targets("X ::= 1").is_empty());
        assert!(make_targets(".PHONY: build").is_empty());
        assert!(make_targets("%.o: %.c").is_empty());
        assert!(make_targets("\tcc -o x").is_empty());
    }

    #[test]
    fn just_recipes_skip_settings_and_assignments() {
        assert_eq!(just_recipe("test *args:"), Some("test"));
        assert_eq!(just_recipe("@lint: build"), Some("lint"));
        assert_eq!(just_recipe("version := \"1\""), None);
        assert_eq!(just_recipe("set shell := [\"bash\"]"), None);
        assert_eq!(just_recipe("    echo x: y"), None);
    }

    #[test]
    fn main_class_settings_are_read_from_build_scripts() {
        assert_eq!(
            assigned_main_class("    mainClass = \"app.Main\""),
            Some("app.Main".to_owned())
        );
        assert_eq!(
            assigned_main_class("mainClass.set('app.Cli')"),
            Some("app.Cli".to_owned())
        );
        assert_eq!(
            assigned_main_class("mainClassName = 'x.Y'"),
            Some("x.Y".to_owned())
        );
        assert_eq!(assigned_main_class("version = '1'"), None);
    }

    #[test]
    fn element_text_reads_one_line_elements() {
        assert_eq!(
            element_text("  <mainClass>a.B</mainClass>", "mainClass"),
            Some("a.B")
        );
        assert_eq!(element_text("<mainClass></mainClass>", "mainClass"), None);
    }

    #[test]
    fn nested_manifests_of_one_build_are_found_in_depth_order() {
        let files = [
            "crates/a/Cargo.toml".to_owned(),
            "Cargo.toml".to_owned(),
            "a/b/c/d/e/package.json".to_owned(),
        ];
        let found = find(&files);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].path, "Cargo.toml");
        assert!(has_outer(&found, &found[1]));
    }
}
