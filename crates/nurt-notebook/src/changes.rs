// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Lists of cell operations applied as one change (`notebook.ops`).

use nurt_contracts::record::CellType;
use nurt_contracts::value::Range;
use serde_json::{Map, Value};

use crate::address::{CellRef, Position, Selection};
use crate::error::NotebookError;
use crate::find::Pattern;
use crate::notebook::Notebook;
use crate::ops::{Inserted, Replace, Replaced};

/// Fields allowed in a change, by action.
const INSERT_FIELDS: &[&str] = &["action", "index", "after_id", "cell_type", "source"];
const REPLACE_FIELDS: &[&str] = &[
    "action",
    "index",
    "id",
    "source",
    "cell_type",
    "find",
    "with",
    "whole_word",
    "reset_outputs",
];
const MOVE_FIELDS: &[&str] = &["action", "index", "id", "to"];
const SELECTION_FIELDS: &[&str] = &["action", "index", "id"];

/// One operation of a list of changes.
///
/// In the JSON form a change is an object with the member `action` (`insert`,
/// `replace`, `move`, `copy`, `remove`, `clear_outputs`) and the parameters
/// of the corresponding command: `index`, `id`, `after_id`, `to`,
/// `cell_type`, `source`, `find`, `with`, `whole_word`, `reset_outputs`. In
/// `move` and `copy`, `index` or `id` names the cell to move or copy.
#[derive(Debug)]
#[non_exhaustive]
pub enum Change {
    /// New cell, as [`Notebook::insert`].
    Insert {
        /// Position of the new cell.
        position: Position,
        /// Type of the new cell.
        cell_type: CellType,
        /// Source of the new cell.
        source: String,
    },
    /// Change of a cell or of text within cells, as [`Notebook::replace`].
    Replace {
        /// Cell to change.
        target: Option<CellRef>,
        /// New source of the target cell.
        source: Option<String>,
        /// New type of the target cell.
        cell_type: Option<CellType>,
        /// Text to replace within sources.
        find: Option<Box<dyn Pattern>>,
        /// Replacement for each match of `find`.
        with: Option<String>,
        /// Clear the outputs of changed code cells.
        reset_outputs: bool,
    },
    /// Move of a cell, as [`Notebook::move_cell`].
    Move {
        /// Cell to move.
        from: CellRef,
        /// Final index of the cell.
        to: i64,
    },
    /// Copy of a cell, as [`Notebook::copy_cell`].
    Copy {
        /// Cell to copy.
        from: CellRef,
        /// Index of the copy; directly after the original when missing.
        to: Option<i64>,
    },
    /// Removal of cells, as [`Notebook::remove`].
    Remove(Selection),
    /// Clearing of outputs, as [`Notebook::clear_outputs`].
    ClearOutputs(Selection),
}

/// Result of one operation of [`Notebook::apply`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChangeOutcome {
    /// A cell was inserted.
    Inserted(Inserted),
    /// Cells were changed.
    Replaced(Replaced),
    /// A cell was moved from this index.
    Moved(usize),
    /// A cell was copied.
    Copied(Inserted),
    /// Cells with these former indexes were removed.
    Removed(Vec<usize>),
    /// Outputs of cells with these indexes were cleared.
    Cleared(Vec<usize>),
}

/// Function that turns the JSON value of `find` and the flag `whole_word`
/// into a pattern.
pub type PatternFactory<'a> =
    dyn FnMut(&Value, bool) -> Result<Box<dyn Pattern>, NotebookError> + 'a;

