// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Normalized state of repositories, read with the `git` program.
//!
//! Every section is a map from a key (path, reference name, configuration
//! key) to a text value. Values never contain what legitimately differs
//! between two copies of one repository: `stat` data of the index, the
//! location of the copy (replaced by `$ROOT`), and, outside strict mode, dates
//! and identities. Identifiers of commits created after the initial state are
//! replaced by `new#<n>`, numbered by a deterministic walk from the
//! references, so that both sides are comparable even when dates differ.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

use crate::error::{HarnessError, Result};
use crate::fsutil::{self, Entry};
use crate::git::{GitEnv, GitProgram, join_args};
use crate::scenario::{Section, StateSelector};

/// Placeholder of the case directory in normalized values.
pub const ROOT_PLACEHOLDER: &str = "$ROOT";

/// Longest file content shown as text in the `worktree` section.
const INLINE_CONTENT_LIMIT: usize = 1024;

/// Flag of an index entry that is outside the sparse checkout.
const FLAG_SKIP_WORKTREE: u32 = 0x4000_0000;
/// Flag of an index entry added with `git add -N`.
const FLAG_INTENT_TO_ADD: u32 = 0x2000_0000;

/// Files of an operation in progress, read from the directory of the
/// working tree.
const STATE_FILES: &[&str] = &[
    "MERGE_HEAD",
    "MERGE_MSG",
    "MERGE_MODE",
    "MERGE_AUTOSTASH",
    "MERGE_RR",
    "ORIG_HEAD",
    "AUTO_MERGE",
    "FETCH_HEAD",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
    "REBASE_HEAD",
    "SQUASH_MSG",
    "COMMIT_EDITMSG",
    "info/sparse-checkout",
];

/// Directories of an operation in progress, read recursively.
const STATE_DIRS: &[&str] = &["rebase-merge", "rebase-apply", "sequencer"];

/// Files of the common directory that belong to the state.
const COMMON_STATE_FILES: &[&str] = &["shallow"];

/// Normalized content of one section.
pub type SectionState = BTreeMap<String, String>;

/// Normalized state of the repositories of one copy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoState {
    sections: BTreeMap<StateSelector, SectionState>,
}

impl RepoState {
    /// Content of one section; empty when it was not captured.
    #[must_use]
    pub fn section(&self, selector: &StateSelector) -> Option<&SectionState> {
        self.sections.get(selector)
    }

    /// Captured sections.
    pub fn selectors(&self) -> impl Iterator<Item = &StateSelector> {
        self.sections.keys()
    }

    /// Adds or replaces a section.
    pub fn insert(&mut self, selector: StateSelector, state: SectionState) {
        self.sections.insert(selector, state);
    }
}

/// Object identifiers that existed in the initial state, per repository.
///
/// Commits reachable from them are old; everything else reachable from the
/// references after the operation is new.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Baseline {
    tips: BTreeMap<Option<String>, BTreeSet<String>>,
}

/// One repository as seen by the normalizer.
#[derive(Debug, Clone)]
struct RepoView {
    dir: PathBuf,
    git_dir: PathBuf,
    common_dir: PathBuf,
    bare: bool,
}

/// A parsed commit or annotated tag.
#[derive(Debug, Clone)]
struct ObjectRecord {
    kind: String,
    headers: Vec<(String, String)>,
    message: Vec<u8>,
}

/// Mapping of new object identifiers to stable labels.
#[derive(Debug, Clone, Default)]
struct NewObjects {
    labels: HashMap<String, String>,
    order: Vec<(String, String)>,
    records: HashMap<String, ObjectRecord>,
}

/// Reads normalized states with the `git` program.
#[derive(Debug, Clone)]
pub struct Normalizer<'a> {
    git: &'a GitProgram,
    env: GitEnv,
    strict: bool,
}

impl<'a> Normalizer<'a> {
    /// Creates a normalizer; `strict` keeps dates, identities and raw object
    /// identifiers of new commits.
    #[must_use]
    pub fn new(git: &'a GitProgram, env: &GitEnv, strict: bool) -> Self {
        Self {
            git,
            env: env.with_var("GIT_OPTIONAL_LOCKS", "0"),
            strict,
        }
    }

    /// Records the initial object identifiers of every repository in `selectors`.
    ///
    /// # Errors
    ///
    /// Fails with [`HarnessError`] when a repository cannot be read.
    pub fn baseline(&self, repo_root: &Path, selectors: &[StateSelector]) -> Result<Baseline> {
        let mut baseline = Baseline::default();
        for repo in repos_of(selectors) {
            let view = self.view(&locate(repo_root, repo.as_deref()))?;
            let tips = self.tips(&view)?.into_iter().map(|(_, oid)| oid).collect();
            baseline.tips.insert(repo, tips);
        }
        Ok(baseline)
    }

