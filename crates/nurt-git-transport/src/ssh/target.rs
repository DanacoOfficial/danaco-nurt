// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Remote address and remote command of an SSH repository URL.

use bstr::{BStr, BString, ByteSlice, ByteVec};
use gix_transport::Service;
use nurt_contracts::error::{Error, ErrorCode, Result};

/// Host, account and repository path taken from an `ssh://` or
/// `[user@]host:path` URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    /// Account given in the URL.
    pub(crate) user: Option<String>,
    /// Host as written in the URL, before `ssh_config` substitution.
    pub(crate) host: String,
    /// Port given in the URL.
    pub(crate) port: Option<u16>,
    /// Repository path in the form passed to the remote command: `~` and
    /// `~user` prefixes are kept for the remote side to expand.
    pub(crate) path: BString,
}

impl Target {
    /// Extracts the target of an SSH URL.
    ///
    /// Host names, accounts and paths that start with `-` are refused, since
    /// a server could read them as options.
    pub(crate) fn from_url(url: &gix_url::Url) -> Result<Self> {
        if url.scheme != gix_url::Scheme::Ssh {
            return Err(Error::new(
                ErrorCode::GitUnsupported,
                format!("url scheme `{}` is not ssh", url.scheme.as_str()),
            )
            .with_problem("not_ssh"));
        }
        let host = url
            .host()
            .filter(|host| !host.is_empty())
            .ok_or_else(|| invalid_url("ssh url has no host name"))?;
        if host.starts_with('-') {
            return Err(invalid_url("host name of the ssh url starts with `-`"));
        }
        let user = url.user().filter(|user| !user.is_empty());
        if user.is_some_and(|user| user.starts_with('-')) {
            return Err(invalid_url("account of the ssh url starts with `-`"));
        }
        let path = gix_url::expand_path::for_shell(url.path.clone());
        if path.trim().first() == Some(&b'-') {
            return Err(invalid_url("repository path of the ssh url starts with `-`"));
        }
        if path.is_empty() {
            return Err(invalid_url("ssh url has no repository path"));
        }
        Ok(Self {
            user: user.map(str::to_owned),
            host: host.to_owned(),
            port: url.port,
            path,
        })
    }
}

/// Command line executed on the server, for example
/// `git-upload-pack '/srv/repo.git'`.
///
/// The path is quoted for a POSIX shell like the `git` program quotes it:
/// single quotes, with `'` and `!` written as `'\''` and `'\!'`.
pub(crate) fn remote_command(service: Service, path: &BStr) -> BString {
    let mut command = BString::from(service.as_str());
    command.push_byte(b' ');
    command.push_str(gix_quote::single(path));
    command
}

fn invalid_url(message: &str) -> Error {
    Error::new(ErrorCode::Arg, message).with_problem("invalid_url")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(url: &str) -> Result<Target> {
        let url = gix_url::parse(url.into()).map_err(|err| invalid_url(&err.to_string()))?;
        Target::from_url(&url)
    }

    #[test]
    fn reads_url_with_user_and_port() {
        let target = target("ssh://git@example.com:2222/srv/repo.git").unwrap();
        assert_eq!(target.user.as_deref(), Some("git"));
        assert_eq!(target.host, "example.com");
        assert_eq!(target.port, Some(2222));
        assert_eq!(target.path, "/srv/repo.git");
    }

    #[test]
    fn reads_scp_form_with_relative_path() {
        let target = target("git@example.com:team/repo.git").unwrap();
        assert_eq!(target.user.as_deref(), Some("git"));
        assert_eq!(target.host, "example.com");
        assert_eq!(target.port, None);
        assert_eq!(target.path, "team/repo.git");
    }

    #[test]
    fn keeps_home_prefix_for_the_server() {
        assert_eq!(target("ssh://example.com/~/repo.git").unwrap().path, "~/repo.git");
        assert_eq!(target("ssh://example.com/~alice/repo.git").unwrap().path, "~alice/repo.git");
    }

    #[test]
    fn rejects_other_schemes() {
        let error = target("https://example.com/repo.git").unwrap_err();
        assert_eq!(error.code(), ErrorCode::GitUnsupported);
    }

    #[test]
    fn rejects_host_that_looks_like_an_option() {
        let error = target("ssh://-oProxyCommand=x/repo").unwrap_err();
        assert_eq!(error.code(), ErrorCode::Arg);
    }

    #[test]
    fn quotes_path_like_git() {
        let command = remote_command(Service::UploadPack, "/srv/it's here!.git".into());
        assert_eq!(command, r"git-upload-pack '/srv/it'\''s here'\!'.git'");
        let command = remote_command(Service::ReceivePack, "repo.git".into());
        assert_eq!(command, "git-receive-pack 'repo.git'");
    }
}
