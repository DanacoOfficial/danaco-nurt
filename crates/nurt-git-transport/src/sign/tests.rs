// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Tests of the signing settings, of signatures inside Git objects and of
//! OpenPGP key files.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use nurt_contracts::config::Config;
use nurt_contracts::env::WallTime;
use nurt_testkit::ctx::TestContext;
use nurt_testkit::host::TestHost;
use pgp::bytes::Bytes;
use pgp::composed::{ArmorOptions, SignedKeyDetails, SignedSecretKey};
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::public_key::PublicKeyAlgorithm;
use pgp::crypto::sym::SymmetricKeyAlgorithm;
use pgp::packet::{PubKeyInner, SignatureConfig, SignatureType, Subpacket, SubpacketData, UserId};
use pgp::types::{
    Ed25519PublicParams, KeyDetails, KeyVersion, Password, PlainSecretParams, PublicParams,
    S2kParams, SecretParams, SignedUser, StringToKey, Tag, Timestamp,
};

use super::sshsig::fixtures::{FINGERPRINT, PUBLIC, private_key_pem};
use super::*;

/// 2026-01-01T00:00:00Z, the time of all fixture objects.
const CREATED: i64 = 1_767_225_600;
const NANOS_PER_SECOND: i128 = 1_000_000_000;
const SECONDS_PER_DAY: i64 = 86_400;

/// The commit of [`SIGNED_COMMIT`] before signing.
const UNSIGNED_COMMIT: &str = "tree 08585692ce06452da6f82ae66b90d98b55536fca
author Tester <tester@example.test> 1767225600 +0000
committer Tester <tester@example.test> 1767225600 +0000

signed commit
";

/// `git commit -S` with `gpg.format = ssh` and the SSH fixture key.
const SIGNED_COMMIT: &str = "tree 08585692ce06452da6f82ae66b90d98b55536fca
author Tester <tester@example.test> 1767225600 +0000
committer Tester <tester@example.test> 1767225600 +0000
gpgsig -----BEGIN SSH SIGNATURE-----
 U1NIU0lHAAAAAQAAADMAAAALc3NoLWVkMjU1MTkAAAAg6kpsY+KcUgq+9VB7Ey7F+ZVHdq
 6+vnuSQh7qaRRG0iwAAAADZ2l0AAAAAAAAAAZzaGE1MTIAAABTAAAAC3NzaC1lZDI1NTE5
 AAAAQEApspBvUHXfArdtPu/9u4OCqb0TgwpRINAzTAJ1YG2KrMBjaOgGghYnKkeKoENnye
 F4XiKcKSFJrlSKMn1QzAg=
 -----END SSH SIGNATURE-----

signed commit
";

const SIGNED_COMMIT_ID: &str = "15f354fa41d7b460c3090616723516b2275c4ef2";

/// The tag of [`SIGNED_TAG`] before signing.
const UNSIGNED_TAG: &str = "object 15f354fa41d7b460c3090616723516b2275c4ef2
type commit
tag v1
tagger Tester <tester@example.test> 1767225600 +0000

release v1
";

/// `git tag -s v1 -m 'release v1'` with the SSH fixture key.
const SIGNED_TAG: &str = "object 15f354fa41d7b460c3090616723516b2275c4ef2
type commit
tag v1
tagger Tester <tester@example.test> 1767225600 +0000

release v1
-----BEGIN SSH SIGNATURE-----
U1NIU0lHAAAAAQAAADMAAAALc3NoLWVkMjU1MTkAAAAg6kpsY+KcUgq+9VB7Ey7F+ZVHdq
6+vnuSQh7qaRRG0iwAAAADZ2l0AAAAAAAAAAZzaGE1MTIAAABTAAAAC3NzaC1lZDI1NTE5
AAAAQCYYiHfQsECKOypNu5UO6lRmdtqL8Dr9vCziB71gqzK6mbpDG6IMV6cdpE6NBMluUd
8Sjb3ZB1QpaRiVc01T2Qc=
-----END SSH SIGNATURE-----
";

