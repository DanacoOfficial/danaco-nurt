// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Verification of SSH host keys against `known_hosts` files.
//!
//! Lines have the form `[marker] hosts keytype base64 [comment]`. Host lists
//! use the patterns of `ssh_config` with negation, the form `[host]:port`
//! for ports other than 22, or the hashed form `|1|salt|hash`, where the hash
//! is HMAC-SHA1 of the name keyed with the salt. The marker `@revoked`
//! rejects a key; lines with `@cert-authority` describe certificate
//! authorities and are not used, since host certificates are not requested.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::AbsPath;
use nurt_contracts::journal::{Expect, Mutation, MutationTx, TargetAction, TargetPath, TargetSpec};
use russh::keys::{Algorithm, HashAlg, PublicKey};
use sha1::{Digest, Sha1};

use super::config::Location;
use super::pattern::match_list;
use super::SshNotice;

/// Block size of SHA-1 in bytes, used by HMAC (RFC 2104).
const SHA1_BLOCK: usize = 64;

/// Permission bits of a directory created for the known hosts file.
const DIRECTORY_MODE: u32 = 0o700;

/// A host key that is not yet in any known hosts file and was accepted for
/// this connection.
///
/// [`record_host_key`] appends [`HostKeyAddition::line`] to
/// [`HostKeyAddition::file`] through the journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKeyAddition {
    /// Known hosts file of Nurt that receives the line.
    pub file: PathBuf,
    /// Line without the line terminator, for example
    /// `[git.example.com]:2222 ssh-ed25519 AAAA…`.
    pub line: String,
    /// Name of the host in the line.
    pub host: String,
    /// SHA-256 fingerprint of the key, for example `SHA256:…`.
    pub fingerprint: String,
}

/// Appends an accepted host key to the known hosts file of Nurt.
///
/// The file and its missing parent directories are declared as targets of
/// `tx`, so that undoing the operation removes the line. The OpenSSH files
/// are never changed.
///
/// # Errors
///
/// Errors of the journal: `E_IO`, `E_PERMISSION`, `E_STALE` and related
/// codes; `E_ARG` when the file path is not absolute.
///
/// # Examples
///
/// ```no_run
/// use nurt_contracts::journal::MutationTx;
/// use nurt_git_transport::ssh::{record_host_key, SshTransport};
///
/// fn keep_new_host_key(
///     transport: &mut SshTransport,
///     tx: &mut dyn MutationTx,
/// ) -> nurt_contracts::error::Result<()> {
///     if let Some(addition) = transport.take_host_key_addition() {
///         record_host_key(tx, &addition)?;
///     }
///     Ok(())
/// }
/// ```
pub fn record_host_key(tx: &mut dyn MutationTx, addition: &HostKeyAddition) -> Result<()> {
    let file = AbsPath::new(&addition.file)?;
    let mut directories = Vec::new();
    let mut parent = file.parent();
    while let Some(dir) = parent {
        if dir.as_path().exists() {
            break;
        }
        parent = dir.parent();
        directories.push(dir);
    }
    directories.reverse();
    let existing = std::fs::read(file.as_path()).ok();
    let mut targets: Vec<TargetSpec> = directories
        .iter()
        .map(|dir| spec(dir.clone(), TargetAction::Create, Expect::Absent))
        .collect();
    let mut content = Vec::new();
    let file_action = match &existing {
        Some(bytes) => {
            if bytes.last().is_some_and(|&last| last != b'\n') {
                content.push(b'\n');
            }
            TargetAction::Append
        }
        None => TargetAction::Create,
    };
    content.extend_from_slice(addition.line.as_bytes());
    content.push(b'\n');
    let expect = if existing.is_some() { Expect::Unchecked } else { Expect::Absent };
    targets.push(spec(file, file_action, expect));
    let preserved = tx.preserve(&targets)?;
    let (file_target, dir_targets) = preserved
        .split_last()
        .ok_or_else(|| Error::new(ErrorCode::Internal, "journal returned no preserved target"))?;
    for dir in dir_targets {
        tx.apply(dir.index, Mutation::CreateDir { mode: DIRECTORY_MODE })?;
    }
    let mutation = if existing.is_some() {
        Mutation::AppendBytes(content)
    } else {
        Mutation::WriteBytes(content)
    };
    tx.apply(file_target.index, mutation)?;
    Ok(())
}

