// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Author and committer identities of a new commit.
//!
//! The rules are those of `git commit`: the name comes from
//! `GIT_AUTHOR_NAME` (`GIT_COMMITTER_NAME`), then `author.name`
//! (`committer.name`), then `user.name`; the e-mail from
//! `GIT_AUTHOR_EMAIL` (`GIT_COMMITTER_EMAIL`), then `author.email`
//! (`committer.email`), then `user.email`, then `EMAIL`; the time from
//! `GIT_AUTHOR_DATE` (`GIT_COMMITTER_DATE`), otherwise the clock of the
//! execution context with the UTC offset of the host. An amended
//! commit keeps its author and author time; an `author` argument replaces
//! the name and e-mail only. Unlike `git`, no identity is guessed from the
//! account or the host name.

use bstr::BString;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::stage::ExecCtx;

use crate::repo::GitRepo;

const NANOSECONDS_PER_SECOND: i128 = 1_000_000_000;

/// Which identity is resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Author,
    Committer,
}

impl Role {
    const fn env_name(self) -> &'static str {
        match self {
            Self::Author => "GIT_AUTHOR_NAME",
            Self::Committer => "GIT_COMMITTER_NAME",
        }
    }

    const fn env_email(self) -> &'static str {
        match self {
            Self::Author => "GIT_AUTHOR_EMAIL",
            Self::Committer => "GIT_COMMITTER_EMAIL",
        }
    }

    const fn env_date(self) -> &'static str {
        match self {
            Self::Author => "GIT_AUTHOR_DATE",
            Self::Committer => "GIT_COMMITTER_DATE",
        }
    }

    const fn section(self) -> &'static str {
        match self {
            Self::Author => "author",
            Self::Committer => "committer",
        }
    }
}

/// Name and e-mail of a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ident {
    /// Name.
    pub(crate) name: String,
    /// E-mail address without angle brackets.
    pub(crate) email: String,
}

impl Ident {
    /// Parses `Name <email>`.
    pub(crate) fn parse(text: &str) -> Result<Self> {
        let invalid = || {
            Error::new(
                ErrorCode::InvalidValue,
                format!("author `{text}` is not in the form `Name <email>`; write for example `Ada Lovelace <ada@example.com>`"),
            )
            .with_problem("invalid_author")
            .with_found(text.to_owned())
        };
        let (name, rest) = text.split_once('<').ok_or_else(invalid)?;
        let (email, tail) = rest.split_once('>').ok_or_else(invalid)?;
        if !tail.trim().is_empty() {
            return Err(invalid());
        }
        let name = sanitize(name);
        let email = sanitize(email);
        if name.is_empty() || email.is_empty() {
            return Err(invalid());
        }
        Ok(Self { name, email })
    }
}

/// Removes the characters `git` drops from identities (`<`, `>`, line
/// feeds) and trims leading and trailing punctuation and white space.
fn sanitize(value: &str) -> String {
    let kept: String = value
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | '\n'))
        .collect();
    kept.trim_matches(is_crud).to_owned()
}

/// Characters trimmed from both ends of an identity part by `git`.
fn is_crud(c: char) -> bool {
    c <= ' ' || matches!(c, '.' | ',' | ':' | ';' | '<' | '>' | '"' | '\\' | '\'')
}

/// Author and committer of a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Identities {
    /// Author.
    pub(crate) author: gix::actor::Signature,
    /// Committer.
    pub(crate) committer: gix::actor::Signature,
}

/// Inputs of the identity resolution.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct IdentityInput<'a> {
    /// `author` argument, `Name <email>`.
    pub(crate) author: Option<&'a str>,
    /// Author of the commit being amended.
    pub(crate) amended: Option<&'a gix::actor::Signature>,
}

/// Resolves the author and the committer.
///
/// A committer without a configured name or e-mail takes the missing parts
/// from the `author` argument.
pub(crate) fn resolve(
    repo: &GitRepo,
    ctx: &ExecCtx,
    input: IdentityInput<'_>,
) -> Result<Identities> {
    let given = input.author.map(Ident::parse).transpose()?;
    let author = match (input.amended, &given) {
        (Some(amended), Some(ident)) => gix::actor::Signature {
            name: BString::from(ident.name.as_str()),
            email: BString::from(ident.email.as_str()),
            time: amended.time,
        },
        (Some(amended), None) => amended.clone(),
        (None, _) => {
            let ident = match &given {
                Some(ident) => ident.clone(),
                None => configured(repo, ctx, Role::Author, None)?,
            };
            signature(&ident, time(ctx, Role::Author)?)
        }
    };
    let fallback = given.clone().or_else(|| {
        input.amended.is_none().then(|| Ident {
            name: author.name.to_string(),
            email: author.email.to_string(),
        })
    });
    let committer = configured(repo, ctx, Role::Committer, fallback.as_ref())?;
    Ok(Identities {
        author,
        committer: signature(&committer, time(ctx, Role::Committer)?),
    })
}