    /// Captures the sections named by `selectors`.
    ///
    /// The case directory `case_root` is replaced by [`ROOT_PLACEHOLDER`] in
    /// every value; the directory `repo_root` is the repository of the
    /// scenario.
    ///
    /// # Errors
    ///
    /// Fails with [`HarnessError`] when a repository cannot be read; a
    /// repository missing after the operation is reported as a section value
    /// instead.
    pub fn snapshot(
        &self,
        case_root: &Path,
        repo_root: &Path,
        selectors: &[StateSelector],
        baseline: &Baseline,
    ) -> Result<RepoState> {
        let mut state = RepoState::default();
        for repo in repos_of(selectors) {
            let dir = locate(repo_root, repo.as_deref());
            let wanted: Vec<&StateSelector> =
                selectors.iter().filter(|selector| selector.repo == repo).collect();
            if !dir.exists() {
                for selector in wanted {
                    let missing = SectionState::from([("repository".to_owned(), "missing".to_owned())]);
                    state.insert(selector.clone(), missing);
                }
                continue;
            }
            let view = self.view(&dir)?;
            let empty = BTreeSet::new();
            let old = baseline.tips.get(&repo).unwrap_or(&empty);
            let new_objects = self.new_objects(&view, old)?;
            for selector in wanted {
                let section = self.section(&view, &selector.section, old, &new_objects)?;
                state.insert(selector.clone(), substitute_root(section, case_root));
            }
        }
        Ok(state)
    }

    fn section(
        &self,
        view: &RepoView,
        section: &Section,
        old: &BTreeSet<String>,
        new_objects: &NewObjects,
    ) -> Result<SectionState> {
        let state = match section {
            Section::Index => self.index(view)?,
            Section::Worktree => worktree(view)?,
            Section::Status => self.status(view)?,
            Section::Refs => self.refs(view)?,
            Section::Reflog => self.reflog(view)?,
            Section::StateFiles => self.state_files(view)?,
            Section::Config => self.config(view)?,
            Section::Objects(rev) => self.objects(view, rev.as_deref(), old, new_objects)?,
            Section::Fsck => self.fsck(view)?,
        };
        Ok(state
            .into_iter()
            .map(|(key, value)| (key, new_objects.relabel(&value)))
            .collect())
    }

    fn run(&self, dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
        self.git.run_ok(&self.env, dir, args)
    }

    fn view(&self, dir: &Path) -> Result<RepoView> {
        let args = [
            "rev-parse",
            "--absolute-git-dir",
            "--git-common-dir",
            "--is-bare-repository",
        ];
        let output = String::from_utf8_lossy(&self.run(dir, &args)?).into_owned();
        let mut lines = output.lines();
        let mut next = || {
            lines.next().map(str::to_owned).ok_or_else(|| HarnessError::GitOutput {
                args: join_args(&args),
                detail: "fewer than three lines".to_owned(),
            })
        };
        let git_dir = PathBuf::from(next()?);
        let common = PathBuf::from(next()?);
        let bare = next()? == "true";
        let common_dir = if common.is_absolute() {
            common
        } else {
            dir.join(common)
        };
        Ok(RepoView {
            dir: dir.to_path_buf(),
            git_dir,
            common_dir,
            bare,
        })
    }

    /// Tips of the repository: `HEAD`, every reference, every reflog entry
    /// and every identifier in a state file, labelled for a deterministic
    /// walk.
    fn tips(&self, view: &RepoView) -> Result<Vec<(String, String)>> {
        let mut tips = Vec::new();
        if let Some(head) = self.head_oid(view)? {
            tips.push(("HEAD".to_owned(), head));
        }
        for (name, (oid, _)) in self.ref_values(view)? {
            tips.push((name, oid));
        }
        for (name, text) in self.state_files(view)?.into_iter().chain(self.reflog(view)?) {
            for oid in find_oids(&text) {
                tips.push((name.clone(), oid));
            }
        }
        let known = self.existing_commitish(view, tips.iter().map(|(_, oid)| oid.as_str()))?;
        tips.retain(|(_, oid)| known.contains(oid));
        Ok(tips)
    }

