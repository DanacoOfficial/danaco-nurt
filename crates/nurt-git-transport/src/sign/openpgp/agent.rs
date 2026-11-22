// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Signing through a running `gpg-agent` over the Assuan protocol.
//!
//! The agent is reached through the socket `S.gpg-agent`: in
//! `/run/user/<uid>/gnupg` for the default home directory, in
//! `/run/user/<uid>/gnupg/d.<hash>` for any other one (`<hash>` is the
//! z-base-32 form of the first 15 bytes of the SHA-1 of the home directory
//! path), or in the home directory itself. A regular file in place of the
//! socket either redirects (`%Assuan%` followed by `socket=<path>`) or, on
//! Windows, holds the TCP port and a 16-byte nonce of the emulated socket.
//!
//! The session is `RESET`, `SIGKEY <keygrip>`, `SETHASH <algorithm>
//! <digest>`, `PKSIGN`; the agent answers with the S-expression
//! `(sig-val(<algorithm>(r …)(s …)))`. Inquiries are answered with `END`.
//! The keygrip is the SHA-1 of the public key parameters as libgcrypt
//! defines it: the RSA modulus in two's complement form, or for Ed25519 the
//! canonical S-expression of the curve parameters `p a b g n` and the point
//! `q`.

use std::cell::RefCell;
use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use nurt_contracts::error::{Error, Result};
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::public_key::PublicKeyAlgorithm;
use pgp::ser::Serialize;
use pgp::types::{
    Fingerprint, KeyDetails, KeyId, KeyVersion, Mpi, Password, PublicParams, SignatureBytes,
    SigningKey, Timestamp,
};
use sha1::{Digest, Sha1};

use super::{armor, signature_packet, Selection};
use crate::sign::error::{agent_unavailable, key_locked, no_key, sign_error, UNSUPPORTED_FORMAT};
use crate::sign::SigningConfig;

const SOCKET_NAME: &str = "S.gpg-agent";
const RUNTIME_ROOTS: [&str; 2] = ["/run/user", "/var/run/user"];
const SOCKET_DIR_HASH_BYTES: usize = 15;
const ZBASE32_ALPHABET: &[u8; 32] = b"ybndrfg8ejkmcpqxot1uwisza345h769";
const REDIRECT_MAGIC: &[u8] = b"%Assuan%";
const NONCE_LEN: usize = 16;
const LOOPBACK: &str = "127.0.0.1";
/// Time allowed for one answer of the agent; a passphrase entry through
/// pinentry happens within it.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(300);
const READ_CHUNK: usize = 4096;

/// libgpg-error codes (lower 16 bits of the error number).
const GPG_ERR_BAD_PASSPHRASE: u32 = 11;
const GPG_ERR_NO_SECKEY: u32 = 17;
const GPG_ERR_TIMEOUT: u32 = 62;
const GPG_ERR_NO_PIN_ENTRY: u32 = 85;
const GPG_ERR_BAD_PIN: u32 = 87;
const GPG_ERR_CANCELED: u32 = 99;
const GPG_ERR_FULLY_CANCELED: u32 = 198;
const GPG_ERR_CODE_MASK: u32 = 0xFFFF;

/// Ed25519 domain parameters as libgcrypt hashes them into keygrips; `a`
/// and `b` are reduced modulo `p`, `g` is the uncompressed base point.
const ED25519_P: &str = "7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFED";
const ED25519_A: &str = "7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEC";
const ED25519_B: &str = "52036CEE2B6FFE738CC740797779E89800700A4D4141D8AB75EB4DCA135978A3";
const ED25519_G: &str = "04216936D3CD6E53FEC0A4E231FDD6DC5C692CC7609525A7B2C9562D608F25D51A6666666666666666666666666666666666666666666666666666666666666658";
const ED25519_N: &str = "1000000000000000000000000000000014DEF9DEA2F79CD65812631A5CF5D3ED";
const ED25519_POINT_LEN: usize = 32;
const EDDSA_NATIVE_PREFIX: u8 = 0x40;

