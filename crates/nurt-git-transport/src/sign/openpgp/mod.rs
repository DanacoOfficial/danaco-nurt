// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! OpenPGP signatures: key selection, signature packets and verification.
//!
//! Signature packets are version 4, type binary, with the hashed subpackets
//! issuer fingerprint, creation time and (when the key was found by a user
//! ID) signer's user ID, and the unhashed subpacket issuer key ID, in the
//! order GnuPG writes them. The digest algorithm is the first of SHA-512,
//! SHA-384, SHA-256 and SHA-224 in the preferences of the key, otherwise
//! SHA-256.

mod agent;
mod keyring;

use std::path::Path;

use nurt_contracts::error::{Error, Result};
use pgp::bytes::Bytes;
use pgp::composed::{
    ArmorOptions, Deserializable, DetachedSignature, SignedPublicKey, SignedSecretKey,
};
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::public_key::PublicKeyAlgorithm;
use pgp::packet::{
    PacketHeader, PacketTrait, SignatureConfig, SignatureType, Subpacket, SubpacketData,
};
use pgp::types::{
    Fingerprint, KeyDetails, KeyVersion, PacketHeaderVersion, Password, SigningKey, Tag, Timestamp,
};

use super::error::{
    AGENT_UNAVAILABLE, NO_KEY, UNSUPPORTED_FORMAT, is_problem, key_locked, no_key, sign_error,
    with_path,
};
use super::{
    CommitSignature, KeySource, OpenPgpSource, Signature, SignatureFormat, SignatureStatus,
    SigningConfig, VerifyConfig,
};

/// Digest algorithms used for signatures, in the order of preference when
/// the key states none.
const SIGNATURE_HASHES: [HashAlgorithm; 4] = [
    HashAlgorithm::Sha512,
    HashAlgorithm::Sha384,
    HashAlgorithm::Sha256,
    HashAlgorithm::Sha224,
];

pub(crate) fn sign(payload: &[u8], config: &SigningConfig) -> Result<Signature> {
    let mut first_error: Option<Error> = None;
    for source in &config.openpgp_sources {
        let attempt = match source {
            OpenPgpSource::Agent => sign_with_agent(payload, config),
            OpenPgpSource::KeyFile => sign_with_key_file(payload, config),
        };
        match attempt {
            Ok(Some(signature)) => return Ok(signature),
            Ok(None) => {}
            Err(error) if is_problem(&error, NO_KEY) || is_problem(&error, AGENT_UNAVAILABLE) => {
                first_error.get_or_insert(error);
            }
            Err(error) => return Err(error),
        }
    }
    Err(first_error.unwrap_or_else(|| {
        no_key(
            "no OpenPGP signing key is configured; set `user.signingKey` or `git.signing.key_file`",
        )
    }))
}

pub(crate) fn verify(
    payload: &[u8],
    signature: &[u8],
    config: &VerifyConfig,
) -> Result<CommitSignature> {
    let mut status = CommitSignature {
        status: SignatureStatus::Bad,
        format: Some(SignatureFormat::Openpgp),
        key: None,
        signer: None,
    };
    let Ok((detached, _)) = DetachedSignature::from_armor_single(signature) else {
        return Ok(status);
    };
    let issuer_fingerprints: Vec<Fingerprint> = detached
        .signature
        .issuer_fingerprint()
        .into_iter()
        .cloned()
        .collect();
    let issuer_ids: Vec<String> = detached
        .signature
        .issuer_key_id()
        .into_iter()
        .map(|id| hex::encode_upper(id.as_ref()))
        .collect();
    status.key = issuer_fingerprints
        .first()
        .map(fingerprint_hex)
        .or_else(|| issuer_ids.first().cloned());
    let issued_by = |key: &dyn KeyDetails| {
        let fingerprint = key.fingerprint();
        issuer_fingerprints.contains(&fingerprint)
            || issuer_ids.contains(&hex::encode_upper(key.legacy_key_id().as_ref()))
    };
    let mut certs = keyring::load_files(&config.public_key_files)?;
    if let Some(home) = &config.gnupg_home {
        certs.extend(keyring::load_gnupg(home));
    }
    let now = unix_now();
    let mut matched = false;
    for cert in &certs {
        if issued_by(&cert.primary_key) {
            matched = true;
            if detached.verify(&cert.primary_key, payload).is_ok() {
                // A revoked or expired key does not vouch for the signature,
                // as in `gpg`, which reports it as not good.
                if cert_unusable(cert, now) {
                    return Ok(status);
                }
                return Ok(good(status, cert, &cert.primary_key));
            }
        }
        for subkey in &cert.public_subkeys {
            if issued_by(&subkey.key) {
                matched = true;
                if detached.verify(&subkey.key, payload).is_ok() {
                    if cert_unusable(cert, now) || !subkey_usable(cert, subkey, now) {
                        return Ok(status);
                    }
                    return Ok(good(status, cert, &subkey.key));
                }
            }
        }
    }
    if !matched {
        status.status = SignatureStatus::UnknownKey;
    }
    Ok(status)
}

