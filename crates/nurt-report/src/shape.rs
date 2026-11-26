// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reduction of a report to the level of detail that a call asked for.

use nurt_contracts::envelope::{DiffMode, Verbosity};
use nurt_contracts::report::{RecordsPage, Report};
use nurt_contracts::value::{Fields, Record, Value};

use crate::options::RenderOptions;

/// Name of the record field that marks a record with shortened text fields.
const TRUNCATED_FIELD: &str = "truncated";

/// Amounts that [`shape`] left out of a report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Omitted {
    /// Changes left out of [`Report::changes`].
    pub changes: usize,
    /// Hunks left out of [`Report::diff`].
    pub hunks: usize,
    /// Records left out of the rows of [`Report::records`].
    pub rows: usize,
}

/// A report reduced by [`shape`] with the amounts that were left out.
#[derive(Debug, Clone, PartialEq)]
pub struct Shaped {
    /// The reduced report; it satisfies [`Report::validate`] whenever the
    /// original does.
    pub report: Report,
    /// Amounts left out of the report.
    pub omitted: Omitted,
}

/// Reduces a report to the level of detail that the options ask for.
///
/// The verbosity, the limits and the field selection come from the options.
///
/// Verbosity `quiet` removes the changes, the differences, the bulk summary
/// and the timing. Verbosity `normal` keeps the changes up to `limit`, the
/// differences up to `diff_max_hunks` hunks and `diff_max_lines` lines, and
/// removes the timing. Verbosity `detailed` keeps the changes up to `limit`,
/// all differences and the timing.
///
/// The diff modes `none` and `stat` remove the differences. The numbers of
/// added and removed lines stay in the changes. Text fields of records longer
/// than `max_chars` are shortened and the record gets `truncated: true`. The
/// option `fields` keeps only the named fields of the rows of a reading
/// program.
///
/// # Examples
///
/// ```
/// use nurt_contracts::envelope::Verbosity;
/// use nurt_contracts::record::{Change, ChangeAction};
/// use nurt_contracts::report::{Report, ReportStatus};
/// use nurt_report::{RenderOptions, shape};
///
/// let mut report = Report::new(ReportStatus::Ok, "remove");
/// report.changes.push(Change::new("/srv/app/a.tmp", ChangeAction::Delete));
///
/// let mut options = RenderOptions::default();
/// options.verbosity = Verbosity::Quiet;
/// let shaped = shape(&report, &options);
/// assert!(shaped.report.changes.is_empty());
/// assert_eq!(shaped.omitted.changes, 1);
/// ```
#[must_use]
pub fn shape(report: &Report, options: &RenderOptions) -> Shaped {
    let mut report = report.clone();
    let mut omitted = Omitted::default();
    shorten_texts(&mut report, options.max_chars);
    select_fields(&mut report, options.fields.as_deref());
    limit_rows(&mut report, options.limit, &mut omitted);
    match options.verbosity {
        Verbosity::Quiet => {
            omitted.changes = report.changes.len();
            omitted.hunks = report.diff.len();
            report.changes.clear();
            report.diff.clear();
            report.bulk = None;
            report.timing = None;
        }
        Verbosity::Detailed => limit_changes(&mut report, options.limit, &mut omitted),
        _ => {
            limit_changes(&mut report, options.limit, &mut omitted);
            limit_diff(&mut report, options, &mut omitted);
            report.timing = None;
        }
    }
    if options.diff != DiffMode::Unified {
        omitted.hunks += report.diff.len();
        report.diff.clear();
    }
    if report
        .targets
        .is_some_and(|targets| targets.listed.is_some())
    {
        let listed = u64::try_from(report.changes.len()).unwrap_or(u64::MAX);
        if let Some(targets) = report.targets.as_mut() {
            targets.listed = Some(listed);
        }
    }
    Shaped { report, omitted }
}

fn limit_changes(report: &mut Report, limit: usize, omitted: &mut Omitted) {
    if report.changes.len() > limit {
        omitted.changes = report.changes.len() - limit;
        report.changes.truncate(limit);
    }
}