/// Signs `payload` with `key` held by the agent.
pub(super) fn sign<K: KeyDetails>(
    key: &K,
    config: &SigningConfig,
    hash: HashAlgorithm,
    selection: &Selection,
    payload: &[u8],
) -> Result<Vec<u8>> {
    let Some(grip) = keygrip(key.public_params()) else {
        return Err(sign_error(
            UNSUPPORTED_FORMAT,
            format!(
                "keys of algorithm {:?} cannot sign through gpg-agent; set `git.signing.key_file` to the secret key",
                key.algorithm()
            ),
        ));
    };
    let session = Session::connect(config)?;
    let signer = AgentKey {
        key,
        grip,
        hash,
        session: RefCell::new(session),
        failure: RefCell::new(None),
    };
    match signature_packet(&signer, &Password::empty(), hash, config.created, selection, payload) {
        Ok(packet) => armor(packet),
        Err(error) => Err(signer.failure.into_inner().unwrap_or(error)),
    }
}

/// Keygrip of a public key in upper-case hexadecimal, for RSA and Ed25519.
pub(crate) fn keygrip(params: &PublicParams) -> Option<String> {
    let serialized = params.to_bytes().ok()?;
    let digest = match params {
        PublicParams::RSA(_) => {
            let (modulus, _) = read_mpi(&serialized)?;
            let mut hasher = Sha1::new();
            if modulus.first().is_some_and(|byte| byte & 0x80 != 0) {
                hasher.update([0]);
            }
            hasher.update(modulus);
            hasher.finalize().to_vec()
        }
        PublicParams::EdDSALegacy(_) => {
            let oid_len = usize::from(*serialized.first()?);
            let (point, _) = read_mpi(serialized.get(1 + oid_len..)?)?;
            let q = match point {
                [EDDSA_NATIVE_PREFIX, rest @ ..] => rest,
                other => other,
            };
            ed25519_grip(q)?
        }
        PublicParams::Ed25519(_) => ed25519_grip(serialized.get(..ED25519_POINT_LEN)?)?,
        _ => return None,
    };
    Some(hex::encode_upper(digest))
}

fn ed25519_grip(q: &[u8]) -> Option<Vec<u8>> {
    if q.len() != ED25519_POINT_LEN {
        return None;
    }
    let mut hasher = Sha1::new();
    for (name, value) in [
        ("p", ED25519_P),
        ("a", ED25519_A),
        ("b", ED25519_B),
        ("g", ED25519_G),
        ("n", ED25519_N),
    ] {
        let bytes = hex::decode(value).ok()?;
        hasher.update(format!("(1:{name}{}:", bytes.len()).as_bytes());
        hasher.update(&bytes);
        hasher.update(b")");
    }
    hasher.update(format!("(1:q{}:", q.len()).as_bytes());
    hasher.update(q);
    hasher.update(b")");
    Some(hasher.finalize().to_vec())
}

/// Splits an OpenPGP multiprecision integer (bit count, then bytes) off
/// `bytes`.
fn read_mpi(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let bits = usize::from(u16::from_be_bytes(bytes.get(..2)?.try_into().ok()?));
    let len = bits.div_ceil(8);
    let value = bytes.get(2..2 + len)?;
    Some((value, &bytes[2 + len..]))
}

