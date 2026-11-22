// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! The state directory `rebase-merge/` of a rebase in progress.
//!
//! The files have the names and the format written by Git 2.53 for a rebase
//! with the `merge` backend, so that a rebase stopped by one implementation
//! can be continued, skipped or aborted by the other.

use std::collections::BTreeMap;
use std::path::Path;

use bstr::{BStr, BString, ByteSlice};
use gix::ObjectId;
use nurt_contracts::error::Result;

use super::error;
use super::history::TodoItem;

/// Name of the state directory inside the Git directory.
pub(super) const DIR: &str = "rebase-merge";
/// Value of `head-name` when the rebase started on a detached `HEAD`.
pub(super) const DETACHED: &str = "detached HEAD";

/// Empty marker files that Git writes into the state directory.
pub(super) const MARKERS: [&str; 3] = ["interactive", "drop_redundant_commits", "no-reschedule-failed-exec"];

/// Help text appended by Git to `git-rebase-todo.backup`.
const TODO_HELP: &str = "#
# Commands:
# p, pick <commit> = use commit
# r, reword <commit> = use commit, but edit the commit message
# e, edit <commit> = use commit, but stop for amending
# s, squash <commit> = use commit, but meld into previous commit
# f, fixup [-C | -c] <commit> = like \"squash\" but keep only the previous
#                    commit's log message, unless -C is used, in which case
#                    keep only this commit's message; -c is same as -C but
#                    opens the editor
# x, exec <command> = run command (the rest of the line) using shell
# b, break = stop here (continue rebase later with 'git rebase --continue')
# d, drop <commit> = remove commit
# l, label <label> = label current HEAD with a name
# t, reset <label> = reset HEAD to a label
# m, merge [-C <commit> | -c <commit>] <label> [# <oneline>]
#         create a merge commit using the original merge commit's
#         message (or the oneline, if no original merge commit was
#         specified); use -c <commit> to reword the commit message
# u, update-ref <ref> = track a placeholder for the <ref> to be updated
#                       to this position in the new commits. The <ref> is
#                       updated at the end of the rebase
#
# These lines can be re-ordered; they are executed from top to bottom.
#
# If you remove a line here THAT COMMIT WILL BE LOST.
#
# However, if you remove everything, the rebase will be aborted.
#
";

/// State of a stopped rebase as read from the state directory.
#[derive(Debug, Clone)]
pub(super) struct RebaseState {
    /// Branch being rebased (`refs/heads/<b>`) or [`DETACHED`].
    pub(super) head_name: String,
    /// Commit the branch is rebased onto.
    pub(super) onto: ObjectId,
    /// Commit of `HEAD` when the rebase started.
    pub(super) orig_head: ObjectId,
    /// Raw content of `done`.
    pub(super) done: BString,
    /// Number of commands in `done`.
    pub(super) done_count: usize,
    /// Commands still to run, in order.
    pub(super) todo: Vec<TodoItem>,
    /// Value of `end`.
    pub(super) end: usize,
    /// Author of the stopped commit from `author-script`.
    pub(super) author: Option<AuthorScript>,
    /// Content of `message`.
    pub(super) message: Option<BString>,
    /// Commit at which the rebase stopped with conflicts.
    pub(super) stopped: Option<ObjectId>,
    /// Content of `rewritten-list`.
    pub(super) rewritten: BString,
    /// Every file of the directory with its content, for the journal.
    pub(super) files: BTreeMap<String, Vec<u8>>,
}

impl RebaseState {
    /// Branch reference being rebased; `None` on a detached `HEAD`.
    pub(super) fn branch(&self) -> Option<&str> {
        (self.head_name != DETACHED).then_some(self.head_name.as_str())
    }
}

/// Identity of an author as stored in `author-script`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AuthorScript {
    /// Name.
    pub(super) name: BString,
    /// Email address.
    pub(super) email: BString,
    /// Date in the form `@<seconds> <offset>`.
    pub(super) date: BString,
}

impl AuthorScript {
    /// Author fields of a commit from its raw `author` header.
    pub(super) fn from_header(header: &BStr) -> Result<Self> {
        let signature = gix_actor::SignatureRef::from_bytes(header)
            .map_err(|cause| error::corrupt("cannot parse the author of a commit", cause))?;
        Ok(Self {
            name: signature.name.to_owned(),
            email: signature.email.to_owned(),
            date: format!("@{}", signature.time.trim()).into(),
        })
    }

    /// Raw `author` header: `Name <email> seconds offset`.
    pub(super) fn header(&self) -> BString {
        let mut out = self.name.clone();
        out.extend_from_slice(b" <");
        out.extend_from_slice(&self.email);
        out.extend_from_slice(b"> ");
        out.extend_from_slice(self.date.strip_prefix(b"@").unwrap_or(&self.date));
        out
    }