impl Change {
    /// Reads a change from its JSON form.
    ///
    /// `patterns` builds the pattern of `find`; [`literal_pattern`] accepts
    /// text and suits callers without regular expressions.
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::InvalidChange`] for an unknown action, a
    /// field that does not apply to the action, a missing required field or
    /// a field of the wrong type, and any error of `patterns`.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_notebook::{Change, literal_pattern};
    ///
    /// let json = serde_json::json!({"action": "move", "id": "a1", "to": 0});
    /// let change = Change::from_json(json.as_object().unwrap(), &mut literal_pattern)?;
    /// assert!(matches!(change, Change::Move { to: 0, .. }));
    /// # Ok::<(), nurt_notebook::NotebookError>(())
    /// ```
    pub fn from_json(
        map: &Map<String, Value>,
        patterns: &mut PatternFactory<'_>,
    ) -> Result<Self, NotebookError> {
        let fields = Fields(map);
        let action = fields.text("action")?.ok_or_else(|| {
            NotebookError::invalid(format!("change needs action, one of {ACTIONS}"))
        })?;
        match action {
            "insert" => {
                fields.only(action, INSERT_FIELDS)?;
                let position = match (fields.int("index")?, fields.text("after_id")?) {
                    (Some(_), Some(_)) => {
                        return Err(NotebookError::invalid(
                            "insert takes index or after_id, not both",
                        ));
                    }
                    (Some(index), None) => Position::Index(index),
                    (None, Some(id)) => Position::AfterId(id.to_owned()),
                    (None, None) => Position::End,
                };
                Ok(Self::Insert {
                    position,
                    cell_type: fields
                        .cell_type()?
                        .ok_or_else(|| missing(action, "cell_type"))?,
                    source: fields
                        .text("source")?
                        .ok_or_else(|| missing(action, "source"))?
                        .to_owned(),
                })
            }
            "replace" => {
                fields.only(action, REPLACE_FIELDS)?;
                let whole_word = fields.flag("whole_word")?;
                let find = map
                    .get("find")
                    .filter(|value| !value.is_null())
                    .map(|value| patterns(value, whole_word))
                    .transpose()?;
                Ok(Self::Replace {
                    target: CellRef::from_parts(fields.int("index")?, fields.owned("id")?),
                    source: fields.owned("source")?,
                    cell_type: fields.cell_type()?,
                    find,
                    with: fields.owned("with")?,
                    reset_outputs: fields.flag("reset_outputs")?,
                })
            }
            "move" | "copy" => {
                fields.only(action, MOVE_FIELDS)?;
                let from = CellRef::from_parts(fields.int("index")?, fields.owned("id")?)
                    .ok_or_else(|| missing(action, "index or id"))?;
                let to = fields.int("to")?;
                if action == "move" {
                    Ok(Self::Move {
                        from,
                        to: to.ok_or_else(|| missing(action, "to"))?,
                    })
                } else {
                    Ok(Self::Copy { from, to })
                }
            }
            "remove" | "clear_outputs" => {
                fields.only(action, SELECTION_FIELDS)?;
                let selection = fields.selection()?;
                if action == "remove" {
                    Ok(Self::Remove(
                        selection.ok_or_else(|| missing(action, "index or id"))?,
                    ))
                } else {
                    Ok(Self::ClearOutputs(selection.unwrap_or(Selection::All)))
                }
            }
            other => Err(NotebookError::invalid(format!(
                "unknown action `{other}`; use one of {ACTIONS}"
            ))),
        }
    }
}

/// Builds a literal pattern from a text value of `find`.
///
/// # Errors
///
/// Returns [`NotebookError::InvalidChange`] when the value is not text.
pub fn literal_pattern(value: &Value, whole_word: bool) -> Result<Box<dyn Pattern>, NotebookError> {
    value
        .as_str()
        .map(|text| Box::new(crate::find::Literal::new(text, whole_word)) as Box<dyn Pattern>)
        .ok_or_else(|| NotebookError::invalid("field find must be text"))
}

const ACTIONS: &str = "insert, replace, move, copy, remove, clear_outputs";

fn missing(action: &str, field: &str) -> NotebookError {
    NotebookError::invalid(format!("action {action} needs {field}"))
}

/// Typed access to the fields of a change.
struct Fields<'a>(&'a Map<String, Value>);

impl<'a> Fields<'a> {
    fn only(&self, action: &str, allowed: &[&str]) -> Result<(), NotebookError> {
        match self.0.keys().find(|key| !allowed.contains(&key.as_str())) {
            Some(key) => Err(NotebookError::invalid(format!(
                "field {key} does not apply to action {action}; use {}",
                allowed.join(", ")
            ))),
            None => Ok(()),
        }
    }