fn good(
    mut status: CommitSignature,
    cert: &SignedPublicKey,
    key: &dyn KeyDetails,
) -> CommitSignature {
    status.status = SignatureStatus::Good;
    status.key = Some(fingerprint_hex(&key.fingerprint()));
    status.signer = cert
        .details
        .users
        .first()
        .map(|user| String::from_utf8_lossy(user.id.id()).into_owned());
    status
}

pub(crate) fn fingerprint_hex(fingerprint: &Fingerprint) -> String {
    hex::encode_upper(fingerprint.as_bytes())
}

fn sign_with_key_file(payload: &[u8], config: &SigningConfig) -> Result<Option<Signature>> {
    let Some(path) = &config.key_file else {
        return Ok(None);
    };
    let bytes = std::fs::read(path).map_err(|source| {
        with_path(
            no_key(format!(
                "cannot read the OpenPGP key file `{}`: {source}; set `git.signing.key_file` to a readable secret key file",
                path.display()
            )),
            "key_file",
            path,
        )
    })?;
    let secret = parse_secret_key(&bytes).ok_or_else(|| {
        with_path(
            no_key(format!(
                "`{}` is not an OpenPGP secret key; export it with `gpg --export-secret-keys --armor`",
                path.display()
            )),
            "key_file",
            path,
        )
    })?;
    let public = secret.to_public_key();
    let selection = match &config.signing_key {
        Some(spec) => select(&public, &KeySpec::parse(spec)).ok_or_else(|| {
            with_path(
                no_key(format!(
                    "the key file `{}` holds no signing key matching `{spec}`; correct `user.signingKey`",
                    path.display()
                )),
                "key_file",
                path,
            )
        })?,
        None => best_signing_key(&public, None, unix_now()).ok_or_else(|| {
            with_path(
                no_key(format!("the key file `{}` holds no key that can sign", path.display())),
                "key_file",
                path,
            )
        })?,
    };
    let hash = preferred_hash(&public);
    let armored = if selection.fingerprint == secret.primary_key.fingerprint() {
        sign_with_secret(&secret.primary_key, path, payload, config, hash, &selection)?
    } else {
        let subkey = secret
            .secret_subkeys
            .iter()
            .find(|subkey| subkey.key.fingerprint() == selection.fingerprint)
            .ok_or_else(|| {
                no_key(format!(
                    "the key file `{}` holds only the public part of the signing subkey",
                    path.display()
                ))
            })?;
        sign_with_secret(&subkey.key, path, payload, config, hash, &selection)?
    };
    Ok(Some(Signature {
        format: SignatureFormat::Openpgp,
        armored,
        key: fingerprint_hex(&selection.fingerprint),
        source: KeySource::KeyFile,
    }))
}

trait LockableKey: SigningKey {
    fn is_locked(&self) -> bool;
}

impl LockableKey for pgp::packet::SecretKey {
    fn is_locked(&self) -> bool {
        self.secret_params().is_encrypted()
    }
}

impl LockableKey for pgp::packet::SecretSubkey {
    fn is_locked(&self) -> bool {
        self.secret_params().is_encrypted()
    }
}

fn sign_with_secret(
    key: &impl LockableKey,
    path: &Path,
    payload: &[u8],
    config: &SigningConfig,
    hash: HashAlgorithm,
    selection: &Selection,
) -> Result<Vec<u8>> {
    let password = if key.is_locked() {
        let Some(file) = &config.passphrase_file else {
            return Err(with_path(
                key_locked(format!(
                    "the OpenPGP key in `{}` is protected by a passphrase; set `NURT_GPG_PASSPHRASE_FILE` to a file holding it",
                    path.display()
                )),
                "key_file",
                path,
            ));
        };
        Password::from(read_passphrase(file)?.as_str())
    } else {
        Password::empty()
    };
    let packet = signature_packet(key, &password, hash, config.created, selection, payload)
        .map_err(|error| {
            if key.is_locked() {
                key_locked(format!(
                    "the passphrase from `NURT_GPG_PASSPHRASE_FILE` does not unlock the key in `{}`",
                    path.display()
                ))
            } else {
                error
            }
        })?;
    armor(packet)
}

