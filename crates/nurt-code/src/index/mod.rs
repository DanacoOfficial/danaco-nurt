// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Symbol index of a directory tree.
//!
//! [`SymbolIndex`] keeps the symbols of every source file of a root. A
//! refresh reads only the files whose size or modification time changed
//! since the previous refresh; the index is kept in the state directory of
//! the engine ([`ExecCtx::state_dir`], see [`store`]) and shared by later
//! executions. Without a state directory the index lives for one execution.
//!
//! Files larger than `code.file_bytes_max` are skipped; a listing stops at
//! `code.index_max_files` files. Files are parsed on up to
//! [`Limits::threads`](nurt_contracts::stage::Limits::threads) threads; the
//! result does not depend on the number of threads.

pub mod store;
pub mod walk;

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::thread;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, ContentHash, Follow};
use nurt_contracts::services::{LanguageId, SymbolInfo};
use nurt_contracts::stage::ExecCtx;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::registry::Registry;
use crate::symbols::extract;

pub use walk::{ALWAYS_SKIPPED, Listing, SourceFilter, list_sources};

/// Digest of content in the form of the entity tags of code records.
///
/// The value equals the content hash of the file system for the same bytes.
///
/// ```
/// use nurt_code::index::content_hash;
///
/// let hash = content_hash(b"abc");
/// assert_eq!(
///     hash.to_string(),
///     "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
/// );
/// ```
#[must_use]
pub fn content_hash(bytes: &[u8]) -> ContentHash {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    ContentHash(digest)
}

/// Content of a source file read through the context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceText {
    /// Path of the file.
    pub path: PathBuf,
    /// Content.
    pub bytes: Vec<u8>,
    /// Size in bytes when opened.
    pub size: u64,
    /// Modification time when opened, in nanoseconds since the Unix epoch.
    pub modified_ns: i64,
    /// Digest of the content.
    pub etag: ContentHash,
}

/// Reads a source file through the file system of the context.
///
/// # Errors
///
/// Fails with `E_LIMIT` when the file is larger than `code.file_bytes_max`,
/// with `E_STAGE_FAILED` when the context has no file system, and with the
/// error of the file system when the file cannot be read.
pub fn read_source(ctx: &ExecCtx, path: &Path) -> Result<SourceText> {
    read_limited(ctx, path, ctx.config().code.file_bytes_max.bytes())
}

fn read_limited(ctx: &ExecCtx, path: &Path, max: u64) -> Result<SourceText> {
    let fs = ctx.require_fs()?;
    let absolute = AbsPath::new(path).map_err(Error::from)?;
    let mut handle = fs.open_read(&absolute).map_err(Error::from)?;
    let meta = handle.meta().map_err(Error::from)?;
    if meta.size > max {
        return Err(too_large(path, meta.size, max));
    }
    let mut bytes = Vec::new();
    handle.read_to_end(&mut bytes).map_err(|error| {
        Error::new(
            ErrorCode::Io,
            format!("cannot read {}: {error}", path.display()),
        )
    })?;
    Ok(SourceText {
        path: path.to_path_buf(),
        etag: content_hash(&bytes),
        size: meta.size,
        modified_ns: meta.modified_ns,
        bytes,
    })
}

fn too_large(path: &Path, size: u64, max: u64) -> Error {
    Error::new(
        ErrorCode::Limit,
        format!(
            "{} has {size} bytes, more than code.file_bytes_max ({max}); raise the limit to analyse it",
            path.display()
        ),
    )
}

/// Symbols of one file of the index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedFile {
    /// Path of the file.
    pub path: PathBuf,
    /// Language of the file; `None` when no grammar applies.
    pub language: Option<LanguageId>,
    /// Size in bytes when indexed.
    pub size: u64,
    /// Modification time when indexed, in nanoseconds since the Unix epoch.
    pub modified_ns: i64,
    /// Digest of the indexed content (`sha256:…`).
    pub etag: String,
    /// Symbols in source order.
    pub symbols: Vec<SymbolInfo>,
}

