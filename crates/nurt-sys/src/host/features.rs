// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-sys::host::features
// Responsibility:   Closed vocabulary of the `features` field of the `host` record.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Closed vocabulary of the `features` field of the `host` record.

use std::path::Path;

use nurt_contracts::fs::AbsPath;
use nurt_contracts::record::ServiceAvailability;
use nurt_contracts::stage::ExecCtx;

/// Feature reported in `host.features`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum HostFeature {
    /// The operation journal is available, so changes can be undone.
    Journal,
    /// The background service is running.
    Service,
    /// The volume of the home directory clones blocks (`FICLONE`,
    /// `clonefile`, block cloning).
    Reflink,
    /// The volume of the home directory supports hard links.
    Hardlink,
    /// The volume of the home directory exchanges two names atomically.
    Exchange,
    /// The volume of the home directory stores extended attributes.
    Xattr,
    /// The volume of the home directory stores access control lists.
    Acl,
    /// Names on the volume of the home directory differ by case.
    CaseSensitive,
}

impl HostFeature {
    /// Name of the feature in the record.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Journal => "journal",
            Self::Service => "service",
            Self::Reflink => "reflink",
            Self::Hardlink => "hardlink",
            Self::Exchange => "exchange",
            Self::Xattr => "xattr",
            Self::Acl => "acl",
            Self::CaseSensitive => "case_sensitive",
        }
    }
}

/// Features available to programs, in the order of [`HostFeature`].
pub(crate) fn detect(ctx: &ExecCtx, home: &Path, service: ServiceAvailability) -> Vec<String> {
    let mut found = Vec::new();
    if ctx.journal().is_some() {
        found.push(HostFeature::Journal);
    }
    if service == ServiceAvailability::Running {
        found.push(HostFeature::Service);
    }
    let volume = ctx
        .fs()
        .zip(AbsPath::new(home).ok())
        .and_then(|(fs, home)| fs.volume(&home).ok());
    if let Some(volume) = volume {
        let features = volume.features;
        let flags = [
            (features.reflink, HostFeature::Reflink),
            (features.hardlink, HostFeature::Hardlink),
            (features.exchange, HostFeature::Exchange),
            (features.xattr, HostFeature::Xattr),
            (features.acl, HostFeature::Acl),
            (features.case_sensitive, HostFeature::CaseSensitive),
        ];
        found.extend(
            flags
                .into_iter()
                .filter(|(on, _)| *on)
                .map(|(_, feature)| feature),
        );
    }
    found
        .into_iter()
        .map(|feature| feature.as_str().to_owned())
        .collect()
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
