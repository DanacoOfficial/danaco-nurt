// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `hash`: digests of file contents.
//!
//! The source form hashes the files given by `path` and returns one `entry`
//! record per file. The stage form sets the `hash` field of the input
//! `entry` records and passes records of directories and other entries that
//! are not regular files through unchanged. The digest has the form
//! `<algorithm>:<hex>`, for example `sha256:ba78…`.
//!
//! Content is read in chunks, so a file of any size is hashed in fixed
//! memory, and cancellation is checked between chunks. With more than one
//! thread in the limits of the execution, files are hashed in parallel in
//! batches; the records keep the input order.

use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, FileKind, Follow, Fs};
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::{Record, RecordKind, Value};
use nurt_text::hash::{Digest, HashAlgorithm, Hasher};

use crate::common::entry::entry_record;
use crate::common::path::{path_list_arg, record_path};

/// Name of the command in the catalog.
const COMMAND: &str = "hash";

/// Name of the `path` parameter.
const PATH: &str = "path";

/// Name of the `algorithm` parameter.
const ALGORITHM: &str = "algorithm";

/// Field of `entry` records that receives the digest.
const HASH_FIELD: &str = "hash";

/// Algorithm used when `algorithm` has no value, as in the catalog.
const DEFAULT_ALGORITHM: HashAlgorithm = HashAlgorithm::Sha256;

/// Size of the chunks in which content is read; cancellation is checked
/// between chunks.
const CHUNK_SIZE: usize = 64 * 1024;

/// Files given to each thread per batch when hashing in parallel.
const FILES_PER_THREAD: usize = 4;

/// Stage of `hash`: source form with `path`, stage form on `entry` records.
pub(crate) struct Hash;

impl StageImpl for Hash {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::Only(vec![RecordKind::Entry]),
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for Hash {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let hashing = Hashing {
            fs: ctx.require_fs()?,
            ctx,
            algorithm: algorithm_param(args)?,
        };
        if input.is_none() {
            let paths = path_list_arg(args, PATH, ctx)?;
            if paths.is_empty() {
                return Err(missing_path_error());
            }
            let jobs = paths.into_iter().map(|path| Ok(Job::Path(path)));
            return Ok(StageOutput::records(hashed(jobs, hashing)));
        }
        if args.is_given(PATH) {
            return Err(path_conflict_error());
        }
        input.map_records(|stream| hashed(stream.map(|item| item.map(Job::Record)), hashing))
    }
}

/// Stages of this module, for the registry.
pub(crate) fn stages() -> Vec<Arc<dyn StageImpl>> {
    vec![Arc::new(Hash)]
}

/// Reads `algorithm`; [`DEFAULT_ALGORITHM`] without a value.
fn algorithm_param(args: &BoundArgs) -> Result<HashAlgorithm> {
    match args.value(ALGORITHM) {
        None | Some(Value::Null) => Ok(DEFAULT_ALGORITHM),
        Some(Value::Enum(name) | Value::Text(name)) => name.parse().map_err(Error::from),
        Some(_) => Err(Error::new(
            ErrorCode::Type,
            format!("`{COMMAND}` expects a value of the enumeration for `{ALGORITHM}`"),
        )),
    }
}

/// One file to hash: a path of the source form or an input record.
enum Job {
    Path(AbsPath),
    Record(Record),
}

/// What every hashing thread shares.
#[derive(Clone, Copy)]
struct Hashing<'a> {
    fs: &'a dyn Fs,
    ctx: &'a ExecCtx,
    algorithm: HashAlgorithm,
}