fn spec(path: AbsPath, action: TargetAction, expect: Expect) -> TargetSpec {
    TargetSpec {
        path: TargetPath::Fs(path),
        action,
        expect,
        size_hint: None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marker {
    None,
    Revoked,
    CertAuthority,
}

#[derive(Debug, Clone)]
enum Hosts {
    Patterns(Vec<String>),
    Hashed { salt: Vec<u8>, hash: Vec<u8> },
}

impl Hosts {
    fn matches(&self, name: &str) -> bool {
        match self {
            Self::Patterns(patterns) => match_list(name, patterns.iter().map(String::as_str)),
            Self::Hashed { salt, hash } => hmac_sha1(salt, name.as_bytes()) == hash.as_slice(),
        }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    marker: Marker,
    hosts: Hosts,
    key: PublicKey,
    location: Location,
}

/// A key recorded for a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KnownKey {
    pub(crate) fingerprint: String,
    pub(crate) location: Location,
}

/// Result of looking up a presented key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// A line for the host holds this key.
    Trusted,
    /// The key is listed with `@revoked`.
    Revoked(Location),
    /// The host has a different key of the same type.
    Changed(Vec<KnownKey>),
    /// No key of this type is recorded for the host.
    Unknown,
}

/// Entries of several `known_hosts` files.
#[derive(Debug, Clone, Default)]
pub(crate) struct KnownHosts {
    entries: Vec<Entry>,
}

impl KnownHosts {
    /// Reads the files in order; absent files and malformed lines are
    /// skipped, as OpenSSH does.
    pub(crate) fn load(files: &[PathBuf]) -> Self {
        let mut entries = Vec::new();
        for file in files {
            if let Ok(bytes) = std::fs::read(file) {
                entries.extend(parse(&String::from_utf8_lossy(&bytes), file));
            }
        }
        Self { entries }
    }

    /// Looks up `key` for the host recorded as `name`.
    pub(crate) fn check(&self, name: &str, key: &PublicKey) -> Verdict {
        let name = name.to_ascii_lowercase();
        let for_host = || self.entries.iter().filter(|entry| entry.hosts.matches(&name));
        if let Some(revoked) = for_host().find(|entry| entry.marker == Marker::Revoked && same_key(&entry.key, key))
        {
            return Verdict::Revoked(revoked.location.clone());
        }
        let plain = || for_host().filter(|entry| entry.marker == Marker::None);
        if plain().any(|entry| same_key(&entry.key, key)) {
            return Verdict::Trusted;
        }
        let changed: Vec<KnownKey> = plain()
            .filter(|entry| same_type(&entry.key.algorithm(), &key.algorithm()))
            .map(|entry| KnownKey {
                fingerprint: fingerprint(&entry.key),
                location: entry.location.clone(),
            })
            .collect();
        if changed.is_empty() {
            Verdict::Unknown
        } else {
            Verdict::Changed(changed)
        }
    }

    /// Key algorithms recorded for the host, in file order, so that the
    /// key exchange can prefer a key that is already known.
    pub(crate) fn algorithms_for(&self, name: &str) -> Vec<Algorithm> {
        let name = name.to_ascii_lowercase();
        let mut algorithms: Vec<Algorithm> = Vec::new();
        for entry in &self.entries {
            if entry.marker == Marker::None && entry.hosts.matches(&name) {
                let algorithm = entry.key.algorithm();
                if !algorithms.contains(&algorithm) {
                    algorithms.push(algorithm);
                }
            }
        }
        algorithms
    }
}

fn parse(text: &str, file: &Path) -> Vec<Entry> {
    text.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            parse_line(
                line,
                Location {
                    file: file.to_path_buf(),
                    line: index + 1,
                },
            )
        })
        .collect()
}

