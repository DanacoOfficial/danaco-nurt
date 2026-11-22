// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Signing and verification settings from the Git configuration, the Nurt
//! configuration and the environment.

use std::path::{Path, PathBuf};

use bstr::ByteSlice;
use nurt_contracts::env::Os;
use nurt_contracts::error::Result;
use nurt_contracts::stage::ExecCtx;

use super::error::{UNSUPPORTED_FORMAT, sign_error};
use super::{OpenPgpSource, SignatureFormat, SigningConfig, VerifyConfig};

const NANOS_PER_SECOND: i128 = 1_000_000_000;

/// Configuration keys naming signing programs; Nurt does not run them.
const PROGRAM_KEYS: [&str; 5] = [
    "gpg.program",
    "gpg.openpgp.program",
    "gpg.x509.program",
    "gpg.ssh.program",
    "gpg.ssh.defaultKeyCommand",
];

const ENV_GNUPGHOME: &str = "GNUPGHOME";
const ENV_SSH_AUTH_SOCK: &str = "SSH_AUTH_SOCK";
const ENV_GPG_PASSPHRASE_FILE: &str = "NURT_GPG_PASSPHRASE_FILE";
const ENV_SSH_PASSPHRASE_FILE: &str = "NURT_SSH_KEY_PASSPHRASE_FILE";
const ENV_APPDATA: &str = "APPDATA";
const WINDOWS_SSH_AGENT_PIPE: &str = r"\\.\pipe\openssh-ssh-agent";

impl SigningConfig {
    /// Empty settings for `format`: no key named, sources `agent` then
    /// `key_file`, creation time `created` (seconds since the Unix epoch).
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git_transport::sign::{OpenPgpSource, SignatureFormat, SigningConfig};
    ///
    /// let config = SigningConfig::new(SignatureFormat::Openpgp, 1_767_225_600);
    /// assert_eq!(config.openpgp_sources, [OpenPgpSource::Agent, OpenPgpSource::KeyFile]);
    /// assert!(config.signing_key.is_none());
    /// ```
    #[must_use]
    pub fn new(format: SignatureFormat, created: i64) -> Self {
        Self {
            format,
            signing_key: None,
            default_identity: None,
            openpgp_sources: vec![OpenPgpSource::Agent, OpenPgpSource::KeyFile],
            key_file: None,
            passphrase_file: None,
            ssh_passphrase_file: None,
            gnupg_home: None,
            agent_socket_dir: None,
            ssh_auth_sock: None,
            home: None,
            created,
            ignored_settings: Vec::new(),
        }
    }

    /// Settings for signing an object of the repository whose configuration
    /// is `git`, on behalf of `identity` (`Name <email>` of the committer or
    /// tagger).
    ///
    /// Reads `gpg.format` and `user.signingKey` from `git`, the key
    /// `git.signing.key_file` from the Nurt configuration of `ctx`, the
    /// variables `GNUPGHOME`, `SSH_AUTH_SOCK`, `NURT_GPG_PASSPHRASE_FILE` and
    /// `NURT_SSH_KEY_PASSPHRASE_FILE` from the host of `ctx`, and the time
    /// from its clock. Keys naming programs (`gpg.program`,
    /// `gpg.ssh.program`, `gpg.ssh.defaultKeyCommand` and the like) are
    /// listed in [`ignored_settings`](SigningConfig::ignored_settings).
    ///
    /// # Errors
    ///
    /// `E_GIT_SIGN` with the problem `unsupported_format` when `gpg.format`
    /// is not `openpgp`, `ssh` or `x509`.
    pub fn resolve(git: &gix::config::File, ctx: &ExecCtx, identity: Option<&str>) -> Result<Self> {
        let format = format_of(git)?;
        let env = Env::new(ctx);
        let mut config = Self::new(format, now_seconds(ctx));
        config.home = Some(env.home.clone());
        config.signing_key = git
            .string("user.signingKey")
            .map(|value| value.to_str_lossy().trim().to_owned())
            .filter(|value| !value.is_empty());
        config.default_identity = identity.map(str::to_owned);
        config.key_file = ctx
            .config()
            .git
            .signing
            .key_file
            .as_deref()
            .map(|path| expand_home(path, &env.home));
        config.passphrase_file = env.path(ENV_GPG_PASSPHRASE_FILE);
        config.ssh_passphrase_file = env.path(ENV_SSH_PASSPHRASE_FILE);
        config.gnupg_home = Some(env.gnupg_home());
        config.ssh_auth_sock = env.ssh_auth_sock();
        config.ignored_settings = PROGRAM_KEYS
            .iter()
            .filter(|key| git.string(**key).is_some())
            .map(|key| (*key).to_owned())
            .collect();
        Ok(config)
    }
}

