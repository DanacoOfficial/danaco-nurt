// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `registry:` paths in the file commands.
//!
//! The commands `read`, `info`, `list`, `find`, `write`, `remove`, `copy` and
//! `move` take paths of the Windows registry in place of file paths. The
//! registry is reached through [`RegistryAccess`], which the host registers
//! in the extensions of the execution context; where none is registered the
//! registry does not exist and a `registry:` path is rejected with
//! `E_OS_ONLY`. Reading returns the records of the provider. Every change is
//! declared to the journal as a target of a provider key, so that it is
//! preserved and undone like the change of a file: a removal is a `delete`
//! target, a copy a `create` target that receives [`Mutation::ProviderCopy`],
//! and a move a copy followed by the removal of the source in the same
//! transaction.
//!
//! A command that does not take registry paths rejects them in
//! [`crate::common::path::resolve`], which names the commands that do.

use std::sync::Arc;

use globset::{GlobBuilder, GlobMatcher};
use nurt_contracts::env::Os;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{
    FsError, FsErrorKind, RegistryAccess, RegistryKind, RegistryNode, RegistryNodeKind,
    RegistryTarget,
};
use nurt_contracts::journal::{Expect, Mutation, TargetAction, TargetPath, TargetSpec};
use nurt_contracts::record::Entry;
use nurt_contracts::stage::{BoundArgs, ExecCtx, RecordStream, StageOutput};
use nurt_contracts::value::{Path, Record, RecordKind, Value};

use crate::commands::conflict::{INTO, Step, TO};
use crate::common::mutation::planned_change;
use crate::common::path::PATH_FIELD;
use crate::walk::KindFilter;

/// Prefix of the paths of the registry.
pub(crate) const PREFIX: &str = "registry:";

/// Flag of the change of a key removed with its subtree.
const TREE_FLAG: &str = "recursive";

/// Whether `text` is a path of the registry.
pub(crate) fn is_registry_path(text: &str) -> bool {
    text.get(..PREFIX.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(PREFIX))
}

/// The registry path an `entry` or `registry_value` record describes; `None`
/// for a file. The path of a value record is its key, so the name of the
/// value is appended, escaped as in a path, and `(default)` stands for the
/// empty name.
pub(crate) fn record_text(record: &Record) -> Option<String> {
    let Some(Value::Path(text) | Value::Text(text)) = record.get(PATH_FIELD) else {
        return None;
    };
    if !is_registry_path(text.as_str()) {
        return None;
    }
    if record.kind != RecordKind::RegistryValue {
        return Some(text.as_str().to_owned());
    }
    let name = match record.get("name") {
        Some(Value::Text(name)) if !name.is_empty() => name.replace('%', "%25").replace('/', "%2F"),
        _ => "(default)".to_owned(),
    };
    Some(format!("{}/{name}", text.as_str().trim_end_matches('/')))
}

/// The `registry:` paths of the parameter `name`; `None` when it holds no
/// path of the registry.
///
/// # Errors
///
/// Fails with `E_INVALID_VALUE` when registry paths and file paths are mixed.
pub(crate) fn path_texts(args: &BoundArgs, name: &str) -> Result<Option<Vec<String>>> {
    let items: Vec<&Value> = match args.value(name) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::List(items)) => items.iter().collect(),
        Some(single) => vec![single],
    };
    let texts: Vec<String> = items
        .iter()
        .filter_map(|item| match item {
            Value::Path(text) | Value::Text(text) => Some(text.as_str().to_owned()),
            _ => None,
        })
        .collect();
    let registry = texts.iter().filter(|text| is_registry_path(text)).count();
    if registry == 0 {
        return Ok(None);
    }
    if registry == items.len() {
        return Ok(Some(texts));
    }
    Err(mixed_error(args.command()))
}

fn mixed_error(command: &str) -> Error {
    Error::new(
        ErrorCode::InvalidValue,
        format!("`{command}` takes either registry paths or file paths, not both in one call"),
    )
}

