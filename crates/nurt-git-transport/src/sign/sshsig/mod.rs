// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! SSH signatures (`SSHSIG`, namespace `git`, digest SHA-512).
//!
//! `user.signingKey` names a private key file, a public key file whose
//! private key is held by an SSH agent, or a public key literal after
//! `key::`. An unencrypted private key file signs directly. An encrypted
//! one is used through the agent when the agent holds it, otherwise with
//! the passphrase from `NURT_SSH_KEY_PASSPHRASE_FILE`. A public key file
//! whose agent is not reachable falls back to the private key file of the
//! same name without `.pub`.
//!
//! Verification follows `ssh-keygen -Y verify` as Git calls it: the
//! signature must be valid, its key must not be listed in the revocation
//! file, and an entry of the allowed signers file must accept the key for
//! the namespace `git` at the time the object claims.

mod agent;
mod allowed;

use std::path::{Path, PathBuf};

use nurt_contracts::error::{Error, Result};
use russh::keys::ssh_key::{HashAlg, LineEnding, PrivateKey, PublicKey, SshSig};

use super::config::expand_home;
use super::error::{
    is_problem, key_locked, no_key, sign_error, with_path, AGENT_UNAVAILABLE, NO_KEY,
    UNSUPPORTED_FORMAT,
};
use super::openpgp::read_passphrase;
use super::{
    CommitSignature, KeySource, Signature, SignatureFormat, SignatureStatus, SigningConfig,
    VerifyConfig,
};

pub(crate) const NAMESPACE: &str = "git";
const LITERAL_PREFIX: &str = "key::";
const PUBLIC_KEY_SUFFIX: &str = ".pub";

pub(crate) fn sign(payload: &[u8], config: &SigningConfig) -> Result<Signature> {
    let Some(spec) = config.signing_key.as_deref() else {
        return Err(no_key(
            "no SSH signing key is configured; set `user.signingKey` to the path of the key",
        ));
    };
    if let Some(literal) = spec.strip_prefix(LITERAL_PREFIX) {
        let public = PublicKey::from_openssh(literal.trim()).map_err(|error| {
            no_key(format!("`user.signingKey` holds an invalid SSH public key literal: {error}"))
        })?;
        return sign_with_agent(&public, payload, config);
    }
    let path = config
        .home
        .as_deref()
        .map_or_else(|| PathBuf::from(spec), |home| expand_home(spec, home));
    let text = std::fs::read_to_string(&path).map_err(|source| {
        with_path(
            no_key(format!(
                "cannot read the SSH key `{}`: {source}; correct `user.signingKey`",
                path.display()
            )),
            "key",
            &path,
        )
    })?;
    if let Ok(private) = PrivateKey::from_openssh(&text) {
        return sign_with_private(private, &path, payload, config);
    }
    let public = PublicKey::from_openssh(text.trim()).map_err(|_| {
        with_path(
            no_key(format!(
                "`{}` is neither an OpenSSH private key nor a public key",
                path.display()
            )),
            "key",
            &path,
        )
    })?;
    match sign_with_agent(&public, payload, config) {
        Err(error) if is_problem(&error, AGENT_UNAVAILABLE) || is_problem(&error, NO_KEY) => {
            let private_path = path
                .to_str()
                .and_then(|text| text.strip_suffix(PUBLIC_KEY_SUFFIX))
                .map(PathBuf::from)
                .filter(|candidate| candidate.is_file());
            let Some(private_path) = private_path else {
                return Err(error);
            };
            let private = std::fs::read_to_string(&private_path)
                .ok()
                .and_then(|text| PrivateKey::from_openssh(text).ok())
                .filter(|private| private.public_key().key_data() == public.key_data());
            match private {
                Some(private) => sign_with_private(private, &private_path, payload, config),
                None => Err(error),
            }
        }
        other => other,
    }
}