/// Limits of an index refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexOptions {
    /// Largest number of files listed.
    pub max_files: usize,
    /// Largest file indexed, in bytes.
    pub file_bytes_max: u64,
    /// Threads used for parsing.
    pub threads: usize,
}

impl IndexOptions {
    /// Limits from the configuration (`code.index_max_files`,
    /// `code.file_bytes_max`) and the resource limits of the context.
    #[must_use]
    pub fn from_ctx(ctx: &ExecCtx) -> Self {
        let code = &ctx.config().code;
        Self {
            max_files: usize::try_from(code.index_max_files).unwrap_or(usize::MAX),
            file_bytes_max: code.file_bytes_max.bytes(),
            threads: usize::try_from(ctx.limits().threads).unwrap_or(1).max(1),
        }
    }
}

/// Counts of one refresh.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RefreshStats {
    /// Files in the index after the refresh.
    pub files: usize,
    /// Files read and parsed.
    pub parsed: usize,
    /// Files kept from the previous refresh without reading them.
    pub reused: usize,
    /// Files skipped: too large or unreadable.
    pub skipped: usize,
    /// Files of the previous refresh no longer listed.
    pub removed: usize,
    /// The listing stopped at `code.index_max_files`.
    pub truncated: bool,
}

/// Symbols of the source files of one root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolIndex {
    root: PathBuf,
    files: Vec<IndexedFile>,
}

impl SymbolIndex {
    /// Empty index of `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            files: Vec::new(),
        }
    }

    /// Index of `root` stored in the state directory of the context, or an
    /// empty index when there is none, it is unreadable or it was built by
    /// another program or grammar version.
    #[must_use]
    pub fn open(ctx: &ExecCtx, registry: &Registry, root: &Path) -> Self {
        let files = ctx
            .state_dir()
            .and_then(|state| {
                store::load(
                    &store::index_file(state, root),
                    root,
                    &store::stamp(registry),
                )
            })
            .unwrap_or_default();
        Self {
            root: root.to_path_buf(),
            files,
        }
    }

    /// Root of the index.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Indexed files in path order.
    #[must_use]
    pub fn files(&self) -> &[IndexedFile] {
        &self.files
    }

    /// Brings the index to the files `paths`: files whose size and
    /// modification time are unchanged keep their symbols, other files are
    /// read and parsed, files not listed are removed.
    ///
    /// # Errors
    ///
    /// Fails with `E_STAGE_FAILED` when the context has no file system and
    /// with the error of the cancellation when the execution is cancelled.
    pub fn refresh(
        &mut self,
        ctx: &ExecCtx,
        registry: &Registry,
        paths: &[PathBuf],
        options: &IndexOptions,
    ) -> Result<RefreshStats> {
        let fs = ctx.require_fs()?;
        let mut previous: HashMap<PathBuf, IndexedFile> = std::mem::take(&mut self.files)
            .into_iter()
            .map(|file| (file.path.clone(), file))
            .collect();
        let mut stats = RefreshStats::default();
        let mut kept = Vec::with_capacity(paths.len());
        let mut pending = Vec::new();
        for path in paths {
            ctx.checkpoint()?;
            let meta = AbsPath::new(path)
                .ok()
                .and_then(|absolute| fs.metadata(&absolute, Follow::Follow).ok());
            let Some(meta) = meta.filter(|meta| meta.size <= options.file_bytes_max) else {
                stats.skipped += 1;
                continue;
            };
            match previous.remove(path) {
                Some(file) if file.size == meta.size && file.modified_ns == meta.modified_ns => {
                    stats.reused += 1;
                    kept.push(file);
                }
                _ => pending.push(path.clone()),
            }
        }
        stats.removed = previous.len();
        let parsed = parse_all(ctx, registry, &pending, options)?;
        stats.parsed = parsed.iter().filter(|file| file.is_some()).count();
        stats.skipped += pending.len() - stats.parsed;
        kept.extend(parsed.into_iter().flatten());
        kept.sort_by(|left, right| left.path.cmp(&right.path));
        stats.files = kept.len();
        self.files = kept;
        Ok(stats)
    }

    /// Writes the index to the state directory of the context; `Ok(false)`
    /// when the context has no state directory.
    ///
    /// # Errors
    ///
    /// Fails with `E_IO` when the index cannot be written.
    pub fn save(&self, ctx: &ExecCtx, registry: &Registry) -> Result<bool> {
        let Some(state) = ctx.state_dir() else {
            return Ok(false);
        };
        let file = store::index_file(state, &self.root);
        store::save(&file, &self.root, &store::stamp(registry), &self.files).map_err(|error| {
            Error::new(
                ErrorCode::Io,
                format!(
                    "cannot write the symbol index {}: {error}; check the state directory",
                    file.display()
                ),
            )
        })?;
        Ok(true)
    }
}