/// Reads the first line of a passphrase file, as `gpg` does; the file must
/// grant no access to the group or to other accounts.
pub(crate) fn read_passphrase(path: &Path) -> Result<String> {
    let metadata = std::fs::metadata(path).map_err(|source| {
        with_path(
            key_locked(format!(
                "cannot read the passphrase file `{}`: {source}",
                path.display()
            )),
            "passphrase_file",
            path,
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        const GROUP_OTHER_BITS: u32 = 0o077;
        if metadata.permissions().mode() & GROUP_OTHER_BITS != 0 {
            return Err(with_path(
                key_locked(format!(
                    "the passphrase file `{}` is readable by other accounts; restrict it with mode 0600",
                    path.display()
                )),
                "passphrase_file",
                path,
            ));
        }
    }
    #[cfg(not(unix))]
    let _ = metadata;
    let text = std::fs::read_to_string(path).map_err(|source| {
        with_path(
            key_locked(format!(
                "cannot read the passphrase file `{}`: {source}",
                path.display()
            )),
            "passphrase_file",
            path,
        )
    })?;
    Ok(text.lines().next().unwrap_or_default().to_owned())
}

fn parse_secret_key(bytes: &[u8]) -> Option<SignedSecretKey> {
    if keyring::is_armored(bytes) {
        SignedSecretKey::from_armor_single(bytes)
            .ok()
            .map(|(key, _)| key)
    } else {
        SignedSecretKey::from_bytes(bytes).ok()
    }
}

fn sign_with_agent(payload: &[u8], config: &SigningConfig) -> Result<Option<Signature>> {
    let Some(spec) = config
        .signing_key
        .as_ref()
        .or(config.default_identity.as_ref())
    else {
        return Err(no_key(
            "no OpenPGP key is named; set `user.signingKey` or the committer identity",
        ));
    };
    let Some(home) = &config.gnupg_home else {
        return Err(no_key(
            "the GnuPG home directory is unknown; set `GNUPGHOME`",
        ));
    };
    let key_spec = KeySpec::parse(spec);
    let certs = keyring::load_gnupg(home);
    let found = certs
        .iter()
        .find_map(|cert| select(cert, &key_spec).map(|selection| (cert, selection)));
    let Some((cert, selection)) = found else {
        return Err(with_path(
            no_key(format!(
                "no OpenPGP public key matching `{spec}` is in the GnuPG keyrings of `{}`; import it or correct `user.signingKey`",
                home.display()
            )),
            "gnupg_home",
            home,
        ));
    };
    let hash = preferred_hash(cert);
    let armored = if selection.fingerprint == cert.primary_key.fingerprint() {
        let armored = agent::sign(&cert.primary_key, config, hash, &selection, payload)?;
        check_created(&armored, &cert.primary_key, payload)?;
        armored
    } else {
        let subkey = cert
            .public_subkeys
            .iter()
            .find(|subkey| subkey.key.fingerprint() == selection.fingerprint)
            .ok_or_else(|| no_key(format!("no OpenPGP subkey matches `{spec}`")))?;
        let armored = agent::sign(&subkey.key, config, hash, &selection, payload)?;
        check_created(&armored, &subkey.key, payload)?;
        armored
    };
    Ok(Some(Signature {
        format: SignatureFormat::Openpgp,
        armored,
        key: fingerprint_hex(&selection.fingerprint),
        source: KeySource::Agent,
    }))
}

/// Verifies a signature just obtained from the agent against the public key
/// that was meant to make it, so that a wrong or damaged answer of the agent
/// is never stored in a commit.
fn check_created(
    armored: &[u8],
    key: &impl pgp::types::VerifyingKey,
    payload: &[u8],
) -> Result<()> {
    let invalid = |reason: String| {
        sign_error(
            "signature_check",
            format!(
                "the signature made by the agent does not verify: {reason}; check the key in `gpg-agent`"
            ),
        )
    };
    let (detached, _) = DetachedSignature::from_armor_single(armored)
        .map_err(|error| invalid(error.to_string()))?;
    detached
        .verify(key, payload)
        .map_err(|error| invalid(error.to_string()))
}

/// Builds a version 4 binary signature packet over `payload`.
pub(crate) fn signature_packet(
    key: &impl SigningKey,
    password: &Password,
    hash: HashAlgorithm,
    created: i64,
    selection: &Selection,
    payload: &[u8],
) -> Result<pgp::packet::Signature> {
    if key.version() != KeyVersion::V4 {
        return Err(sign_error(
            UNSUPPORTED_FORMAT,
            format!(
                "OpenPGP keys of version {:?} cannot sign Git objects; use a version 4 key",
                key.version()
            ),
        ));
    }
    let created = Timestamp::from_secs(u32::try_from(created.max(0)).unwrap_or(u32::MAX));
    let packet_error = |error: pgp::errors::Error| {
        sign_error(
            UNSUPPORTED_FORMAT,
            format!("cannot create the signature: {error}"),
        )
    };
    let mut hashed = vec![
        Subpacket::regular(SubpacketData::IssuerFingerprint(key.fingerprint()))
            .map_err(packet_error)?,
        Subpacket::regular(SubpacketData::SignatureCreationTime(created)).map_err(packet_error)?,
    ];
    if let Some(mailbox) = &selection.signer_mailbox {
        hashed.push(
            Subpacket::regular(SubpacketData::SignersUserID(Bytes::from(
                mailbox.clone().into_bytes(),
            )))
            .map_err(packet_error)?,
        );
    }
    let mut signature_config = SignatureConfig::v4(SignatureType::Binary, key.algorithm(), hash);
    signature_config.hashed_subpackets = hashed;
    signature_config.unhashed_subpackets = vec![
        Subpacket::regular(SubpacketData::IssuerKeyId(key.legacy_key_id()))
            .map_err(packet_error)?,
    ];
    let packet = signature_config
        .sign(key, password, payload)
        .map_err(packet_error)?;
    with_legacy_header(packet).map_err(packet_error)
}

/// The same packet with the legacy packet header that GnuPG writes for
/// signatures, so that the armored output equals `gpg --detach-sign`.
fn with_legacy_header(
    packet: pgp::packet::Signature,
) -> pgp::errors::Result<pgp::packet::Signature> {
    let header = PacketHeader::from_parts(
        PacketHeaderVersion::Old,
        Tag::Signature,
        packet.packet_header().packet_length(),
    )?;
    let (Some(config), Some(signed_hash_value), Some(values)) = (
        packet.config(),
        packet.signed_hash_value(),
        packet.signature(),
    ) else {
        return Ok(packet);
    };
    Ok(pgp::packet::Signature::v4(
        header,
        config.typ,
        config.pub_alg,
        config.hash_alg,
        signed_hash_value,
        values.clone(),
        config.hashed_subpackets.clone(),
        config.unhashed_subpackets.clone(),
    ))
}

pub(crate) fn armor(packet: pgp::packet::Signature) -> Result<Vec<u8>> {
    DetachedSignature::new(packet)
        .to_armored_bytes(ArmorOptions::default())
        .map_err(|error| {
            sign_error(
                UNSUPPORTED_FORMAT,
                format!("cannot armor the signature: {error}"),
            )
        })
}

fn preferred_hash(cert: &SignedPublicKey) -> HashAlgorithm {
    cert.details
        .users
        .iter()
        .flat_map(|user| user.signatures.iter())
        .flat_map(|signature| signature.preferred_hash_algs().iter().copied())
        .find(|hash| SIGNATURE_HASHES.contains(hash))
        .unwrap_or(HashAlgorithm::Sha256)
}

/// A key name in the forms GnuPG accepts after `-u`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KeySpec {
    /// A full fingerprint.
    Fingerprint { bytes: Vec<u8>, exact: bool },
    /// A 64-bit or 32-bit key ID, upper-case hexadecimal.
    KeyId { id: String, exact: bool },
    /// An e-mail address, matched exactly against the mailbox of user IDs.
    Mailbox(String),
    /// Any other text, matched case-insensitively inside user IDs.
    Substring(String),
}

const FINGERPRINT_V4_HEX: usize = 40;
const FINGERPRINT_V6_HEX: usize = 64;
const LONG_KEY_ID_HEX: usize = 16;
const SHORT_KEY_ID_HEX: usize = 8;

impl KeySpec {
    pub(crate) fn parse(text: &str) -> Self {
        let trimmed = text.trim();
        let (body, exact) = trimmed
            .strip_suffix('!')
            .map_or((trimmed, false), |body| (body, true));
        let hex_digits = body
            .strip_prefix("0x")
            .or_else(|| body.strip_prefix("0X"))
            .unwrap_or(body);
        let compact: String = hex_digits.chars().filter(|c| !c.is_whitespace()).collect();
        if !compact.is_empty() && compact.chars().all(|c| c.is_ascii_hexdigit()) {
            match compact.len() {
                FINGERPRINT_V4_HEX | FINGERPRINT_V6_HEX => {
                    if let Ok(bytes) = hex::decode(&compact) {
                        return Self::Fingerprint { bytes, exact };
                    }
                }
                LONG_KEY_ID_HEX | SHORT_KEY_ID_HEX => {
                    return Self::KeyId {
                        id: compact.to_ascii_uppercase(),
                        exact,
                    };
                }
                _ => {}
            }
        }
        if let Some(mailbox) = body
            .strip_prefix('<')
            .and_then(|rest| rest.strip_suffix('>'))
        {
            return Self::Mailbox(mailbox.to_lowercase());
        }
        if body.contains('@') && !body.contains(char::is_whitespace) && !body.contains('<') {
            return Self::Mailbox(body.to_lowercase());
        }
        Self::Substring(body.to_lowercase())
    }

    fn names_key(&self, key: &dyn KeyDetails) -> Option<bool> {
        match self {
            Self::Fingerprint { bytes, exact } => {
                (key.fingerprint().as_bytes() == bytes.as_slice()).then_some(*exact)
            }
            Self::KeyId { id, exact } => {
                let long = hex::encode_upper(key.legacy_key_id().as_ref());
                long.ends_with(id.as_str()).then_some(*exact)
            }
            Self::Mailbox(_) | Self::Substring(_) => None,
        }
    }

    fn matching_user<'a>(&self, cert: &'a SignedPublicKey) -> Option<&'a [u8]> {
        cert.details
            .users
            .iter()
            .map(|user| user.id.id())
            .find(|id| {
                let text = String::from_utf8_lossy(id).to_lowercase();
                match self {
                    Self::Mailbox(mailbox) => {
                        mailbox_of(&text).is_some_and(|found| found == *mailbox)
                    }
                    Self::Substring(part) => text.contains(part.as_str()),
                    Self::Fingerprint { .. } | Self::KeyId { .. } => false,
                }
            })
    }
}