const SIGNED_TAG_ID: &str = "736668c90a3a34b2e2ae9dd5877858ae4ccf303c";

const SHA1: gix_hash::Kind = gix_hash::Kind::Sha1;

fn parse_commit(data: &str) -> gix_object::Commit {
    gix_object::CommitRef::from_bytes(data.as_bytes(), SHA1)
        .expect("commit parses")
        .into_owned()
        .expect("owned commit")
}

fn parse_tag(data: &str) -> gix_object::Tag {
    gix_object::TagRef::from_bytes(data.as_bytes(), SHA1)
        .expect("tag parses")
        .into_owned()
        .expect("owned tag")
}

fn encode(object: &impl gix_object::WriteTo) -> Vec<u8> {
    let mut out = Vec::new();
    object.write_to(&mut out).expect("serialize");
    out
}

fn object_id(kind: gix_object::Kind, data: &[u8]) -> String {
    gix_object::compute_hash(SHA1, kind, data)
        .expect("hash")
        .to_string()
}

/// SSH settings naming the fixture private key, written to `dir`.
fn ssh_config(dir: &Path) -> SigningConfig {
    let key = dir.join("id_ed25519");
    std::fs::write(&key, private_key_pem()).expect("write key");
    let mut config = SigningConfig::new(SignatureFormat::Ssh, CREATED);
    config.signing_key = Some(key.display().to_string());
    config
}

/// Verification settings whose allowed signers file accepts the fixture key
/// with `options`.
fn allowed_config(dir: &Path, options: &str) -> VerifyConfig {
    let path = dir.join("allowed_signers");
    std::fs::write(&path, format!("tester@example.test {options} {PUBLIC}\n")).expect("write");
    VerifyConfig {
        allowed_signers_file: Some(path),
        ..VerifyConfig::default()
    }
}

#[test]
fn signs_commit_byte_for_byte_like_git_with_ssh_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut commit = parse_commit(UNSIGNED_COMMIT);
    let signature =
        sign_commit(&mut commit, &BuiltinSigner, &ssh_config(dir.path())).expect("signed");
    let bytes = encode(&commit);
    assert_eq!(
        String::from_utf8(bytes.clone()).expect("utf-8"),
        SIGNED_COMMIT
    );
    assert_eq!(
        object_id(gix_object::Kind::Commit, &bytes),
        SIGNED_COMMIT_ID
    );
    assert_eq!(signature.format, SignatureFormat::Ssh);
    assert_eq!(signature.key, FINGERPRINT);
}

#[test]
fn replaces_existing_commit_signature() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut commit = parse_commit(SIGNED_COMMIT);
    sign_commit(&mut commit, &BuiltinSigner, &ssh_config(dir.path())).expect("signed");
    let signature_headers = commit
        .extra_headers
        .iter()
        .filter(|(name, _)| name == "gpgsig")
        .count();
    assert_eq!(signature_headers, 1);
    assert_eq!(encode(&commit), SIGNED_COMMIT.as_bytes());
}

#[test]
fn signs_tag_byte_for_byte_like_git_with_ssh_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    for source in [UNSIGNED_TAG, SIGNED_TAG] {
        let mut tag = parse_tag(source);
        sign_tag(&mut tag, &BuiltinSigner, &ssh_config(dir.path())).expect("signed");
        let bytes = encode(&tag);
        assert_eq!(String::from_utf8(bytes.clone()).expect("utf-8"), SIGNED_TAG);
        assert_eq!(object_id(gix_object::Kind::Tag, &bytes), SIGNED_TAG_ID);
    }
}