fn os_only(text: &str) -> Error {
    Error::new(
        ErrorCode::OsOnly,
        format!("`{text}` is a registry path, and the registry exists only on Windows"),
    )
    .with_found(text)
}

fn unavailable(ctx: &ExecCtx, text: &str) -> Error {
    if ctx.host().info().os == Os::Windows {
        Error::new(
            ErrorCode::Unsupported,
            format!("`{text}` is a registry path, but this host gives no access to the registry"),
        )
        .with_found(text)
    } else {
        os_only(text)
    }
}

/// Access to the registry registered by the host.
///
/// # Errors
///
/// Fails with `E_OS_ONLY` outside Windows, where none is registered.
pub(crate) fn access(ctx: &ExecCtx, text: &str) -> Result<Arc<dyn RegistryAccess>> {
    ctx.extensions()
        .get::<dyn RegistryAccess>()
        .ok_or_else(|| unavailable(ctx, text))
}

/// Value records of the values `texts` name.
///
/// # Errors
///
/// Fails with `E_NOT_FOUND` for a missing value, `E_IS_A_DIRECTORY` for a
/// key, and as [`access`].
pub(crate) fn read(ctx: &ExecCtx, texts: &[String]) -> Result<Vec<Record>> {
    let Some(first) = texts.first() else {
        return Ok(Vec::new());
    };
    let registry = access(ctx, first)?;
    texts.iter().map(|text| registry.read_value(text)).collect()
}

/// Records of the keys and values `texts` name; a path that names nothing is
/// described by an `entry` with `exists: false`, as for a file.
///
/// # Errors
///
/// As [`access`].
pub(crate) fn describe(ctx: &ExecCtx, texts: &[String]) -> Result<Vec<Record>> {
    let Some(first) = texts.first() else {
        return Ok(Vec::new());
    };
    let registry = access(ctx, first)?;
    texts
        .iter()
        .map(|text| {
            Ok(registry.describe(text)?.unwrap_or_else(|| {
                Record::from(Entry {
                    path: Path::new(text.as_str()),
                    name: text.rsplit('/').next().unwrap_or_default().to_owned(),
                    exists: Some(false),
                    ..Entry::default()
                })
            }))
        })
        .collect()
}

/// Selection of a listing of keys and values.
#[derive(Debug, Clone)]
pub(crate) struct Walk {
    /// Deepest level shown; `None` is unlimited.
    pub(crate) depth: Option<u32>,
    /// Kinds shown: values are files, keys are directories and symbolic
    /// link keys are links.
    pub(crate) kind: KindFilter,
    /// Name patterns; a node is shown when one matches its name or its path
    /// below the listed key.
    pub(crate) globs: Vec<String>,
    /// Whether the listed key itself is a candidate, as the root of `find`.
    pub(crate) root: bool,
    /// Whether keys precede values, as in `list`.
    pub(crate) keys_first: bool,
}

/// Records of the keys and values below `text` selected by `walk`.
///
/// # Errors
///
/// Fails with `E_INVALID_VALUE` for a pattern that does not compile,
/// `E_NOT_A_DIRECTORY` for a value, and as [`access`].
pub(crate) fn walk(ctx: &ExecCtx, text: &str, walk: &Walk) -> Result<Vec<Record>> {
    let registry = access(ctx, text)?;
    let matchers = matchers(&walk.globs)?;
    let mut nodes = registry.walk(text, walk.depth)?;
    if walk.keys_first {
        nodes.sort_by_key(|node| node.kind == RegistryNodeKind::Value);
    }
    let mut records = Vec::new();
    if walk.root
        && matchers.is_empty()
        && matches!(walk.kind, KindFilter::Any | KindFilter::Directory)
        && let Some(root) = registry.describe(text)?
    {
        records.push(root);
    }
    records.extend(
        nodes
            .into_iter()
            .filter(|node| kind_matches(walk.kind, node) && name_matches(&matchers, node))
            .map(|node| node.record),
    );
    Ok(records)
}