/// The e-mail address inside a user ID, `Name <mail>` or a bare address.
pub(crate) fn mailbox_of(user_id: &str) -> Option<String> {
    if let (Some(start), Some(end)) = (user_id.rfind('<'), user_id.rfind('>'))
        && start < end
    {
        return Some(user_id[start + 1..end].to_owned());
    }
    user_id.contains('@').then(|| user_id.trim().to_owned())
}

/// The (sub)key chosen to sign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Selection {
    pub(crate) fingerprint: Fingerprint,
    /// Mailbox written as the signer's user ID subpacket.
    pub(crate) signer_mailbox: Option<String>,
}

/// Chooses the signing key of `cert` named by `spec`, as GnuPG does: a key
/// named with `!` is used as is; otherwise the newest valid signing subkey,
/// then the primary key.
///
/// A certificate whose primary key is revoked or expired, and a subkey that
/// is revoked or expired, are never selected.
pub(crate) fn select(cert: &SignedPublicKey, spec: &KeySpec) -> Option<Selection> {
    select_at(cert, spec, unix_now())
}

fn select_at(cert: &SignedPublicKey, spec: &KeySpec, now: u64) -> Option<Selection> {
    if cert_unusable(cert, now) {
        return None;
    }
    if let Some(exact) = spec.names_key(&cert.primary_key) {
        if exact {
            return primary_can_sign(cert).then(|| Selection {
                fingerprint: cert.primary_key.fingerprint(),
                signer_mailbox: None,
            });
        }
        return best_signing_key(cert, None, now);
    }
    for subkey in &cert.public_subkeys {
        if let Some(exact) = spec.names_key(&subkey.key) {
            if subkey_usable(cert, subkey, now) && (exact || subkey_can_sign(subkey)) {
                return Some(Selection {
                    fingerprint: subkey.key.fingerprint(),
                    signer_mailbox: None,
                });
            }
            return best_signing_key(cert, None, now);
        }
    }
    let user = spec.matching_user(cert)?;
    let mailbox = mailbox_of(&String::from_utf8_lossy(user)).map(|mailbox| mailbox.to_lowercase());
    best_signing_key(cert, mailbox, now)
}

