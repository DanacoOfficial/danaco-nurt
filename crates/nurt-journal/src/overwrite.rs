// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Selection and execution of overwrite preservation strategies.
//!
//! The default strategy keeps the previous file itself (zero copied bytes):
//! the new content is written to a staged file in the directory of the
//! target, receives the metadata of the previous file and is switched in by
//! an atomic exchange of names, by a replacement with a backup name, or by a
//! hard link to the previous file followed by a rename. The previous file
//! then lives on as the blob. This applies only when the file has a single
//! name and the staged file took over every metadata element (account and
//! group of the file, permissions, ACL, extended attributes, flags). The
//! birth time and the inode number change; that is accepted.
//!
//! When the exchange is not possible, the previous content is cloned or
//! copied into the journal before the target changes. A file with several
//! hard links is always written in place after a clone or copy, so that
//! every name sees the new content.

use nurt_contracts::config::OverwriteStrategy;
use nurt_contracts::fs::{
    AbsPath, ExtendedMeta, FsFeatures, FsResult, FsWrite, RenameMode, TxScope,
};
use nurt_contracts::journal::PreserveMode;

/// Variant keeping the previous file itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Exchange {
    /// Atomic exchange of names; the previous file ends under the staged
    /// name and is then moved into the journal.
    Names,
    /// Replacement keeping the previous file under the blob name.
    Backup,
    /// Hard link from the blob to the previous file, then a rename of the
    /// staged file over the target.
    Link,
}

impl Exchange {
    /// Preserve mode recorded for the variant.
    pub(crate) fn mode(self) -> PreserveMode {
        match self {
            Self::Names => PreserveMode::Exchange,
            Self::Backup => PreserveMode::ReplaceBackup,
            Self::Link => PreserveMode::Hardlink,
        }
    }
}

/// Preservation plan of one overwritten file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Plan {
    /// Variants keeping the previous file, in order of preference.
    pub(crate) exchange: Vec<Exchange>,
    /// The volume shares blocks between files.
    pub(crate) clone: bool,
    /// The new content is written into the existing file.
    pub(crate) in_place: bool,
}

impl Plan {
    /// Mode recorded in the prepared manifest.
    pub(crate) fn planned_mode(&self) -> PreserveMode {
        match self.exchange.first() {
            Some(variant) => variant.mode(),
            None if self.clone => PreserveMode::Clone,
            None => PreserveMode::Copy,
        }
    }

    /// The previous content must be cloned or copied before the change.
    pub(crate) fn copies_first(&self) -> bool {
        self.exchange.is_empty()
    }
}

/// Plans the preservation of an overwritten file with `links` names on a
/// volume with `features`.
pub(crate) fn plan(strategy: OverwriteStrategy, features: &FsFeatures, links: u64) -> Plan {
    let in_place = links > 1;
    let mut exchange = Vec::new();
    if strategy == OverwriteStrategy::Exchange && !in_place {
        if features.exchange {
            exchange.push(Exchange::Names);
        }
        if features.replace_with_backup {
            exchange.push(Exchange::Backup);
        }
        if features.hardlink {
            exchange.push(Exchange::Link);
        }
    }
    Plan {
        exchange,
        clone: features.reflink,
        in_place,
    }
}

/// Switches the staged file in by `variant`, leaving the previous file at
/// `blob`.
///
/// After a failure the target still holds the previous content, except
/// when the move of the previous file from the staged name into the journal
/// fails after an exchange; the error then tells the caller to exchange the
/// names back ([`Switch::Exchanged`]).
pub(crate) fn switch(
    fs: &dyn FsWrite,
    scope: &TxScope,
    variant: Exchange,
    target: &AbsPath,
    staged: &AbsPath,
    blob: &AbsPath,
) -> Result<(), Switch> {
    match variant {
        Exchange::Names => {
            fs.exchange(scope, staged, target).map_err(Switch::Before)?;
            fs.rename(scope, staged, blob, RenameMode::NoReplace)
                .map_err(Switch::Exchanged)
        }
        Exchange::Backup => fs
            .replace_with_backup(scope, target, staged, blob)
            .map_err(Switch::Before),
        Exchange::Link => {
            fs.hard_link(scope, target, blob).map_err(Switch::Before)?;
            // The second name of the previous file is durable before the
            // target name changes; otherwise a crash can persist the rename
            // without the link and lose the previous file.
            let switched = blob
                .parent()
                .map_or(Ok(()), |dir| fs.sync_dir(scope, &dir))
                .and_then(|()| fs.rename(scope, staged, target, RenameMode::Replace));
            if let Err(error) = switched {
                let _ = fs.remove_file(scope, blob);
                return Err(Switch::Before(error));
            }
            Ok(())
        }
    }
}

/// Failure of [`switch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Switch {
    /// Nothing changed: the target holds the previous content.
    Before(nurt_contracts::fs::FsError),
    /// The names were exchanged: the target holds the new content and the
    /// staged name the previous content.
    Exchanged(nurt_contracts::fs::FsError),
}