    fn head_oid(&self, view: &RepoView) -> Result<Option<String>> {
        let output = self.git.run(&self.env, &view.dir, &["rev-parse", "-q", "--verify", "HEAD"])?;
        Ok((output.code == 0).then(|| output.stdout_text()))
    }

    fn ref_values(&self, view: &RepoView) -> Result<BTreeMap<String, (String, String)>> {
        let args = ["for-each-ref", "--format=%(refname)%00%(objectname)%00%(symref)"];
        let output = String::from_utf8_lossy(&self.run(&view.dir, &args)?).into_owned();
        let mut refs = BTreeMap::new();
        for line in output.lines() {
            let mut fields = line.split('\0');
            let (Some(name), Some(oid), Some(symref)) = (fields.next(), fields.next(), fields.next())
            else {
                return Err(HarnessError::GitOutput {
                    args: join_args(&args),
                    detail: format!("line `{line}` has fewer than three fields"),
                });
            };
            refs.insert(name.to_owned(), (oid.to_owned(), symref.to_owned()));
        }
        Ok(refs)
    }

    /// Subset of `oids` that names existing commits or tags.
    fn existing_commitish<'o>(
        &self,
        view: &RepoView,
        oids: impl Iterator<Item = &'o str>,
    ) -> Result<BTreeSet<String>> {
        let unique: BTreeSet<&str> = oids.collect();
        if unique.is_empty() {
            return Ok(BTreeSet::new());
        }
        let input: String = unique.iter().map(|oid| format!("{oid}\n")).collect();
        let args = ["cat-file", "--batch-check=%(objectname) %(objecttype)"];
        let output = self
            .git
            .run_with_input(&self.env, &view.dir, &args, Some(input.as_bytes()))?;
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.split_once(' '))
            .filter(|(_, kind)| matches!(*kind, "commit" | "tag"))
            .map(|(oid, _)| oid.to_owned())
            .collect())
    }

    fn new_objects(&self, view: &RepoView, old: &BTreeSet<String>) -> Result<NewObjects> {
        let tips = self.tips(view)?;
        let new_commits = self.rev_list(view, tips.iter().map(|(_, oid)| oid.as_str()), old)?;
        let new_tags: BTreeSet<String> = self
            .ref_values(view)?
            .into_values()
            .map(|(oid, _)| oid)
            .filter(|oid| !old.contains(oid))
            .collect();
        let candidates: Vec<String> = new_commits.iter().chain(&new_tags).cloned().collect();
        let mut records = self.read_objects(view, &candidates)?;
        records.retain(|oid, record| new_commits.contains(oid) || record.kind == "tag");
        Ok(NewObjects::number(&tips, records))
    }

    fn rev_list<'o>(
        &self,
        view: &RepoView,
        tips: impl Iterator<Item = &'o str>,
        old: &BTreeSet<String>,
    ) -> Result<BTreeSet<String>> {
        let mut input = String::new();
        for tip in tips {
            input.push_str(tip);
            input.push('\n');
        }
        if input.is_empty() {
            return Ok(BTreeSet::new());
        }
        input.push_str("--not\n");
        for oid in old {
            input.push_str(oid);
            input.push('\n');
        }
        let args = ["rev-list", "--ignore-missing", "--stdin"];
        let output = self
            .git
            .run_with_input(&self.env, &view.dir, &args, Some(input.as_bytes()))?;
        crate::git::expect_exit(&output, 0, &view.dir, &args)?;
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_owned)
            .collect())
    }

    fn read_objects(&self, view: &RepoView, oids: &[String]) -> Result<HashMap<String, ObjectRecord>> {
        if oids.is_empty() {
            return Ok(HashMap::new());
        }
        let input: String = oids.iter().map(|oid| format!("{oid}\n")).collect();
        let args = ["cat-file", "--batch"];
        let output = self
            .git
            .run_with_input(&self.env, &view.dir, &args, Some(input.as_bytes()))?;
        parse_batch(&output.stdout).map_err(|detail| HarnessError::GitOutput {
            args: join_args(&args),
            detail,
        })
    }

    fn index(&self, view: &RepoView) -> Result<SectionState> {
        let mut state = SectionState::new();
        if view.bare {
            return Ok(state);
        }
        let index_file = view.git_dir.join("index");
        if let Ok(bytes) = std::fs::read(&index_file) {
            if let Some(version) = bytes.get(4..8) {
                let version = u32::from_be_bytes([version[0], version[1], version[2], version[3]]);
                state.insert("version".to_owned(), version.to_string());
            }
        }
        let listing = self.run(&view.dir, &["ls-files", "-s", "--debug", "-z"])?;
        for (header, flags) in parse_debug_listing(&listing) {
            let Some((fields, path)) = header.split_once('\t') else {
                continue;
            };
            let stage = fields.rsplit(' ').next().unwrap_or("0");
            let mut value = fields.to_owned();
            if flags & FLAG_SKIP_WORKTREE != 0 {
                value.push_str(" skip-worktree");
            }
            if flags & FLAG_INTENT_TO_ADD != 0 {
                value.push_str(" intent-to-add");
            }
            state.insert(format!("{path} [{stage}]"), value);
        }
        let undo = self.run(&view.dir, &["ls-files", "--resolve-undo", "-z"])?;
        for record in undo.split(|byte| *byte == 0).filter(|record| !record.is_empty()) {
            let record = String::from_utf8_lossy(record);
            if let Some((fields, path)) = record.split_once('\t') {
                state
                    .entry(format!("resolve-undo {path}"))
                    .or_default()
                    .push_str(&format!("{fields}\n"));
            }
        }
        Ok(state)
    }

    fn status(&self, view: &RepoView) -> Result<SectionState> {
        let mut state = SectionState::new();
        if view.bare {
            return Ok(state);
        }
        let args = [
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--untracked-files=all",
            "--ignored",
        ];
        let output = self.run(&view.dir, &args)?;
        let mut records = output.split(|byte| *byte == 0).filter(|record| !record.is_empty());
        while let Some(record) = records.next() {
            let line = String::from_utf8_lossy(record).into_owned();
            let key = if let Some(header) = line.strip_prefix("# ") {
                header.split(' ').next().unwrap_or(header).to_owned()
            } else if line.starts_with("2 ") {
                let original = records
                    .next()
                    .map(|path| String::from_utf8_lossy(path).into_owned())
                    .unwrap_or_default();
                let key = status_path(&line).to_owned();
                state.insert(key, format!("{line} <- {original}"));
                continue;
            } else {
                status_path(&line).to_owned()
            };
            state.insert(key, line);
        }
        Ok(state)
    }

    fn refs(&self, view: &RepoView) -> Result<SectionState> {
        let mut state: SectionState = self
            .ref_values(view)?
            .into_iter()
            .map(|(name, (oid, symref))| {
                let value = if symref.is_empty() {
                    oid
                } else {
                    format!("{oid} -> {symref}")
                };
                (name, value)
            })
            .collect();
        state.insert("HEAD".to_owned(), read_text_or_missing(&view.git_dir.join("HEAD")));
        let worktrees = view.common_dir.join("worktrees");
        if let Ok(names) = std::fs::read_dir(&worktrees) {
            for name in names.flatten() {
                let key = format!("worktrees/{}/HEAD", name.file_name().to_string_lossy());
                state.insert(key, read_text_or_missing(&name.path().join("HEAD")));
            }
        }
        Ok(state)
    }

    fn reflog(&self, view: &RepoView) -> Result<SectionState> {
        let mut state = SectionState::new();
        let mut files: Vec<(String, PathBuf)> = Vec::new();
        let logs = view.common_dir.join("logs");
        if logs.is_dir() {
            for (key, entry) in fsutil::walk(&logs, &|_| false)? {
                if matches!(entry, Entry::File { .. }) {
                    files.push((format!("logs/{key}"), logs.join(&key)));
                }
            }
        }
        if view.git_dir != view.common_dir {
            files.push(("logs/HEAD@worktree".to_owned(), view.git_dir.join("logs/HEAD")));
        }
        for (key, path) in files {
            if let Ok(bytes) = std::fs::read(&path) {
                let text = String::from_utf8_lossy(&bytes);
                let lines: Vec<String> = text.lines().map(|line| self.reflog_line(line)).collect();
                state.insert(key, lines.join("\n"));
            }
        }
        Ok(state)
    }

    fn reflog_line(&self, line: &str) -> String {
        if self.strict {
            return line.to_owned();
        }
        let (head, message) = line.split_once('\t').unwrap_or((line, ""));
        let mut fields = head.splitn(3, ' ');
        let old = fields.next().unwrap_or_default();
        let new = fields.next().unwrap_or_default();
        format!("{old} {new}\t{message}")
    }

    fn state_files(&self, view: &RepoView) -> Result<SectionState> {
        let mut state = SectionState::new();
        for name in STATE_FILES {
            self.add_state_file(&mut state, &view.git_dir, name);
        }
        for name in COMMON_STATE_FILES {
            self.add_state_file(&mut state, &view.common_dir, name);
        }
        for dir in STATE_DIRS {
            self.add_state_dir(&mut state, &view.git_dir, dir, &|_| false)?;
        }
        let skip = |path: &Path| {
            path.file_name()
                .is_some_and(|name| name == "logs" || name == "index" || name == "HEAD")
        };
        self.add_state_dir(&mut state, &view.common_dir, "worktrees", &skip)?;
        Ok(state)
    }

    fn add_state_file(&self, state: &mut SectionState, dir: &Path, name: &str) {
        if let Ok(bytes) = std::fs::read(dir.join(name)) {
            state.insert(name.to_owned(), self.state_value(name, &bytes));
        }
    }

    fn add_state_dir(
        &self,
        state: &mut SectionState,
        base: &Path,
        name: &str,
        skip: &dyn Fn(&Path) -> bool,
    ) -> Result<()> {
        let dir = base.join(name);
        if !dir.is_dir() {
            return Ok(());
        }
        for (key, entry) in fsutil::walk(&dir, skip)? {
            if let Entry::File { bytes, .. } = entry {
                let key = format!("{name}/{key}");
                let value = self.state_value(&key, &bytes);
                state.insert(key, value);
            }
        }
        Ok(())
    }

    fn state_value(&self, name: &str, bytes: &[u8]) -> String {
        let text = describe_bytes(bytes);
        if self.strict || !name.ends_with("author-script") {
            return text;
        }
        text.lines()
            .filter(|line| !line.starts_with("GIT_AUTHOR_DATE="))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn config(&self, view: &RepoView) -> Result<SectionState> {
        let mut state = SectionState::new();
        let mut files = vec![(String::new(), view.common_dir.join("config"))];
        let worktree_config = view.git_dir.join("config.worktree");
        if worktree_config.is_file() {
            files.push(("worktree:".to_owned(), worktree_config));
        }
        for (prefix, file) in files {
            if !file.is_file() {
                continue;
            }
            let file_arg = file.to_string_lossy().into_owned();
            let output = self.run(&view.dir, &["config", "--file", &file_arg, "--list", "-z"])?;
            for record in output.split(|byte| *byte == 0).filter(|record| !record.is_empty()) {
                let record = String::from_utf8_lossy(record);
                let (key, value) = record.split_once('\n').unwrap_or((&record, "(no value)"));
                let slot = state.entry(format!("{prefix}{key}")).or_default();
                if !slot.is_empty() {
                    slot.push('\n');
                }
                slot.push_str(value);
            }
        }
        Ok(state)
    }

    fn objects(
        &self,
        view: &RepoView,
        rev: Option<&str>,
        old: &BTreeSet<String>,
        new_objects: &NewObjects,
    ) -> Result<SectionState> {
        let reachable = match rev {
            None => None,
            Some(rev) => {
                let output = self.git.run(&self.env, &view.dir, &["rev-parse", "-q", "--verify", rev])?;
                if output.code != 0 {
                    return Ok(SectionState::from([(rev.to_owned(), "unresolved".to_owned())]));
                }
                let tip = output.stdout_text();
                Some(self.rev_list(view, std::iter::once(tip.as_str()), old)?)
            }
        };
        let mut state = SectionState::new();
        for (oid, label) in &new_objects.order {
            if reachable.as_ref().is_some_and(|set| !set.contains(oid)) {
                continue;
            }
            if let Some(record) = new_objects.records.get(oid) {
                state.insert(label.clone(), self.describe_object(oid, record));
            }
        }
        Ok(state)
    }

    fn describe_object(&self, oid: &str, record: &ObjectRecord) -> String {
        let mut lines = Vec::new();
        if self.strict {
            lines.push(format!("id {oid}"));
        }
        for (name, value) in &record.headers {
            let value = match name.as_str() {
                "author" | "committer" | "tagger" if !self.strict => strip_date(value),
                "gpgsig" | "gpgsig-sha256" if !self.strict => "(present)".to_owned(),
                _ => value.clone(),
            };
            lines.push(format!("{name} {value}"));
        }
        lines.push(String::new());
        lines.push(describe_bytes(&record.message));
        lines.join("\n")
    }

    fn fsck(&self, view: &RepoView) -> Result<SectionState> {
        let args = ["fsck", "--strict", "--no-dangling", "--no-progress"];
        let output = self.git.run(&self.env, &view.dir, &args)?;
        let value = if output.code == 0 {
            "ok".to_owned()
        } else {
            format!(
                "exit {}: {} {}",
                output.code,
                output.stdout_text(),
                output.stderr_text()
            )
        };
        Ok(SectionState::from([("result".to_owned(), value)]))
    }
}