impl Hashing<'_> {
    fn run(self, job: Job) -> Result<Record> {
        match job {
            Job::Path(path) => self.hash_path(&path),
            Job::Record(record) => self.hash_record(record),
        }
    }

    /// `entry` record of a file given by path, with its digest. The record
    /// describes the entry itself; a link is hashed by the content of its
    /// target.
    fn hash_path(self, path: &AbsPath) -> Result<Record> {
        let meta = self.fs.metadata(path, Follow::NoFollow)?;
        let kind = match meta.kind {
            FileKind::File => FileKind::File,
            _ => self.fs.metadata(path, Follow::Follow)?.kind,
        };
        if kind != FileKind::File {
            return Err(not_a_file_error(path, kind));
        }
        let digest = self.digest(path)?;
        let mut entry = entry_record(path, &meta);
        entry.hash = Some(digest.to_string());
        Ok(entry.into())
    }

    /// The input record with its digest; records of entries that are not
    /// regular files are returned unchanged.
    fn hash_record(self, record: Record) -> Result<Record> {
        let path = record_path(&record, self.ctx)?;
        if self.fs.metadata(&path, Follow::Follow)?.kind != FileKind::File {
            return Ok(record);
        }
        let digest = self.digest(&path)?;
        Ok(record.with(HASH_FIELD, Value::Text(digest.to_string())))
    }

    fn digest(self, path: &AbsPath) -> Result<Digest> {
        let mut handle = self.fs.open_read(path)?;
        let mut hasher = Hasher::new(self.algorithm);
        if let Some(bytes) = handle.as_bytes() {
            for chunk in bytes.chunks(CHUNK_SIZE) {
                self.ctx.checkpoint()?;
                hasher.update(chunk);
            }
            return Ok(hasher.finalize());
        }
        let mut buffer = vec![0_u8; CHUNK_SIZE];
        loop {
            self.ctx.checkpoint()?;
            match handle.read(&mut buffer) {
                Ok(0) => return Ok(hasher.finalize()),
                Ok(read) => hasher.update(buffer.get(..read).unwrap_or_default()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(read_error(path, &error)),
            }
        }
    }

    /// Hashes a batch in up to `threads` scoped threads; the results keep
    /// the order of the batch.
    fn run_parallel(self, batch: Vec<Result<Job>>, threads: usize) -> Vec<Result<Record>> {
        let per_thread = batch.len().div_ceil(threads).max(1);
        let mut jobs = batch.into_iter();
        let groups: Vec<Vec<Result<Job>>> = std::iter::from_fn(|| {
            let group: Vec<Result<Job>> = jobs.by_ref().take(per_thread).collect();
            (!group.is_empty()).then_some(group)
        })
        .collect();
        std::thread::scope(|scope| {
            let handles: Vec<_> = groups
                .into_iter()
                .map(|group| {
                    scope.spawn(move || {
                        group
                            .into_iter()
                            .map(|job| job.and_then(|job| self.run(job)))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|handle| handle.join().unwrap_or_else(|_| vec![Err(thread_error())]))
                .collect()
        })
    }
}

/// Lazy stream of hashed records: one file at a time with one thread,
/// otherwise batches of [`FILES_PER_THREAD`] files per thread.
struct Batched<'a, I> {
    jobs: I,
    hashing: Hashing<'a>,
    threads: usize,
    ready: VecDeque<Result<Record>>,
}

impl<I> Iterator for Batched<'_, I>
where
    I: Iterator<Item = Result<Job>>,
{
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(record) = self.ready.pop_front() {
            return Some(record);
        }
        if self.threads <= 1 {
            let job = self.jobs.next()?;
            return Some(job.and_then(|job| self.hashing.run(job)));
        }
        let batch: Vec<Result<Job>> = self
            .jobs
            .by_ref()
            .take(self.threads.saturating_mul(FILES_PER_THREAD))
            .collect();
        if batch.is_empty() {
            return None;
        }
        self.ready
            .extend(self.hashing.run_parallel(batch, self.threads));
        self.ready.pop_front()
    }
}

fn hashed<'a, I>(jobs: I, hashing: Hashing<'a>) -> RecordStream<'a>
where
    I: Iterator<Item = Result<Job>> + Send + 'a,
{
    let ctx = hashing.ctx;
    let threads = usize::try_from(ctx.limits().threads).unwrap_or(1).max(1);
    RecordStream::new(Batched {
        jobs,
        hashing,
        threads,
        ready: VecDeque::new(),
    })
    .checked(ctx)
}

fn missing_path_error() -> Error {
    Error::new(
        ErrorCode::MissingParam,
        format!("`{COMMAND}` needs a path: give the files to hash or pass entry records"),
    )
}

fn path_conflict_error() -> Error {
    Error::new(
        ErrorCode::ParamConflict,
        format!(
            "`{PATH}` cannot be given when `{COMMAND}` receives records: remove `{PATH}` or start the pipeline with `{COMMAND}`"
        ),
    )
}

fn not_a_file_error(path: &AbsPath, kind: FileKind) -> Error {
    let message = if kind == FileKind::Directory {
        format!("`{path}` is a directory: hash its files with `find` and `|> {COMMAND}`")
    } else {
        format!("`{path}` is not a regular file: give a file")
    };
    Error::new(ErrorCode::InvalidValue, message)
        .with_problem("not_a_file")
        .with_found(path.to_language_string())
        .with_detail("path", serde_json::Value::from(path.to_language_string()))
}

fn read_error(path: &AbsPath, error: &io::Error) -> Error {
    Error::new(ErrorCode::Io, format!("reading `{path}` failed: {error}"))
        .with_detail("path", serde_json::Value::from(path.to_language_string()))
}