/// Encodes `bytes` in z-base-32, most significant bits first.
pub(crate) fn zbase32(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for byte in bytes {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(ZBASE32_ALPHABET[((buffer >> bits) & 0x1F) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(ZBASE32_ALPHABET[((buffer << (5 - bits)) & 0x1F) as usize]));
    }
    out
}

/// Directory name under `/run/user/<uid>/gnupg` used by GnuPG for a home
/// directory other than the default one.
pub(crate) fn socket_subdir(home: &Path) -> String {
    let digest = Sha1::digest(home.as_os_str().as_encoded_bytes());
    format!("d.{}", zbase32(&digest[..SOCKET_DIR_HASH_BYTES]))
}

/// Socket paths of the agent, most specific first.
pub(crate) fn socket_candidates(config: &SigningConfig) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = &config.agent_socket_dir {
        candidates.push(dir.join(SOCKET_NAME));
    }
    let Some(home) = &config.gnupg_home else {
        return candidates;
    };
    #[cfg(unix)]
    if let Ok(metadata) = std::fs::metadata(home) {
        use std::os::unix::fs::MetadataExt;
        let is_default = config
            .home
            .as_ref()
            .is_some_and(|user_home| user_home.join(".gnupg") == *home);
        for root in RUNTIME_ROOTS {
            let mut dir = Path::new(root).join(metadata.uid().to_string()).join("gnupg");
            if !is_default {
                dir = dir.join(socket_subdir(home));
            }
            candidates.push(dir.join(SOCKET_NAME));
        }
    }
    #[cfg(not(unix))]
    let _ = RUNTIME_ROOTS;
    candidates.push(home.join(SOCKET_NAME));
    candidates
}

trait Stream: Read + Write {}

impl<T: Read + Write> Stream for T {}

fn open_stream(path: &Path, redirected: bool) -> std::io::Result<Box<dyn Stream>> {
    let metadata = std::fs::metadata(path)?;
    if metadata.is_file() {
        let content = std::fs::read(path)?;
        if let Some(rest) = content.strip_prefix(REDIRECT_MAGIC) {
            let target = String::from_utf8_lossy(rest)
                .lines()
                .find_map(|line| line.trim().strip_prefix("socket=").map(PathBuf::from));
            return match target {
                Some(target) if !redirected => open_stream(&target, true),
                _ => Err(std::io::Error::other("invalid socket redirection")),
            };
        }
        return open_emulated(&content);
    }
    open_unix(path)
}

/// Connects to an emulated socket: the file holds the port in decimal, a
/// line feed and the nonce to send first.
fn open_emulated(content: &[u8]) -> std::io::Result<Box<dyn Stream>> {
    let newline = content
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or_else(|| std::io::Error::other("invalid emulated socket file"))?;
    let port: u16 = String::from_utf8_lossy(&content[..newline])
        .trim()
        .parse()
        .map_err(std::io::Error::other)?;
    let nonce = content
        .get(newline + 1..newline + 1 + NONCE_LEN)
        .ok_or_else(|| std::io::Error::other("invalid emulated socket nonce"))?;
    let mut stream = std::net::TcpStream::connect((LOOPBACK, port))?;
    stream.set_read_timeout(Some(ANSWER_TIMEOUT))?;
    stream.write_all(nonce)?;
    Ok(Box::new(stream))
}

#[cfg(unix)]
fn open_unix(path: &Path) -> std::io::Result<Box<dyn Stream>> {
    let stream = std::os::unix::net::UnixStream::connect(path)?;
    stream.set_read_timeout(Some(ANSWER_TIMEOUT))?;
    Ok(Box::new(stream))
}

#[cfg(not(unix))]
fn open_unix(_path: &Path) -> std::io::Result<Box<dyn Stream>> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

/// Failure of one Assuan command.
#[derive(Debug)]
enum AgentError {
    Io(std::io::Error),
    Agent { code: u32, text: String },
    Protocol(String),
}

impl AgentError {
    fn into_error(self) -> Error {
        match self {
            Self::Io(source) => agent_unavailable(format!(
                "the connection to gpg-agent failed: {source}; start gpg-agent outside Nurt or set `git.signing.key_file`"
            )),
            Self::Protocol(line) => agent_unavailable(format!(
                "gpg-agent sent an unexpected answer `{line}`; restart gpg-agent"
            )),
            Self::Agent { code, text } => match code & GPG_ERR_CODE_MASK {
                GPG_ERR_NO_SECKEY => no_key(format!(
                    "gpg-agent holds no secret key for the signing key ({text}); import the secret key into GnuPG or set `git.signing.key_file`"
                )),
                GPG_ERR_BAD_PASSPHRASE | GPG_ERR_TIMEOUT | GPG_ERR_NO_PIN_ENTRY | GPG_ERR_BAD_PIN
                | GPG_ERR_CANCELED | GPG_ERR_FULLY_CANCELED => key_locked(format!(
                    "gpg-agent could not unlock the signing key ({text}); unlock it once with `gpg --sign` so that the agent caches the passphrase"
                )),
                _ => agent_unavailable(format!("gpg-agent refused to sign: {text}")),
            },
        }
    }
}