impl NewObjects {
    /// Numbers new objects by a depth-first walk from `tips` in order,
    /// visiting parents in order and labelling each object after its
    /// parents.
    fn number(tips: &[(String, String)], records: HashMap<String, ObjectRecord>) -> Self {
        let mut numbered = Self {
            records,
            ..Self::default()
        };
        for (_, tip) in tips {
            numbered.visit(tip);
        }
        numbered
    }

    fn visit(&mut self, start: &str) {
        let mut stack: Vec<(String, bool)> = vec![(start.to_owned(), false)];
        while let Some((oid, expanded)) = stack.pop() {
            if self.labels.contains_key(&oid) {
                continue;
            }
            let Some(record) = self.records.get(&oid) else {
                continue;
            };
            if expanded {
                let prefix = if record.kind == "tag" { "newtag" } else { "new" };
                let label = format!("{prefix}#{}", self.order.len() + 1);
                self.labels.insert(oid.clone(), label.clone());
                self.order.push((oid, label));
                continue;
            }
            let parents: Vec<String> = record
                .headers
                .iter()
                .filter(|(name, _)| name == "parent" || name == "object")
                .map(|(_, value)| value.clone())
                .collect();
            stack.push((oid, true));
            for parent in parents.into_iter().rev() {
                stack.push((parent, false));
            }
        }
    }