/// Index of the source files under `root`: the stored index refreshed with
/// the current listing, written back when it changed.
///
/// # Errors
///
/// Fails like [`list_sources`] and [`SymbolIndex::refresh`]. A failure to
/// write the index is not an error: the index is then rebuilt by the next
/// execution.
pub fn index_tree(
    ctx: &ExecCtx,
    registry: &Registry,
    root: &Path,
    filter: &SourceFilter,
    accept: &dyn Fn(&Path) -> bool,
) -> Result<(SymbolIndex, RefreshStats)> {
    let options = IndexOptions::from_ctx(ctx);
    let listing = list_sources(ctx, root, filter, accept, options.max_files)?;
    let mut index = SymbolIndex::open(ctx, registry, root);
    let mut stats = index.refresh(ctx, registry, &listing.files, &options)?;
    stats.truncated = listing.truncated;
    if stats.parsed > 0 || stats.removed > 0 {
        let _ = index.save(ctx, registry);
    }
    Ok((index, stats))
}

/// Parses `paths` on up to `options.threads` threads; `None` for a file
/// that cannot be read.
fn parse_all(
    ctx: &ExecCtx,
    registry: &Registry,
    paths: &[PathBuf],
    options: &IndexOptions,
) -> Result<Vec<Option<IndexedFile>>> {
    let threads = options.threads.min(paths.len()).max(1);
    if threads == 1 {
        return parse_chunk(ctx, registry, paths, options.file_bytes_max);
    }
    let chunk = paths.len().div_ceil(threads);
    thread::scope(|scope| {
        let workers: Vec<_> = paths
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || parse_chunk(ctx, registry, part, options.file_bytes_max))
            })
            .collect();
        let mut all = Vec::with_capacity(paths.len());
        for worker in workers {
            let part = worker.join().map_err(|_| {
                Error::new(
                    ErrorCode::Internal,
                    "a parser thread of the symbol index stopped",
                )
            })??;
            all.extend(part);
        }
        Ok(all)
    })
}

fn parse_chunk(
    ctx: &ExecCtx,
    registry: &Registry,
    paths: &[PathBuf],
    max: u64,
) -> Result<Vec<Option<IndexedFile>>> {
    paths
        .iter()
        .map(|path| {
            ctx.checkpoint()?;
            Ok(index_file(ctx, registry, path, max))
        })
        .collect()
}

fn index_file(ctx: &ExecCtx, registry: &Registry, path: &Path, max: u64) -> Option<IndexedFile> {
    let source = read_limited(ctx, path, max).ok()?;
    let language = registry.detect(Some(path), &source.bytes);
    let symbols = language
        .as_ref()
        .and_then(|language| registry.parse(language, &source.bytes).ok())
        .map(|parsed| extract(&parsed))
        .unwrap_or_default();
    Some(IndexedFile {
        path: path.to_path_buf(),
        language,
        size: source.size,
        modified_ns: source.modified_ns,
        etag: source.etag.to_string(),
        symbols,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_round_trips_through_its_text_form() {
        let hash = content_hash(b"fn main() {}\n");
        let text = hash.to_string();
        assert!(text.starts_with(ContentHash::PREFIX));
        assert_eq!(ContentHash::parse(&text), Ok(hash));
        assert_ne!(content_hash(b""), hash);
    }

    #[test]
    fn new_index_is_empty() {
        let index = SymbolIndex::new("/repo");
        assert_eq!(index.root(), Path::new("/repo"));
        assert!(index.files().is_empty());
    }
}