/// One Assuan session.
struct Session {
    stream: Box<dyn Stream>,
    buffer: Vec<u8>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session").finish_non_exhaustive()
    }
}

impl Session {
    fn connect(config: &SigningConfig) -> Result<Self> {
        let candidates = socket_candidates(config);
        for path in &candidates {
            if let Ok(stream) = open_stream(path, false) {
                let mut session = Self {
                    stream,
                    buffer: Vec::new(),
                };
                session.expect_ok().map_err(AgentError::into_error)?;
                return Ok(session);
            }
        }
        let shown = candidates
            .first()
            .map_or_else(|| SOCKET_NAME.to_owned(), |path| path.display().to_string());
        Err(agent_unavailable(format!(
            "no gpg-agent answers on `{shown}`; start gpg-agent outside Nurt or set `git.signing.key_file`"
        ))
        .with_detail("socket", serde_json::Value::String(shown)))
    }

    fn read_line(&mut self) -> std::result::Result<Vec<u8>, AgentError> {
        loop {
            if let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
                let mut line: Vec<u8> = self.buffer.drain(..=end).collect();
                line.pop();
                return Ok(line);
            }
            let mut chunk = [0_u8; READ_CHUNK];
            let read = self.stream.read(&mut chunk).map_err(AgentError::Io)?;
            if read == 0 {
                return Err(AgentError::Io(std::io::ErrorKind::UnexpectedEof.into()));
            }
            self.buffer.extend_from_slice(&chunk[..read]);
        }
    }

    /// Reads answers up to the final `OK`, collecting data lines.
    fn expect_ok(&mut self) -> std::result::Result<Vec<u8>, AgentError> {
        let mut data = Vec::new();
        loop {
            let line = self.read_line()?;
            if line == b"OK" || line.starts_with(b"OK ") {
                return Ok(data);
            }
            if let Some(rest) = line.strip_prefix(b"ERR ") {
                let text = String::from_utf8_lossy(rest).into_owned();
                let (code, text) = text
                    .split_once(' ')
                    .map_or((text.as_str(), ""), |(code, text)| (code, text));
                return Err(AgentError::Agent {
                    code: code.parse().unwrap_or(0),
                    text: text.to_owned(),
                });
            }
            if let Some(rest) = line.strip_prefix(b"D ") {
                data.extend(unescape(rest));
            } else if line.starts_with(b"INQUIRE ") {
                self.stream.write_all(b"END\n").map_err(AgentError::Io)?;
            } else if !(line.starts_with(b"S ") || line.starts_with(b"#") || line == b"S") {
                return Err(AgentError::Protocol(String::from_utf8_lossy(&line).into_owned()));
            }
        }
    }

    fn command(&mut self, line: &str) -> std::result::Result<Vec<u8>, AgentError> {
        self.stream
            .write_all(format!("{line}\n").as_bytes())
            .map_err(AgentError::Io)?;
        self.expect_ok()
    }
}