#[test]
fn leaves_objects_unchanged_when_signing_fails() {
    let config = SigningConfig::new(SignatureFormat::Ssh, CREATED);
    let mut commit = parse_commit(UNSIGNED_COMMIT);
    let error = sign_commit(&mut commit, &BuiltinSigner, &config).expect_err("no key");
    assert_eq!(error.data().problem.as_deref(), Some("no_key"));
    assert_eq!(encode(&commit), UNSIGNED_COMMIT.as_bytes());
    let mut tag = parse_tag(UNSIGNED_TAG);
    sign_tag(&mut tag, &BuiltinSigner, &config).expect_err("no key");
    assert_eq!(encode(&tag), UNSIGNED_TAG.as_bytes());
}

#[test]
fn verifies_git_signed_commit_and_tag() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = allowed_config(dir.path(), "namespaces=\"git\"");
    for result in [
        verify_commit(SIGNED_COMMIT.as_bytes(), SHA1, &BuiltinSigner, &config),
        verify_tag(SIGNED_TAG.as_bytes(), SHA1, &BuiltinSigner, &config),
    ] {
        let result = result.expect("verify");
        assert_eq!(result.status, SignatureStatus::Good);
        assert_eq!(result.format, Some(SignatureFormat::Ssh));
        assert_eq!(result.signer.as_deref(), Some("tester@example.test"));
        assert_eq!(result.key.as_deref(), Some(FINGERPRINT));
    }
}

#[test]
fn reports_changed_objects_as_bad() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = allowed_config(dir.path(), "");
    let commit = SIGNED_COMMIT.replace("signed commit", "changed commit");
    let result = verify_commit(commit.as_bytes(), SHA1, &BuiltinSigner, &config).expect("verify");
    assert_eq!(result.status, SignatureStatus::Bad);
    let tag = SIGNED_TAG.replace("release v1", "release v2");
    let result = verify_tag(tag.as_bytes(), SHA1, &BuiltinSigner, &config).expect("verify");
    assert_eq!(result.status, SignatureStatus::Bad);
}

#[test]
fn checks_allowed_signer_validity_at_committer_time() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = allowed_config(dir.path(), "valid-before=\"20251231Z\"");
    let result =
        verify_commit(SIGNED_COMMIT.as_bytes(), SHA1, &BuiltinSigner, &config).expect("verify");
    assert_eq!(result.status, SignatureStatus::UnknownKey);
    let result = verify_tag(SIGNED_TAG.as_bytes(), SHA1, &BuiltinSigner, &config).expect("verify");
    assert_eq!(result.status, SignatureStatus::UnknownKey);
    config.signed_at = Some(CREATED - 2 * SECONDS_PER_DAY);
    let result =
        verify_commit(SIGNED_COMMIT.as_bytes(), SHA1, &BuiltinSigner, &config).expect("verify");
    assert_eq!(result.status, SignatureStatus::Good);
}

#[test]
fn reports_unsigned_objects_as_not_signed() {
    let config = VerifyConfig::default();
    let commit =
        verify_commit(UNSIGNED_COMMIT.as_bytes(), SHA1, &BuiltinSigner, &config).expect("verify");
    assert_eq!(commit.status, SignatureStatus::None);
    assert_eq!(commit.format, None);
    let tag = verify_tag(UNSIGNED_TAG.as_bytes(), SHA1, &BuiltinSigner, &config).expect("verify");
    assert_eq!(tag.status, SignatureStatus::None);
}

#[test]
fn reports_x509_and_unknown_signatures_as_unsupported() {
    let config = VerifyConfig::default();
    let x509 = BuiltinSigner
        .verify(
            b"payload",
            b"-----BEGIN SIGNED MESSAGE-----\nAAAA\n",
            &config,
        )
        .expect("verify");
    assert_eq!(x509.status, SignatureStatus::Unsupported);
    assert_eq!(x509.format, Some(SignatureFormat::X509));
    let unknown = BuiltinSigner
        .verify(b"payload", b"garbage", &config)
        .expect("verify");
    assert_eq!(unknown.status, SignatureStatus::Unsupported);
    assert_eq!(unknown.format, None);
    let error = BuiltinSigner
        .sign(
            b"payload",
            &SigningConfig::new(SignatureFormat::X509, CREATED),
        )
        .expect_err("x509");
    assert_eq!(error.data().problem.as_deref(), Some("unsupported_format"));
}

