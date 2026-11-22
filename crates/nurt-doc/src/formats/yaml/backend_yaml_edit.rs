// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reading and editing with the `yaml-edit` library.
//!
//! The library was the second candidate of the choice of the YAML
//! implementation. It is compiled for tests only: the tests below measure
//! both implementations on the same corpus and operations (byte identity
//! after reading, scope and meaning of scalar and key changes, refused
//! changes, time and memory on a 10 MiB document), and the functions serve
//! as a differential reference for the model.

use std::str::FromStr;

use nurt_contracts::value::Value;
use yaml_edit::{Mapping, YamlFile};

/// Text written back by the library after reading `text`.
pub(crate) fn round_trip(text: &str) -> Result<String, String> {
    YamlFile::from_str(text)
        .map(|file| file.to_string())
        .map_err(|error| error.to_string())
}

fn mapping_at(file: &YamlFile, path: &[String]) -> Option<Mapping> {
    let mut mapping = file.document()?.as_mapping()?;
    for name in path {
        mapping = mapping.get_mapping(name.as_str())?;
    }
    Some(mapping)
}

/// Text after setting the key `path` of nested mappings to `value` with the
/// editing API of the library; a missing last key is added.
pub(crate) fn set(text: &str, path: &[String], value: &Value) -> Result<String, String> {
    let file = YamlFile::from_str(text).map_err(|error| error.to_string())?;
    let (last, parents) = path.split_last().ok_or("the path is empty")?;
    let mapping = mapping_at(&file, parents).ok_or("the parent is not a mapping")?;
    match value {
        Value::Text(text) => mapping.set(last.as_str(), text.as_str()),
        Value::Int(number) => mapping.set(last.as_str(), *number),
        Value::Number(number) => mapping.set(last.as_str(), *number),
        Value::Bool(flag) => mapping.set(last.as_str(), *flag),
        _ => return Err("the value has no scalar form".to_owned()),
    }
    Ok(file.to_string())
}

/// Text after removing the key `path` of nested mappings.
pub(crate) fn remove(text: &str, path: &[String]) -> Result<String, String> {
    let file = YamlFile::from_str(text).map_err(|error| error.to_string())?;
    let (last, parents) = path.split_last().ok_or("the path is empty")?;
    let mapping = mapping_at(&file, parents).ok_or("the parent is not a mapping")?;
    mapping
        .remove(last.as_str())
        .ok_or("the key does not exist")?;
    Ok(file.to_string())
}