/// Decodes the `%XX` escapes of an Assuan data line.
fn unescape(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut index = 0;
    while index < data.len() {
        if data[index] == b'%' {
            if let Some(byte) = data
                .get(index + 1..index + 3)
                .and_then(|hex_digits| std::str::from_utf8(hex_digits).ok())
                .and_then(|text| u8::from_str_radix(text, 16).ok())
            {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(data[index]);
        index += 1;
    }
    out
}

/// A canonical S-expression.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Sexp {
    Atom(Vec<u8>),
    List(Vec<Sexp>),
}

impl Sexp {
    fn parse(bytes: &[u8]) -> Option<Self> {
        let (value, rest) = Self::parse_at(bytes, 0)?;
        rest.is_empty().then_some(value)
    }

    fn parse_at(bytes: &[u8], depth: usize) -> Option<(Self, &[u8])> {
        const MAX_DEPTH: usize = 16;
        if depth > MAX_DEPTH {
            return None;
        }
        match bytes.first()? {
            b'(' => {
                let mut items = Vec::new();
                let mut rest = &bytes[1..];
                while *rest.first()? != b')' {
                    let (item, next) = Self::parse_at(rest, depth + 1)?;
                    items.push(item);
                    rest = next;
                }
                Some((Self::List(items), &rest[1..]))
            }
            b'0'..=b'9' => {
                let colon = bytes.iter().position(|byte| *byte == b':')?;
                let len: usize = std::str::from_utf8(&bytes[..colon]).ok()?.parse().ok()?;
                let value = bytes.get(colon + 1..colon + 1 + len)?;
                Some((Self::Atom(value.to_vec()), &bytes[colon + 1 + len..]))
            }
            _ => None,
        }
    }

    fn name(&self) -> Option<&[u8]> {
        match self {
            Self::List(items) => match items.first()? {
                Self::Atom(name) => Some(name),
                Self::List(_) => None,
            },
            Self::Atom(_) => None,
        }
    }

    fn child(&self, name: &[u8]) -> Option<&Self> {
        match self {
            Self::List(items) => items.iter().skip(1).find(|item| item.name() == Some(name)),
            Self::Atom(_) => None,
        }
    }

    fn value(&self) -> Option<&[u8]> {
        match self {
            Self::List(items) => match items.get(1)? {
                Self::Atom(value) => Some(value),
                Self::List(_) => None,
            },
            Self::Atom(_) => None,
        }
    }

    /// The algorithm list inside `(sig-val …)`.
    fn signature_values(&self) -> Option<&Self> {
        if self.name() != Some(b"sig-val") {
            return None;
        }
        match self {
            Self::List(items) => items
                .iter()
                .skip(1)
                .find(|item| matches!(item.name(), Some(name) if name != b"flags")),
            Self::Atom(_) => None,
        }
    }
}

/// Converts the agent answer into the signature values of `algorithm`.
fn signature_bytes(answer: &[u8], algorithm: PublicKeyAlgorithm) -> Option<SignatureBytes> {
    let sexp = Sexp::parse(answer)?;
    let values = sexp.signature_values()?;
    let part = |name: &[u8]| values.child(name).and_then(Sexp::value);
    match algorithm {
        PublicKeyAlgorithm::RSA | PublicKeyAlgorithm::RSASign => {
            Some(SignatureBytes::Mpis(vec![Mpi::from_slice(part(b"s")?)]))
        }
        PublicKeyAlgorithm::EdDSALegacy | PublicKeyAlgorithm::ECDSA => Some(SignatureBytes::Mpis(
            vec![Mpi::from_slice(part(b"r")?), Mpi::from_slice(part(b"s")?)],
        )),
        PublicKeyAlgorithm::Ed25519 => {
            let mut native = Vec::with_capacity(2 * ED25519_POINT_LEN);
            for name in [b"r".as_slice(), b"s"] {
                let value = part(name)?;
                let padding = ED25519_POINT_LEN.checked_sub(value.len())?;
                native.extend(std::iter::repeat_n(0, padding));
                native.extend_from_slice(value);
            }
            Some(SignatureBytes::Native(native.into()))
        }
        _ => None,
    }
}

/// A key whose secret part is held by the agent.
struct AgentKey<'a, K> {
    key: &'a K,
    grip: String,
    hash: HashAlgorithm,
    session: RefCell<Session>,
    failure: RefCell<Option<Error>>,
}