fn sign_with_private(
    private: PrivateKey,
    path: &Path,
    payload: &[u8],
    config: &SigningConfig,
) -> Result<Signature> {
    if !private.is_encrypted() {
        return finish(private.sign(NAMESPACE, HashAlg::Sha512, payload), private.public_key(), KeySource::KeyFile);
    }
    let through_agent = sign_with_agent(private.public_key(), payload, config);
    match through_agent {
        Ok(signature) => return Ok(signature),
        Err(error) if !(is_problem(&error, AGENT_UNAVAILABLE) || is_problem(&error, NO_KEY)) => {
            return Err(error);
        }
        Err(_) => {}
    }
    let Some(file) = &config.ssh_passphrase_file else {
        return Err(with_path(
            key_locked(format!(
                "the SSH key `{}` is encrypted and no SSH agent holds it; add it with `ssh-add` or set `NURT_SSH_KEY_PASSPHRASE_FILE`",
                path.display()
            )),
            "key",
            path,
        ));
    };
    let passphrase = read_passphrase(file)?;
    let decrypted = private.decrypt(passphrase.as_bytes()).map_err(|_| {
        with_path(
            key_locked(format!(
                "the passphrase from `NURT_SSH_KEY_PASSPHRASE_FILE` does not decrypt `{}`",
                path.display()
            )),
            "key",
            path,
        )
    })?;
    finish(
        decrypted.sign(NAMESPACE, HashAlg::Sha512, payload),
        decrypted.public_key(),
        KeySource::KeyFile,
    )
}

fn sign_with_agent(public: &PublicKey, payload: &[u8], config: &SigningConfig) -> Result<Signature> {
    let Some(socket) = &config.ssh_auth_sock else {
        return Err(sign_error(
            AGENT_UNAVAILABLE,
            "no SSH agent is available; start one and set `SSH_AUTH_SOCK`, or set `user.signingKey` to a private key file",
        ));
    };
    let data = SshSig::signed_data(NAMESPACE, HashAlg::Sha512, payload)
        .map_err(|error| unsupported(&error.to_string()))?;
    let signature = agent::sign(socket, public, &data)?;
    finish(
        SshSig::new(public.key_data().clone(), NAMESPACE, HashAlg::Sha512, signature),
        public,
        KeySource::Agent,
    )
}

fn unsupported(detail: &str) -> Error {
    sign_error(
        UNSUPPORTED_FORMAT,
        format!("the SSH key cannot create a signature: {detail}; use an Ed25519, ECDSA or RSA key"),
    )
}

fn finish(
    signature: std::result::Result<SshSig, russh::keys::ssh_key::Error>,
    public: &PublicKey,
    source: KeySource,
) -> Result<Signature> {
    let signature = signature.map_err(|error| unsupported(&error.to_string()))?;
    let armored = signature
        .to_pem(LineEnding::LF)
        .map_err(|error| unsupported(&error.to_string()))?;
    let mut armored = armored.into_bytes();
    if !armored.ends_with(b"\n") {
        armored.push(b'\n');
    }
    Ok(Signature {
        format: SignatureFormat::Ssh,
        armored,
        key: public.fingerprint(HashAlg::Sha256).to_string(),
        source,
    })
}