/// Output of `list` or `find` for one registry path.
///
/// # Errors
///
/// Fails with `E_INVALID_VALUE` when `texts` is not exactly one path, and as
/// [`walk`].
pub(crate) fn listing<'a>(
    ctx: &'a ExecCtx,
    texts: &[String],
    selection: &Walk,
) -> Result<StageOutput<'a>> {
    let [text] = texts else {
        return Err(Error::new(
            ErrorCode::InvalidValue,
            "a listing takes one registry path: give the key to list",
        ));
    };
    let records = walk(ctx, text, selection)?;
    Ok(StageOutput::records(
        RecordStream::new(records.into_iter().map(Ok)).checked(ctx),
    ))
}

fn kind_matches(kind: KindFilter, node: &RegistryNode) -> bool {
    match kind {
        KindFilter::File => node.kind == RegistryNodeKind::Value,
        KindFilter::Directory => node.kind == RegistryNodeKind::Key,
        KindFilter::Link => node.kind == RegistryNodeKind::Link,
        _ => true,
    }
}

fn matchers(globs: &[String]) -> Result<Vec<GlobMatcher>> {
    globs
        .iter()
        .map(|glob| {
            GlobBuilder::new(glob)
                .case_insensitive(true)
                .build()
                .map(|glob| glob.compile_matcher())
                .map_err(|error| {
                    Error::new(
                        ErrorCode::InvalidValue,
                        format!("`{glob}` is not a valid pattern: {error}"),
                    )
                    .with_found(glob.as_str())
                })
        })
        .collect()
}

fn name_matches(matchers: &[GlobMatcher], node: &RegistryNode) -> bool {
    matchers.is_empty()
        || matchers
            .iter()
            .any(|matcher| matcher.is_match(&node.name) || matcher.is_match(&node.relative))
}

/// Removals planned for registry paths.
#[derive(Debug, Default)]
pub(crate) struct Removals {
    /// One `delete` target for every key and value removed.
    pub(crate) steps: Vec<Step>,
    /// Keys removed with their subtrees.
    pub(crate) keys: u64,
    /// Values removed.
    pub(crate) values: u64,
    /// Paths that name nothing, skipped.
    pub(crate) missing: u64,
}

/// Plans the removal of the keys and values `texts` name. A key inside a key
/// that is removed as well, and a value of such a key, are removed once with
/// the enclosing key.
///
/// # Errors
///
/// Fails with `E_NOT_FOUND` for a missing path unless `skip_missing`, with
/// `E_PERMISSION` for a root key, and as [`access`].
pub(crate) fn plan_removals(
    ctx: &ExecCtx,
    texts: &[String],
    skip_missing: bool,
) -> Result<Removals> {
    let mut removals = Removals::default();
    let Some(first) = texts.first() else {
        return Ok(removals);
    };
    let registry = access(ctx, first)?;
    let mut found: Vec<(&str, RegistryTarget)> = Vec::new();
    for text in texts {
        match registry.locate(text) {
            Ok(target) => found.push((text, target)),
            Err(error) if skip_missing && error.code() == ErrorCode::NotFound => {
                removals.missing += 1;
            }
            Err(error) => return Err(error),
        }
    }
    for (text, target) in &found {
        if is_root(target) {
            return Err(root_error("remove", text));
        }
    }
    found.sort_by_key(|(_, target)| {
        (
            target.key.segments.len(),
            target.kind == RegistryKind::Value,
        )
    });
    let mut kept: Vec<RegistryTarget> = Vec::new();
    for (_, target) in found {
        if kept.iter().any(|outer| encloses(outer, &target)) {
            continue;
        }
        let tree = target.kind == RegistryKind::Key;
        let spec = TargetSpec {
            path: TargetPath::Provider(target.key.clone()),
            action: TargetAction::Delete,
            expect: Expect::Unchecked,
            size_hint: None,
        };
        let mut change = planned_change(&spec);
        if tree {
            change.flags = Some(vec![TREE_FLAG.to_owned()]);
            removals.keys += 1;
        } else {
            removals.values += 1;
        }
        removals
            .steps
            .push(Step::new(spec, vec![Mutation::ProviderRemove]).with_detail(change));
        kept.push(target);
    }
    Ok(removals)
}