    /// Replaces identifiers of new objects in `text` by their labels.
    fn relabel(&self, text: &str) -> String {
        if self.labels.is_empty() {
            return text.to_owned();
        }
        replace_oids(text, |oid| self.labels.get(oid).cloned())
    }
}

/// Repositories named by `selectors`, the scenario repository first.
fn repos_of(selectors: &[StateSelector]) -> Vec<Option<String>> {
    let mut repos: Vec<Option<String>> = vec![None];
    for selector in selectors {
        if !repos.contains(&selector.repo) {
            repos.push(selector.repo.clone());
        }
    }
    repos
}

fn locate(repo_root: &Path, repo: Option<&str>) -> PathBuf {
    match repo {
        None => repo_root.to_path_buf(),
        Some(relative) => repo_root.join(relative),
    }
}

fn worktree(view: &RepoView) -> Result<SectionState> {
    if view.bare {
        return Ok(SectionState::new());
    }
    let git_dir = view.git_dir.clone();
    let skip = move |path: &Path| path == git_dir || path.file_name().is_some_and(|name| name == ".git" && path.is_dir());
    Ok(fsutil::walk(&view.dir, &skip)?
        .into_iter()
        .map(|(key, entry)| (key, describe_entry(&entry)))
        .collect())
}

fn describe_entry(entry: &Entry) -> String {
    match entry {
        Entry::File { executable, bytes } => {
            let mode = if *executable { "755" } else { "644" };
            format!("file {mode} {}", describe_bytes(bytes))
        }
        Entry::Dir => "dir".to_owned(),
        Entry::Link { target } => format!("link -> {target}"),
        Entry::Other => "special".to_owned(),
    }
}