    /// Content of `author-script`.
    pub(super) fn render(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (key, value) in [
            ("GIT_AUTHOR_NAME", &self.name),
            ("GIT_AUTHOR_EMAIL", &self.email),
            ("GIT_AUTHOR_DATE", &self.date),
        ] {
            out.extend_from_slice(key.as_bytes());
            out.push(b'=');
            out.extend_from_slice(&shell_quote(value));
            out.push(b'\n');
        }
        out
    }

    /// Parses the content of `author-script`.
    pub(super) fn parse(data: &[u8]) -> Result<Self> {
        let mut fields: BTreeMap<&str, BString> = BTreeMap::new();
        for line in data.lines() {
            let Some(at) = line.find_byte(b'=') else {
                continue;
            };
            let key = line[..at].to_str().unwrap_or_default();
            fields.insert(key, shell_unquote(&line[at + 1..])?);
        }
        let mut take = |key: &str| {
            fields
                .remove(key)
                .ok_or_else(|| error::corrupt("invalid rebase-merge/author-script", format!("{key} is missing")))
        };
        Ok(Self {
            name: take("GIT_AUTHOR_NAME")?,
            email: take("GIT_AUTHOR_EMAIL")?,
            date: take("GIT_AUTHOR_DATE")?,
        })
    }
}

/// Quotes a value for a POSIX shell like Git: in single quotes, with `'`
/// and `!` written outside the quotes.
fn shell_quote(value: &[u8]) -> Vec<u8> {
    let mut out = vec![b'\''];
    for byte in value {
        match byte {
            b'\'' => out.extend_from_slice(b"'\\''"),
            b'!' => out.extend_from_slice(b"'\\!'"),
            _ => out.push(*byte),
        }
    }
    out.push(b'\'');
    out
}

/// Reverses [`shell_quote`].
fn shell_unquote(value: &[u8]) -> Result<BString> {
    let mut out = BString::default();
    let mut quoted = false;
    let mut bytes = value.iter().copied().peekable();
    while let Some(byte) = bytes.next() {
        match (byte, quoted) {
            (b'\'', _) => quoted = !quoted,
            (b'\\', false) => {
                if let Some(next) = bytes.next() {
                    out.push(next);
                }
            }
            (byte, true) => out.push(byte),
            (byte, false) if byte.is_ascii_whitespace() => {}
            (_, false) => {
                return Err(error::corrupt(
                    "invalid rebase-merge/author-script",
                    "unexpected character outside quotes",
                ));
            }
        }
    }
    Ok(out)
}

/// One todo line: `pick <full id> # <subject>`.
pub(super) fn todo_line(item: &TodoItem) -> String {
    format!("pick {} # {}\n", item.id.to_hex(), item.subject)
}

/// Content of `git-rebase-todo` for the given commands.
pub(super) fn todo_text(items: &[TodoItem]) -> Vec<u8> {
    items.iter().map(todo_line).collect::<String>().into_bytes()
}

/// Content of `git-rebase-todo.backup`: the initial list followed by the
/// summary line and the help text.
pub(super) fn todo_backup(items: &[TodoItem], range: &str, onto: &str) -> Vec<u8> {
    let count = items.len();
    let plural = if count == 1 { "" } else { "s" };
    let mut text: String = items.iter().map(todo_line).collect();
    text.push_str(&format!("\n# Rebase {range} onto {onto} ({count} command{plural})\n"));
    text.push_str(TODO_HELP);
    text.into_bytes()
}

/// A number followed by a line break, as in `msgnum` and `end`.
pub(super) fn number(value: usize) -> Vec<u8> {
    format!("{value}\n").into_bytes()
}

/// An object name followed by a line break.
pub(super) fn object_line(id: ObjectId) -> Vec<u8> {
    format!("{}\n", id.to_hex()).into_bytes()
}

/// Reads the state directory; `None` when no rebase is in progress.
///
/// `resolve` turns the possibly abbreviated object names of todo lines into
/// full names.
pub(super) fn read(
    dir: &Path,
    resolve: &dyn Fn(&str) -> Result<(ObjectId, String)>,
) -> Result<Option<RebaseState>> {
    if !dir.is_dir() {
        return Ok(None);
    }
    let files = read_files(dir)?;
    let text = |name: &str| -> Option<String> {
        files
            .get(name)
            .map(|data| String::from_utf8_lossy(data).trim().to_owned())
    };
    let head_name = text("head-name").ok_or_else(|| missing("head-name"))?;
    let onto = parse_object(&text("onto").ok_or_else(|| missing("onto"))?)?;
    let orig_head = parse_object(&text("orig-head").ok_or_else(|| missing("orig-head"))?)?;
    let todo = parse_todo(files.get("git-rebase-todo").map(Vec::as_slice).unwrap_or_default(), resolve)?;
    let done = BString::from(files.get("done").cloned().unwrap_or_default());
    let done_count = done.lines().filter(|line| !line.trim().is_empty()).count();
    let end = text("end").and_then(|value| value.parse().ok()).unwrap_or(done_count + todo.len());
    let author = files
        .get("author-script")
        .map(|data| AuthorScript::parse(data))
        .transpose()?;
    let stopped = text("stopped-sha")
        .filter(|value| !value.is_empty())
        .map(|value| parse_object(&value))
        .transpose()?;
    Ok(Some(RebaseState {
        head_name,
        onto,
        orig_head,
        done,
        done_count,
        todo,
        end,
        author,
        message: files.get("message").cloned().map(BString::from),
        stopped,
        rewritten: BString::from(files.get("rewritten-list").cloned().unwrap_or_default()),
        files,
    }))
}