/// Seconds since the Unix epoch.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Whether a key created at `created` with the given lifetime in seconds has
/// expired at `now`; a lifetime of zero or none never expires.
fn has_expired(created: u64, lifetime: Option<u64>, now: u64) -> bool {
    lifetime.is_some_and(|seconds| seconds != 0 && now >= created.saturating_add(seconds))
}

/// Key lifetime stated by the newest self-signature among `signatures`, as
/// `gpg` takes it: a newer signature without a lifetime lifts the expiry.
fn newest_lifetime<'a>(
    signatures: impl Iterator<Item = &'a pgp::packet::Signature>,
    issuer: &Fingerprint,
) -> Option<u64> {
    signatures
        .filter(|signature| {
            !matches!(
                signature.typ(),
                Some(
                    SignatureType::CertRevocation
                        | SignatureType::KeyRevocation
                        | SignatureType::SubkeyRevocation
                )
            )
        })
        .filter(|signature| {
            let issued = signature.issuer_fingerprint();
            issued.is_empty() || issued.contains(&issuer)
        })
        .max_by_key(|signature| signature.created().map(|created| created.as_secs()))
        .and_then(|signature| signature.key_expiration_time())
        .map(|lifetime| u64::from(lifetime.as_secs()))
}

/// Whether the primary key of `cert` is revoked or expired at `now`.
fn cert_unusable(cert: &SignedPublicKey, now: u64) -> bool {
    if !cert.details.revocation_signatures.is_empty() {
        return true;
    }
    let signatures = cert
        .details
        .users
        .iter()
        .flat_map(|user| user.signatures.iter())
        .chain(cert.details.direct_signatures.iter());
    let lifetime = newest_lifetime(signatures, &cert.primary_key.fingerprint());
    let created = u64::from(cert.primary_key.created_at().as_secs());
    has_expired(created, lifetime, now)
}

/// Whether `subkey` of `cert` is neither revoked nor expired at `now`; the
/// binding signatures of a subkey are made by the primary key.
fn subkey_usable(
    cert: &SignedPublicKey,
    subkey: &pgp::composed::SignedPublicSubKey,
    now: u64,
) -> bool {
    let revoked = subkey
        .signatures
        .iter()
        .any(|signature| signature.typ() == Some(SignatureType::SubkeyRevocation));
    let lifetime = newest_lifetime(subkey.signatures.iter(), &cert.primary_key.fingerprint());
    let created = u64::from(subkey.key.created_at().as_secs());
    !revoked && !has_expired(created, lifetime, now)
}

fn best_signing_key(
    cert: &SignedPublicKey,
    signer_mailbox: Option<String>,
    now: u64,
) -> Option<Selection> {
    if cert_unusable(cert, now) {
        return None;
    }
    let newest_subkey = cert
        .public_subkeys
        .iter()
        .filter(|subkey| subkey_usable(cert, subkey, now) && subkey_can_sign(subkey))
        .max_by_key(|subkey| subkey.key.created_at().as_secs());
    if let Some(subkey) = newest_subkey {
        return Some(Selection {
            fingerprint: subkey.key.fingerprint(),
            signer_mailbox,
        });
    }
    primary_can_sign(cert).then(|| Selection {
        fingerprint: cert.primary_key.fingerprint(),
        signer_mailbox,
    })
}