/// Short text form of bytes: the text itself when it is short UTF-8,
/// otherwise the length and SHA-256.
fn describe_bytes(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) if bytes.len() <= INLINE_CONTENT_LIMIT => format!("{text:?}"),
        _ => format!("{} bytes sha256:{}", bytes.len(), hex::encode(Sha256::digest(bytes))),
    }
}

fn read_text_or_missing(path: &Path) -> String {
    std::fs::read(path).map_or_else(
        |_| "missing".to_owned(),
        |bytes| String::from_utf8_lossy(&bytes).trim_end().to_owned(),
    )
}

/// Path field of a `git status --porcelain=v2` record: the last field,
/// whose position depends on the record type.
fn status_path(line: &str) -> &str {
    let fields = match line.as_bytes().first() {
        Some(b'1') => 8,
        Some(b'2') => 9,
        Some(b'u') => 10,
        _ => 1,
    };
    line.splitn(fields + 1, ' ').nth(fields).unwrap_or(line)
}

/// Removes the date and time zone from an identity `Name <mail> 123 +0100`.
fn strip_date(identity: &str) -> String {
    match identity.rfind('>') {
        Some(end) => identity[..=end].to_owned(),
        None => identity.to_owned(),
    }
}

/// Splits the output of `git ls-files -s --debug -z` into entry headers
/// (`<mode> <oid> <stage>\t<path>`) and index flags.
fn parse_debug_listing(bytes: &[u8]) -> Vec<(String, u32)> {
    let mut entries = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        let Some(end) = rest.iter().position(|byte| *byte == 0) else {
            break;
        };
        let header = String::from_utf8_lossy(&rest[..end]).into_owned();
        rest = &rest[end + 1..];
        let mut flags = 0;
        while rest.starts_with(b"  ") {
            let line_end = rest.iter().position(|byte| *byte == b'\n').unwrap_or(rest.len());
            let line = String::from_utf8_lossy(&rest[..line_end]);
            if let Some(value) = line.split("flags: ").nth(1) {
                flags = u32::from_str_radix(value.trim(), 16).unwrap_or(0);
            }
            rest = rest.get(line_end + 1..).unwrap_or_default();
        }
        entries.push((header, flags));
    }
    entries
}