fn missing(name: &str) -> nurt_contracts::error::Error {
    error::corrupt(
        &format!("invalid state directory {DIR}"),
        format!("the file {name} is missing"),
    )
}

fn parse_object(text: &str) -> Result<ObjectId> {
    ObjectId::from_hex(text.as_bytes())
        .map_err(|cause| error::corrupt(&format!("invalid object name `{text}` in {DIR}"), cause))
}

/// Every regular file of the state directory with its content.
fn read_files(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    let entries = std::fs::read_dir(dir).map_err(|cause| error::io(&format!("cannot read {}", dir.display()), cause))?;
    for entry in entries {
        let entry = entry.map_err(|cause| error::io(&format!("cannot read {}", dir.display()), cause))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let data = std::fs::read(&path).map_err(|cause| error::io(&format!("cannot read {}", path.display()), cause))?;
        files.insert(entry.file_name().to_string_lossy().into_owned(), data);
    }
    Ok(files)
}

/// Parses todo lines; only `pick` and `drop` commands are accepted.
fn parse_todo(data: &[u8], resolve: &dyn Fn(&str) -> Result<(ObjectId, String)>) -> Result<Vec<TodoItem>> {
    let mut items = Vec::new();
    for line in data.lines() {
        let line = line.to_str_lossy();
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut words = line.splitn(2, char::is_whitespace);
        let command = words.next().unwrap_or_default();
        let rest = words.next().unwrap_or_default().trim();
        match command {
            "pick" | "p" => {
                let name = rest.split_whitespace().next().unwrap_or_default();
                let (id, subject) = resolve(name)?;
                items.push(TodoItem { id, subject });
            }
            "drop" | "d" | "noop" => {}
            other => {
                return Err(error::unsupported(
                    "interactive_command",
                    format!("the todo list contains the command `{other}`, which only an interactive rebase supports"),
                ));
            }
        }
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn author_script_quotes_like_git() {
        let author = AuthorScript {
            name: "O'Brien!".into(),
            email: "o@e".into(),
            date: "@1700000000 +0100".into(),
        };
        let text = author.render();
        assert_eq!(
            text,
            b"GIT_AUTHOR_NAME='O'\\''Brien'\\!''\nGIT_AUTHOR_EMAIL='o@e'\nGIT_AUTHOR_DATE='@1700000000 +0100'\n".to_vec()
        );
        assert_eq!(AuthorScript::parse(&text).ok(), Some(author));
    }

    #[test]
    fn author_header_round_trips() {
        let author = AuthorScript::from_header(BStr::new("A U <a@e> 1700000000 +0100")).ok();
        assert_eq!(author.as_ref().map(|author| author.header()), Some(BString::from("A U <a@e> 1700000000 +0100")));
        assert_eq!(author.map(|author| author.date), Some(BString::from("@1700000000 +0100")));
    }

    #[test]
    fn backup_has_summary_and_help() {
        let item = TodoItem {
            id: ObjectId::from_bytes_or_panic(&[1; 20]),
            subject: "t1".into(),
        };
        let text = String::from_utf8(todo_backup(&[item], "aaaaaaa..bbbbbbb", "aaaaaaa")).unwrap_or_default();
        assert!(text.starts_with("pick 0101010101010101010101010101010101010101 # t1\n\n# Rebase aaaaaaa..bbbbbbb onto aaaaaaa (1 command)\n#\n# Commands:\n"));
        assert!(text.ends_with("# However, if you remove everything, the rebase will be aborted.\n#\n"));
    }

    #[test]
    fn todo_parser_rejects_interactive_commands() {
        let resolve = |_: &str| -> Result<(ObjectId, String)> { Ok((ObjectId::null(gix_hash::Kind::Sha1), String::new())) };
        assert!(parse_todo(b"pick 0101 # a\n# note\n\ndrop 0202 # b\n", &resolve).is_ok_and(|items| items.len() == 1));
        assert!(parse_todo(b"edit 0101 # a\n", &resolve).is_err());
    }
}