fn parse_line(line: &str, location: Location) -> Option<Entry> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut fields = line.split_whitespace();
    let mut first = fields.next()?;
    let marker = match first {
        "@revoked" => Marker::Revoked,
        "@cert-authority" => Marker::CertAuthority,
        _ => Marker::None,
    };
    if marker != Marker::None {
        first = fields.next()?;
    }
    let hosts = parse_hosts(first)?;
    let _key_type = fields.next()?;
    let key = russh::keys::parse_public_key_base64(fields.next()?).ok()?;
    Some(Entry {
        marker,
        hosts,
        key,
        location,
    })
}

fn parse_hosts(field: &str) -> Option<Hosts> {
    match field.strip_prefix("|1|") {
        Some(hashed) => {
            let (salt, hash) = hashed.split_once('|')?;
            Some(Hosts::Hashed {
                salt: STANDARD.decode(salt).ok()?,
                hash: STANDARD.decode(hash).ok()?,
            })
        }
        None => Some(Hosts::Patterns(field.split(',').map(str::to_owned).collect())),
    }
}

/// HMAC-SHA1 (RFC 2104).
fn hmac_sha1(key: &[u8], message: &[u8]) -> Vec<u8> {
    let mut block = [0u8; SHA1_BLOCK];
    if key.len() > SHA1_BLOCK {
        let digest = Sha1::digest(key);
        block[..digest.len()].copy_from_slice(&digest);
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let pad = |byte: u8| block.iter().map(move |k| k ^ byte).collect::<Vec<u8>>();
    let mut inner = Sha1::new();
    inner.update(pad(0x36));
    inner.update(message);
    let inner = inner.finalize();
    let mut outer = Sha1::new();
    outer.update(pad(0x5c));
    outer.update(inner);
    outer.finalize().to_vec()
}

fn same_key(a: &PublicKey, b: &PublicKey) -> bool {
    a.key_data() == b.key_data()
}

fn same_type(a: &Algorithm, b: &Algorithm) -> bool {
    match (a, b) {
        (Algorithm::Rsa { .. }, Algorithm::Rsa { .. }) => true,
        _ => a == b,
    }
}

/// SHA-256 fingerprint in the OpenSSH form `SHA256:…`.
pub(crate) fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

/// Line of a known hosts file for `name` and `key`, without a comment.
pub(crate) fn host_line(name: &str, key: &PublicKey) -> Result<String> {
    let bare = PublicKey::new(key.key_data().clone(), "");
    let encoded = bare.to_openssh().map_err(|err| {
        Error::new(ErrorCode::Internal, format!("cannot encode host key: {err}"))
    })?;
    Ok(format!("{name} {encoded}"))
}

/// Checking requested for unknown hosts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Policy {
    /// Unknown hosts are refused.
    Strict,
    /// Unknown hosts are accepted and recorded.
    AcceptNew,
    /// Unknown hosts are accepted without recording.
    Off,
}

/// Everything needed to judge the key presented by one host.
#[derive(Debug, Clone)]
pub(crate) struct HostKeyCheck {
    /// Host as requested, for messages.
    pub(crate) host: String,
    /// Name of the host in known hosts files.
    pub(crate) name: String,
    /// Entries of every known hosts file in use.
    pub(crate) known: KnownHosts,
    /// Files that were read, for messages.
    pub(crate) files: Vec<PathBuf>,
    /// Checking of unknown hosts.
    pub(crate) policy: Policy,
    /// Fingerprint accepted explicitly for an unknown host.
    pub(crate) accept_fingerprint: Option<String>,
    /// File that receives accepted keys.
    pub(crate) record_file: Option<PathBuf>,
    /// Call to repeat with an accepted fingerprint, for the fix of errors.
    pub(crate) retry_command: Option<String>,
}