impl<K> fmt::Debug for AgentKey<'_, K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentKey").field("grip", &self.grip).finish_non_exhaustive()
    }
}

impl<K: KeyDetails> AgentKey<'_, K> {
    fn request(&self, hash: HashAlgorithm, digest: &[u8]) -> Result<SignatureBytes> {
        let mut session = self.session.borrow_mut();
        let answer = (|| {
            session.command("RESET")?;
            session.command(&format!("SIGKEY {}", self.grip))?;
            session.command(&format!("SETHASH {} {}", u8::from(hash), hex::encode_upper(digest)))?;
            session.command("PKSIGN")
        })()
        .map_err(AgentError::into_error)?;
        signature_bytes(&answer, self.key.algorithm()).ok_or_else(|| {
            agent_unavailable("gpg-agent returned a signature that cannot be decoded; restart gpg-agent")
        })
    }
}

impl<K: KeyDetails> KeyDetails for AgentKey<'_, K> {
    fn version(&self) -> KeyVersion {
        self.key.version()
    }

    fn legacy_key_id(&self) -> KeyId {
        self.key.legacy_key_id()
    }

    fn fingerprint(&self) -> Fingerprint {
        self.key.fingerprint()
    }

    fn algorithm(&self) -> PublicKeyAlgorithm {
        self.key.algorithm()
    }

    fn created_at(&self) -> Timestamp {
        self.key.created_at()
    }

    fn legacy_v3_expiration_days(&self) -> Option<u16> {
        self.key.legacy_v3_expiration_days()
    }

    fn public_params(&self) -> &PublicParams {
        self.key.public_params()
    }
}

impl<K: KeyDetails> SigningKey for AgentKey<'_, K> {
    fn sign(
        &self,
        _key_pw: &Password,
        hash: HashAlgorithm,
        data: &[u8],
    ) -> pgp::errors::Result<SignatureBytes> {
        self.request(hash, data).map_err(|error| {
            *self.failure.borrow_mut() = Some(error);
            pgp::errors::Error::InvalidInput { backtrace: None }
        })
    }

    fn hash_alg(&self) -> HashAlgorithm {
        self.hash
    }
}

#[cfg(test)]
pub(crate) mod mock {
    //! A `gpg-agent` stand-in on a Unix socket that answers `PKSIGN` with
    //! fixed signature values when the digest matches.

    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::path::Path;
    use std::thread::JoinHandle;

    pub(crate) struct MockAgent {
        pub(crate) handle: JoinHandle<Vec<String>>,
    }

    fn escape(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for byte in data {
            if matches!(byte, b'%' | b'\n' | b'\r') {
                out.extend_from_slice(format!("%{byte:02X}").as_bytes());
            } else {
                out.push(*byte);
            }
        }
        out
    }