impl VerifyConfig {
    /// Settings for verifying objects of the repository whose configuration
    /// is `git`.
    ///
    /// Reads `gpg.ssh.allowedSignersFile` and `gpg.ssh.revocationFile` from
    /// `git`, the key file `git.signing.key_file` from the Nurt
    /// configuration, `GNUPGHOME` from the host of `ctx` and the UTC offset
    /// of the host time zone at the current time.
    #[must_use]
    pub fn resolve(git: &gix::config::File, ctx: &ExecCtx) -> Self {
        let env = Env::new(ctx);
        let path_of = |key: &str| {
            git.string(key)
                .map(|value| value.to_str_lossy().trim().to_owned())
                .filter(|value| !value.is_empty())
                .map(|value| expand_home(&value, &env.home))
        };
        Self {
            gnupg_home: Some(env.gnupg_home()),
            public_key_files: ctx
                .config()
                .git
                .signing
                .key_file
                .as_deref()
                .map(|path| expand_home(path, &env.home))
                .into_iter()
                .collect(),
            allowed_signers_file: path_of("gpg.ssh.allowedSignersFile"),
            revocation_file: path_of("gpg.ssh.revocationFile"),
            signed_at: None,
            utc_offset: utc_offset(&ctx.host().info().timezone, now_seconds(ctx)),
        }
    }
}

fn format_of(git: &gix::config::File) -> Result<SignatureFormat> {
    let Some(value) = git.string("gpg.format") else {
        return Ok(SignatureFormat::Openpgp);
    };
    match gix_object::signature::Format::parse(value.as_bstr()) {
        Ok(gix_object::signature::Format::OpenPgp) => Ok(SignatureFormat::Openpgp),
        Ok(gix_object::signature::Format::Ssh) => Ok(SignatureFormat::Ssh),
        Ok(gix_object::signature::Format::X509) => Ok(SignatureFormat::X509),
        Err(_) => Err(sign_error(
            UNSUPPORTED_FORMAT,
            format!(
                "`gpg.format` has the unknown value `{}`; use `openpgp` or `ssh`",
                value.to_str_lossy()
            ),
        )),
    }
}

fn now_seconds(ctx: &ExecCtx) -> i64 {
    i64::try_from(ctx.clock().now().0 / NANOS_PER_SECOND).unwrap_or(i64::MAX)
}

fn utc_offset(timezone: &str, at: i64) -> i32 {
    let Ok(zone) = jiff::tz::TimeZone::get(timezone) else {
        return 0;
    };
    jiff::Timestamp::from_second(at).map_or(0, |instant| zone.to_offset(instant).seconds())
}

/// Expands a leading `~/` (or `~\` on Windows) to `home`, as Git does for
/// key paths.
pub(crate) fn expand_home(path: &str, home: &Path) -> PathBuf {
    if path == "~" {
        return home.to_path_buf();
    }
    path.strip_prefix("~/")
        .or_else(|| cfg!(windows).then(|| path.strip_prefix("~\\")).flatten())
        .map_or_else(|| PathBuf::from(path), |rest| home.join(rest))
}

struct Env<'a> {
    ctx: &'a ExecCtx,
    home: PathBuf,
}

impl<'a> Env<'a> {
    fn new(ctx: &'a ExecCtx) -> Self {
        Self {
            ctx,
            home: ctx.host().info().home.clone(),
        }
    }

    fn var(&self, name: &str) -> Option<String> {
        self.ctx
            .host()
            .env_var(name)
            .map(|value| value.to_string_lossy().into_owned())
            .filter(|value| !value.is_empty())
    }

    fn path(&self, name: &str) -> Option<PathBuf> {
        self.var(name).map(|value| expand_home(&value, &self.home))
    }

    fn gnupg_home(&self) -> PathBuf {
        if let Some(home) = self.path(ENV_GNUPGHOME) {
            return home;
        }
        match self.ctx.host().info().os {
            Os::Windows => self.var(ENV_APPDATA).map_or_else(
                || self.home.join("AppData").join("Roaming").join("gnupg"),
                |appdata| PathBuf::from(appdata).join("gnupg"),
            ),
            _ => self.home.join(".gnupg"),
        }
    }

    fn ssh_auth_sock(&self) -> Option<PathBuf> {
        self.path(ENV_SSH_AUTH_SOCK).or_else(|| {
            (self.ctx.host().info().os == Os::Windows)
                .then(|| PathBuf::from(WINDOWS_SSH_AGENT_PIPE))
        })
    }
}