/// Outcome of a host key check.
#[derive(Debug, Clone)]
pub(crate) enum Decision {
    /// The key is trusted; the addition, if any, is to be recorded.
    Accept {
        addition: Option<HostKeyAddition>,
        notice: Option<SshNotice>,
    },
    /// The key is refused with this error.
    Reject(Error),
}

impl HostKeyCheck {
    /// Judges the key presented by the server.
    pub(crate) fn decide(&self, key: &PublicKey) -> Decision {
        match self.known.check(&self.name, key) {
            Verdict::Trusted => Decision::Accept {
                addition: None,
                notice: None,
            },
            Verdict::Revoked(location) => Decision::Reject(self.revoked(key, &location)),
            Verdict::Changed(known) => Decision::Reject(self.changed(key, &known)),
            Verdict::Unknown => self.unknown(key),
        }
    }

    fn unknown(&self, key: &PublicKey) -> Decision {
        let presented = fingerprint(key);
        let explicit = self.accept_fingerprint.as_deref() == Some(presented.as_str());
        if explicit || self.policy == Policy::AcceptNew {
            return match self.addition(key, &presented) {
                Ok(addition) => Decision::Accept { addition, notice: None },
                Err(error) => Decision::Reject(error),
            };
        }
        if self.policy == Policy::Off {
            return Decision::Accept {
                addition: None,
                notice: Some(SshNotice::HostKeyUnverified {
                    host: self.host.clone(),
                    fingerprint: presented,
                }),
            };
        }
        Decision::Reject(self.unknown_error(&presented))
    }

    fn addition(&self, key: &PublicKey, presented: &str) -> Result<Option<HostKeyAddition>> {
        let Some(file) = &self.record_file else {
            return Ok(None);
        };
        Ok(Some(HostKeyAddition {
            file: file.clone(),
            line: host_line(&self.name, key)?,
            host: self.name.clone(),
            fingerprint: presented.to_owned(),
        }))
    }

    fn unknown_error(&self, presented: &str) -> Error {
        let mismatch = self.accept_fingerprint.is_some();
        let message = if mismatch {
            format!(
                "host key of `{}` has fingerprint {presented}, not the accepted one; verify the fingerprint with the server administrator",
                self.host
            )
        } else {
            format!(
                "host key of `{}` is unknown; verify fingerprint {presented} with the server administrator and accept it",
                self.host
            )
        };
        let mut error = Error::new(ErrorCode::GitHostKey, message)
            .with_problem(if mismatch { "fingerprint_mismatch" } else { "unknown" })
            .with_found(presented.to_owned())
            .with_help("git.ssh");
        if let Some(command) = &self.retry_command {
            error = error.with_fix(format!("{command} accept_host_key: \"{presented}\""));
        }
        for file in &self.files {
            error = error.with_sample(file.display().to_string());
        }
        error
    }

    fn changed(&self, key: &PublicKey, known: &[KnownKey]) -> Error {
        let mut error = Error::new(
            ErrorCode::GitHostKey,
            format!(
                "host key of `{}` changed to {}; the connection may be intercepted; if the change is expected, remove the old line from the file",
                self.host,
                fingerprint(key)
            ),
        )
        .with_problem("changed")
        .with_found(fingerprint(key))
        .with_help("git.ssh");
        for entry in known {
            error = error.with_sample(format!(
                "{}:{} {}",
                entry.location.file.display(),
                entry.location.line,
                entry.fingerprint
            ));
        }
        error
    }