fn is_root(target: &RegistryTarget) -> bool {
    target.kind == RegistryKind::Key && target.key.segments.len() == 1
}

fn root_error(operation: &str, text: &str) -> Error {
    FsError::new(
        FsErrorKind::PermissionDenied,
        operation,
        Some(std::path::Path::new(text)),
    )
    .into()
}

/// Whether `outer` is the same address as `inner` or a key that holds it.
fn encloses(outer: &RegistryTarget, inner: &RegistryTarget) -> bool {
    if outer.kind == RegistryKind::Value {
        return outer.key == inner.key;
    }
    let held = &inner.key.segments;
    let holder = &outer.key.segments;
    held.len() >= holder.len()
        && holder
            .iter()
            .zip(held)
            .all(|(left, right)| left.to_lowercase() == right.to_lowercase())
}

/// Destination of a copy or a move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Destination {
    /// `to`: the new path of the single source.
    To(String),
    /// `into`: the key that receives every source under its own name.
    Into(String),
}

/// What the parameters of `copy` or `move` address: the sources and the
/// destination when both name the registry, `None` when neither does.
///
/// # Errors
///
/// Fails with `E_INVALID_VALUE` when one side names the registry and the
/// other a file, which no command transfers between.
pub(crate) fn transfer_request(
    args: &BoundArgs,
    source_param: &str,
) -> Result<Option<(Vec<String>, Destination)>> {
    let sources = path_texts(args, source_param)?;
    let destination = [(TO, true), (INTO, false)]
        .into_iter()
        .find_map(|(name, to)| match args.value(name) {
            Some(Value::Path(text) | Value::Text(text)) if is_registry_path(text.as_str()) => {
                let text = text.as_str().to_owned();
                Some(if to {
                    Destination::To(text)
                } else {
                    Destination::Into(text)
                })
            }
            _ => None,
        });
    match (sources, destination) {
        (Some(sources), Some(destination)) => Ok(Some((sources, destination))),
        (None, None) => Ok(None),
        _ => Err(Error::new(
            ErrorCode::InvalidValue,
            format!(
                "`{}` does not transfer between the registry and files: give registry paths for the source and the destination",
                args.command()
            ),
        )),
    }
}

/// Copies and moves planned for registry paths.
#[derive(Debug, Default)]
pub(crate) struct Transfers {
    /// `create` target of every destination, each followed, for a move, by
    /// the `delete` target of its source.
    pub(crate) steps: Vec<Step>,
    /// Keys transferred with their subtrees.
    pub(crate) keys: u64,
    /// Values transferred.
    pub(crate) values: u64,
}