/// An Ed25519 OpenPGP key derived from a fixed seed, with the certified
/// user ID `Test Key <key@example.test>`, protected by `passphrase` when
/// one is given.
fn openpgp_key(passphrase: Option<&str>) -> SignedSecretKey {
    let created = Timestamp::from_secs(u32::try_from(CREATED).expect("time"));
    let secret = pgp::crypto::ed25519::SecretKey::try_from_bytes(
        [9; 32],
        pgp::crypto::ed25519::Mode::Ed25519,
    )
    .expect("seed");
    let public_params = PublicParams::Ed25519(Ed25519PublicParams::from(&secret));
    let inner = PubKeyInner::new(
        KeyVersion::V4,
        PublicKeyAlgorithm::Ed25519,
        created,
        None,
        public_params,
    )
    .expect("public key");
    let public = pgp::packet::PublicKey::from_inner(inner).expect("public key packet");
    let mut primary = pgp::packet::SecretKey::new(
        public.clone(),
        SecretParams::Plain(PlainSecretParams::Ed25519(secret)),
    )
    .expect("secret key packet");
    let user_id =
        UserId::from_str(Default::default(), "Test Key <key@example.test>").expect("user id");
    let mut certification = SignatureConfig::v4(
        SignatureType::CertPositive,
        PublicKeyAlgorithm::Ed25519,
        HashAlgorithm::Sha256,
    );
    certification.hashed_subpackets = vec![
        Subpacket::regular(SubpacketData::SignatureCreationTime(created)).expect("subpacket"),
        Subpacket::regular(SubpacketData::IssuerFingerprint(public.fingerprint()))
            .expect("subpacket"),
    ];
    let certification = certification
        .sign_certification(&primary, &public, &Password::empty(), Tag::UserId, &user_id)
        .expect("certification");
    if let Some(passphrase) = passphrase {
        let s2k = S2kParams::Cfb {
            sym_alg: SymmetricKeyAlgorithm::AES128,
            s2k: StringToKey::IteratedAndSalted {
                hash_alg: HashAlgorithm::Sha256,
                salt: [1; 8],
                count: 96,
            },
            iv: Bytes::from(vec![2; 16]),
        };
        primary
            .set_password_with_s2k(&Password::from(passphrase), s2k)
            .expect("protect key");
    }
    let details = SignedKeyDetails::new(
        Vec::new(),
        Vec::new(),
        vec![SignedUser::new(user_id, vec![certification])],
        Vec::new(),
    );
    SignedSecretKey::new(primary, details, Vec::new(), Vec::new())
}

/// OpenPGP settings naming the secret key file `key`, written to `dir`.
fn openpgp_config(dir: &Path, key: &SignedSecretKey) -> SigningConfig {
    let path = dir.join("secret.asc");
    std::fs::write(
        &path,
        key.to_armored_bytes(ArmorOptions::default())
            .expect("armor"),
    )
    .expect("write");
    let mut config = SigningConfig::new(SignatureFormat::Openpgp, CREATED);
    config.openpgp_sources = vec![OpenPgpSource::KeyFile];
    config.key_file = Some(path);
    config.signing_key = Some("key@example.test".into());
    config
}

