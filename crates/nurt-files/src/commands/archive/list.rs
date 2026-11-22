// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! `archive.list`: lists the members of an archive.
//!
//! The format is recognised from the content of the file (zip, including
//! jar, whl and apk, tar, compressed tar and 7z), never from its name. The
//! records are produced in archive order; a compressed tar archive is
//! decoded once from the start, so the members are collected before the
//! first record is produced, within the record limit of the execution.

use nurt_archive::list::list;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::{
    BoundArgs, ExecCtx, InputKinds, ReadStage, RecordStream, StageDescriptor, StageImpl,
    StageInput, StageOutput,
};
use nurt_contracts::value::Record;

use super::{Carried, PATH, archive_entry, path_filter, required_path};

const COMMAND: &str = "archive.list";
const GLOB: &str = "glob";

/// Stage of `archive.list`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ArchiveList;

impl StageImpl for ArchiveList {
    fn descriptor(&self) -> StageDescriptor {
        StageDescriptor {
            command: COMMAND.to_owned(),
            source: true,
            input: InputKinds::None,
        }
    }

    fn as_read(&self) -> Option<&dyn ReadStage> {
        Some(self)
    }
}

impl ReadStage for ArchiveList {
    fn execute<'a>(
        &'a self,
        args: &'a BoundArgs,
        _input: StageInput<'a>,
        ctx: &'a ExecCtx,
    ) -> Result<StageOutput<'a>> {
        let fs = ctx.require_fs()?;
        let archive = required_path(args, PATH, ctx)?;
        let filter = path_filter(args, GLOB)?;
        let select = filter
            .as_ref()
            .map(|filter| move |path: &str| filter.matches(path));
        let max_records = ctx.limits().max_records;
        let reader = fs.open_read(&archive)?;
        let mut members = Vec::new();
        let mut carried = Carried::default();
        let listed = list(
            reader,
            select
                .as_ref()
                .map(|select| select as &dyn Fn(&str) -> bool),
            |member| {
                if let Err(error) = ctx.checkpoint() {
                    return Err(carried.stop(error));
                }
                if super::count(members.len()) >= max_records {
                    return Err(carried.stop(record_limit(max_records)));
                }
                members.push(member);
                Ok(())
            },
        );
        let format = listed.map_err(|error| carried.resolve(&error, &archive))?;
        let records: Vec<Record> = members
            .into_iter()
            .map(|member| Record::from(archive_entry(&archive, format, member)))
            .collect();
        Ok(StageOutput::records(RecordStream::from_records(records)))
    }
}

/// `E_LIMIT` for an archive with more members than records may be held.
fn record_limit(max_records: u64) -> Error {
    Error::new(
        ErrorCode::Limit,
        format!("the archive has more than {max_records} members: narrow the listing with `glob`"),
    )
    .with_expected(serde_json::Value::from(max_records))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nurt_archive::codec::Algorithm;
    use nurt_archive::create::{CreateOptions, Input, create};
    use nurt_archive::list::Format;
    use nurt_contracts::record::{ArchiveEntry, EntryType};
    use nurt_contracts::stage::{ArgValue, Limits};
    use nurt_contracts::value::{Instant, Size, Value};
    use nurt_testkit::ctx::TestContext;

    use super::*;

    fn abs(path: &str) -> nurt_contracts::fs::AbsPath {
        nurt_testkit::vfs::test_path(path).unwrap()
    }

    fn archive(format: Format) -> Vec<u8> {
        let inputs = vec![
            Input::directory("docs").with_mode(0o755),
            Input::file("docs/a.txt", 2)
                .with_mode(0o640)
                .with_modified(Instant::from_unix_nanos(1_700_000_000_000_000_000)),
            Input::symlink("docs/latest", "a.txt"),
        ];
        create(Vec::new(), &CreateOptions::new(format), inputs, |_| {
            Ok(Box::new(&b"hi"[..]))
        })
        .unwrap()
    }

    fn args(path: &str, glob: Option<&str>) -> BoundArgs {
        let args = BoundArgs::new(COMMAND).with(
            PATH,
            ArgValue::Value(Value::Path(abs(path).to_language_string())),
            false,
        );
        match glob {
            Some(glob) => args.with(GLOB, ArgValue::Value(Value::Glob(glob.to_owned())), false),
            None => args,
        }
    }

    fn run(test: &TestContext, args: &BoundArgs) -> Result<Vec<ArchiveEntry>> {
        let ctx = test.ctx();
        let output = ArchiveList.execute(args, StageInput::none(), &ctx)?;
        let records = StageInput::from(output).into_records()?;
        Ok(records
            .map(|record| ArchiveEntry::try_from(record.unwrap()).unwrap())
            .collect())
    }

    #[test]
    fn lists_members_with_their_fields() {
        let test = TestContext::new();
        let path = abs("/data/docs.tar.gz");
        test.fs()
            .write_file(&path, &archive(Format::Tar(Some(Algorithm::Gzip))))
            .unwrap();
        let entries = run(&test, &args("/data/docs.tar.gz", None)).unwrap();
        let paths: Vec<Option<&str>> = entries.iter().map(|entry| entry.path.as_deref()).collect();
        assert_eq!(
            paths,
            [Some("docs"), Some("docs/a.txt"), Some("docs/latest")]
        );
        let file = &entries[1];
        assert_eq!(file.name, "a.txt");
        assert_eq!(file.kind, EntryType::File);
        assert_eq!(file.size, Size::from_bytes(2));
        assert_eq!(file.mode.as_deref(), Some("0640"));
        assert_eq!(file.format.as_deref(), Some("tar_gz"));
        assert_eq!(file.encrypted, Some(false));
        assert_eq!(file.archive.as_str(), path.to_language_string());
        assert_eq!(entries[2].kind, EntryType::Link);
        assert_eq!(entries[2].link_target.as_deref(), Some("a.txt"));
    }

    #[test]
    fn zip_members_carry_checksums() {
        let test = TestContext::new();
        test.fs()
            .write_file(&abs("/data/docs.bin"), &archive(Format::Zip))
            .unwrap();
        let entries = run(&test, &args("/data/docs.bin", Some("*.txt"))).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].format.as_deref(), Some("zip"));
        assert_eq!(entries[0].crc32.as_deref(), Some("d8932aac"));
    }

    #[test]
    fn content_other_than_an_archive_is_a_format_error() {
        let test = TestContext::new();
        test.fs()
            .write_file(&abs("/data/notes.zip"), b"plain text")
            .unwrap();
        let error = run(&test, &args("/data/notes.zip", None)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Format);
        assert!(error.details.contains_key("archive"));
    }

    #[test]
    fn a_missing_archive_is_not_found() {
        let test = TestContext::new();
        let error = run(&test, &args("/data/none.zip", None)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
    }

    #[test]
    fn listings_beyond_the_record_limit_fail_with_limit() {
        let test = TestContext::new();
        test.fs()
            .write_file(&abs("/data/docs.zip"), &archive(Format::Zip))
            .unwrap();
        let ctx = test
            .builder()
            .limits(Limits {
                max_records: 2,
                ..Limits::default()
            })
            .build();
        let args = args("/data/docs.zip", None);
        let error = ArchiveList
            .execute(&args, StageInput::none(), &ctx)
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::Limit);
    }

    #[test]
    fn the_path_is_required() {
        let test = TestContext::new();
        let error = run(&test, &BoundArgs::new(COMMAND)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::MissingParam);
        let stage: Arc<dyn StageImpl> = Arc::new(ArchiveList);
        assert!(stage.as_read().is_some());
        assert!(stage.descriptor().source);
    }
}