    /// Serves one session on `socket`. `answer` maps the `SETHASH` line to
    /// the `PKSIGN` answer line.
    pub(crate) fn serve(
        socket: &Path,
        answer: impl Fn(&str) -> Vec<u8> + Send + 'static,
    ) -> MockAgent {
        let listener = UnixListener::bind(socket).expect("bind mock agent");
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut writer = stream.try_clone().expect("clone");
            let reader = BufReader::new(stream);
            let mut lines = Vec::new();
            let mut sethash = String::new();
            writer.write_all(b"# comment\nOK Pleased to meet you\n").expect("greet");
            for line in reader.lines() {
                let Ok(line) = line else { break };
                lines.push(line.clone());
                if line.starts_with("SETHASH") {
                    sethash = line.clone();
                }
                if line == "PKSIGN" {
                    writer.write_all(b"INQUIRE PINENTRY_LAUNCHED 1234\n").expect("inquire");
                    let mut reply = answer(&sethash);
                    if reply.starts_with(b"(") {
                        let mut data = b"S PROGRESS\nD ".to_vec();
                        data.extend(escape(&reply));
                        data.extend_from_slice(b"\nOK\n");
                        reply = data;
                    }
                    writer.write_all(&reply).expect("answer");
                } else if line == "END" {
                    continue;
                } else {
                    writer.write_all(b"OK\n").expect("ok");
                }
            }
            lines
        });
        MockAgent { handle }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::*;
    use super::super::{fingerprint_hex, select, KeySpec};
    use super::*;
    use crate::sign::{OpenPgpSource, SignatureFormat};
    use pgp::composed::{Deserializable, DetachedSignature};

    #[test]
    fn computes_gnupg_keygrips() {
        assert_eq!(keygrip(cert(ED25519_PUBLIC).primary_key.public_params()).as_deref(), Some(ED25519_KEYGRIP));
        assert_eq!(keygrip(cert(RSA_PUBLIC).primary_key.public_params()).as_deref(), Some(RSA_KEYGRIP));
    }

    #[test]
    fn names_socket_directories_like_gnupg() {
        assert_eq!(socket_subdir(Path::new("/home/user/gnupg-test")), "d.kjat6rfapiq3ym1xhsksf6ni");
        assert_eq!(socket_subdir(Path::new("/home/user/.gnupg")), "d.pb7zganozcuhu3ro1ykqk8gj");
        assert_eq!(zbase32(&[0xF0]), "6y");
    }

    #[test]
    fn decodes_escaped_data_and_signature_expressions() {
        assert_eq!(unescape(b"a%25b%0Ac%2"), b"a%b\nc%2");
        let answer = b"(7:sig-val(5:flags5:eddsa)(5:eddsa(1:r2:\x01\x02)(1:s1:\x03)))";
        let Some(SignatureBytes::Native(native)) = signature_bytes(answer, PublicKeyAlgorithm::Ed25519) else {
            panic!("native signature expected");
        };
        assert_eq!(native.len(), 64);
        assert_eq!(&native[30..32], &[1, 2]);
        assert!(signature_bytes(b"(7:sig-val", PublicKeyAlgorithm::RSA).is_none());
        assert!(Sexp::parse(b"(3:abc)x").is_none());
    }

    fn signature_values(armored: &str) -> Vec<Vec<u8>> {
        let (detached, _) = DetachedSignature::from_armor_single(armored.as_bytes()).expect("fixture");
        match detached.signature.signature() {
            Some(SignatureBytes::Mpis(mpis)) => mpis.iter().map(|mpi| mpi.as_ref().to_vec()).collect(),
            other => panic!("unexpected signature {other:?}"),
        }
    }

    fn sexp_answer(algorithm: &str, values: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut out = format!("(7:sig-val({}:{algorithm}", algorithm.len()).into_bytes();
        for (name, value) in values {
            out.extend_from_slice(format!("(1:{name}{}:", value.len()).as_bytes());
            out.extend_from_slice(value);
            out.push(b')');
        }
        out.extend_from_slice(b"))");
        out
    }

    fn agent_config(dir: &Path) -> SigningConfig {
        let mut config = SigningConfig::new(SignatureFormat::Openpgp, CREATED);
        config.openpgp_sources = vec![OpenPgpSource::Agent];
        config.agent_socket_dir = Some(dir.to_path_buf());
        config.gnupg_home = Some(dir.join("absent-home"));
        config
    }

    #[test]
    fn reproduces_gnupg_signatures_through_the_agent() {
        for (public, signature, algorithm, spec) in [
            (ED25519_PUBLIC, ED25519_SIGNATURE, "eddsa", "ed@example.test"),
            (RSA_PUBLIC, RSA_SIGNATURE, "rsa", "rsa@example.test"),
        ] {
            let dir = tempfile::tempdir().expect("tempdir");
            let values = signature_values(signature);
            let names = if values.len() == 2 { vec!["r", "s"] } else { vec!["s"] };
            let answer = sexp_answer(
                algorithm,
                &names.into_iter().zip(values).collect::<Vec<_>>(),
            );
            let agent = mock::serve(&dir.path().join(SOCKET_NAME), move |_| answer.clone());
            let cert = cert(public);
            let selection = select(&cert, &KeySpec::parse(spec)).expect("selection");
            let config = agent_config(dir.path());
            let armored = sign(&cert.primary_key, &config, HashAlgorithm::Sha512, &selection, PAYLOAD)
                .expect("signed");
            assert_eq!(String::from_utf8(armored).expect("utf-8"), signature);
            let lines = agent.handle.join().expect("agent");
            let grip = keygrip(cert.primary_key.public_params()).expect("grip");
            assert_eq!(lines[1], format!("SIGKEY {grip}"));
            assert!(lines[2].starts_with("SETHASH 10 "));
            assert_eq!(fingerprint_hex(&selection.fingerprint).len(), 40);
        }
    }

    #[test]
    fn maps_agent_errors_to_sign_problems() {
        for (reply, problem) in [
            (&b"ERR 67108881 No secret key <GPG Agent>\n"[..], "no_key"),
            (b"ERR 83886179 Operation cancelled <Pinentry>\n", "key_locked"),
            (b"ERR 67108949 No pinentry <GPG Agent>\n", "key_locked"),
            (b"ERR 1 General error\n", "agent_unavailable"),
            (b"BOGUS\n", "agent_unavailable"),
        ] {
            let dir = tempfile::tempdir().expect("tempdir");
            let reply = reply.to_vec();
            let agent = mock::serve(&dir.path().join(SOCKET_NAME), move |_| reply.clone());
            let cert = cert(ED25519_PUBLIC);
            let selection = select(&cert, &KeySpec::parse("ed@example.test")).expect("selection");
            let error = sign(&cert.primary_key, &agent_config(dir.path()), HashAlgorithm::Sha512, &selection, PAYLOAD)
                .expect_err("refused");
            assert_eq!(error.data().problem.as_deref(), Some(problem));
            drop(agent);
        }
    }

    #[test]
    fn reports_missing_agent_as_unavailable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cert = cert(ED25519_PUBLIC);
        let selection = select(&cert, &KeySpec::parse("ed@example.test")).expect("selection");
        let error = sign(&cert.primary_key, &agent_config(dir.path()), HashAlgorithm::Sha512, &selection, PAYLOAD)
            .expect_err("no agent");
        assert_eq!(error.data().problem.as_deref(), Some("agent_unavailable"));
        assert!(error.data().details.contains_key("socket"));
    }

    #[test]
    fn follows_socket_redirection_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let real = dir.path().join("real-socket");
        std::fs::create_dir(dir.path().join("redirect")).expect("mkdir");
        std::fs::write(
            dir.path().join("redirect").join(SOCKET_NAME),
            format!("%Assuan%\nsocket={}\n", real.display()),
        )
        .expect("write");
        let signature = signature_values(ED25519_SIGNATURE);
        let answer = sexp_answer("eddsa", &[("r", signature[0].clone()), ("s", signature[1].clone())]);
        let agent = mock::serve(&real, move |_| answer.clone());
        let cert = cert(ED25519_PUBLIC);
        let selection = select(&cert, &KeySpec::parse("ed@example.test")).expect("selection");
        let config = agent_config(&dir.path().join("redirect"));
        let armored = sign(&cert.primary_key, &config, HashAlgorithm::Sha512, &selection, PAYLOAD).expect("signed");
        assert_eq!(armored, ED25519_SIGNATURE.as_bytes());
        agent.handle.join().expect("agent");
    }

    #[test]
    fn lists_runtime_and_home_sockets() {
        let home = tempfile::tempdir().expect("tempdir");
        let mut config = SigningConfig::new(SignatureFormat::Openpgp, CREATED);
        config.gnupg_home = Some(home.path().to_path_buf());
        let candidates = socket_candidates(&config);
        assert_eq!(candidates.last(), Some(&home.path().join(SOCKET_NAME)));
        assert!(candidates[0].starts_with("/run/user"));
        assert!(candidates[0].to_string_lossy().contains("/gnupg/d."));
    }
}