#[path = "../../../tests/u05_support/suite.rs"]
mod suite;

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashSet};
    use std::fs;
    use std::ops::Range;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use nurt_contracts::keypath::{KeyPath, KeySegment};

    use super::super::YamlDoc;
    use super::super::comment::Comments;
    use super::super::model::{NodeId, NodeKind, YamlModel, YamlVersion};
    use super::*;
    use crate::model::{DocHandle, MapSemantics, SetOptions};
    use crate::value::{apply_remove, apply_set, equivalent};

    /// Scalar values changed per file.
    const MAX_LEAVES: usize = 50;

    /// Size of the generated large document.
    const LARGE_BYTES: usize = 10 * 1024 * 1024;

    const NEW_TEXT: &str = "trial value";
    const NEW_KEY: &str = "nurt_trial_key";

    /// Outcome counts of one implementation.
    #[derive(Debug, Default)]
    struct Tally {
        files: usize,
        read: usize,
        identical: usize,
        operations: usize,
        clean: usize,
        out_of_scope: usize,
        wrong_value: usize,
        refused: BTreeMap<String, usize>,
        examples: Vec<String>,
    }

    /// Examples of each kind of deviation printed with the counts.
    const MAX_EXAMPLES: usize = 6;

    impl Tally {
        fn refuse(&mut self, reason: String) {
            *self.refused.entry(reason).or_default() += 1;
        }

        fn note(&mut self, line: &str) {
            let kind = line.split(' ').take(2).collect::<Vec<_>>().join(" ");
            let same = self
                .examples
                .iter()
                .filter(|known| known.starts_with(&kind))
                .count();
            if same < MAX_EXAMPLES {
                self.examples.push(line.to_owned());
            }
        }

        fn example(&mut self, kind: &str, original: &str, new: &str) {
            let same = self
                .examples
                .iter()
                .filter(|line| line.starts_with(kind))
                .count();
            if same < MAX_EXAMPLES {
                let region = changed(original, new);
                let shown = new.len() + region.end - original.len();
                self.examples.push(format!(
                    "{kind}: {:?} -> {:?}",
                    &original[region.clone()],
                    new.get(region.start..shown).unwrap_or_default()
                ));
            }
        }

        fn line(&self, name: &str) -> String {
            let refused: usize = self.refused.values().sum();
            format!(
                "{name}: files {} read {} identical {} operations {} clean {} out_of_scope {} wrong_value {} refused {refused} {:?}",
                self.files,
                self.read,
                self.identical,
                self.operations,
                self.clean,
                self.out_of_scope,
                self.wrong_value,
                self.refused
            )
        }
    }

    /// Change checked against the original text: the bytes it may change and
    /// the value the document must have afterwards.
    struct Expected {
        allowed: Range<usize>,
        value: Value,
        label: String,
    }

    /// Returns `true` when `new` keeps every byte of `original` outside
    /// `allowed`, extended over adjacent blanks and commas.
    fn keeps_outside(original: &str, new: &str, allowed: &Range<usize>) -> bool {
        let bytes = original.as_bytes();
        let separator = |byte: u8| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b',');
        let mut start = allowed.start.min(bytes.len());
        while start > 0 && separator(bytes[start - 1]) {
            start -= 1;
        }
        let mut end = allowed.end.min(bytes.len());
        while end < bytes.len() && separator(bytes[end]) {
            end += 1;
        }
        let tail = bytes.len() - end;
        new.len() >= start + tail
            && new.as_bytes()[..start] == bytes[..start]
            && new.as_bytes()[new.len() - tail..] == bytes[end..]
    }

    fn corpus() -> Vec<(String, String)> {
        let mut files: Vec<(String, String)> = suite::load_cases()
            .into_iter()
            .filter(|case| !case.fail)
            .map(|case| (case.id, case.yaml))
            .collect();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/d5/corpus/yaml");
        let mut paths = Vec::new();
        collect(&root, &mut paths);
        paths.sort();
        for path in paths {
            if let Ok(text) = fs::read_to_string(&path) {
                files.push((path.display().to_string(), text));
            }
        }
        files
    }

    fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect(&path, out);
            } else if path
                .extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
            {
                out.push(path);
            }
        }
    }

    /// Paths of plain-named scalar values under mappings, in source order.
    fn leaves(model: &YamlModel) -> Vec<(Vec<String>, NodeId)> {
        let mut out = Vec::new();
        let root = model.documents()[0].root;
        let mut stack = vec![(Vec::new(), root)];
        while let Some((path, node)) = stack.pop() {
            let Some(mapping) = model.node(node).as_mapping() else {
                continue;
            };
            let mut seen = HashSet::new();
            for entry in mapping.entries.iter().rev() {
                let Some(name) = model.key_text(entry.key) else {
                    continue;
                };
                if !seen.insert(name.clone()) {
                    continue;
                }
                if entry.merge || model.node(entry.value).anchor.is_some() {
                    continue;
                }
                let mut child = path.clone();
                child.push(name);
                match &model.node(entry.value).kind {
                    NodeKind::Mapping(_) => stack.push((child, entry.value)),
                    NodeKind::Scalar(_) if !model.node(entry.value).is_empty_node() => {
                        out.push((child, entry.value))
                    }
                    _ => {}
                }
            }
        }
        out.sort_by_key(|(_, node)| model.node(*node).span.start);
        out.truncate(MAX_LEAVES);
        out
    }

    fn key_of(path: &[String]) -> KeyPath {
        KeyPath::new(
            None,
            path.iter()
                .map(|name| KeySegment::Key(name.clone()))
                .collect(),
        )
        .unwrap()
    }

    /// Range of the original text that differs from `new`.
    fn changed(old: &str, new: &str) -> Range<usize> {
        let (old, new) = (old.as_bytes(), new.as_bytes());
        let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        let limit = old.len().min(new.len()) - prefix;
        let suffix = old
            .iter()
            .rev()
            .zip(new.iter().rev())
            .take(limit)
            .take_while(|(a, b)| a == b)
            .count();
        prefix..old.len() - suffix
    }

    fn judge(
        tally: &mut Tally,
        original: &str,
        result: Result<String, String>,
        expected: &Expected,
    ) {
        tally.operations += 1;
        let new = match result {
            Ok(new) => new,
            Err(reason) => {
                tally.note(&format!("refused {reason} at {}", expected.label));
                return tally.refuse(reason);
            }
        };
        let in_scope = keeps_outside(original, &new, &expected.allowed);
        let value = YamlModel::parse(&new).and_then(|model| model.document_value(0));
        let right = value.is_ok_and(|value| equivalent(&value, &expected.value));
        if !in_scope {
            tally.out_of_scope += 1;
            tally.note(&format!("out_of_scope at {}", expected.label));
            tally.example("out_of_scope", original, &new);
        }
        if !right {
            tally.wrong_value += 1;
            tally.example("wrong_value", original, &new);
        }
        if in_scope && right {
            tally.clean += 1;
        }
    }

    fn model_set(text: &str, key: &KeyPath, value: &Value, create: bool) -> Result<String, String> {
        let mut doc = YamlDoc::parse(text, YamlVersion::V1_2)
            .map_err(|error| error.code.as_str().to_owned())?;
        doc.set(key, value, &SetOptions { create })
            .map_err(|error| {
                error
                    .found
                    .clone()
                    .unwrap_or_else(|| error.code.as_str().to_owned())
            })?;
        Ok(String::from_utf8_lossy(doc.text()).into_owned())
    }

    fn model_remove(text: &str, key: &KeyPath) -> Result<String, String> {
        let mut doc = YamlDoc::parse(text, YamlVersion::V1_2)
            .map_err(|error| error.code.as_str().to_owned())?;
        doc.remove(key).map_err(|error| {
            error
                .found
                .clone()
                .unwrap_or_else(|| error.code.as_str().to_owned())
        })?;
        Ok(String::from_utf8_lossy(doc.text()).into_owned())
    }

    fn trial_file(name: &str, model: &YamlModel, text: &str, mine: &mut Tally, other: &mut Tally) {
        let before = model.document_value(0).unwrap_or_default();
        let semantics = MapSemantics::LAST_WINS;
        for (path, node) in leaves(model) {
            let key = key_of(&path);
            let value = Value::Text(NEW_TEXT.to_owned());
            let mut after = before.clone();
            if apply_set(&mut after, &key, value.clone(), &semantics).is_err() {
                continue;
            }
            let expected = Expected {
                allowed: model.node(node).full_span(),
                value: after,
                label: format!("{name} {key}"),
            };
            judge(mine, text, model_set(text, &key, &value, false), &expected);
            judge(other, text, set(text, &path, &value), &expected);
        }
        let root = model.documents()[0].root;
        let Some(mapping) = model.node(root).as_mapping() else {
            return;
        };
        let key = key_of(&[NEW_KEY.to_owned()]);
        let mut after = before.clone();
        if apply_set(&mut after, &key, Value::Int(7), &semantics).is_ok() {
            let open = mapping
                .delimiters
                .as_ref()
                .map_or(0, |delimiters| delimiters.open.end);
            let end = mapping.entries.last().map_or(open, |entry| entry.span.end);
            let expected = Expected {
                allowed: end..end,
                value: after,
                label: format!("{name} {key}"),
            };
            judge(
                mine,
                text,
                model_set(text, &key, &Value::Int(7), true),
                &expected,
            );
            judge(
                other,
                text,
                set(text, &[NEW_KEY.to_owned()], &Value::Int(7)),
                &expected,
            );
        }
        let Some(first) = mapping
            .entries
            .first()
            .filter(|_| mapping.entries.len() > 1)
        else {
            return;
        };
        let Some(name) = model
            .node(first.key)
            .as_scalar()
            .map(|scalar| scalar.value.clone())
        else {
            return;
        };
        let key = key_of(std::slice::from_ref(&name));
        let mut after = before;
        if apply_remove(&mut after, &key, &semantics).is_ok() {
            let column = model.column(first.span.start);
            let expected = Expected {
                allowed: Comments::new(model).removal_span(first.span.clone(), Some(column + 1)),
                value: after,
                label: format!("{name} {key}"),
            };
            judge(mine, text, model_remove(text, &key), &expected);
            judge(other, text, remove(text, &[name]), &expected);
        }
    }

    #[test]
    fn trial_compares_both_implementations_on_the_corpus() {
        let mut mine = Tally::default();
        let mut other = Tally::default();
        for (name, text) in corpus() {
            mine.files += 1;
            other.files += 1;
            if round_trip(&text).is_ok_and(|written| written == text) {
                other.identical += 1;
            }
            if round_trip(&text).is_ok() {
                other.read += 1;
            }
            let Ok(model) = YamlModel::parse(&text) else {
                mine.refuse("E_DOC_PARSE on reading".to_owned());
                continue;
            };
            mine.read += 1;
            if model.source() == text {
                mine.identical += 1;
            }
            if model.documents().len() == 1 {
                trial_file(&name, &model, &text, &mut mine, &mut other);
            }
        }
        println!("{}", mine.line("model (saphyr)"));
        println!("{}", other.line("yaml-edit"));
        for line in mine.examples.iter().chain(&other.examples) {
            println!("{line}");
        }
        assert_eq!(mine.identical, mine.read);
        assert_eq!(mine.out_of_scope, 0);
        assert_eq!(mine.wrong_value, 0);
    }

    #[test]
    fn trial_counts_invalid_inputs_accepted() {
        let cases = suite::load_cases();
        let described = cases
            .iter()
            .filter(|case| case.tree.is_some() || case.json.is_some())
            .count();
        let invalid: Vec<String> = cases
            .into_iter()
            .filter(|case| case.fail)
            .map(|case| case.yaml)
            .collect();
        println!("suite cases with an event tree or JSON: {described}");
        let accepted_by_model = invalid
            .iter()
            .filter(|text| YamlModel::parse(text).is_ok())
            .count();
        let accepted_by_library = invalid
            .iter()
            .filter(|text| round_trip(text).is_ok())
            .count();
        println!(
            "invalid cases {} accepted: model {accepted_by_model} yaml-edit {accepted_by_library}",
            invalid.len()
        );
        assert_eq!(accepted_by_model, 0);
    }

    fn large_text() -> String {
        let mut text = String::from("services:\n");
        let mut index = 0usize;
        while text.len() < LARGE_BYTES {
            text.push_str(&format!(
                "  - name: svc-{index} # service {index}\n    image: \"registry.example/app:{index}\"\n    replicas: {}\n    labels: &l{index}\n      app: a{index}\n      tier: web\n    env:\n      - {{name: MODE, value: \"on\"}}\n    copy: *l{index}\n",
                index % 7
            ));
            index += 1;
        }
        text
    }

    fn resident_kib() -> usize {
        fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|status| {
                status
                    .lines()
                    .find(|line| line.starts_with("VmRSS:"))
                    .and_then(|line| line.split_whitespace().nth(1))
                    .and_then(|value| value.parse().ok())
            })
            .unwrap_or_default()
    }

    fn timed<T>(work: impl FnOnce() -> T) -> (T, Duration) {
        let start = Instant::now();
        let result = work();
        (result, start.elapsed())
    }

    #[test]
    #[ignore = "measures reading of a 10 MiB document; run alone with --ignored"]
    fn trial_large_document_with_the_model() {
        let text = large_text();
        let before = resident_kib();
        let (doc, read) = timed(|| YamlDoc::parse(text.clone(), YamlVersion::V1_2).unwrap());
        let held = resident_kib().saturating_sub(before);
        let mut doc = doc;
        let key = KeyPath::parse("services[1000].image").unwrap();
        let (result, edit) =
            timed(|| doc.set(&key, &Value::Text("x".to_owned()), &SetOptions::default()));
        assert!(result.is_ok());
        println!(
            "model: {} bytes read {read:?} edit {edit:?} resident +{held} KiB",
            text.len()
        );
    }

    #[test]
    #[ignore = "measures reading of a 10 MiB document; run alone with --ignored"]
    fn trial_large_document_with_yaml_edit() {
        let text = large_text();
        let before = resident_kib();
        let (file, read) = timed(|| YamlFile::from_str(&text).unwrap());
        let held = resident_kib().saturating_sub(before);
        let (written, write) = timed(|| file.to_string());
        assert_eq!(written, text);
        println!(
            "yaml-edit: {} bytes read {read:?} write {write:?} resident +{held} KiB",
            text.len()
        );
    }

    #[test]
    fn edits_with_the_library_api() {
        let text = "a: 1 # one\nb:\n  c: x\n";
        assert_eq!(round_trip(text).unwrap(), text);
        let path = vec!["b".to_owned(), "c".to_owned()];
        assert_eq!(
            set(text, &path, &Value::Text("y".to_owned())).unwrap(),
            "a: 1 # one\nb:\n  c: y\n"
        );
        assert_eq!(remove(text, &["a".to_owned()]).unwrap(), "b:\n  c: x\n");
        assert!(set(text, &[], &Value::Null).is_err());
        assert!(set(text, &["a".to_owned(), "z".to_owned()], &Value::Int(1)).is_err());
    }
}
