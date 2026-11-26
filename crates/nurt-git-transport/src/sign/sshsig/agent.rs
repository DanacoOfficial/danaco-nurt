// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Signing through an SSH agent.
//!
//! The agent listens on a Unix domain socket or, on Windows, on a named
//! pipe. Every message is a 32-bit big-endian length followed by the
//! message type and its body. A sign request carries the public key blob,
//! the data and flags; RSA keys request `rsa-sha2-512`, the algorithm
//! `ssh-keygen -Y sign` uses.

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use nurt_contracts::error::{Error, Result};
use ssh_key::encoding::{Decode, Encode};
use ssh_key::{Algorithm, HashAlg, PublicKey, Signature};

use crate::sign::error::{UNSUPPORTED_FORMAT, agent_unavailable, no_key, sign_error, with_path};

const SSH_AGENT_FAILURE: u8 = 5;
const SSH_AGENTC_SIGN_REQUEST: u8 = 13;
const SSH_AGENT_SIGN_RESPONSE: u8 = 14;
const SSH_AGENT_RSA_SHA2_512: u32 = 4;
/// Upper bound of an answer; OpenSSH limits agent messages to 256 KiB.
const MAX_MESSAGE_LEN: usize = 256 * 1024;
const LENGTH_LEN: usize = 4;
/// Time allowed for the answer; an agent may ask for confirmation first.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(300);

/// Asks the agent on `socket` to sign `data` with the private key of
/// `public`.
pub(super) fn sign(socket: &Path, public: &PublicKey, data: &[u8]) -> Result<Signature> {
    let blob = public.key_data().encode_vec().map_err(|error| {
        sign_error(
            UNSUPPORTED_FORMAT,
            format!("the SSH public key cannot be encoded: {error}"),
        )
    })?;
    let request = sign_request(&blob, data, sign_flags(&public.algorithm()))?;
    let answer = connect(socket)
        .and_then(|mut stream| exchange(&mut stream, &request))
        .map_err(|source| unavailable(socket, &source.to_string()))?;
    match answer.split_first() {
        Some((&SSH_AGENT_SIGN_RESPONSE, body)) => decode_signature(body).ok_or_else(|| {
            unavailable(
                socket,
                "the agent returned a signature that cannot be decoded",
            )
        }),
        Some((&SSH_AGENT_FAILURE, _)) => Err(with_path(
            no_key(format!(
                "the SSH agent does not hold the key {} or refused to use it; add it with `ssh-add`",
                public.fingerprint(HashAlg::Sha256)
            )),
            "socket",
            socket,
        )),
        _ => Err(unavailable(socket, "the agent sent an unexpected answer")),
    }
}

fn unavailable(socket: &Path, detail: &str) -> Error {
    with_path(
        agent_unavailable(format!(
            "no SSH agent answers on `{}`: {detail}; start ssh-agent and add the key with `ssh-add`, or set `user.signingKey` to a private key file",
            socket.display()
        )),
        "socket",
        socket,
    )
}

/// Flags of a sign request for a key of `algorithm`.
fn sign_flags(algorithm: &Algorithm) -> u32 {
    if matches!(algorithm, Algorithm::Rsa { .. }) {
        SSH_AGENT_RSA_SHA2_512
    } else {
        0
    }
}

fn sign_request(blob: &[u8], data: &[u8], flags: u32) -> Result<Vec<u8>> {
    let mut body = vec![SSH_AGENTC_SIGN_REQUEST];
    put_string(&mut body, blob)?;
    put_string(&mut body, data)?;
    body.extend_from_slice(&flags.to_be_bytes());
    Ok(body)
}

fn put_string(out: &mut Vec<u8>, value: &[u8]) -> Result<()> {
    let length = u32::try_from(value.len()).map_err(|_| {
        sign_error(
            UNSUPPORTED_FORMAT,
            "the data to sign exceeds the size an SSH agent accepts",
        )
    })?;
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(value);
    Ok(())
}

/// Splits an SSH string (32-bit length, then bytes) off `bytes`.
fn get_string(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let length: [u8; LENGTH_LEN] = bytes.get(..LENGTH_LEN)?.try_into().ok()?;
    let length = usize::try_from(u32::from_be_bytes(length)).ok()?;
    let end = LENGTH_LEN.checked_add(length)?;
    Some((bytes.get(LENGTH_LEN..end)?, &bytes[end..]))
}