/// Plans the copy of the keys and values `sources` name to `destination`,
/// and for a move the removal of the sources in the same transaction. The
/// destination must not exist; missing keys above it are created.
///
/// # Errors
///
/// Fails with `E_NOT_FOUND` for a missing source, `E_PARAM_CONFLICT` for
/// several sources given to `to`, `E_PERMISSION` for a root key that is
/// moved, and as [`access`].
pub(crate) fn plan_transfers(
    ctx: &ExecCtx,
    sources: &[String],
    destination: &Destination,
    remove_sources: bool,
) -> Result<Transfers> {
    let mut transfers = Transfers::default();
    let Some(first) = sources.first() else {
        return Ok(transfers);
    };
    let registry = access(ctx, first)?;
    if sources.len() > 1 && matches!(destination, Destination::To(_)) {
        return Err(Error::new(
            ErrorCode::ParamConflict,
            "`to` names one destination: give `into` to transfer several sources",
        ));
    }
    for text in sources {
        let source = registry.locate(text)?;
        if remove_sources && is_root(&source) {
            return Err(root_error("move", text));
        }
        let key = match destination {
            Destination::To(path) => registry.address(path, source.kind)?.key,
            Destination::Into(path) => {
                let mut key = registry.address(path, RegistryKind::Key)?.key;
                match &source.key.value {
                    Some(name) => key.value = Some(name.clone()),
                    None => key.segments.extend(source.key.segments.last().cloned()),
                }
                key
            }
        };
        let spec = TargetSpec {
            path: TargetPath::Provider(key),
            action: TargetAction::Create,
            expect: Expect::Absent,
            size_hint: None,
        };
        let mut change = planned_change(&spec);
        change.from = Some(Path::new(text.as_str()));
        transfers.steps.push(
            Step::new(spec, vec![Mutation::ProviderCopy(source.key.clone())]).with_detail(change),
        );
        if remove_sources {
            let spec = TargetSpec {
                path: TargetPath::Provider(source.key.clone()),
                action: TargetAction::Delete,
                expect: Expect::Unchecked,
                size_hint: None,
            };
            transfers
                .steps
                .push(Step::new(spec, vec![Mutation::ProviderRemove]));
        }
        match source.kind {
            RegistryKind::Key => transfers.keys += 1,
            RegistryKind::Value => transfers.values += 1,
        }
    }
    Ok(transfers)
}

#[cfg(test)]
pub(crate) mod testing {
    //! Registry in memory for the tests of the commands.

    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use nurt_contracts::error::{Error, ErrorCode, Result};
    use nurt_contracts::fs::{
        ProviderKey, ProviderScheme, RegistryAccess, RegistryKind, RegistryNode, RegistryNodeKind,
        RegistryTarget,
    };
    use nurt_contracts::record::{Entry, EntryType};
    use nurt_contracts::stage::Extensions;
    use nurt_contracts::value::{Path, Record};

    /// Keys and values by lower-case path, as text.
    #[derive(Debug, Default)]
    pub(crate) struct MemRegistry {
        keys: Mutex<BTreeMap<String, ()>>,
        values: Mutex<BTreeMap<String, String>>,
    }

    impl MemRegistry {
        /// Registry with the given keys and values (`key/name` to text).
        pub(crate) fn new(keys: &[&str], values: &[(&str, &str)]) -> Arc<Self> {
            let registry = Self::default();
            for key in keys {
                registry.keys.lock().unwrap().insert(key.to_lowercase(), ());
            }
            for (path, value) in values {
                registry
                    .values
                    .lock()
                    .unwrap()
                    .insert(path.to_lowercase(), (*value).to_owned());
            }
            Arc::new(registry)
        }

        /// Extensions of an execution context that give access to it.
        pub(crate) fn extensions(self: &Arc<Self>) -> Extensions {
            let mut extensions = Extensions::new();
            extensions.insert::<dyn RegistryAccess>(self.clone());
            extensions
        }

        fn text(path: &str) -> Result<String> {
            path.strip_prefix("registry:/")
                .map(|rest| rest.trim_end_matches('/').to_owned())
                .ok_or_else(|| Error::new(ErrorCode::InvalidValue, "not a registry path"))
        }

        fn key_key(text: &str) -> ProviderKey {
            ProviderKey {
                scheme: ProviderScheme("registry".to_owned()),
                segments: text.split('/').map(str::to_owned).collect(),
                value: None,
            }
        }
    }

    impl RegistryAccess for MemRegistry {
        fn read_value(&self, path: &str) -> Result<Record> {
            let text = Self::text(path)?;
            if self.keys.lock().unwrap().contains_key(&text.to_lowercase()) {
                return Err(Error::new(ErrorCode::Io, "a key holds no data"));
            }
            let value = self
                .values
                .lock()
                .unwrap()
                .get(&text.to_lowercase())
                .cloned()
                .ok_or_else(|| Error::new(ErrorCode::NotFound, "no such value"))?;
            let (parent, name) = path.rsplit_once('/').unwrap_or((path, ""));
            Ok(Record::from(nurt_contracts::record::RegistryValue {
                path: Path::new(parent),
                name: name.to_owned(),
                value: nurt_contracts::value::Value::Text(value),
                ..Default::default()
            }))
        }