#[test]
fn signs_commit_with_openpgp_key_file_and_verifies_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let key = openpgp_key(None);
    let mut commit = parse_commit(UNSIGNED_COMMIT);
    let signature = sign_commit(
        &mut commit,
        &BuiltinSigner,
        &openpgp_config(dir.path(), &key),
    )
    .expect("signed");
    let fingerprint = hex::encode_upper(key.primary_key.fingerprint().as_bytes());
    assert!(
        signature
            .armored
            .starts_with(b"-----BEGIN PGP SIGNATURE-----\n")
    );
    assert_eq!(signature.key, fingerprint);
    assert_eq!(signature.source, KeySource::KeyFile);
    let public = dir.path().join("public.asc");
    std::fs::write(
        &public,
        key.to_public_key()
            .to_armored_bytes(ArmorOptions::default())
            .expect("armor"),
    )
    .expect("write");
    let config = VerifyConfig {
        public_key_files: vec![public],
        ..VerifyConfig::default()
    };
    let bytes = encode(&commit);
    let result = verify_commit(&bytes, SHA1, &BuiltinSigner, &config).expect("verify");
    assert_eq!(result.status, SignatureStatus::Good);
    assert_eq!(result.key.as_deref(), Some(fingerprint.as_str()));
    assert_eq!(
        result.signer.as_deref(),
        Some("Test Key <key@example.test>")
    );
    let changed = String::from_utf8(bytes)
        .expect("utf-8")
        .replace("signed commit", "other");
    let result = verify_commit(changed.as_bytes(), SHA1, &BuiltinSigner, &config).expect("verify");
    assert_eq!(result.status, SignatureStatus::Bad);
}

#[test]
fn reports_key_file_without_matching_user_as_missing_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = openpgp_config(dir.path(), &openpgp_key(None));
    config.signing_key = Some("someone@example.test".into());
    let error = BuiltinSigner
        .sign(b"payload", &config)
        .expect_err("no match");
    assert_eq!(error.data().problem.as_deref(), Some("no_key"));
    config.signing_key = None;
    assert!(BuiltinSigner.sign(b"payload", &config).is_ok());
}

#[cfg(unix)]
#[test]
fn unlocks_openpgp_key_file_with_passphrase_file() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = openpgp_config(dir.path(), &openpgp_key(Some("correct horse")));
    let error = BuiltinSigner.sign(b"payload", &config).expect_err("locked");
    assert_eq!(error.data().problem.as_deref(), Some("key_locked"));
    let passphrase = dir.path().join("passphrase");
    std::fs::write(&passphrase, "wrong\n").expect("write");
    std::fs::set_permissions(&passphrase, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    config.passphrase_file = Some(passphrase.clone());
    let error = BuiltinSigner
        .sign(b"payload", &config)
        .expect_err("wrong passphrase");
    assert_eq!(error.data().problem.as_deref(), Some("key_locked"));
    std::fs::write(&passphrase, "correct horse\n").expect("write");
    let signature = BuiltinSigner.sign(b"payload", &config).expect("signed");
    assert_eq!(signature.format, SignatureFormat::Openpgp);
}

fn git_config(text: &str) -> gix::config::File {
    text.parse().expect("git configuration parses")
}

fn context(host: TestHost, key_file: Option<&str>) -> nurt_contracts::stage::ExecCtx {
    let test = TestContext::with_host(host);
    test.clock()
        .set_wall(WallTime(i128::from(CREATED) * NANOS_PER_SECOND));
    let mut nurt = Config::default();
    nurt.git.signing.key_file = key_file.map(str::to_owned);
    test.builder().config(Arc::new(nurt)).build()
}

