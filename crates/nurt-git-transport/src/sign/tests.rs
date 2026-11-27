// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-transport::sign::tests
// Responsibility:   Tests of the signing settings, of signatures inside Git objects and of.
// Dependencies:     nurt_contracts, nurt_testkit
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tests of the signing settings, of signatures inside Git objects and of
//! OpenPGP key files.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use nurt_contracts::config::Config;
use nurt_contracts::env::WallTime;
use nurt_testkit::ctx::TestContext;
use nurt_testkit::host::TestHost;

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
fn reads_openpgp_source_from_nurt_configuration() {
    let sources = |value: Option<nurt_contracts::config::OpenPgpSource>| {
        let test = TestContext::with_host(TestHost::linux());
        let mut nurt = Config::default();
        nurt.git.signing.openpgp_source = value;
        let ctx = test.builder().config(Arc::new(nurt)).build();
        SigningConfig::resolve(&git_config(""), &ctx, None)
            .expect("resolve")
            .openpgp_sources
    };
    assert_eq!(
        sources(Some(nurt_contracts::config::OpenPgpSource::GpgAgent)),
        [OpenPgpSource::Agent]
    );
    assert_eq!(
        sources(Some(nurt_contracts::config::OpenPgpSource::KeyFile)),
        [OpenPgpSource::KeyFile]
    );
    assert_eq!(
        sources(None),
        [OpenPgpSource::Agent, OpenPgpSource::KeyFile]
    );
}

#[test]
fn reports_ignored_signing_programs_as_notable() {
    let ctx = context(TestHost::linux(), None);
    let git = git_config("[gpg]\n\tprogram = gpg2\n[gpg \"ssh\"]\n\tprogram = ssh-keygen\n");
    let config = SigningConfig::resolve(&git, &ctx, None).expect("resolve");
    assert_eq!(config.gpg_program.as_deref(), Some("gpg2"));
    let note = config.ignored_programs_note().expect("note");
    assert_eq!(
        note.code,
        nurt_contracts::report::Notable::SigningProgramIgnored
    );
    assert_eq!(note.count, Some(1));
    assert!(note.detail.expect("detail").contains("gpg.ssh.program"));
    let specific =
        git_config("[gpg]\n\tprogram = gpg2\n[gpg \"openpgp\"]\n\tprogram = /usr/bin/gpg\n");
    let config = SigningConfig::resolve(&specific, &ctx, None).expect("resolve");
    assert_eq!(config.gpg_program.as_deref(), Some("/usr/bin/gpg"));
    assert!(config.ignored_programs_note().is_none());
    let verify = VerifyConfig::resolve(&specific, &ctx);
    assert_eq!(verify.gpg_program.as_deref(), Some("/usr/bin/gpg"));
    let plain = SigningConfig::resolve(&git_config(""), &ctx, None).expect("resolve");
    assert!(plain.ignored_programs_note().is_none());
}

#[test]
fn resolves_relative_paths_against_work_dir() {
    let mut config =
        SigningConfig::new(SignatureFormat::Ssh, CREATED).in_work_dir(Path::new("/work/repo"));
    config.home = Some(PathBuf::from("/home/user"));
    assert_eq!(
        config.key_path("keys/id"),
        PathBuf::from("/work/repo/keys/id")
    );
    assert_eq!(config.key_path("~/id"), PathBuf::from("/home/user/id"));
    let verify = VerifyConfig {
        allowed_signers_file: Some(PathBuf::from("signers")),
        public_key_files: vec![PathBuf::from("/abs/key.asc")],
        ..VerifyConfig::default()
    }
    .in_work_dir(Path::new("/work/repo"));
    assert_eq!(
        verify.allowed_signers_file,
        Some(PathBuf::from("/work/repo/signers"))
    );
    assert_eq!(verify.public_key_files, [PathBuf::from("/abs/key.asc")]);
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

#[test]
fn signers_without_a_program_service_refuse_openpgp_signatures() {
    let config = SigningConfig::new(SignatureFormat::Openpgp, CREATED);
    let program = ProgramSigner::new(None, nurt_contracts::stage::CancelToken::new());
    for error in [
        BuiltinSigner
            .sign(b"payload", &config)
            .expect_err("builtin"),
        program.sign(b"payload", &config).expect_err("program"),
    ] {
        assert_eq!(error.data().problem.as_deref(), Some("program_unavailable"));
    }
    let armored = b"-----BEGIN PGP SIGNATURE-----\n\nAAAA\n-----END PGP SIGNATURE-----\n";
    let result = program
        .verify(b"payload", armored, &VerifyConfig::default())
        .expect("verify");
    assert_eq!(result.status, SignatureStatus::Unsupported);
    assert_eq!(result.format, Some(SignatureFormat::Openpgp));
}

#[test]
fn program_signer_signs_commits_with_ssh_keys_like_the_builtin_signer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let program = ProgramSigner::new(None, nurt_contracts::stage::CancelToken::new());
    let mut commit = parse_commit(UNSIGNED_COMMIT);
    sign_commit(&mut commit, &program, &ssh_config(dir.path())).expect("signed");
    assert_eq!(encode(&commit), SIGNED_COMMIT.as_bytes());
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
