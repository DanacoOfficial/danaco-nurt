// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Author and committer identity of new commits and reflog entries.

use std::collections::BTreeMap;

use gix::actor::Signature;
use gix::bstr::BString;
use gix::date::Time;
use nurt_contracts::stage::ExecCtx;

use super::repo::Repo;

/// Seconds in a minute and in an hour, for time zone offsets.
const SECONDS_PER_MINUTE: i32 = 60;
const SECONDS_PER_HOUR: i32 = 3600;

/// Nanoseconds in a second.
const NANOS_PER_SECOND: i128 = 1_000_000_000;

/// Environment variables that determine identities and dates.
const IDENTITY_VARS: [&str; 6] = [
    "GIT_AUTHOR_NAME",
    "GIT_AUTHOR_EMAIL",
    "GIT_AUTHOR_DATE",
    "GIT_COMMITTER_NAME",
    "GIT_COMMITTER_EMAIL",
    "GIT_COMMITTER_DATE",
];

/// Facts of the execution context that determine the identity and time of
/// new commits and reflog entries.
///
/// # Examples
///
/// ```
/// use nurt_git_ops::reset::Environment;
///
/// let env = Environment::fixed(1_767_265_200, 3600)
///     .with_var("GIT_COMMITTER_NAME", "C")
///     .with_var("GIT_COMMITTER_EMAIL", "c@example.com");
/// assert_eq!(env.var("GIT_COMMITTER_NAME"), Some("C"));
/// assert_eq!(env.var("GIT_AUTHOR_NAME"), None);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Environment {
    vars: BTreeMap<String, String>,
    now: i64,
    offset: i32,
    user: String,
    host: String,
}

impl Environment {
    /// Environment with the given time and local offset in seconds, no
    /// variables and the account `user` on host `localhost`.
    #[must_use]
    pub fn fixed(now: i64, offset: i32) -> Self {
        Self {
            vars: BTreeMap::new(),
            now,
            offset,
            user: "user".to_owned(),
            host: "localhost".to_owned(),
        }
    }

    /// Environment of an execution context: the identity variables of the
    /// process environment, the clock and the local time zone of the host.
    #[must_use]
    pub fn from_ctx(ctx: &ExecCtx) -> Self {
        let host = ctx.host();
        let vars = IDENTITY_VARS
            .iter()
            .filter_map(|name| {
                let value = host.env_var(name)?.into_string().ok()?;
                Some(((*name).to_owned(), value))
            })
            .collect();
        let nanos = ctx.clock().now().unix_nanos();
        let now = i64::try_from(nanos.div_euclid(NANOS_PER_SECOND)).unwrap_or_default();
        let info = host.info();
        let offset = jiff::tz::TimeZone::get(&info.timezone)
            .ok()
            .zip(jiff::Timestamp::from_second(now).ok())
            .map_or(0, |(zone, instant)| zone.to_offset(instant).seconds());
        Self {
            vars,
            now,
            offset,
            user: info.user.clone(),
            host: info.hostname.clone(),
        }
    }

    /// Adds or replaces a variable.
    #[must_use]
    pub fn with_var(mut self, name: &str, value: &str) -> Self {
        self.vars.insert(name.to_owned(), value.to_owned());
        self
    }

    /// Value of a variable.
    #[must_use]
    pub fn var(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str)
    }
}

/// Identity and time used for new objects and reflog entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Identity {
    /// Author of new commits.
    pub(crate) author: Signature,
    /// Committer of new commits and author of reflog entries.
    pub(crate) committer: Signature,
}

impl Identity {
    /// Identity from `GIT_AUTHOR_*` and `GIT_COMMITTER_*`, then `user.name`
    /// and `user.email` of the repository, then the account and host names.
    pub(crate) fn resolve(repo: &Repo, env: &Environment) -> Self {
        let config = repo.repo.config_snapshot();
        let default_name = config
            .string("user.name")
            .map_or_else(|| env.user.clone(), |value| value.to_string());
        let default_email = config
            .string("user.email")
            .map_or_else(|| format!("{}@{}", env.user, env.host), |value| value.to_string());
        let now = Time::new(env.now, env.offset);
        let pick = |role: &str| -> Signature {
            let name = env
                .var(&format!("GIT_{role}_NAME"))
                .map_or_else(|| default_name.clone(), ToOwned::to_owned);
            let email = env
                .var(&format!("GIT_{role}_EMAIL"))
                .map_or_else(|| default_email.clone(), ToOwned::to_owned);
            let time = env
                .var(&format!("GIT_{role}_DATE"))
                .and_then(|text| gix::date::parse(text, None).ok())
                .unwrap_or(now);
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

/// Time as written in commit headers and reflog entries: seconds and the
/// offset as `±hhmm`.
pub(crate) fn format_time(time: Time) -> String {
    let sign = if time.offset < 0 { '-' } else { '+' };
    let offset = time.offset.abs();
    let hours = offset / SECONDS_PER_HOUR;
    let minutes = (offset % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE;
    format!("{} {sign}{hours:02}{minutes:02}", time.seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_time_writes_signed_offset() {
        assert_eq!(format_time(Time::new(10, 3600)), "10 +0100");
        assert_eq!(format_time(Time::new(10, -5400)), "10 -0130");
        assert_eq!(format_time(Time::new(0, 0)), "0 +0000");
    }

    #[test]
    fn environment_variables_replace_each_other() {
        let env = Environment::fixed(0, 0).with_var("A", "1").with_var("A", "2");
        assert_eq!(env.var("A"), Some("2"));
    }
}
