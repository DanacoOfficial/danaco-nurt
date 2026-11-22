// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! OpenPGP and SSH signatures of commits and tags.
//!
//! [`Signer`] creates detached signatures over the bytes of a commit or tag
//! object and verifies them. [`BuiltinSigner`] implements it without running
//! a program: OpenPGP signatures come from a running `gpg-agent` reached
//! through its socket or from a secret key file, SSH signatures (`SSHSIG`,
//! namespace `git`) from an SSH agent or a private key file. The armored
//! output has the form written by `gpg --detach-sign --armor` and
//! `ssh-keygen -Y sign`.
//!
//! [`sign_commit`] and [`sign_tag`] place a signature where Git expects it:
//! in the `gpgsig` header (`gpgsig-sha256` in SHA-256 repositories) of a
//! commit and after the message of an annotated tag. [`verify_commit`] and
//! [`verify_tag`] check an object read from a repository.
//!
//! SSH signatures require the build feature `git-ssh`; without it, a
//! configuration with `gpg.format = ssh` fails with `E_GIT_SIGN` and
//! verification reports the status `unsupported`.
//!
//! # Examples
//!
//! ```
//! use nurt_git_transport::sign::{BuiltinSigner, Signer, SignatureFormat, SigningConfig};
//!
//! let config = SigningConfig::new(SignatureFormat::X509, 1_767_225_600);
//! let error = BuiltinSigner.sign(b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n", &config);
//! assert_eq!(error.unwrap_err().data().problem.as_deref(), Some("unsupported_format"));
//! ```

mod config;
mod error;
mod object;
mod openpgp;
#[cfg(feature = "git-ssh")]
mod sshsig;

use std::path::PathBuf;

use nurt_contracts::error::Result;

pub use nurt_contracts::record::{CommitSignature, SignatureFormat, SignatureStatus};
pub use object::{sign_commit, sign_tag, verify_commit, verify_tag};

/// Place an OpenPGP secret key is taken from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpenPgpSource {
    /// A running `gpg-agent`, reached through its socket in the GnuPG home
    /// directory or the user runtime directory.
    Agent,
    /// The secret key file named by `git.signing.key_file`.
    KeyFile,
}

/// Where the secret key of a created signature was held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeySource {
    /// A key agent (`gpg-agent` or an SSH agent).
    Agent,
    /// A key file.
    KeyFile,
}

/// Settings of one signing operation.
///
/// [`SigningConfig::resolve`] reads them from the Git configuration of a
/// repository, the Nurt configuration and the environment of the execution
/// context; [`SigningConfig::new`] starts from empty settings.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SigningConfig {
    /// Signature format (`gpg.format`).
    pub format: SignatureFormat,
    /// Key named by `user.signingKey`. OpenPGP: a fingerprint, key ID, e-mail
    /// address or user ID; a trailing `!` selects exactly that (sub)key. SSH:
    /// the path of a private or public key file, or `key::` followed by a
    /// public key literal.
    pub signing_key: Option<String>,
    /// Identity used for OpenPGP when no key is named: the committer or
    /// tagger as `Name <email>`.
    pub default_identity: Option<String>,
    /// OpenPGP key sources, tried in order.
    pub openpgp_sources: Vec<OpenPgpSource>,
    /// OpenPGP secret key file (`git.signing.key_file`), ASCII-armored or
    /// binary.
    pub key_file: Option<PathBuf>,
    /// File holding the passphrase of the OpenPGP key file
    /// (`NURT_GPG_PASSPHRASE_FILE`).
    pub passphrase_file: Option<PathBuf>,
    /// File holding the passphrase of an encrypted SSH private key
    /// (`NURT_SSH_KEY_PASSPHRASE_FILE`).
    pub ssh_passphrase_file: Option<PathBuf>,
    /// GnuPG home directory (`GNUPGHOME`, otherwise `~/.gnupg`; on Windows
    /// `%APPDATA%\gnupg`).
    pub gnupg_home: Option<PathBuf>,
    /// Directory of the `gpg-agent` socket when it is known in advance; the
    /// standard locations are searched otherwise.
    pub agent_socket_dir: Option<PathBuf>,
    /// SSH agent socket or Windows named pipe (`SSH_AUTH_SOCK`).
    pub ssh_auth_sock: Option<PathBuf>,
    /// Home directory used to expand `~/` in key paths.
    pub home: Option<PathBuf>,
    /// Creation time of the signature, seconds since the Unix epoch.
    pub created: i64,
    /// Configuration keys that name programs and were therefore not used,
    /// for example `gpg.program`. The caller reports them as notable.
    pub ignored_settings: Vec<String>,
}

