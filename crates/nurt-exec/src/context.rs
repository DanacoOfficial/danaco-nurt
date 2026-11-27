// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-exec::context
// Responsibility:   Context lines of the hunks of an effect.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Context lines of the hunks of an effect.

use nurt_contracts::record::{Change, Hunk};

/// Prefix of a context line of a hunk.
const CONTEXT_PREFIX: char = ' ';

/// Limits the context of every hunk of the changes to `context` lines.
///
/// Stages produce hunks with the context asked for by the request options;
/// the executor enforces the limit once, so that a stage that produced more
/// context than `diff_context` cannot change what the report shows. Only
/// surplus context at the start and the end of a hunk is removed; a hunk
/// that changed loses its fingerprint, which no longer describes its lines.
pub(crate) fn limit_context(changes: &mut [Change], context: u32) {
    let keep = usize::try_from(context).unwrap_or(usize::MAX);
    for hunk in changes
        .iter_mut()
        .filter_map(|change| change.diff.as_mut())
        .flatten()
    {
        limit_hunk(hunk, keep);
    }
}

fn is_context(line: &str) -> bool {
    line.starts_with(CONTEXT_PREFIX)
}

fn limit_hunk(hunk: &mut Hunk, keep: usize) {
    let leading = hunk
        .lines
        .iter()
        .take_while(|line| is_context(line))
        .count();
    if leading == hunk.lines.len() {
        return;
    }
    let trailing = hunk
        .lines
        .iter()
        .rev()
        .take_while(|line| is_context(line))
        .count();
    let drop_front = leading.saturating_sub(keep);
    let drop_back = trailing.saturating_sub(keep);
    if drop_front == 0 && drop_back == 0 {
        return;
    }
    let front = drop_front as u64;
    let back = drop_back as u64;
    let end = hunk.lines.len() - drop_back;
    hunk.lines = hunk.lines.drain(drop_front..end).collect();
    hunk.old_start += front;
    hunk.new_start += front;
    hunk.old_len = hunk.old_len.saturating_sub(front + back);
    hunk.new_len = hunk.new_len.saturating_sub(front + back);
    hunk.fingerprint = None;
    if let Some(id) = hunk.id.as_mut()
        && let Some(cut) = id.find(':')
    {
        id.truncate(cut);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nurt_contracts::record::ChangeAction;
    use nurt_contracts::value::Path;

    fn hunk(lines: &[&str]) -> Hunk {
        let count = lines.len() as u64;
        Hunk {
            path: Path::new("/f"),
            old_start: 1,
            old_len: count,
            new_start: 1,
            new_len: count,
            lines: lines.iter().map(|line| (*line).to_owned()).collect(),
            id: Some("/f#1:abcdef".to_owned()),
            fingerprint: Some("abcdef".to_owned()),
            ..Hunk::default()
        }
    }

    fn change(hunks: Vec<Hunk>) -> Change {
        let mut change = Change::new(Path::new("/f"), ChangeAction::Modify);
        change.diff = Some(hunks);
        change
    }

    fn limited(lines: &[&str], context: u32) -> Hunk {
        let mut changes = [change(vec![hunk(lines)])];
        limit_context(&mut changes, context);
        changes[0].diff.take().unwrap_or_default().remove(0)
    }

    #[test]
    fn surplus_context_is_removed_at_both_ends() {
        let result = limited(&[" a", " b", " c", "-x", "+y", " d", " e", " f"], 1);
        assert_eq!(result.lines, [" c", "-x", "+y", " d"].map(String::from));
        assert_eq!((result.old_start, result.old_len), (3, 4));
        assert_eq!((result.new_start, result.new_len), (3, 4));
        assert_eq!(result.fingerprint, None);
        assert_eq!(result.id.as_deref(), Some("/f#1"));
    }

    #[test]
    fn zero_context_keeps_only_changed_lines() {
        let result = limited(&[" a", "-x", " b"], 0);
        assert_eq!(result.lines, ["-x"].map(String::from));
        assert_eq!((result.old_start, result.old_len), (2, 1));
    }

    #[test]
    fn id_without_a_fingerprint_is_kept() {
        let mut wide = hunk(&[" a", " b", "-x", " c"]);
        wide.id = Some("/f#1".to_owned());
        let mut changes = [change(vec![wide])];
        limit_context(&mut changes, 0);
        let result = changes[0].diff.take().unwrap_or_default().remove(0);
        assert_eq!(result.lines, ["-x"].map(String::from));
        assert_eq!(result.id.as_deref(), Some("/f#1"));
    }

    #[test]
    fn hunk_within_the_limit_is_untouched() {
        let original = hunk(&[" a", "-x", " b"]);
        let result = limited(&[" a", "-x", " b"], 3);
        assert_eq!(result, original);
    }

    #[test]
    fn hunk_of_context_only_is_untouched() {
        let result = limited(&[" a", " b"], 0);
        assert_eq!(result.lines.len(), 2);
    }

    #[test]
    fn changes_without_a_difference_are_skipped() {
        let mut changes = [Change::new(Path::new("/f"), ChangeAction::Create)];
        limit_context(&mut changes, 0);
        assert!(changes[0].diff.is_none());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