/// Parses the output of `git cat-file --batch` for commits and tags.
fn parse_batch(bytes: &[u8]) -> Result<HashMap<String, ObjectRecord>, String> {
    let mut objects = HashMap::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        let end = rest
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or("object header without a line end")?;
        let header = String::from_utf8_lossy(&rest[..end]).into_owned();
        rest = &rest[end + 1..];
        let fields: Vec<&str> = header.split(' ').collect();
        if fields.len() != 3 {
            continue;
        }
        let size: usize = fields[2]
            .parse()
            .map_err(|_| format!("object header `{header}` has an invalid size"))?;
        let body = rest.get(..size).ok_or("object content is truncated")?;
        rest = rest.get(size + 1..).unwrap_or_default();
        if matches!(fields[1], "commit" | "tag") {
            objects.insert(fields[0].to_owned(), parse_object(fields[1], body));
        }
    }
    Ok(objects)
}

fn parse_object(kind: &str, body: &[u8]) -> ObjectRecord {
    let split = body
        .windows(2)
        .position(|pair| pair == b"\n\n")
        .unwrap_or(body.len());
    let head = String::from_utf8_lossy(&body[..split]);
    let message = body.get(split + 2..).unwrap_or_default().to_vec();
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in head.lines() {
        if let Some(continuation) = line.strip_prefix(' ') {
            if let Some((_, value)) = headers.last_mut() {
                value.push('\n');
                value.push_str(continuation);
            }
            continue;
        }
        let (name, value) = line.split_once(' ').unwrap_or((line, ""));
        headers.push((name.to_owned(), value.to_owned()));
    }
    ObjectRecord {
        kind: kind.to_owned(),
        headers,
        message,
    }
}

fn is_oid_length(length: usize) -> bool {
    length == 40 || length == 64
}

/// Every full object identifier (40 or 64 hexadecimal digits) in `text`.
fn find_oids(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    replace_oids(text, |oid| {
        if oid.bytes().any(|byte| byte != b'0') {
            found.push(oid.to_owned());
        }
        None
    });
    found
}

/// Replaces maximal runs of lower-case hexadecimal digits of identifier
/// length for which `label` returns a replacement.
fn replace_oids(text: &str, mut label: impl FnMut(&str) -> Option<String>) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        if !is_lower_hex(bytes[index]) {
            index += 1;
            continue;
        }
        let run_start = index;
        while index < bytes.len() && is_lower_hex(bytes[index]) {
            index += 1;
        }
        let run = &text[run_start..index];
        if is_oid_length(run.len()) {
            if let Some(replacement) = label(run) {
                out.push_str(&text[start..run_start]);
                out.push_str(&replacement);
                start = index;
            }
        }
    }
    out.push_str(&text[start..]);
    out
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