#[test]
fn resolves_signing_config_from_git_configuration_and_host() {
    let host = TestHost::linux()
        .with_var("GNUPGHOME", "~/gpg-home")
        .with_var("SSH_AUTH_SOCK", "/run/agent.sock")
        .with_var("NURT_GPG_PASSPHRASE_FILE", "~/passphrase")
        .with_var("NURT_SSH_KEY_PASSPHRASE_FILE", "/secrets/ssh");
    let ctx = context(host, Some("~/keys/signing.asc"));
    let git = git_config(
        "[gpg]\n\tformat = ssh\n\tprogram = gpg2\n[gpg \"ssh\"]\n\tprogram = ssh-keygen\n[user]\n\tsigningKey = ~/.ssh/id_ed25519.pub\n",
    );
    let config =
        SigningConfig::resolve(&git, &ctx, Some("Tester <tester@example.test>")).expect("resolve");
    assert_eq!(config.format, SignatureFormat::Ssh);
    assert_eq!(config.signing_key.as_deref(), Some("~/.ssh/id_ed25519.pub"));
    assert_eq!(
        config.default_identity.as_deref(),
        Some("Tester <tester@example.test>")
    );
    assert_eq!(
        config.key_file,
        Some(PathBuf::from("/home/user/keys/signing.asc"))
    );
    assert_eq!(
        config.passphrase_file,
        Some(PathBuf::from("/home/user/passphrase"))
    );
    assert_eq!(
        config.ssh_passphrase_file,
        Some(PathBuf::from("/secrets/ssh"))
    );
    assert_eq!(
        config.gnupg_home,
        Some(PathBuf::from("/home/user/gpg-home"))
    );
    assert_eq!(config.ssh_auth_sock, Some(PathBuf::from("/run/agent.sock")));
    assert_eq!(config.home, Some(PathBuf::from("/home/user")));
    assert_eq!(config.created, CREATED);
    assert_eq!(config.ignored_settings, ["gpg.program", "gpg.ssh.program"]);
}

#[test]
fn resolves_default_signing_config() {
    let ctx = context(TestHost::linux(), None);
    let config = SigningConfig::resolve(&git_config(""), &ctx, None).expect("resolve");
    assert_eq!(config.format, SignatureFormat::Openpgp);
    assert_eq!(
        config.openpgp_sources,
        [OpenPgpSource::Agent, OpenPgpSource::KeyFile]
    );
    assert_eq!(config.gnupg_home, Some(PathBuf::from("/home/user/.gnupg")));
    assert_eq!(config.signing_key, None);
    assert_eq!(config.key_file, None);
    assert_eq!(config.ssh_auth_sock, None);
    assert!(config.ignored_settings.is_empty());
    let windows = context(TestHost::windows(), None);
    let config = SigningConfig::resolve(&git_config(""), &windows, None).expect("resolve");
    assert_eq!(
        config.ssh_auth_sock,
        Some(PathBuf::from(r"\\.\pipe\openssh-ssh-agent"))
    );
    assert!(
        config
            .gnupg_home
            .is_some_and(|home| home.ends_with("gnupg"))
    );
}

#[test]
fn rejects_unknown_signature_format() {
    let ctx = context(TestHost::linux(), None);
    let error = SigningConfig::resolve(&git_config("[gpg]\n\tformat = smime\n"), &ctx, None)
        .expect_err("unknown");
    assert_eq!(error.data().problem.as_deref(), Some("unsupported_format"));
    let config =
        SigningConfig::resolve(&git_config("[gpg]\n\tformat = x509\n"), &ctx, None).expect("x509");
    assert_eq!(config.format, SignatureFormat::X509);
}

#[test]
fn resolves_verify_config_from_git_configuration() {
    let ctx = context(TestHost::linux(), Some("/keys/public.asc"));
    let git = git_config(
        "[gpg \"ssh\"]\n\tallowedSignersFile = ~/.ssh/allowed_signers\n\trevocationFile = /etc/ssh/revoked\n",
    );
    let config = VerifyConfig::resolve(&git, &ctx);
    assert_eq!(
        config.allowed_signers_file,
        Some(PathBuf::from("/home/user/.ssh/allowed_signers"))
    );
    assert_eq!(
        config.revocation_file,
        Some(PathBuf::from("/etc/ssh/revoked"))
    );
    assert_eq!(config.public_key_files, [PathBuf::from("/keys/public.asc")]);
    assert_eq!(config.gnupg_home, Some(PathBuf::from("/home/user/.gnupg")));
    assert_eq!(config.signed_at, None);
    assert_eq!(config.utc_offset, 0);
}