fn thread_error() -> Error {
    Error::new(
        ErrorCode::Internal,
        "a hashing thread stopped unexpectedly: run the command again",
    )
}

#[cfg(test)]
mod tests {
    use nurt_contracts::record::{Entry, EntryType};
    use nurt_contracts::stage::{ArgValue, CancelReason, Limits, StageData};
    use nurt_testkit::ctx::TestContext;
    use nurt_testkit::vfs::test_path;
    use nurt_text::hash::hash_reader;

    use super::*;

    const SAMPLE_SHA256: &str =
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn abs(text: &str) -> AbsPath {
        test_path(text).unwrap()
    }

    fn with_files(files: &[(&str, &[u8])]) -> TestContext {
        let test = TestContext::new();
        for (path, content) in files {
            test.fs().write_file(&abs(path), content).unwrap();
        }
        test
    }

    fn paths(items: &[&str]) -> Value {
        Value::List(
            items
                .iter()
                .map(|path| Value::Path(abs(path).to_language_string()))
                .collect(),
        )
    }

    fn args(path: Option<Value>, algorithm: Option<&str>) -> BoundArgs {
        let mut args = BoundArgs::new(COMMAND);
        if let Some(path) = path {
            args.insert(PATH, ArgValue::Value(path), false);
        }
        if let Some(algorithm) = algorithm {
            args.insert(
                ALGORITHM,
                ArgValue::Value(Value::Enum(algorithm.to_owned())),
                false,
            );
        }
        args
    }

    fn records(output: StageOutput<'_>) -> Result<Vec<Record>> {
        match output.data {
            StageData::Records(stream) => stream.collect(),
            other => panic!("expected records, got {other:?}"),
        }
    }

    fn run(ctx: &ExecCtx, args: &BoundArgs, input: StageInput<'_>) -> Result<Vec<Record>> {
        records(Hash.execute(args, input, ctx)?)
    }

    fn hash_of(record: &Record) -> Option<String> {
        match record.get(HASH_FIELD) {
            Some(Value::Text(text)) => Some(text.clone()),
            _ => None,
        }
    }