/// Replaces the case directory in every value by [`ROOT_PLACEHOLDER`].
fn substitute_root(section: SectionState, case_root: &Path) -> SectionState {
    let native = case_root.to_string_lossy().into_owned();
    let forward = native.replace('\\', "/");
    let quoted = format!("{native:?}");
    let quoted = quoted.trim_matches('"').to_owned();
    section
        .into_iter()
        .map(|(key, value)| {
            let value = value
                .replace(&native, ROOT_PLACEHOLDER)
                .replace(&forward, ROOT_PLACEHOLDER)
                .replace(&quoted, ROOT_PLACEHOLDER);
            (key, value)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OID_A: &str = "1111111111111111111111111111111111111111";
    const OID_B: &str = "2222222222222222222222222222222222222222";

    #[test]
    fn replaces_only_full_identifiers() {
        let text = format!("{OID_A} {OID_B}x abc {OID_A}0");
        let replaced = replace_oids(&text, |oid| (oid == OID_A).then(|| "new#1".to_owned()));
        assert_eq!(replaced, format!("new#1 {OID_B}x abc {OID_A}0"));
        assert_eq!(find_oids(&format!("{} {OID_B}", "0".repeat(40))), vec![OID_B]);
    }

    #[test]
    fn parses_debug_listing_with_flags() {
        let listing = format!(
            "100644 {OID_A} 0\ta b\0  ctime: 1:2\n  mtime: 1:2\n  dev: 1\tino: 2\n  uid: 1\tgid: 2\n  size: 2\tflags: 40004000\n\
             100644 {OID_B} 0\tn\0  ctime: 0:0\n  size: 0\tflags: 20004000\n"
        );
        let entries = parse_debug_listing(listing.as_bytes());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, format!("100644 {OID_A} 0\ta b"));
        assert_eq!(entries[0].1 & FLAG_SKIP_WORKTREE, FLAG_SKIP_WORKTREE);
        assert_eq!(entries[1].1 & FLAG_INTENT_TO_ADD, FLAG_INTENT_TO_ADD);
    }

    #[test]
    fn parses_batch_output_and_headers() {
        let commit = format!(
            "tree {OID_A}\nparent {OID_B}\nauthor A <a@x> 1 +0000\ncommitter A <a@x> 1 +0000\ngpgsig -----BEGIN\n line\n -----END\n\nsubject\n\nbody\n"
        );
        let batch = format!("{OID_A} commit {}\n{commit}\n{OID_B} blob 1\nx\n", commit.len());
        let objects = parse_batch(batch.as_bytes()).unwrap();
        let record = &objects[OID_A];
        assert_eq!(record.kind, "commit");
        assert_eq!(record.headers[4].1, "-----BEGIN\nline\n-----END");
        assert_eq!(record.message, b"subject\n\nbody\n");
        assert!(!objects.contains_key(OID_B));
        assert!(parse_batch(format!("{OID_A} commit 99\nx").as_bytes()).is_err());
    }

    #[test]
    fn numbers_new_objects_parents_first() {
        let record = |parents: &[&str]| ObjectRecord {
            kind: "commit".into(),
            headers: parents.iter().map(|p| ("parent".to_owned(), (*p).to_owned())).collect(),
            message: Vec::new(),
        };
        let records = HashMap::from([
            (OID_A.to_owned(), record(&[])),
            (OID_B.to_owned(), record(&[OID_A])),
        ]);
        let numbered = NewObjects::number(&[("HEAD".into(), OID_B.into())], records);
        assert_eq!(numbered.labels[OID_A], "new#1");
        assert_eq!(numbered.labels[OID_B], "new#2");
        assert_eq!(numbered.relabel(&format!("x {OID_B}")), "x new#2");
    }

    #[test]
    fn extracts_status_paths_by_record_type() {
        assert_eq!(status_path("1 .M N... 100644 100644 100644 a b c d e"), "c d e");
        assert_eq!(status_path("? new file.txt"), "new file.txt");
        assert_eq!(status_path("u UU N... 1 2 3 4 a b c x y"), "x y");
    }

    #[test]
    fn strips_dates_from_identities() {
        assert_eq!(strip_date("A B <a@x> 1767268800 +0100"), "A B <a@x>");
        assert_eq!(strip_date("broken"), "broken");
    }

    #[test]
    fn describes_short_text_inline_and_binary_by_hash() {
        assert_eq!(describe_bytes(b"a\n"), "\"a\\n\"");
        assert!(describe_bytes(&[0xff, 0]).starts_with("2 bytes sha256:"));
        assert!(describe_entry(&Entry::Link { target: "t".into() }).ends_with("t"));
    }

    #[test]
    fn substitutes_the_case_directory() {
        let section = SectionState::from([("k".to_owned(), "gitdir: /tmp/case/a/repo/.git".to_owned())]);
        let replaced = substitute_root(section, Path::new("/tmp/case/a"));
        assert_eq!(replaced["k"], "gitdir: $ROOT/repo/.git");
    }

    #[test]
    fn lists_repositories_of_selectors() {
        let selectors: Vec<StateSelector> = ["refs", "refs@../up", "fsck@../up", "index"]
            .iter()
            .map(|text| text.parse().unwrap())
            .collect();
        assert_eq!(repos_of(&selectors), vec![None, Some("../up".to_owned())]);
    }
}