    fn text(&self, key: &str) -> Result<Option<&'a str>, NotebookError> {
        match self.0.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(text)),
            Some(_) => Err(wrong_type(key, "text")),
        }
    }

    fn owned(&self, key: &str) -> Result<Option<String>, NotebookError> {
        Ok(self.text(key)?.map(str::to_owned))
    }

    fn int(&self, key: &str) -> Result<Option<i64>, NotebookError> {
        match self.0.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value
                .as_i64()
                .map(Some)
                .ok_or_else(|| wrong_type(key, "an integer")),
        }
    }

    fn flag(&self, key: &str) -> Result<bool, NotebookError> {
        match self.0.get(key) {
            None | Some(Value::Null) => Ok(false),
            Some(Value::Bool(flag)) => Ok(*flag),
            Some(_) => Err(wrong_type(key, "true or false")),
        }
    }

    fn cell_type(&self) -> Result<Option<CellType>, NotebookError> {
        self.text("cell_type")?
            .map(|name| {
                name.parse().map_err(|_| {
                    NotebookError::invalid(format!(
                        "unknown cell_type `{name}`; use code, markdown or raw"
                    ))
                })
            })
            .transpose()
    }

    /// Selection from `index` (integer or range text) or `id` (text or list
    /// of text).
    fn selection(&self) -> Result<Option<Selection>, NotebookError> {
        let index = match self.0.get("index") {
            None | Some(Value::Null) => None,
            Some(Value::Number(number)) => {
                let index = number
                    .as_i64()
                    .ok_or_else(|| wrong_type("index", "an integer or a range"))?;
                Some(Range::single(index))
            }
            Some(Value::String(text)) => Some(parse_range(text)?),
            Some(_) => return Err(wrong_type("index", "an integer or a range")),
        };
        let ids = match self.0.get("id") {
            None | Some(Value::Null) => None,
            Some(Value::String(id)) => Some(vec![id.clone()]),
            Some(Value::Array(items)) => Some(
                items
                    .iter()
                    .map(|item| item.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| wrong_type("id", "text or a list of text"))?,
            ),
            Some(_) => return Err(wrong_type("id", "text or a list of text")),
        };
        match (index, ids) {
            (Some(_), Some(_)) => Err(NotebookError::invalid("give index or id, not both")),
            (Some(range), None) => Ok(Some(Selection::Range(range))),
            (None, Some(ids)) => Ok(Some(Selection::Ids(ids))),
            (None, None) => Ok(None),
        }
    }
}

fn wrong_type(key: &str, expected: &str) -> NotebookError {
    NotebookError::invalid(format!("field {key} must be {expected}"))
}

/// Parses a range written as `5`, `1..5`, `3..`, `..4`, `..` or `all`.
fn parse_range(text: &str) -> Result<Range, NotebookError> {
    let invalid = || {
        NotebookError::invalid(format!(
            "invalid range `{text}`; write 5, 1..5, 3.., ..4 or all"
        ))
    };
    let bound = |part: &str| -> Result<Option<i64>, NotebookError> {
        if part.is_empty() {
            Ok(None)
        } else {
            part.parse().map(Some).map_err(|_| invalid())
        }
    };
    let trimmed = text.trim();
    if trimmed == "all" {
        return Ok(Range::full());
    }
    match trimmed.split_once("..") {
        Some((start, end)) => Range::new(bound(start)?, bound(end)?).map_err(|_| invalid()),
        None => bound(trimmed)?.map(Range::single).ok_or_else(invalid),
    }
}

impl Notebook {
    /// Applies a list of changes as one change (`notebook.ops`).
    ///
    /// Each operation sees the result of the previous ones. When an
    /// operation fails, the notebook stays exactly as before the call.
    ///
    /// # Errors
    ///
    /// Returns [`NotebookError::InvalidChange`] for an empty list and
    /// [`NotebookError::Change`] with the position and error of the first
    /// failed operation.
    pub fn apply(&mut self, changes: &[Change]) -> Result<Vec<ChangeOutcome>, NotebookError> {
        if changes.is_empty() {
            return Err(NotebookError::invalid(
                "changes is empty; give one or more operations",
            ));
        }
        let mut work = self.clone();
        let mut outcomes = Vec::with_capacity(changes.len());
        for (position, change) in changes.iter().enumerate() {
            let outcome = work
                .apply_one(change)
                .map_err(|error| NotebookError::Change {
                    position,
                    source: Box::new(error),
                })?;
            outcomes.push(outcome);
        }
        *self = work;
        Ok(outcomes)
    }