fn decode_signature(body: &[u8]) -> Option<Signature> {
    let (mut blob, _) = get_string(body)?;
    Signature::decode(&mut blob).ok()
}

/// Sends one message and reads one answer.
fn exchange(stream: &mut (impl Read + Write), body: &[u8]) -> std::io::Result<Vec<u8>> {
    let length = u32::try_from(body.len()).map_err(std::io::Error::other)?;
    stream.write_all(&length.to_be_bytes())?;
    stream.write_all(body)?;
    stream.flush()?;
    let mut header = [0_u8; LENGTH_LEN];
    stream.read_exact(&mut header)?;
    let length = usize::try_from(u32::from_be_bytes(header)).map_err(std::io::Error::other)?;
    if length == 0 || length > MAX_MESSAGE_LEN {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("the answer length {length} is outside the agent protocol limits"),
        ));
    }
    let mut answer = vec![0; length];
    stream.read_exact(&mut answer)?;
    Ok(answer)
}

#[cfg(unix)]
fn connect(path: &Path) -> std::io::Result<std::os::unix::net::UnixStream> {
    let stream = std::os::unix::net::UnixStream::connect(path)?;
    stream.set_read_timeout(Some(ANSWER_TIMEOUT))?;
    Ok(stream)
}

/// Opens the named pipe of the agent; pipes have no read timeout.
#[cfg(windows)]
fn connect(path: &Path) -> std::io::Result<std::fs::File> {
    let _ = ANSWER_TIMEOUT;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
}

#[cfg(test)]
#[cfg(unix)]
pub(crate) mod mock {
    //! An SSH agent stand-in on a Unix socket that answers one request with
    //! a fixed message.

    use std::io::{Read, Write};
    use std::os::unix::net::UnixListener;
    use std::path::Path;
    use std::thread::JoinHandle;