/// Puts the previous file back after a switch whose result was rejected:
/// the new content moves to the staged name, the blob back to the target.
pub(crate) fn revert(
    fs: &dyn FsWrite,
    scope: &TxScope,
    target: &AbsPath,
    staged: &AbsPath,
    blob: &AbsPath,
) -> FsResult<()> {
    fs.rename(scope, target, staged, RenameMode::NoReplace)?;
    fs.rename(scope, blob, target, RenameMode::NoReplace)
}

/// Whether the security-relevant metadata of the switched-in file equals the
/// metadata of the previous file: account identifier, ACL and alternate
/// streams, which a replacement call may fail to carry over.
pub(crate) fn security_kept(before: &ExtendedMeta, after: &ExtendedMeta) -> bool {
    before.account_sid == after.account_sid
        && before.acl == after.acl
        && stream_names(before) == stream_names(after)
}

/// Names of the alternate streams of an entry, sorted.
fn stream_names(meta: &ExtendedMeta) -> Vec<&str> {
    let mut names: Vec<&str> = meta
        .streams
        .iter()
        .map(|stream| stream.name.as_str())
        .collect();
    names.sort_unstable();
    names
}

#[cfg(test)]
mod tests {
    use nurt_contracts::fs::{AlternateStream, Follow, Fs, FsErrorKind, ScopePurpose};
    use nurt_contracts::journal::OpId;

    use super::*;
    use crate::store::{scope, test_support::Fixture};

    fn features(exchange: bool, backup: bool, hardlink: bool, reflink: bool) -> FsFeatures {
        FsFeatures {
            exchange,
            replace_with_backup: backup,
            hardlink,
            reflink,
            ..FsFeatures::default()
        }
    }

    #[test]
    fn exchange_strategy_prefers_keeping_the_previous_file() {
        let plan = plan(
            OverwriteStrategy::Exchange,
            &features(true, false, true, true),
            1,
        );
        assert_eq!(plan.exchange, vec![Exchange::Names, Exchange::Link]);
        assert_eq!(plan.planned_mode(), PreserveMode::Exchange);
        assert!(!plan.copies_first());
        let windows = super::plan(
            OverwriteStrategy::Exchange,
            &features(false, true, true, false),
            1,
        );
        assert_eq!(windows.planned_mode(), PreserveMode::ReplaceBackup);
        let link = super::plan(
            OverwriteStrategy::Exchange,
            &features(false, false, true, false),
            1,
        );
        assert_eq!(link.planned_mode(), PreserveMode::Hardlink);
    }

    #[test]
    fn several_links_or_clone_or_copy_strategy_copy_first() {
        let shared = plan(
            OverwriteStrategy::Exchange,
            &features(true, true, true, false),
            2,
        );
        assert!(shared.in_place && shared.copies_first());
        assert_eq!(shared.planned_mode(), PreserveMode::Copy);
        let cloning = plan(
            OverwriteStrategy::CloneOrCopy,
            &features(true, true, true, true),
            1,
        );
        assert!(!cloning.in_place && cloning.copies_first());
        assert_eq!(cloning.planned_mode(), PreserveMode::Clone);
        assert_eq!(Exchange::Link.mode(), PreserveMode::Hardlink);
    }

    #[test]
    fn without_any_feature_the_previous_content_is_copied() {
        let bare = plan(
            OverwriteStrategy::Exchange,
            &features(false, false, false, false),
            1,
        );
        assert!(bare.exchange.is_empty());
        assert!(!bare.in_place);
        assert!(bare.copies_first());
        assert_eq!(bare.planned_mode(), PreserveMode::Copy);
    }

    #[test]
    fn link_counts_decide_between_exchange_and_writing_in_place() {
        let every_feature = features(true, true, true, true);
        for links in [0, 1] {
            let single = plan(OverwriteStrategy::Exchange, &every_feature, links);
            assert!(!single.in_place, "{links}");
            assert_eq!(
                single.exchange,
                [Exchange::Names, Exchange::Backup, Exchange::Link]
            );
        }
        for links in [2, 3, u64::MAX] {
            let shared = plan(OverwriteStrategy::Exchange, &every_feature, links);
            assert!(shared.in_place, "{links}");
            assert!(shared.copies_first());
            assert_eq!(shared.planned_mode(), PreserveMode::Clone);
        }
    }

    #[test]
    fn every_variant_records_its_own_preserve_mode() {
        assert_eq!(Exchange::Names.mode(), PreserveMode::Exchange);
        assert_eq!(Exchange::Backup.mode(), PreserveMode::ReplaceBackup);
        assert_eq!(Exchange::Link.mode(), PreserveMode::Hardlink);
    }