    fn entry_input(paths: &[&str]) -> StageInput<'static> {
        let records = paths
            .iter()
            .map(|path| {
                Record::new(RecordKind::Entry)
                    .with("path", Value::Path(abs(path).to_language_string()))
                    .with("size", Value::Int(0))
            })
            .collect();
        StageInput::records(RecordStream::from_records(records))
    }

    #[test]
    fn descriptor_registers_source_and_entry_input() {
        let descriptor = Hash.descriptor();
        assert_eq!(descriptor.command, "hash");
        assert!(descriptor.source);
        assert!(descriptor.input.accepts(RecordKind::Entry));
        assert!(!descriptor.input.accepts(RecordKind::Match));
        assert!(Hash.as_read().is_some());
        assert!(Hash.as_effect().is_none());
        assert_eq!(stages().len(), 1);
    }

    #[test]
    fn source_form_returns_entries_with_sha256_by_default() {
        let test = with_files(&[("/data/a.txt", b"abc")]);
        let output = run(
            &test.ctx(),
            &args(Some(paths(&["/data/a.txt"])), None),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(output.len(), 1);
        let entry = Entry::try_from(output[0].clone()).unwrap();
        assert_eq!(entry.hash.as_deref(), Some(SAMPLE_SHA256));
        assert_eq!(entry.name, "a.txt");
        assert_eq!(entry.kind, EntryType::File);
        assert_eq!(entry.path.as_str(), abs("/data/a.txt").to_language_string());
        assert!(entry.etag.is_some());
    }

    #[test]
    fn every_algorithm_gives_its_prefix_and_reference_digest() {
        let test = with_files(&[("/data/a.txt", b"abc")]);
        let expected = [
            ("sha256", SAMPLE_SHA256),
            (
                "sha512",
                "sha512:ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
                 2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
            ),
            ("sha1", "sha1:a9993e364706816aba3e25717850c26c9cd0d89d"),
            ("md5", "md5:900150983cd24fb0d6963f7d28e17f72"),
            ("crc32", "crc32:352441c2"),
        ];
        for (algorithm, digest) in expected {
            let output = run(
                &test.ctx(),
                &args(Some(paths(&["/data/a.txt"])), Some(algorithm)),
                StageInput::none(),
            )
            .unwrap();
            assert_eq!(hash_of(&output[0]).as_deref(), Some(digest), "{algorithm}");
        }
    }

    #[test]
    fn blake3_matches_the_official_vector_of_empty_input() {
        let test = with_files(&[("/data/empty", b"")]);
        let output = run(
            &test.ctx(),
            &args(Some(paths(&["/data/empty"])), Some("blake3")),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(
            hash_of(&output[0]).as_deref(),
            Some("blake3:af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262")
        );
    }

    #[test]
    fn several_paths_keep_their_order() {
        let test = with_files(&[("/data/b.txt", b"b"), ("/data/a.txt", b"a")]);
        let output = run(
            &test.ctx(),
            &args(Some(paths(&["/data/b.txt", "/data/a.txt"])), None),
            StageInput::none(),
        )
        .unwrap();
        let names: Vec<String> = output
            .into_iter()
            .map(|record| Entry::try_from(record).unwrap().name)
            .collect();
        assert_eq!(names, ["b.txt", "a.txt"]);
    }

    #[test]
    fn a_single_path_value_is_accepted() {
        let test = with_files(&[("/data/a.txt", b"abc")]);
        let single = Value::Text(abs("/data/a.txt").to_language_string());
        let output = run(&test.ctx(), &args(Some(single), None), StageInput::none()).unwrap();
        assert_eq!(hash_of(&output[0]).as_deref(), Some(SAMPLE_SHA256));
    }

    #[test]
    fn stage_form_sets_the_hash_and_keeps_other_fields() {
        let test = with_files(&[("/data/a.txt", b"abc")]);
        let output = run(
            &test.ctx(),
            &args(None, None),
            entry_input(&["/data/a.txt"]),
        )
        .unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(hash_of(&output[0]).as_deref(), Some(SAMPLE_SHA256));
        assert_eq!(output[0].get("size"), Some(&Value::Int(0)));
    }

    #[test]
    fn stage_form_passes_directories_through_unchanged() {
        let test = with_files(&[("/data/dir/a.txt", b"abc")]);
        let output = run(
            &test.ctx(),
            &args(None, None),
            entry_input(&["/data/dir", "/data/dir/a.txt"]),
        )
        .unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(hash_of(&output[0]), None);
        assert_eq!(hash_of(&output[1]).as_deref(), Some(SAMPLE_SHA256));
    }

    #[test]
    fn identical_contents_get_identical_hashes() {
        let test = with_files(&[
            ("/data/a.txt", b"same"),
            ("/data/sub/b.txt", b"same"),
            ("/data/c.txt", b"other"),
        ]);
        let output = run(
            &test.ctx(),
            &args(None, Some("blake3")),
            entry_input(&["/data/a.txt", "/data/sub/b.txt", "/data/c.txt"]),
        )
        .unwrap();
        let hashes: Vec<Option<String>> = output.iter().map(hash_of).collect();
        assert_eq!(hashes[0], hashes[1]);
        assert_ne!(hashes[0], hashes[2]);
    }

    #[test]
    fn source_form_rejects_directories() {
        let test = TestContext::new();
        test.fs().create_dir_all(&abs("/data/dir")).unwrap();
        let error = run(
            &test.ctx(),
            &args(Some(paths(&["/data/dir"])), None),
            StageInput::none(),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::InvalidValue);
        assert_eq!(error.problem.as_deref(), Some("not_a_file"));
        assert!(error.message.contains("is a directory"));
    }

    #[test]
    fn links_are_hashed_by_the_content_of_their_target() {
        let test = with_files(&[("/data/a.txt", b"abc")]);
        let link = abs("/data/link");
        test.fs()
            .symlink(abs("/data/a.txt").as_path(), &link)
            .unwrap();
        let output = run(
            &test.ctx(),
            &args(Some(paths(&["/data/link"])), None),
            StageInput::none(),
        )
        .unwrap();
        let entry = Entry::try_from(output[0].clone()).unwrap();
        assert_eq!(entry.kind, EntryType::Link);
        assert_eq!(entry.hash.as_deref(), Some(SAMPLE_SHA256));
    }

    #[test]
    fn missing_files_fail_with_not_found() {
        let test = TestContext::new();
        let error = run(
            &test.ctx(),
            &args(Some(paths(&["/data/none.txt"])), None),
            StageInput::none(),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
    }

    #[test]
    fn invalid_algorithms_are_rejected() {
        let test = with_files(&[("/data/a.txt", b"abc")]);
        let unknown = Hash
            .execute(
                &args(Some(paths(&["/data/a.txt"])), Some("sha3")),
                StageInput::none(),
                &test.ctx(),
            )
            .unwrap_err();
        assert_eq!(unknown.code(), ErrorCode::UnknownValue);
        let mut wrong = args(Some(paths(&["/data/a.txt"])), None);
        wrong.insert(ALGORITHM, ArgValue::Value(Value::Int(1)), false);
        let error = Hash
            .execute(&wrong, StageInput::none(), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Type);
    }

    #[test]
    fn source_form_needs_a_path() {
        let test = TestContext::new();
        let error = Hash
            .execute(&args(None, None), StageInput::none(), &test.ctx())
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::MissingParam);
    }

    #[test]
    fn stage_form_rejects_a_path() {
        let test = with_files(&[("/data/a.txt", b"abc")]);
        let error = Hash
            .execute(
                &args(Some(paths(&["/data/a.txt"])), None),
                entry_input(&["/data/a.txt"]),
                &test.ctx(),
            )
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::ParamConflict);
    }

    #[test]
    fn parallel_hashing_keeps_the_input_order() {
        let names: Vec<String> = (0..23).map(|index| format!("/data/f{index:02}")).collect();
        let test = TestContext::new();
        for (index, name) in names.iter().enumerate() {
            test.fs()
                .write_file(&abs(name), index.to_string().as_bytes())
                .unwrap();
        }
        let listed: Vec<&str> = names.iter().map(String::as_str).collect();
        let sequential = run(
            &test.ctx(),
            &args(Some(paths(&listed)), None),
            StageInput::none(),
        )
        .unwrap();
        let limits = Limits {
            threads: 3,
            ..Limits::default()
        };
        let ctx = test.builder().limits(limits).build();
        let parallel = run(&ctx, &args(None, None), entry_input(&listed)).unwrap();
        assert_eq!(parallel.len(), names.len());
        let expected: Vec<Option<String>> = sequential.iter().map(hash_of).collect();
        let actual: Vec<Option<String>> = parallel.iter().map(hash_of).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn parallel_hashing_reports_the_first_error_and_stops() {
        let test = with_files(&[("/data/a.txt", b"a")]);
        let limits = Limits {
            threads: 2,
            ..Limits::default()
        };
        let ctx = test.builder().limits(limits).build();
        let given = args(None, None);
        let output = Hash
            .execute(
                &given,
                entry_input(&["/data/a.txt", "/data/none.txt", "/data/a.txt"]),
                &ctx,
            )
            .unwrap();
        let StageData::Records(stream) = output.data else {
            panic!("expected records");
        };
        let items: Vec<Result<Record>> = stream.collect();
        assert_eq!(items.len(), 2);
        assert!(items[0].is_ok());
        assert_eq!(items[1].as_ref().unwrap_err().code(), ErrorCode::NotFound);
    }

    #[test]
    fn content_larger_than_a_chunk_is_hashed_completely() {
        let content: Vec<u8> = (0..(3 * CHUNK_SIZE + 17))
            .map(|index| (index % 251) as u8)
            .collect();
        let test = with_files(&[("/data/large.bin", &content)]);
        let output = run(
            &test.ctx(),
            &args(Some(paths(&["/data/large.bin"])), Some("sha1")),
            StageInput::none(),
        )
        .unwrap();
        let expected = hash_reader(HashAlgorithm::Sha1, &content[..]).unwrap();
        assert_eq!(hash_of(&output[0]), Some(expected.to_string()));
    }

    #[test]
    fn an_empty_file_with_a_multibyte_name_is_hashed() {
        let name = "/data/pusty \u{142}\u{f3}d\u{17a}.txt";
        let test = with_files(&[(name, b"")]);
        let output = run(
            &test.ctx(),
            &args(Some(paths(&[name])), None),
            StageInput::none(),
        )
        .unwrap();
        assert_eq!(
            hash_of(&output[0]).as_deref(),
            Some("sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
        let entry = Entry::try_from(output[0].clone()).unwrap();
        assert_eq!(entry.name, "pusty \u{142}\u{f3}d\u{17a}.txt");
    }

    #[test]
    fn stage_form_fails_for_a_record_of_a_missing_file() {
        let test = TestContext::new();
        let error = run(
            &test.ctx(),
            &args(None, None),
            entry_input(&["/data/none.txt"]),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
    }

    #[test]
    fn cancellation_ends_the_stream_with_an_error() {
        let test = with_files(&[("/data/a.txt", b"abc")]);
        let ctx = test.ctx();
        ctx.cancel().cancel(CancelReason::Requested);
        let error = run(
            &ctx,
            &args(Some(paths(&["/data/a.txt"])), None),
            StageInput::none(),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Cancelled);
    }
}