    /// Serves one request on `socket`; the thread returns the request body.
    pub(crate) fn serve(socket: &Path, answer: Vec<u8>) -> JoinHandle<Vec<u8>> {
        let listener = UnixListener::bind(socket).expect("bind mock agent");
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut length = [0_u8; 4];
            stream.read_exact(&mut length).expect("length");
            let mut request = vec![0; u32::from_be_bytes(length) as usize];
            stream.read_exact(&mut request).expect("request");
            stream
                .write_all(&u32::try_from(answer.len()).expect("length").to_be_bytes())
                .expect("answer length");
            stream.write_all(&answer).expect("answer");
            request
        })
    }

    /// A sign response carrying the encoded `signature`.
    pub(crate) fn sign_response(signature: &[u8]) -> Vec<u8> {
        let mut answer = vec![super::SSH_AGENT_SIGN_RESPONSE];
        answer.extend_from_slice(
            &u32::try_from(signature.len())
                .expect("length")
                .to_be_bytes(),
        );
        answer.extend_from_slice(signature);
        answer
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::super::NAMESPACE;
    use super::super::fixtures::{PAYLOAD_SIGNATURE, PUBLIC, private_key};
    use super::*;
    use crate::sign::openpgp::fixtures::PAYLOAD;
    use crate::sign::{KeySource, SignatureFormat, SigningConfig};

    const CREATED: i64 = 1_767_225_600;

    fn fixture_signature() -> Vec<u8> {
        private_key()
            .sign(NAMESPACE, HashAlg::Sha512, PAYLOAD)
            .expect("sign")
            .signature()
            .encode_vec()
            .expect("encode")
    }

    #[test]
    fn signs_public_key_literal_through_agent_like_ssh_keygen() {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = dir.path().join("agent.sock");
        let agent = mock::serve(&socket, mock::sign_response(&fixture_signature()));
        let mut config = SigningConfig::new(SignatureFormat::Ssh, CREATED);
        config.signing_key = Some(format!("key::{PUBLIC}"));
        config.ssh_auth_sock = Some(socket);
        let signature = super::super::sign(PAYLOAD, &config).expect("signed");
        assert_eq!(
            String::from_utf8(signature.armored).expect("utf-8"),
            PAYLOAD_SIGNATURE
        );
        assert_eq!(signature.source, KeySource::Agent);
        let request = agent.join().expect("agent");
        assert_eq!(request[0], SSH_AGENTC_SIGN_REQUEST);
        let public = private_key().public_key().clone();
        let (blob, rest) = get_string(&request[1..]).expect("blob");
        assert_eq!(
            blob,
            public.key_data().encode_vec().expect("encode").as_slice()
        );
        let (data, flags) = get_string(rest).expect("data");
        let expected =
            ssh_key::SshSig::signed_data(NAMESPACE, HashAlg::Sha512, PAYLOAD).expect("data");
        assert_eq!(data, expected.as_slice());
        assert_eq!(flags, [0, 0, 0, 0]);
    }

    #[test]
    fn rejects_agent_signature_that_does_not_verify() {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = dir.path().join("agent.sock");
        let wrong = private_key()
            .sign(NAMESPACE, HashAlg::Sha512, b"other payload")
            .expect("sign")
            .signature()
            .encode_vec()
            .expect("encode");
        let agent = mock::serve(&socket, mock::sign_response(&wrong));
        let mut config = SigningConfig::new(SignatureFormat::Ssh, CREATED);
        config.signing_key = Some(format!("key::{PUBLIC}"));
        config.ssh_auth_sock = Some(socket);
        let error = super::super::sign(PAYLOAD, &config).expect_err("wrong signature");
        assert_eq!(error.data().problem.as_deref(), Some("agent_unavailable"));
        agent.join().expect("agent");
    }

    #[test]
    fn reports_agent_failure_as_missing_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = dir.path().join("agent.sock");
        let agent = mock::serve(&socket, vec![SSH_AGENT_FAILURE]);
        let error = sign(&socket, private_key().public_key(), b"data").expect_err("refused");
        assert_eq!(error.data().problem.as_deref(), Some("no_key"));
        agent.join().expect("agent");
    }

    #[test]
    fn reports_unexpected_and_undecodable_answers_as_unavailable() {
        for answer in [vec![99_u8], mock::sign_response(b"\x00\x00\x00\x05bogus")] {
            let dir = tempfile::tempdir().expect("tempdir");
            let socket = dir.path().join("agent.sock");
            let agent = mock::serve(&socket, answer);
            let error = sign(&socket, private_key().public_key(), b"data").expect_err("unexpected");
            assert_eq!(error.data().problem.as_deref(), Some("agent_unavailable"));
            agent.join().expect("agent");
        }
    }

    #[test]
    fn reports_missing_socket_as_unavailable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = sign(
            &dir.path().join("absent.sock"),
            private_key().public_key(),
            b"data",
        )
        .expect_err("no agent");
        assert_eq!(error.data().problem.as_deref(), Some("agent_unavailable"));
        assert!(error.data().details.contains_key("socket"));
    }

    /// A stream that answers with fixed bytes and records what is written.
    struct Duplex {
        input: std::io::Cursor<Vec<u8>>,
        output: Vec<u8>,
    }

    impl Read for Duplex {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(buf)
        }
    }

    impl Write for Duplex {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.output.write(buf)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn rejects_answer_longer_than_protocol_limit() {
        let oversized = u32::try_from(MAX_MESSAGE_LEN + 1)
            .expect("length")
            .to_be_bytes()
            .to_vec();
        let mut stream = Duplex {
            input: std::io::Cursor::new(oversized),
            output: Vec::new(),
        };
        let error = exchange(&mut stream, b"x").expect_err("oversized");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(stream.output, b"\x00\x00\x00\x01x");
        let mut silent = Duplex {
            input: std::io::Cursor::new(Vec::new()),
            output: Vec::new(),
        };
        let error = exchange(&mut silent, b"x").expect_err("no answer");
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn requests_sha2_512_for_rsa_keys_only() {
        assert_eq!(
            sign_flags(&Algorithm::Rsa { hash: None }),
            SSH_AGENT_RSA_SHA2_512
        );
        assert_eq!(sign_flags(&Algorithm::Ed25519), 0);
    }

    #[test]
    fn splits_ssh_strings_and_rejects_truncated_ones() {
        assert_eq!(
            get_string(b"\x00\x00\x00\x02abc"),
            Some((&b"ab"[..], &b"c"[..]))
        );
        assert!(get_string(b"\x00\x00\x00\x09abc").is_none());
        assert!(get_string(b"\x00\x00").is_none());
    }
}