    fn apply_one(&mut self, change: &Change) -> Result<ChangeOutcome, NotebookError> {
        match change {
            Change::Insert {
                position,
                cell_type,
                source,
            } => self
                .insert(position, *cell_type, source)
                .map(ChangeOutcome::Inserted),
            Change::Replace {
                target,
                source,
                cell_type,
                find,
                with,
                reset_outputs,
            } => self
                .replace(&Replace {
                    target: target.clone(),
                    source: source.clone(),
                    cell_type: *cell_type,
                    find: find.as_deref(),
                    with: with.clone(),
                    reset_outputs: *reset_outputs,
                })
                .map(ChangeOutcome::Replaced),
            Change::Move { from, to } => self.move_cell(from, *to).map(ChangeOutcome::Moved),
            Change::Copy { from, to } => self.copy_cell(from, *to).map(ChangeOutcome::Copied),
            Change::Remove(selection) => self.remove(selection).map(ChangeOutcome::Removed),
            Change::ClearOutputs(selection) => {
                self.clear_outputs(selection).map(ChangeOutcome::Cleared)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(value: Value) -> Result<Change, NotebookError> {
        Change::from_json(value.as_object().unwrap(), &mut literal_pattern)
    }

    #[test]
    fn reads_every_action() {
        assert!(matches!(
            parse(json!({"action": "insert", "after_id": "a", "cell_type": "code", "source": "x"})),
            Ok(Change::Insert {
                position: Position::AfterId(_),
                ..
            })
        ));
        assert!(matches!(
            parse(
                json!({"action": "replace", "id": "a", "find": "x", "with": "y", "whole_word": true})
            ),
            Ok(Change::Replace { find: Some(_), .. })
        ));
        assert!(matches!(
            parse(json!({"action": "copy", "index": 1})),
            Ok(Change::Copy { to: None, .. })
        ));
        assert!(matches!(
            parse(json!({"action": "remove", "index": "1..3"})),
            Ok(Change::Remove(Selection::Range(_)))
        ));
        assert!(matches!(
            parse(json!({"action": "remove", "id": ["a", "b"]})),
            Ok(Change::Remove(Selection::Ids(_)))
        ));
        assert!(matches!(
            parse(json!({"action": "clear_outputs"})),
            Ok(Change::ClearOutputs(Selection::All))
        ));
    }

    #[test]
    fn rejects_unknown_actions_and_fields() {
        let error = parse(json!({"action": "delete", "index": 1})).unwrap_err();
        assert!(
            error.to_string().contains("unknown action `delete`"),
            "{error}"
        );
        let error =
            parse(json!({"action": "move", "index": 1, "to": 2, "source": "x"})).unwrap_err();
        assert!(
            error.to_string().contains("field source does not apply"),
            "{error}"
        );
    }

    #[test]
    fn rejects_missing_and_mistyped_fields() {
        assert!(parse(json!({"action": "insert", "source": "x"})).is_err());
        assert!(parse(json!({"action": "move", "index": 1})).is_err());
        assert!(parse(json!({"action": "remove"})).is_err());
        assert!(parse(json!({"action": "move", "index": "1", "to": 2})).is_err());
        assert!(parse(json!({"index": 1})).is_err());
        assert!(parse(json!({"action": "replace", "index": 1, "find": 5, "with": ""})).is_err());
    }

    #[test]
    fn null_find_is_absent_like_every_other_null_field() {
        let change = parse(json!({
            "action": "replace", "index": 1, "source": "x", "find": null, "with": null
        }))
        .unwrap();
        assert!(matches!(
            change,
            Change::Replace {
                find: None,
                with: None,
                source: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn rejects_conflicting_and_mistyped_selectors() {
        assert!(parse(json!({"action": "insert", "index": 1, "after_id": "a", "cell_type": "code", "source": "x"})).is_err());
        assert!(parse(json!({"action": "remove", "index": 1, "id": "a"})).is_err());
        assert!(parse(json!({"action": "remove", "index": 1.5})).is_err());
        assert!(parse(json!({"action": "remove", "id": [1]})).is_err());
        assert!(parse(json!({"action": "insert", "cell_type": "heading", "source": "x"})).is_err());
        assert!(parse(json!({"action": "replace", "index": 1, "reset_outputs": "yes"})).is_err());
    }

    #[test]
    fn parses_ranges() {
        assert_eq!(parse_range("2").unwrap(), Range::single(2));
        assert_eq!(
            parse_range("1..4").unwrap(),
            Range::new(Some(1), Some(4)).unwrap()
        );
        assert_eq!(parse_range("3..").unwrap(), Range::from_start(3));
        assert_eq!(parse_range("..").unwrap(), Range::full());
        assert_eq!(parse_range("all").unwrap(), Range::full());
        assert!(parse_range("4..1").is_err());
        assert!(parse_range("x").is_err());
    }
}
