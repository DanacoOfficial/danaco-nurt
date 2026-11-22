// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Author and committer identity of new commits and reflog entries.

use gix::actor::Signature;
use gix::bstr::BString;
use gix::date::Time;
use nurt_git::repo::GitRepo;

/// Nanoseconds in a second.
const NANOS_PER_SECOND: i128 = 1_000_000_000;

/// Identity and time used for new objects and reflog entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Identity {
    /// Author of new commits.
    pub(crate) author: Signature,
    /// Committer of new commits and author of reflog entries.
    pub(crate) committer: Signature,
}

impl Identity {
    /// Identity from `GIT_AUTHOR_*` and `GIT_COMMITTER_*` of the execution
    /// context, then `user.name` and `user.email` of the configuration,
    /// then the account and host names; the time is the clock of the
    /// context in the local time zone of the host.
    pub(crate) fn resolve(repo: &GitRepo) -> Self {
        let ctx = repo.ctx();
        let host = ctx.host();
        let info = host.info();
        let var = |name: String| {
            host.env_var(&name)
                .and_then(|value| value.into_string().ok())
        };
        let default_name = repo
            .config_string("user.name")
            .unwrap_or_else(|| info.user.clone());
        let default_email = repo
            .config_string("user.email")
            .unwrap_or_else(|| format!("{}@{}", info.user, info.hostname));
        let nanos = ctx.clock().now().unix_nanos();
        let now = i64::try_from(nanos.div_euclid(NANOS_PER_SECOND)).unwrap_or_default();
        let offset = jiff::tz::TimeZone::get(&info.timezone)
            .ok()
            .zip(jiff::Timestamp::from_second(now).ok())
            .map_or(0, |(zone, instant)| zone.to_offset(instant).seconds());
        let pick = |role: &str| -> Signature {
            let name = var(format!("GIT_{role}_NAME")).unwrap_or_else(|| default_name.clone());
            let email = var(format!("GIT_{role}_EMAIL")).unwrap_or_else(|| default_email.clone());
            let time = var(format!("GIT_{role}_DATE"))
                .and_then(|text| gix::date::parse(&text, None).ok())
                .unwrap_or_else(|| Time::new(now, offset));
            Signature {
                name: BString::from(name),
                email: BString::from(email),
                time,
            }
        };
        Self {
            author: pick("AUTHOR"),
            committer: pick("COMMITTER"),
        }
    }
}