/// Settings of signature verification.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct VerifyConfig {
    /// GnuPG home directory whose public keyrings are searched.
    pub gnupg_home: Option<PathBuf>,
    /// Additional OpenPGP public or secret key files, ASCII-armored or binary.
    pub public_key_files: Vec<PathBuf>,
    /// SSH allowed signers file (`gpg.ssh.allowedSignersFile`).
    pub allowed_signers_file: Option<PathBuf>,
    /// SSH revoked keys file (`gpg.ssh.revocationFile`), one public key per
    /// line.
    pub revocation_file: Option<PathBuf>,
    /// Time the signed object claims, seconds since the Unix epoch; checked
    /// against `valid-after` and `valid-before` of allowed signers.
    pub signed_at: Option<i64>,
    /// Offset of the local time zone in seconds east of UTC; applies to
    /// `valid-after` and `valid-before` values without the `Z` suffix.
    pub utc_offset: i32,
}

/// A created signature.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Signature {
    /// Format of the signature.
    pub format: SignatureFormat,
    /// ASCII-armored signature with LF line endings, ending in a newline.
    pub armored: Vec<u8>,
    /// Key that signed: the OpenPGP fingerprint in upper-case hexadecimal or
    /// the SSH fingerprint `SHA256:…`.
    pub key: String,
    /// Where the secret key was held.
    pub source: KeySource,
}

/// Creation and verification of detached signatures.
///
/// Implementations never run a program. The payload is the exact byte
/// sequence covered by the signature; [`sign_commit`] and [`sign_tag`]
/// build it from an object.
pub trait Signer: Send + Sync {
    /// Signs `payload` with the key selected by `config`.
    ///
    /// # Errors
    ///
    /// `E_GIT_SIGN` with the problem `no_key` (no key configured or found),
    /// `agent_unavailable` (no agent answers on the expected socket),
    /// `key_locked` (an encrypted key without a usable passphrase) or
    /// `unsupported_format` (`x509`, or a key algorithm that cannot sign).
    /// Nothing is written in any of these cases.
    fn sign(&self, payload: &[u8], config: &SigningConfig) -> Result<Signature>;

    /// Verifies `signature`, an armored signature, over `payload`.
    ///
    /// The status is `good`, `bad`, `unknown_key` when no trusted key
    /// matches the signature, or `unsupported` for formats and algorithms
    /// that cannot be checked.
    ///
    /// # Errors
    ///
    /// `E_IO` when a configured key file or allowed signers file exists but
    /// cannot be read.
    fn verify(
        &self,
        payload: &[u8],
        signature: &[u8],
        config: &VerifyConfig,
    ) -> Result<CommitSignature>;
}

/// The [`Signer`] of Nurt: OpenPGP through `gpg-agent` or a key file, SSH
/// through an SSH agent or a key file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BuiltinSigner;

impl Signer for BuiltinSigner {
    fn sign(&self, payload: &[u8], config: &SigningConfig) -> Result<Signature> {
        match config.format {
            SignatureFormat::Openpgp => openpgp::sign(payload, config),
            #[cfg(feature = "git-ssh")]
            SignatureFormat::Ssh => sshsig::sign(payload, config),
            other => Err(error::unsupported_format(other)),
        }
    }

    fn verify(
        &self,
        payload: &[u8],
        signature: &[u8],
        config: &VerifyConfig,
    ) -> Result<CommitSignature> {
        match gix_object::signature::Format::from_signature(signature) {
            Some(gix_object::signature::Format::OpenPgp) => {
                openpgp::verify(payload, signature, config)
            }
            #[cfg(feature = "git-ssh")]
            Some(gix_object::signature::Format::Ssh) => sshsig::verify(payload, signature, config),
            #[cfg(not(feature = "git-ssh"))]
            Some(gix_object::signature::Format::Ssh) => {
                Ok(unsupported(Some(SignatureFormat::Ssh)))
            }
            Some(gix_object::signature::Format::X509) => {
                Ok(unsupported(Some(SignatureFormat::X509)))
            }
            None => Ok(unsupported(None)),
        }
    }
}

fn unsupported(format: Option<SignatureFormat>) -> CommitSignature {
    CommitSignature {
        status: SignatureStatus::Unsupported,
        format,
        key: None,
        signer: None,
    }
}

#[cfg(test)]
mod tests;