    fn revoked(&self, key: &PublicKey, location: &Location) -> Error {
        Error::new(
            ErrorCode::GitHostKey,
            format!("host key {} of `{}` is revoked", fingerprint(key), self.host),
        )
        .with_problem("revoked")
        .with_found(fingerprint(key))
        .with_sample(format!("{}:{}", location.file.display(), location.line))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::PrivateKey;

    fn key(seed: u8) -> PublicKey {
        let private = PrivateKey::from(russh::keys::ssh_key::private::Ed25519Keypair::from_seed(&[seed; 32]));
        private.public_key().clone()
    }

    fn known(text: &str) -> KnownHosts {
        KnownHosts {
            entries: parse(text, Path::new("/home/alice/.ssh/known_hosts")),
        }
    }

    fn line(name: &str, seed: u8) -> String {
        host_line(name, &key(seed)).unwrap()
    }

    fn check(known: KnownHosts, policy: Policy) -> HostKeyCheck {
        HostKeyCheck {
            host: "example.com".into(),
            name: "example.com".into(),
            known,
            files: vec![PathBuf::from("/home/alice/.ssh/known_hosts")],
            policy,
            accept_fingerprint: None,
            record_file: Some(PathBuf::from("/home/alice/.config/nurt/known_hosts")),
            retry_command: Some("git.clone \"git@example.com:repo.git\" to: \"/home/alice/repo\"".into()),
        }
    }

    #[test]
    fn hmac_sha1_matches_rfc_2202_vectors() {
        let digest = hmac_sha1(&[0x0b; 20], b"Hi There");
        assert_eq!(hex::encode(digest), "b617318655057264e28bc0b6fb378c8ef146be00");
        let digest = hmac_sha1(&[0xaa; 80], b"Test Using Larger Than Block-Size Key - Hash Key First");
        assert_eq!(hex::encode(digest), "aa4ae5e15272d00e95705637ce8a3b55ed402112");
    }

    #[test]
    fn plain_entry_is_trusted() {
        let known = known(&format!("{}\n", line("example.com", 1)));
        assert_eq!(known.check("EXAMPLE.com", &key(1)), Verdict::Trusted);
    }

    #[test]
    fn pattern_list_with_negation() {
        let known = known(&format!("*.example.com,!build.example.com {}\n", &line("x", 1)[2..]));
        assert_eq!(known.check("git.example.com", &key(1)), Verdict::Trusted);
        assert_eq!(known.check("build.example.com", &key(1)), Verdict::Unknown);
    }

    #[test]
    fn non_default_port_uses_bracket_form() {
        let known = known(&format!("{}\n", line("[example.com]:2222", 1)));
        assert_eq!(known.check("[example.com]:2222", &key(1)), Verdict::Trusted);
        assert_eq!(known.check("example.com", &key(1)), Verdict::Unknown);
    }

    #[test]
    fn hashed_entry_is_matched() {
        let salt = [7u8; 20];
        let hash = hmac_sha1(&salt, b"example.com");
        let encoded = &line("x", 1)[2..];
        let text = format!("|1|{}|{} {encoded}\n", STANDARD.encode(salt), STANDARD.encode(hash));
        let known = known(&text);
        assert_eq!(known.check("example.com", &key(1)), Verdict::Trusted);
        assert_eq!(known.check("example.org", &key(1)), Verdict::Unknown);
    }

    #[test]
    fn different_key_of_same_type_is_a_change() {
        let known = known(&format!("# comment\n\n{}\n", line("example.com", 1)));
        match known.check("example.com", &key(2)) {
            Verdict::Changed(keys) => {
                assert_eq!(keys.len(), 1);
                assert_eq!(keys[0].location.line, 3);
                assert_eq!(keys[0].fingerprint, fingerprint(&key(1)));
            }
            other => panic!("unexpected verdict {other:?}"),
        }
    }

    #[test]
    fn revoked_key_is_refused_even_when_listed() {
        let text = format!("{}\n@revoked * {}\n", line("example.com", 1), &line("x", 1)[2..]);
        assert!(matches!(known(&text).check("example.com", &key(1)), Verdict::Revoked(_)));
    }

    #[test]
    fn certificate_authority_lines_do_not_trust_keys() {
        let text = format!("@cert-authority * {}\n", &line("x", 1)[2..]);
        assert_eq!(known(&text).check("example.com", &key(1)), Verdict::Unknown);
    }

    #[test]
    fn malformed_lines_are_skipped() {
        let text = format!("garbage\nexample.com ssh-ed25519 !!!\n{}\n", line("example.com", 1));
        assert_eq!(known(&text).check("example.com", &key(1)), Verdict::Trusted);
    }

    #[test]
    fn algorithms_of_known_keys_are_listed_once() {
        let text = format!("{}\n{}\n", line("example.com", 1), line("example.com", 2));
        assert_eq!(known(&text).algorithms_for("example.com"), [Algorithm::Ed25519]);
        assert!(known(&text).algorithms_for("other").is_empty());
    }

    #[test]
    fn load_reads_several_files_and_skips_missing_ones() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("a");
        std::fs::write(&first, format!("{}\n", line("one", 1))).unwrap();
        let known = KnownHosts::load(&[dir.path().join("missing"), first]);
        assert_eq!(known.check("one", &key(1)), Verdict::Trusted);
    }