fn limit_diff(report: &mut Report, options: &RenderOptions, omitted: &mut Omitted) {
    let mut kept = 0;
    let mut lines = 0;
    for hunk in &report.diff {
        if kept >= options.diff_max_hunks || lines + hunk.lines.len() > options.diff_max_lines {
            break;
        }
        kept += 1;
        lines += hunk.lines.len();
    }
    omitted.hunks = report.diff.len() - kept;
    report.diff.truncate(kept);
}

fn limit_rows(report: &mut Report, limit: usize, omitted: &mut Omitted) {
    if let Some(page) = report.records.as_mut()
        && page.rows.len() > limit
    {
        omitted.rows = page.rows.len() - limit;
        page.rows.truncate(limit);
    }
}

fn select_fields(report: &mut Report, names: Option<&[String]>) {
    let (Some(names), Some(page)) = (names, report.records.as_mut()) else {
        return;
    };
    for row in &mut page.rows {
        let mut kept = Fields::new();
        for name in names {
            if let Some(value) = row.fields.swap_remove(name) {
                kept.insert(name.clone(), value);
            }
        }
        row.fields = kept;
    }
}

fn shorten_texts(report: &mut Report, max_chars: usize) {
    if max_chars == 0 {
        return;
    }
    for record in [
        report.exec.as_mut(),
        report.job.as_mut(),
        report.test.as_mut(),
        report.analysis.as_mut(),
        report.git.as_mut(),
    ]
    .into_iter()
    .flatten()
    {
        shorten_record(record, max_chars);
    }
    if let Some(RecordsPage { header, rows, .. }) = report.records.as_mut() {
        for record in header.iter_mut().chain(rows.iter_mut()) {
            shorten_record(record, max_chars);
        }
    }
}

/// Shortens the texts of a record; returns whether any text was shortened.
fn shorten_record(record: &mut Record, max_chars: usize) -> bool {
    let mut shortened = false;
    for value in record.fields.values_mut() {
        shortened |= shorten_value(value, max_chars);
    }
    if shortened {
        record.set(TRUNCATED_FIELD, Value::Bool(true));
    }
    shortened
}