        fn describe(&self, path: &str) -> Result<Option<Record>> {
            let text = Self::text(path)?;
            if self.keys.lock().unwrap().contains_key(&text.to_lowercase()) {
                return Ok(Some(Record::from(Entry {
                    path: Path::new(path),
                    name: text.rsplit('/').next().unwrap_or_default().to_owned(),
                    kind: EntryType::Directory,
                    exists: Some(true),
                    ..Entry::default()
                })));
            }
            match self.read_value(path) {
                Ok(record) => Ok(Some(record)),
                Err(error) if error.code() == ErrorCode::NotFound => Ok(None),
                Err(error) => Err(error),
            }
        }

        fn walk(&self, path: &str, max_depth: Option<u32>) -> Result<Vec<RegistryNode>> {
            let text = Self::text(path)?.to_lowercase();
            if !self.keys.lock().unwrap().contains_key(&text) {
                return Err(Error::new(ErrorCode::NotFound, "no such key"));
            }
            let mut nodes = Vec::new();
            let within = |candidate: &str| -> Option<String> {
                let rest = candidate.strip_prefix(&format!("{text}/"))?;
                let depth = u32::try_from(rest.split('/').count()).unwrap_or(u32::MAX);
                max_depth
                    .is_none_or(|max| depth <= max)
                    .then(|| rest.to_owned())
            };
            for key in self.keys.lock().unwrap().keys() {
                if let Some(relative) = within(key) {
                    nodes.push(RegistryNode {
                        record: Record::from(Entry {
                            path: Path::new(format!("registry:/{key}")),
                            name: relative.rsplit('/').next().unwrap_or_default().to_owned(),
                            kind: EntryType::Directory,
                            ..Entry::default()
                        }),
                        kind: RegistryNodeKind::Key,
                        name: relative.rsplit('/').next().unwrap_or_default().to_owned(),
                        relative,
                    });
                }
            }
            for (value, data) in self.values.lock().unwrap().iter() {
                if let Some(relative) = within(value) {
                    nodes.push(RegistryNode {
                        record: Record::from(nurt_contracts::record::RegistryValue {
                            path: Path::new(format!(
                                "registry:/{}",
                                value.rsplit_once('/').map_or("", |(parent, _)| parent)
                            )),
                            name: relative.rsplit('/').next().unwrap_or_default().to_owned(),
                            value: nurt_contracts::value::Value::Text(data.clone()),
                            ..Default::default()
                        }),
                        kind: RegistryNodeKind::Value,
                        name: relative.rsplit('/').next().unwrap_or_default().to_owned(),
                        relative,
                    });
                }
            }
            Ok(nodes)
        }

        fn locate(&self, path: &str) -> Result<RegistryTarget> {
            let text = Self::text(path)?;
            if self.keys.lock().unwrap().contains_key(&text.to_lowercase()) {
                return Ok(RegistryTarget {
                    key: Self::key_key(&text),
                    kind: RegistryKind::Key,
                });
            }
            if self
                .values
                .lock()
                .unwrap()
                .contains_key(&text.to_lowercase())
            {
                return self.address(path, RegistryKind::Value);
            }
            Err(Error::new(
                ErrorCode::NotFound,
                format!("`{path}` does not exist"),
            ))
        }

        fn address(&self, path: &str, kind: RegistryKind) -> Result<RegistryTarget> {
            let text = Self::text(path)?;
            let mut key = Self::key_key(&text);
            if kind == RegistryKind::Value {
                key.value = key.segments.pop();
                if key.segments.is_empty() {
                    return Err(Error::new(
                        ErrorCode::InvalidValue,
                        "a root key holds no value",
                    ));
                }
            }
            Ok(RegistryTarget { key, kind })
        }
    }
}