    #[test]
    fn strict_policy_refuses_unknown_host_with_fingerprint_fix() {
        let decision = check(KnownHosts::default(), Policy::Strict).decide(&key(1));
        let Decision::Reject(error) = decision else {
            panic!("unknown host accepted");
        };
        assert_eq!(error.code(), ErrorCode::GitHostKey);
        assert_eq!(error.data().problem.as_deref(), Some("unknown"));
        let fix = format!("accept_host_key: \"{}\"", fingerprint(&key(1)));
        assert!(error.data().fix.iter().any(|program| program.ends_with(&fix)));
    }

    #[test]
    fn accept_new_records_unknown_host() {
        let decision = check(KnownHosts::default(), Policy::AcceptNew).decide(&key(1));
        let Decision::Accept { addition: Some(addition), notice: None } = decision else {
            panic!("unknown host not recorded");
        };
        assert_eq!(addition.host, "example.com");
        assert_eq!(addition.line, line("example.com", 1));
        assert_eq!(addition.fingerprint, fingerprint(&key(1)));
    }

    #[test]
    fn explicit_fingerprint_accepts_only_that_key() {
        let mut check = check(KnownHosts::default(), Policy::Strict);
        check.accept_fingerprint = Some(fingerprint(&key(1)));
        assert!(matches!(check.decide(&key(1)), Decision::Accept { addition: Some(_), .. }));
        let Decision::Reject(error) = check.decide(&key(2)) else {
            panic!("other key accepted");
        };
        assert_eq!(error.data().problem.as_deref(), Some("fingerprint_mismatch"));
    }

    #[test]
    fn changed_key_is_refused_whatever_the_policy() {
        let known = known(&format!("{}\n", line("example.com", 1)));
        for policy in [Policy::Strict, Policy::AcceptNew, Policy::Off] {
            let mut check = check(known.clone(), policy);
            check.accept_fingerprint = Some(fingerprint(&key(2)));
            let Decision::Reject(error) = check.decide(&key(2)) else {
                panic!("changed key accepted with {policy:?}");
            };
            assert_eq!(error.data().problem.as_deref(), Some("changed"));
        }
    }

    #[test]
    fn off_policy_accepts_with_notice() {
        let decision = check(KnownHosts::default(), Policy::Off).decide(&key(1));
        assert!(matches!(
            decision,
            Decision::Accept {
                addition: None,
                notice: Some(SshNotice::HostKeyUnverified { .. })
            }
        ));
    }

    #[test]
    fn revoked_decision_is_an_error() {
        let text = format!("@revoked example.com {}\n", &line("x", 1)[2..]);
        let Decision::Reject(error) = check(known(&text), Policy::AcceptNew).decide(&key(1)) else {
            panic!("revoked key accepted");
        };
        assert_eq!(error.data().problem.as_deref(), Some("revoked"));
    }
}