fn signature(ident: &Ident, time: gix::date::Time) -> gix::actor::Signature {
    gix::actor::Signature {
        name: BString::from(ident.name.as_str()),
        email: BString::from(ident.email.as_str()),
        time,
    }
}

fn env(ctx: &ExecCtx, name: &str) -> Option<String> {
    ctx.host()
        .env_var(name)
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.is_empty())
}

fn configured(
    repo: &GitRepo,
    ctx: &ExecCtx,
    role: Role,
    fallback: Option<&Ident>,
) -> Result<Ident> {
    let config = |key: &str| {
        repo.config_string(key)
            .filter(|value| !value.trim().is_empty())
    };
    let name = env(ctx, role.env_name())
        .or_else(|| config(&format!("{}.name", role.section())))
        .or_else(|| config("user.name"))
        .map(|value| sanitize(&value))
        .filter(|value| !value.is_empty())
        .or_else(|| fallback.map(|ident| ident.name.clone()));
    let email = env(ctx, role.env_email())
        .or_else(|| config(&format!("{}.email", role.section())))
        .or_else(|| config("user.email"))
        .or_else(|| env(ctx, "EMAIL"))
        .map(|value| sanitize(&value))
        .filter(|value| !value.is_empty())
        .or_else(|| fallback.map(|ident| ident.email.clone()));
    match (name, email) {
        (Some(name), Some(email)) => Ok(Ident { name, email }),
        (name, _) => Err(missing_identity(repo, role, name.is_none())),
    }
}

fn missing_identity(repo: &GitRepo, role: Role, name_missing: bool) -> Error {
    let part = if name_missing { "name" } else { "e-mail" };
    let section = role.section();
    Error::new(
        ErrorCode::Config,
        format!(
            "the {section} {part} is not configured; set user.name and user.email in the Git configuration or give `author: \"Name <email>\"`"
        ),
    )
    .with_problem("missing_identity")
    .with_fix(format!(
        "git.commit {} message: \"...\" author: \"Name <email>\"",
        crate::repo::error::quote_path(repo.root())
    ))
}