fn shorten_value(value: &mut Value, max_chars: usize) -> bool {
    match value {
        Value::Text(text) => match text.char_indices().nth(max_chars) {
            Some((end, _)) => {
                text.truncate(end);
                true
            }
            None => false,
        },
        Value::List(items) => items
            .iter_mut()
            .fold(false, |acc, item| shorten_value(item, max_chars) | acc),
        Value::Map(fields) => fields
            .values_mut()
            .fold(false, |acc, item| shorten_value(item, max_chars) | acc),
        Value::Record(record) => shorten_record(record, max_chars),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::record::{Change, ChangeAction, Hunk};
    use nurt_contracts::report::{ReportStatus, Targets, Timing};
    use nurt_contracts::value::RecordKind;

    use super::*;

    fn report_with_changes(count: usize) -> Report {
        let mut report = Report::new(ReportStatus::Ok, "replace");
        report.targets = Some(Targets {
            count: count as u64,
            listed: Some(count as u64),
            ..Targets::default()
        });
        for index in 0..count {
            report.changes.push(Change::new(
                format!("/srv/app/f{index}.txt").as_str(),
                ChangeAction::Modify,
            ));
        }
        report.timing = Some(Timing {
            parse_ms: Some(1),
            ..Timing::default()
        });
        report
    }

    fn hunk(lines: usize) -> Hunk {
        Hunk {
            path: "/srv/app/a.go".into(),
            lines: vec!["+x".to_owned(); lines],
            ..Hunk::default()
        }
    }

    fn options(verbosity: Verbosity) -> RenderOptions {
        RenderOptions {
            verbosity,
            ..RenderOptions::default()
        }
    }

    #[test]
    fn quiet_removes_changes_diff_bulk_and_timing() {
        let mut report = report_with_changes(3);
        report.diff.push(hunk(2));
        let shaped = shape(&report, &options(Verbosity::Quiet));
        assert!(shaped.report.changes.is_empty());
        assert!(shaped.report.diff.is_empty());
        assert!(shaped.report.timing.is_none());
        assert_eq!(shaped.omitted.changes, 3);
        assert_eq!(shaped.omitted.hunks, 1);
        assert_eq!(shaped.report.targets.and_then(|t| t.listed), Some(0));
        assert!(shaped.report.validate().is_ok());
    }

    #[test]
    fn normal_limits_changes_and_drops_timing() {
        let report = report_with_changes(5);
        let mut options = options(Verbosity::Normal);
        options.limit = 2;
        let shaped = shape(&report, &options);
        assert_eq!(shaped.report.changes.len(), 2);
        assert_eq!(shaped.omitted.changes, 3);
        assert!(shaped.report.timing.is_none());
        assert_eq!(shaped.report.targets.and_then(|t| t.listed), Some(2));
        assert!(shaped.report.validate().is_ok());
    }

    #[test]
    fn detailed_keeps_timing_and_all_hunks() {
        let mut report = report_with_changes(1);
        report.diff = vec![hunk(1); 30];
        let shaped = shape(&report, &options(Verbosity::Detailed));
        assert!(shaped.report.timing.is_some());
        assert_eq!(shaped.report.diff.len(), 30);
        assert_eq!(shaped.omitted.hunks, 0);
    }

    #[test]
    fn normal_limits_hunks_by_count() {
        let mut report = report_with_changes(1);
        report.diff = vec![hunk(1); 30];
        let shaped = shape(&report, &options(Verbosity::Normal));
        assert_eq!(shaped.report.diff.len(), 20);
        assert_eq!(shaped.omitted.hunks, 10);
    }

    #[test]
    fn normal_limits_hunks_by_lines() {
        let mut report = report_with_changes(1);
        report.diff = vec![hunk(60), hunk(60), hunk(60), hunk(60)];
        let shaped = shape(&report, &options(Verbosity::Normal));
        assert_eq!(shaped.report.diff.len(), 3);
        assert_eq!(shaped.omitted.hunks, 1);
    }

    #[test]
    fn diff_modes_other_than_unified_remove_differences() {
        let mut report = report_with_changes(1);
        report.diff = vec![hunk(1)];
        for mode in [DiffMode::None, DiffMode::Stat] {
            let options = RenderOptions {
                diff: mode,
                ..RenderOptions::default()
            };
            assert!(shape(&report, &options).report.diff.is_empty());
        }
    }

    fn entry(path: &str, name: &str, note: &str) -> Record {
        Record::new(RecordKind::Entry)
            .with("path", Value::Path(path.to_owned()))
            .with("name", Value::Text(name.to_owned()))
            .with("note", Value::Text(note.to_owned()))
    }

    fn read_report(rows: Vec<Record>) -> Report {
        let mut report = Report::new(ReportStatus::Ok, "find");
        report.records = Some(RecordsPage {
            record_type: RecordKind::Entry,
            matched: Some(rows.len() as u64),
            header: None,
            rows,
        });
        report
    }

    #[test]
    fn limits_rows_of_a_read() {
        let report = read_report(vec![
            entry("/a", "a", ""),
            entry("/b", "b", ""),
            entry("/c", "c", ""),
        ]);
        let options = RenderOptions {
            limit: 2,
            ..RenderOptions::default()
        };
        let shaped = shape(&report, &options);
        assert_eq!(shaped.report.records.map(|page| page.rows.len()), Some(2));
        assert_eq!(shaped.omitted.rows, 1);
    }

    #[test]
    fn keeps_only_selected_fields_in_the_given_order() {
        let report = read_report(vec![entry("/a", "a", "x")]);
        let options = RenderOptions {
            fields: Some(vec![
                "name".to_owned(),
                "path".to_owned(),
                "missing".to_owned(),
            ]),
            ..RenderOptions::default()
        };
        let shaped = shape(&report, &options);
        let row = &shaped.report.records.unwrap().rows[0];
        assert_eq!(row.field_names().collect::<Vec<_>>(), ["name", "path"]);
    }

    #[test]
    fn shortens_long_texts_and_marks_the_record() {
        let report = read_report(vec![entry("/a", "a", "0123456789"), entry("/b", "b", "01")]);
        let options = RenderOptions {
            max_chars: 4,
            ..RenderOptions::default()
        };
        let rows = shape(&report, &options).report.records.unwrap().rows;
        assert_eq!(rows[0].get("note"), Some(&Value::Text("0123".to_owned())));
        assert_eq!(rows[0].get("truncated"), Some(&Value::Bool(true)));
        assert_eq!(rows[1].get("note"), Some(&Value::Text("01".to_owned())));
        assert_eq!(rows[1].get("truncated"), None);
    }

    #[test]
    fn shortens_by_characters_not_bytes() {
        let report = read_report(vec![entry("/a", "a\u{e9}b\u{e9}", "")]);
        let options = RenderOptions {
            max_chars: 3,
            ..RenderOptions::default()
        };
        let rows = shape(&report, &options).report.records.unwrap().rows;
        assert_eq!(
            rows[0].get("name"),
            Some(&Value::Text("a\u{e9}b".to_owned()))
        );
    }

    #[test]
    fn shortens_texts_nested_in_lists_maps_and_records() {
        let mut inner = Record::new(RecordKind::Entry);
        inner.set("note", Value::Text("abcdef".to_owned()));
        let mut map = Fields::new();
        map.insert("k".to_owned(), Value::Text("abcdef".to_owned()));
        let mut exec = Record::new(RecordKind::Exec);
        exec.set("list", Value::List(vec![Value::Text("abcdef".to_owned())]));
        exec.set("map", Value::Map(map));
        exec.set("inner", Value::Record(inner));
        let mut report = Report::new(ReportStatus::Ok, "run");
        report.exec = Some(exec);
        let options = RenderOptions {
            max_chars: 2,
            ..RenderOptions::default()
        };
        let exec = shape(&report, &options).report.exec.unwrap();
        assert_eq!(
            exec.get("list"),
            Some(&Value::List(vec![Value::Text("ab".to_owned())]))
        );
        assert_eq!(exec.get("truncated"), Some(&Value::Bool(true)));
        let Some(Value::Record(inner)) = exec.get("inner") else {
            panic!("inner record missing");
        };
        assert_eq!(inner.get("truncated"), Some(&Value::Bool(true)));
    }

    #[test]
    fn marks_the_holder_of_a_nested_record_that_was_already_marked() {
        let mut inner = Record::new(RecordKind::Entry);
        inner.set("note", Value::Text("abcdef".to_owned()));
        inner.set("truncated", Value::Bool(true));
        let mut exec = Record::new(RecordKind::Exec);
        exec.set("inner", Value::Record(inner));
        let mut report = Report::new(ReportStatus::Ok, "run");
        report.exec = Some(exec);
        let options = RenderOptions {
            max_chars: 2,
            ..RenderOptions::default()
        };
        let exec = shape(&report, &options).report.exec.unwrap();
        assert_eq!(exec.get("truncated"), Some(&Value::Bool(true)));
    }

    #[test]
    fn keeps_texts_that_fit_and_unlimited_texts_unchanged() {
        let report = read_report(vec![entry("/a", "abcd", "xy")]);
        for max_chars in [0, 4] {
            let options = RenderOptions {
                max_chars,
                ..RenderOptions::default()
            };
            let rows = shape(&report, &options).report.records.unwrap().rows;
            assert_eq!(rows[0].get("name"), Some(&Value::Text("abcd".to_owned())));
            assert_eq!(rows[0].get("truncated"), None);
        }
    }
}