    #[test]
    fn the_security_check_ignores_the_order_and_size_of_alternate_streams() {
        let fixture = Fixture::new();
        let path = fixture.fs.path("vol-a/streams.txt");
        fixture.write(&path, b"x");
        let stream = |name: &str, size: u64| AlternateStream {
            name: name.to_owned(),
            size,
        };
        let mut before = fixture
            .fs
            .extended_metadata(&path, Follow::NoFollow)
            .unwrap();
        before.streams = vec![stream("a", 1), stream("b", 2)];
        let mut after = before.clone();
        after.streams = vec![stream("b", 9), stream("a", 8)];
        assert!(security_kept(&before, &after));
        after.streams = vec![stream("a", 1)];
        assert!(!security_kept(&before, &after));
        after.streams = vec![stream("a", 1), stream("c", 2)];
        assert!(!security_kept(&before, &after));
    }

    #[test]
    fn switches_keep_the_previous_file_as_blob() {
        for variant in [Exchange::Names, Exchange::Link, Exchange::Backup] {
            let fixture = Fixture::new();
            let fs = &fixture.fs;
            fs.configure(0, |volume| volume.features.replace_with_backup = true);
            let scope = scope(OpId(1), ScopePurpose::Operation);
            let target = fs.path("vol-a/a.txt");
            let staged = fs.path("vol-a/.nurt-tmp-1-0");
            let blob = fs.path("vol-a/blob");
            fixture.write(&target, b"old");
            fixture.write(&staged, b"new");
            let before = fs.metadata(&target, Follow::NoFollow).unwrap().file_id;
            switch(fs, &scope, variant, &target, &staged, &blob).unwrap();
            assert_eq!(fixture.read(&target), b"new", "{variant:?}");
            assert_eq!(fixture.read(&blob), b"old");
            assert_eq!(
                fs.metadata(&blob, Follow::NoFollow).unwrap().file_id,
                before
            );
            if variant != Exchange::Link {
                revert(fs, &scope, &target, &staged, &blob).unwrap();
                assert_eq!(fixture.read(&target), b"old");
                assert_eq!(fixture.read(&staged), b"new");
            }
            assert_eq!(fs.copied(), 0);
        }
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn hard_link_is_durable_before_the_target_changes() {
        let fixture = Fixture::new();
        let fs = &fixture.fs;
        let scope = scope(OpId(1), ScopePurpose::Operation);
        let target = fs.path("vol-a/a.txt");
        let staged = fs.path("vol-a/.nurt-tmp-1-0");
        let blobs = fs.path("vol-a/blobs");
        std::fs::create_dir(blobs.as_path()).unwrap();
        let blob = blobs.join_name("b").unwrap();
        fixture.write(&target, b"old");
        fixture.write(&staged, b"new");
        fs.clear_log();
        switch(fs, &scope, Exchange::Link, &target, &staged, &blob).unwrap();
        let log = fs.log();
        let link = log
            .iter()
            .position(|e| e.starts_with("hard_link "))
            .unwrap();
        let rename = log.iter().position(|e| e.starts_with("rename ")).unwrap();
        let flush = format!("sync_dir {blobs}");
        assert!(log[link..rename].contains(&flush), "{log:#?}");
        assert_eq!(fixture.read(&blob), b"old");
    }

    #[test]
    fn failed_switches_report_where_the_previous_content_is() {
        let fixture = Fixture::new();
        let fs = &fixture.fs;
        let scope = scope(OpId(1), ScopePurpose::Operation);
        let target = fs.path("vol-a/a.txt");
        let staged = fs.path("vol-a/.nurt-tmp-1-0");
        let blob = fs.path("vol-a/blob");
        fixture.write(&target, b"old");
        fixture.write(&staged, b"new");
        fs.fail("rename", "blob", FsErrorKind::Io);
        let result = switch(fs, &scope, Exchange::Names, &target, &staged, &blob);
        assert!(matches!(result, Err(Switch::Exchanged(_))));
        assert_eq!(fixture.read(&staged), b"old");
        fs.exchange(&scope, &staged, &target).unwrap();
        fs.fail("rename", "a.txt", FsErrorKind::Io);
        let result = switch(fs, &scope, Exchange::Link, &target, &staged, &blob);
        assert!(matches!(result, Err(Switch::Before(_))));
        assert_eq!(fixture.read(&target), b"old");
        assert!(!fixture.exists(&blob));
        let result = switch(fs, &scope, Exchange::Backup, &target, &staged, &blob);
        assert!(
            matches!(result, Err(Switch::Before(error)) if error.kind == FsErrorKind::Unsupported)
        );
    }

    #[test]
    fn security_check_compares_account_acl_and_streams() {
        let fixture = Fixture::new();
        let path = fixture.fs.path("vol-a/a.txt");
        fixture.write(&path, b"x");
        let before = fixture
            .fs
            .extended_metadata(&path, Follow::NoFollow)
            .unwrap();
        assert!(security_kept(&before, &before));
        let mut other = before.clone();
        other.account_sid = Some("S-1-5-21-1".to_owned());
        assert!(!security_kept(&before, &other));
        let mut streams = before.clone();
        streams.streams.push(AlternateStream {
            name: "Zone.Identifier".to_owned(),
            size: 1,
        });
        assert!(!security_kept(&before, &streams));
    }
}