fn time(ctx: &ExecCtx, role: Role) -> Result<gix::date::Time> {
    if let Some(value) = env(ctx, role.env_date()) {
        return gix::date::parse(&value, None).map_err(|error| {
            Error::new(
                ErrorCode::InvalidValue,
                format!(
                    "{} has the value `{value}` that is not a date: {error}; use `<seconds> <+hhmm>` or an ISO 8601 date",
                    role.env_date()
                ),
            )
            .with_problem("invalid_date")
        });
    }
    let seconds =
        i64::try_from(ctx.clock().now().unix_nanos() / NANOSECONDS_PER_SECOND).unwrap_or(0);
    let offset = jiff::tz::TimeZone::get(&ctx.host().info().timezone)
        .ok()
        .zip(jiff::Timestamp::from_second(seconds).ok())
        .map_or(0, |(zone, stamp)| zone.to_offset(stamp).seconds());
    Ok(gix::date::Time { seconds, offset })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{RepoBuilder, TestHost, test_ctx_with};

    fn open(fixture: &crate::testutil::Fixture, host: TestHost) -> (GitRepo, ExecCtx) {
        let ctx = test_ctx_with(host);
        (GitRepo::open(fixture.path(), &ctx).unwrap(), ctx)
    }

    #[test]
    fn parse_accepts_name_and_email_and_drops_crud() {
        let ident = Ident::parse("  Ada Lovelace. <ada@example.com>").unwrap();
        assert_eq!(
            (ident.name.as_str(), ident.email.as_str()),
            ("Ada Lovelace", "ada@example.com")
        );
        assert!(Ident::parse("Ada").is_err());
        assert!(Ident::parse("<ada@example.com>").is_err());
        assert!(Ident::parse("Ada <a@x> tail").is_err());
        let error = Ident::parse("Ada").unwrap_err();
        assert_eq!(error.data().problem.as_deref(), Some("invalid_author"));
    }

    #[test]
    fn environment_takes_precedence_over_configuration() {
        let fixture = RepoBuilder::new()
            .config("user.name", "Config Name")
            .config("user.email", "config@example.com")
            .build();
        let host = TestHost::new("/nonexistent-home")
            .with_var("GIT_AUTHOR_NAME", "Env Author")
            .with_var("GIT_COMMITTER_EMAIL", "committer@example.com")
            .with_var("GIT_AUTHOR_DATE", "1700000000 +0200");
        let (repo, ctx) = open(&fixture, host);
        let ids = resolve(&repo, &ctx, IdentityInput::default()).unwrap();
        assert_eq!(ids.author.name, "Env Author");
        assert_eq!(ids.author.email, "config@example.com");
        assert_eq!(ids.author.time.seconds, 1_700_000_000);
        assert_eq!(ids.author.time.offset, 7200);
        assert_eq!(ids.committer.name, "Config Name");
        assert_eq!(ids.committer.email, "committer@example.com");
        assert_eq!(ids.committer.time.seconds, RepoBuilder::EPOCH);
    }

    #[test]
    fn author_section_overrides_user_for_the_author_only() {
        let fixture = RepoBuilder::new()
            .config("user.name", "User")
            .config("user.email", "user@example.com")
            .config("author.name", "Writer")
            .build();
        let (repo, ctx) = open(&fixture, TestHost::new("/nonexistent-home"));
        let ids = resolve(&repo, &ctx, IdentityInput::default()).unwrap();
        assert_eq!(ids.author.name, "Writer");
        assert_eq!(ids.committer.name, "User");
    }

    #[test]
    fn missing_identity_is_a_configuration_error_with_a_fix() {
        let fixture = RepoBuilder::new().build();
        let (repo, ctx) = open(&fixture, TestHost::new("/nonexistent-home"));
        let error = resolve(&repo, &ctx, IdentityInput::default()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Config);
        assert_eq!(error.data().problem.as_deref(), Some("missing_identity"));
        assert!(error.data().fix[0].contains("author:"));
    }

    #[test]
    fn author_argument_also_serves_as_committer_without_configuration() {
        let fixture = RepoBuilder::new().build();
        let (repo, ctx) = open(&fixture, TestHost::new("/nonexistent-home"));
        let input = IdentityInput {
            author: Some("Bo <bo@example.com>"),
            amended: None,
        };
        let ids = resolve(&repo, &ctx, input).unwrap();
        assert_eq!(ids.author.name, "Bo");
        assert_eq!(ids.committer.email, "bo@example.com");
    }

    #[test]
    fn amend_keeps_the_author_time_and_replaces_only_name_and_email() {
        let fixture = RepoBuilder::new()
            .config("user.name", "User")
            .config("user.email", "user@example.com")
            .build();
        let (repo, ctx) = open(&fixture, TestHost::new("/nonexistent-home"));
        let old = gix::actor::Signature {
            name: "Old".into(),
            email: "old@example.com".into(),
            time: gix::date::Time {
                seconds: 42,
                offset: -3600,
            },
        };
        let kept = resolve(
            &repo,
            &ctx,
            IdentityInput {
                author: None,
                amended: Some(&old),
            },
        )
        .unwrap();
        assert_eq!(kept.author, old);
        let renamed = resolve(
            &repo,
            &ctx,
            IdentityInput {
                author: Some("New <new@example.com>"),
                amended: Some(&old),
            },
        )
        .unwrap();
        assert_eq!(renamed.author.name, "New");
        assert_eq!(renamed.author.time.seconds, 42);
    }

    #[test]
    fn invalid_date_variable_is_rejected() {
        let fixture = RepoBuilder::new()
            .config("user.name", "User")
            .config("user.email", "user@example.com")
            .build();
        let host = TestHost::new("/nonexistent-home").with_var("GIT_COMMITTER_DATE", "not a date");
        let (repo, ctx) = open(&fixture, host);
        let error = resolve(&repo, &ctx, IdentityInput::default()).unwrap_err();
        assert_eq!(error.data().problem.as_deref(), Some("invalid_date"));
    }

    #[test]
    fn email_variable_is_the_last_fallback() {
        let fixture = RepoBuilder::new().config("user.name", "User").build();
        let host = TestHost::new("/nonexistent-home").with_var("EMAIL", "mail@example.com");
        let (repo, ctx) = open(&fixture, host);
        let ids = resolve(&repo, &ctx, IdentityInput::default()).unwrap();
        assert_eq!(ids.author.email, "mail@example.com");
    }
}