fn has_key_flags(signature: &pgp::packet::Signature) -> bool {
    signature.config().is_some_and(|config| {
        config
            .hashed_subpackets()
            .any(|packet| matches!(packet.data, SubpacketData::KeyFlags(_)))
    })
}

fn algorithm_can_sign(algorithm: PublicKeyAlgorithm) -> bool {
    matches!(
        algorithm,
        PublicKeyAlgorithm::RSA
            | PublicKeyAlgorithm::RSASign
            | PublicKeyAlgorithm::DSA
            | PublicKeyAlgorithm::ECDSA
            | PublicKeyAlgorithm::EdDSALegacy
            | PublicKeyAlgorithm::Ed25519
            | PublicKeyAlgorithm::Ed448
    )
}

fn primary_can_sign(cert: &SignedPublicKey) -> bool {
    if !algorithm_can_sign(cert.primary_key.algorithm()) {
        return false;
    }
    let mut flagged = cert
        .details
        .users
        .iter()
        .flat_map(|user| user.signatures.iter())
        .chain(cert.details.direct_signatures.iter())
        .filter(|signature| has_key_flags(signature))
        .peekable();
    if flagged.peek().is_none() {
        return true;
    }
    flagged.any(|signature| signature.key_flags().sign())
}

fn subkey_can_sign(subkey: &pgp::composed::SignedPublicSubKey) -> bool {
    algorithm_can_sign(subkey.key.algorithm())
        && !subkey
            .signatures
            .iter()
            .any(|signature| signature.typ() == Some(SignatureType::SubkeyRevocation))
        && subkey
            .signatures
            .iter()
            .any(|signature| has_key_flags(signature) && signature.key_flags().sign())
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Public keys generated with GnuPG 2.4 at 2026-01-01T00:00:00Z, and
    //! detached signatures it made over [`PAYLOAD`].

    pub(crate) const PAYLOAD: &[u8] = b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nauthor A <a@example.test> 1767225600 +0000\ncommitter A <a@example.test> 1767225600 +0000\n\nmsg\n";

    pub(crate) const CREATED: i64 = 1_767_225_600;

    pub(crate) const ED25519_PUBLIC: &str = "-----BEGIN PGP PUBLIC KEY BLOCK-----

mDMEaVW5ABYJKwYBBAHaRw8BAQdAV+rPTIef3tJBFiM6UNBPla/RO3ftgm+AmuMY
DXYa5EW0GVRlc3QgRWQgPGVkQGV4YW1wbGUudGVzdD6IkAQTFgoAOBYhBERlxwSC
b++FVft/WPY2yHmBURVaBQJpVbkAAhsDBQsJCAcCBhUKCQgLAgQWAgMBAh4BAheA
AAoJEPY2yHmBURVaM9wA/RMkov+gzlcZ6fPqx2m8H/UY4JwxbQzGDDyGkEC/nzH1
AQDl2Uclk5LHsLVZobmizzuyi9StkZuBQR8p/uNDduUPCg==
=T5dC
-----END PGP PUBLIC KEY BLOCK-----
";

    pub(crate) const ED25519_FINGERPRINT: &str = "4465C704826FEF8555FB7F58F636C8798151155A";

    pub(crate) const ED25519_KEYGRIP: &str = "3113052AB4195953005D8ECD72D2BF6C02777C8F";

    /// Signature made by `gpg -u ed@example.test --detach-sign --armor`.
    pub(crate) const ED25519_SIGNATURE: &str = "-----BEGIN PGP SIGNATURE-----

iIYEABYKAC4WIQREZccEgm/vhVX7f1j2Nsh5gVEVWgUCaVW5ABAcZWRAZXhhbXBs
ZS50ZXN0AAoJEPY2yHmBURVaPUgA/Av8EydpM0TJBrK36oKD4tz167ZhXm0Fp834
cN//o9dBAQCw+g4Wj0IRDHHraXGwGCgQv5LPNxGbLdqumzoUBj6TCQ==
=lh0N
-----END PGP SIGNATURE-----
";

    pub(crate) const RSA_PUBLIC: &str = "-----BEGIN PGP PUBLIC KEY BLOCK-----

mQENBGlVuQABCACqWC1FVoi61U8gC+4c7wsNJ1wOZDAvPiJ/MJqXXcduvgvV7YNr
7GDrpfFGslrcunJ7QrSc5DWuB6RlxjKbWVpyRoi/YDpQxRfjU0MDrd8eYDokPjEd
n18nBvT0poipyyYHGCQghDUkaJNb1ao09MsJNLC6rFB3WaJjy4nyCiPnXJT0F+L7
6Q4gqJ4PHSfAkwG+PLWfH1oNnfrOOUVQ21sYxU1osbhHRu+Pp3FJZpsA/joCNqV8
JDB4s5YIoHn5tEozA3RefzxBo5cxpK6925NwiXM8vYYPPb7sKs6THHPpF+pr0ifR
NP8uRMX+Q5qTDJFUi0++1x7QIYr32dbOaNl9ABEBAAG0G1Rlc3QgUnNhIDxyc2FA
ZXhhbXBsZS50ZXN0PokBTgQTAQoAOBYhBCYKTE9m3eZ1atHnccMYuqxIE8ZRBQJp
VbkAAhsDBQsJCAcCBhUKCQgLAgQWAgMBAh4BAheAAAoJEMMYuqxIE8ZRKjkH/Ak6
VxoR1VYbEcFf7Hm1rBIeAiwGYluRe96KKksdnJdFQ1ec3DTgSygkqRz34LLRyccO
3OQ+YikOWwLVCwnLRmnVePZrjT5nWlgRuH5I5e04VphC+RiU4dm0tiHYxahYtVJk
z1SgB7WqyD2viTpxsYmwXv+HBMQ7Z++CDmnrHz2RTzFhBtrWB+MRue8p8TOJUVUf
Raj9UcmRUqElbaYR34WfvTl/zy2jzU9H3x82YG9jQsqgSK1rCu9LFPaj1B/YKbMl
FD90B6narcKExP85daV5AZoweFdKoC077eCv/yFntbQEozokR/JyzKId9VrX0DMM
nLT0Q4pAsFK9zNc71t0=
=wVB4
-----END PGP PUBLIC KEY BLOCK-----
";

    pub(crate) const RSA_FINGERPRINT: &str = "260A4C4F66DDE6756AD1E771C318BAAC4813C651";

    pub(crate) const RSA_KEYGRIP: &str = "EA7F4532B9FF10A7C1B24E2EAE9808793D00EF8A";

    /// Signature made by `gpg -u rsa@example.test --detach-sign --armor`.
    pub(crate) const RSA_SIGNATURE: &str = "-----BEGIN PGP SIGNATURE-----

iQFFBAABCgAvFiEEJgpMT2bd5nVq0edxwxi6rEgTxlEFAmlVuQARHHJzYUBleGFt
cGxlLnRlc3QACgkQwxi6rEgTxlEDLQgAoa3yI0x3FBul0oR3Zs4UVrPrWm3wuUEb
4oRlIqHP5Rozr4suu+CivpIo41PeK0ATaMldNy8bo+5yHJVrn3ZP++mS2O+Wn0CG
ChZTmVuLP2nDqjzwKwnXDy0BglX2PV4+Vfsv1E0MJztNol1y6HO0MUl2I6SATsTb
YeK3XNEW4CFpDd8Qc8BiftWgEj4tzUf1Cjap1JHhkxFJ3HPSA2pzCmA4IjcJs5fQ
bJ88Pu6uvQD9WtoIIDTTNIMwBJaxKDKp16WPcSTl8Fnyebsn902/FkcLga90e/7B
cK+mmGjlX3JKrqQs3XEeEFXnooOmSW6lZloBKCvZPBm7TsEgcrgrdw==
=KYyd
-----END PGP SIGNATURE-----
";

    pub(crate) fn cert(armored: &str) -> super::SignedPublicKey {
        use super::Deserializable;
        super::SignedPublicKey::from_armor_single(armored.as_bytes())
            .expect("fixture key parses")
            .0
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    fn verify_config(dir: &Path, armored: &str) -> VerifyConfig {
        let path = dir.join("key.asc");
        std::fs::write(&path, armored).expect("write key");
        VerifyConfig {
            public_key_files: vec![path],
            ..VerifyConfig::default()
        }
    }

    #[test]
    fn key_lifetime_ends_at_creation_plus_seconds() {
        assert!(!has_expired(100, None, 1_000_000));
        assert!(!has_expired(100, Some(0), 1_000_000));
        assert!(!has_expired(100, Some(50), 149));
        assert!(has_expired(100, Some(50), 150));
        assert!(has_expired(u64::MAX - 1, Some(10), u64::MAX));
    }

    #[test]
    fn revoked_certificate_is_not_selected() {
        let cert = cert(ED25519_PUBLIC);
        let spec = KeySpec::parse("ed@example.test");
        assert!(select_at(&cert, &spec, 1_800_000_000).is_some());
        let mut revoked = cert.clone();
        let signature = revoked.details.users[0].signatures[0].clone();
        revoked.details.revocation_signatures.push(signature);
        assert!(cert_unusable(&revoked, 1_800_000_000));
        assert!(select_at(&revoked, &spec, 1_800_000_000).is_none());
        assert!(best_signing_key(&revoked, None, 1_800_000_000).is_none());
    }

    #[test]
    fn parses_key_names_in_gnupg_forms() {
        assert_eq!(
            KeySpec::parse("0x4465C704826FEF8555FB7F58F636C8798151155A!"),
            KeySpec::Fingerprint {
                bytes: hex::decode(ED25519_FINGERPRINT).expect("hex"),
                exact: true
            }
        );
        assert_eq!(
            KeySpec::parse("f636c8798151155a"),
            KeySpec::KeyId {
                id: "F636C8798151155A".into(),
                exact: false
            }
        );
        assert_eq!(
            KeySpec::parse("<Ed@Example.test>"),
            KeySpec::Mailbox("ed@example.test".into())
        );
        assert_eq!(
            KeySpec::parse("ed@example.test"),
            KeySpec::Mailbox("ed@example.test".into())
        );
        assert_eq!(
            KeySpec::parse("Test Ed <ed@example.test>"),
            KeySpec::Substring("test ed <ed@example.test>".into())
        );
    }

    #[test]
    fn selects_primary_key_by_mailbox_with_signer_user_id() {
        let cert = cert(ED25519_PUBLIC);
        let selection = select(&cert, &KeySpec::parse("ed@example.test")).expect("selected");
        assert_eq!(fingerprint_hex(&selection.fingerprint), ED25519_FINGERPRINT);
        assert_eq!(selection.signer_mailbox.as_deref(), Some("ed@example.test"));
        let by_id = select(&cert, &KeySpec::parse("8151155A")).expect("selected");
        assert_eq!(by_id.signer_mailbox, None);
        assert!(select(&cert, &KeySpec::parse("other@example.test")).is_none());
    }

    #[test]
    fn prefers_sha512_from_key_preferences() {
        assert_eq!(preferred_hash(&cert(ED25519_PUBLIC)), HashAlgorithm::Sha512);
    }

    #[test]
    fn verifies_gnupg_signatures_as_good() {
        let dir = tempfile::tempdir().expect("tempdir");
        for (public, signature, fingerprint, signer) in [
            (
                ED25519_PUBLIC,
                ED25519_SIGNATURE,
                ED25519_FINGERPRINT,
                "Test Ed <ed@example.test>",
            ),
            (
                RSA_PUBLIC,
                RSA_SIGNATURE,
                RSA_FINGERPRINT,
                "Test Rsa <rsa@example.test>",
            ),
        ] {
            let config = verify_config(dir.path(), public);
            let result = verify(PAYLOAD, signature.as_bytes(), &config).expect("verify");
            assert_eq!(result.status, SignatureStatus::Good);
            assert_eq!(result.key.as_deref(), Some(fingerprint));
            assert_eq!(result.signer.as_deref(), Some(signer));
        }
    }

    #[test]
    fn reports_changed_payload_as_bad() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = verify_config(dir.path(), ED25519_PUBLIC);
        let result = verify(b"tree 0\n", ED25519_SIGNATURE.as_bytes(), &config).expect("verify");
        assert_eq!(result.status, SignatureStatus::Bad);
    }

    #[test]
    fn reports_signature_of_absent_key_as_unknown_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = verify_config(dir.path(), RSA_PUBLIC);
        let result = verify(PAYLOAD, ED25519_SIGNATURE.as_bytes(), &config).expect("verify");
        assert_eq!(result.status, SignatureStatus::UnknownKey);
        assert_eq!(result.key.as_deref(), Some(ED25519_FINGERPRINT));
    }

    #[test]
    fn reports_malformed_armor_as_bad() {
        let result = verify(
            PAYLOAD,
            b"-----BEGIN PGP SIGNATURE-----\n\nxx\n",
            &VerifyConfig::default(),
        )
        .expect("verify");
        assert_eq!(result.status, SignatureStatus::Bad);
    }

    #[test]
    fn fails_with_no_key_when_nothing_is_configured() {
        let config = SigningConfig::new(SignatureFormat::Openpgp, CREATED);
        let error = sign(PAYLOAD, &config).expect_err("no key");
        assert_eq!(error.data().problem.as_deref(), Some(NO_KEY));
    }

    #[test]
    fn fails_with_no_key_for_unreadable_key_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = SigningConfig::new(SignatureFormat::Openpgp, CREATED);
        config.openpgp_sources = vec![OpenPgpSource::KeyFile];
        config.key_file = Some(dir.path().join("missing.asc"));
        let error = sign(PAYLOAD, &config).expect_err("no key");
        assert_eq!(error.data().problem.as_deref(), Some(NO_KEY));
        std::fs::write(dir.path().join("public.asc"), ED25519_PUBLIC).expect("write");
        config.key_file = Some(dir.path().join("public.asc"));
        let error = sign(PAYLOAD, &config).expect_err("public key only");
        assert_eq!(error.data().problem.as_deref(), Some(NO_KEY));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_passphrase_file_readable_by_others() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("passphrase");
        std::fs::write(&path, "secret\n").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        let error = read_passphrase(&path).expect_err("too open");
        assert_eq!(error.data().problem.as_deref(), Some("key_locked"));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
        assert_eq!(read_passphrase(&path).expect("read"), "secret");
    }

    #[cfg(unix)]
    #[test]
    fn reads_only_the_first_line_of_the_passphrase_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("passphrase");
        std::fs::write(&path, "sécret\r\nsecond line\n").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
        assert_eq!(read_passphrase(&path).expect("read"), "sécret");
        std::fs::write(&path, "").expect("write");
        assert_eq!(read_passphrase(&path).expect("read"), "");
    }
}