pub(crate) fn verify(
    payload: &[u8],
    signature: &[u8],
    config: &VerifyConfig,
) -> Result<CommitSignature> {
    let mut result = CommitSignature {
        status: SignatureStatus::Bad,
        format: Some(SignatureFormat::Ssh),
        key: None,
        signer: None,
    };
    let Ok(sshsig) = SshSig::from_pem(signature) else {
        return Ok(result);
    };
    let public = PublicKey::from(sshsig.public_key().clone());
    result.key = Some(public.fingerprint(HashAlg::Sha256).to_string());
    if sshsig.namespace() != NAMESPACE || public.verify(NAMESPACE, payload, &sshsig).is_err() {
        return Ok(result);
    }
    if let Some(path) = &config.revocation_file {
        if allowed::is_revoked(path, &public)? {
            return Ok(result);
        }
    }
    let principal = match &config.allowed_signers_file {
        Some(path) => allowed::find_principal(path, &public, config.signed_at, config.utc_offset)?,
        None => None,
    };
    match principal {
        Some(principal) => {
            result.status = SignatureStatus::Good;
            result.signer = Some(principal);
        }
        None => result.status = SignatureStatus::UnknownKey,
    }
    Ok(result)
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! A key derived from a fixed seed, and signatures `ssh-keygen -Y sign
    //! -n git` made with the same key.

    use russh::keys::ssh_key::private::{Ed25519Keypair, KeypairData};
    use russh::keys::ssh_key::{LineEnding, PrivateKey};

    pub(crate) const SEED: [u8; 32] = [7; 32];

    pub(crate) const PUBLIC: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOpKbGPinFIKvvVQexMuxfmVR3auvr57kkIe6mkURtIs test@example.test";

    pub(crate) const FINGERPRINT: &str = "SHA256:z/fSv0Z0RZS+Lccbc6ZoOWwt/fbj1VFJGSBKQbo4icE";

    /// `ssh-keygen -Y sign -n git` over the OpenPGP fixture payload.
    pub(crate) const PAYLOAD_SIGNATURE: &str = "-----BEGIN SSH SIGNATURE-----
U1NIU0lHAAAAAQAAADMAAAALc3NoLWVkMjU1MTkAAAAg6kpsY+KcUgq+9VB7Ey7F+ZVHdq
6+vnuSQh7qaRRG0iwAAAADZ2l0AAAAAAAAAAZzaGE1MTIAAABTAAAAC3NzaC1lZDI1NTE5
AAAAQJQttzo/gq6ssmGt7v8+Nw03ghy4RqDLcn5Bzk0gehVblkUkAgMglYPPGMtkCCjGI3
m57YtcbtaNVlMDgmcxfAk=
-----END SSH SIGNATURE-----
";

    pub(crate) fn private_key() -> PrivateKey {
        PrivateKey::new(KeypairData::from(Ed25519Keypair::from_seed(&SEED)), "test@example.test")
            .expect("fixture key")
    }

    pub(crate) fn private_key_pem() -> String {
        private_key().to_openssh(LineEnding::LF).expect("encode").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::sign::openpgp::fixtures::PAYLOAD;

    const CREATED: i64 = 1_767_225_600;

    fn config_for(key: &Path) -> SigningConfig {
        let mut config = SigningConfig::new(SignatureFormat::Ssh, CREATED);
        config.signing_key = Some(key.display().to_string());
        config
    }

    #[test]
    fn signs_like_ssh_keygen_with_a_private_key_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let key = dir.path().join("id_ed25519");
        std::fs::write(&key, private_key_pem()).expect("write");
        let signature = sign(PAYLOAD, &config_for(&key)).expect("signed");
        assert_eq!(String::from_utf8(signature.armored).expect("utf-8"), PAYLOAD_SIGNATURE);
        assert_eq!(signature.key, FINGERPRINT);
        assert_eq!(signature.source, KeySource::KeyFile);
    }

    #[test]
    fn expands_home_in_key_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("id_ed25519"), private_key_pem()).expect("write");
        let mut config = SigningConfig::new(SignatureFormat::Ssh, CREATED);
        config.signing_key = Some("~/id_ed25519".into());
        config.home = Some(dir.path().to_path_buf());
        assert_eq!(sign(PAYLOAD, &config).expect("signed").key, FINGERPRINT);
    }

    #[test]
    fn falls_back_from_public_key_to_private_key_file_without_agent() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("id_ed25519"), private_key_pem()).expect("write");
        let public = dir.path().join("id_ed25519.pub");
        std::fs::write(&public, format!("{PUBLIC}\n")).expect("write");
        let signature = sign(PAYLOAD, &config_for(&public)).expect("signed");
        assert_eq!(String::from_utf8(signature.armored).expect("utf-8"), PAYLOAD_SIGNATURE);
    }

    #[test]
    fn reports_missing_key_and_agent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = sign(PAYLOAD, &SigningConfig::new(SignatureFormat::Ssh, CREATED)).expect_err("no key");
        assert_eq!(error.data().problem.as_deref(), Some(NO_KEY));
        let error = sign(PAYLOAD, &config_for(&dir.path().join("absent"))).expect_err("no file");
        assert_eq!(error.data().problem.as_deref(), Some(NO_KEY));
        let mut literal = SigningConfig::new(SignatureFormat::Ssh, CREATED);
        literal.signing_key = Some(format!("key::{PUBLIC}"));
        let error = sign(PAYLOAD, &literal).expect_err("no agent");
        assert_eq!(error.data().problem.as_deref(), Some(AGENT_UNAVAILABLE));
        literal.signing_key = Some("key::ssh-ed25519 !!".into());
        let error = sign(PAYLOAD, &literal).expect_err("bad literal");
        assert_eq!(error.data().problem.as_deref(), Some(NO_KEY));
        let garbage = dir.path().join("garbage");
        std::fs::write(&garbage, "not a key").expect("write");
        let error = sign(PAYLOAD, &config_for(&garbage)).expect_err("garbage");
        assert_eq!(error.data().problem.as_deref(), Some(NO_KEY));
    }

    #[cfg(unix)]
    #[test]
    fn decrypts_encrypted_key_with_passphrase_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let mut rng = rand::rng();
        let encrypted = private_key().encrypt(&mut rng, "correct horse").expect("encrypt");
        let key = dir.path().join("id_ed25519");
        std::fs::write(&key, encrypted.to_openssh(LineEnding::LF).expect("encode").as_bytes()).expect("write");
        let mut config = config_for(&key);
        let error = sign(PAYLOAD, &config).expect_err("locked");
        assert_eq!(error.data().problem.as_deref(), Some("key_locked"));
        let passphrase = dir.path().join("passphrase");
        std::fs::write(&passphrase, "wrong\n").expect("write");
        std::fs::set_permissions(&passphrase, std::fs::Permissions::from_mode(0o600)).expect("chmod");
        config.ssh_passphrase_file = Some(passphrase.clone());
        let error = sign(PAYLOAD, &config).expect_err("wrong passphrase");
        assert_eq!(error.data().problem.as_deref(), Some("key_locked"));
        std::fs::write(&passphrase, "correct horse\n").expect("write");
        let signature = sign(PAYLOAD, &config).expect("signed");
        assert_eq!(String::from_utf8(signature.armored).expect("utf-8"), PAYLOAD_SIGNATURE);
    }

    fn allowed_config(dir: &Path, line: &str) -> VerifyConfig {
        let path = dir.join("allowed_signers");
        std::fs::write(&path, line).expect("write");
        VerifyConfig {
            allowed_signers_file: Some(path),
            signed_at: Some(CREATED),
            ..VerifyConfig::default()
        }
    }

    #[test]
    fn verifies_ssh_keygen_signature_against_allowed_signers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = allowed_config(dir.path(), &format!("tester@example.test namespaces=\"git\" {PUBLIC}\n"));
        let result = verify(PAYLOAD, PAYLOAD_SIGNATURE.as_bytes(), &config).expect("verify");
        assert_eq!(result.status, SignatureStatus::Good);
        assert_eq!(result.signer.as_deref(), Some("tester@example.test"));
        assert_eq!(result.key.as_deref(), Some(FINGERPRINT));
    }

    #[test]
    fn classifies_unknown_bad_and_revoked_signatures() {
        let dir = tempfile::tempdir().expect("tempdir");
        let unknown = verify(PAYLOAD, PAYLOAD_SIGNATURE.as_bytes(), &VerifyConfig::default()).expect("verify");
        assert_eq!(unknown.status, SignatureStatus::UnknownKey);
        let other_namespace = allowed_config(dir.path(), &format!("tester@example.test namespaces=\"file\" {PUBLIC}\n"));
        let result = verify(PAYLOAD, PAYLOAD_SIGNATURE.as_bytes(), &other_namespace).expect("verify");
        assert_eq!(result.status, SignatureStatus::UnknownKey);
        let config = allowed_config(dir.path(), &format!("tester@example.test {PUBLIC}\n"));
        let bad = verify(b"changed", PAYLOAD_SIGNATURE.as_bytes(), &config).expect("verify");
        assert_eq!(bad.status, SignatureStatus::Bad);
        let revoked = dir.path().join("revoked");
        std::fs::write(&revoked, format!("# revoked keys\n{PUBLIC}\n")).expect("write");
        let mut config = config;
        config.revocation_file = Some(revoked);
        let result = verify(PAYLOAD, PAYLOAD_SIGNATURE.as_bytes(), &config).expect("verify");
        assert_eq!(result.status, SignatureStatus::Bad);
        let garbage = verify(PAYLOAD, b"-----BEGIN SSH SIGNATURE-----\nAAAA\n", &config).expect("verify");
        assert_eq!(garbage.status, SignatureStatus::Bad);
    }
}
