// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! OpenPGP signatures made and checked by GnuPG.
//!
//! The program named by `gpg.openpgp.program` or `gpg.program` (default
//! `gpg`) runs through the process host of the console, with the arguments
//! `git` uses: `--detach-sign --armor --local-user <key>` to sign and
//! `--verify <signature> -` to check, the payload on standard input and the
//! machine readable status on `--status-fd`. Secret keys of a running
//! `gpg-agent` are used in place; a secret key file is imported into a
//! temporary GnuPG home directory that is removed together with its agent.

use std::path::{Path, PathBuf};

use nurt_contracts::error::Result;
use nurt_contracts::services::{EnvVar, ProcessHost, ProgramInput, ProgramOutcome, ProgramSpec};
use nurt_contracts::stage::CancelToken;

use super::error::{
    NO_KEY, PROGRAM_FAILED, PROGRAM_UNAVAILABLE, io_error, is_problem, key_locked, no_key,
    program_failed, program_unavailable, with_path,
};
use super::{
    CommitSignature, KeySource, OpenPgpSource, Signature, SignatureFormat, SignatureStatus,
    SigningConfig, VerifyConfig,
};

/// Program used when the configuration names none.
pub(crate) const DEFAULT_PROGRAM: &str = "gpg";

/// Time limit of one program run, in milliseconds. A key protected by a
/// passphrase that nobody can enter must not block the caller.
const TIMEOUT_MS: u64 = 120_000;

/// Prefix of the lines of the machine readable status.
const STATUS_PREFIX: &str = "[GNUPG:] ";

/// Mask of the error code in the number GnuPG reports with `FAILURE`; the
/// bits above it name the part of GnuPG that failed.
const GPG_ERROR_CODE_MASK: u32 = 0xFFFF;

/// Error code of GnuPG for a passphrase that does not unlock the key.
const GPG_ERR_BAD_PASSPHRASE: u32 = 11;

const ARMOR_HEADER: &[u8] = b"-----BEGIN PGP SIGNATURE-----";

/// Process host and cancellation of the operation that runs GnuPG.
#[derive(Clone, Copy)]
pub(crate) struct Runner<'a> {
    pub(crate) host: &'a dyn ProcessHost,
    pub(crate) cancel: &'a CancelToken,
}

/// Status line of GnuPG: the keyword and the text after it.
type Status = (String, String);

pub(crate) fn sign(
    runner: Option<Runner<'_>>,
    payload: &[u8],
    config: &SigningConfig,
) -> Result<Signature> {
    let Some(runner) = runner else {
        return Err(program_unavailable(
            "OpenPGP signatures are made by GnuPG, which needs the program service of the console",
        ));
    };
    let mut first_error = None;
    for source in &config.openpgp_sources {
        let attempt = match source {
            OpenPgpSource::Agent => sign_with_agent(runner, payload, config).map(Some),
            OpenPgpSource::KeyFile => sign_with_key_file(runner, payload, config),
        };
        match attempt {
            Ok(Some(signature)) => return Ok(signature),
            Ok(None) => {}
            Err(error)
                if is_problem(&error, NO_KEY)
                    || is_problem(&error, PROGRAM_UNAVAILABLE)
                    || is_problem(&error, PROGRAM_FAILED) =>
            {
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
    runner: Option<Runner<'_>>,
    payload: &[u8],
    signature: &[u8],
    config: &VerifyConfig,
) -> Result<CommitSignature> {
    let mut result = CommitSignature {
        status: SignatureStatus::Unsupported,
        format: Some(SignatureFormat::Openpgp),
        key: None,
        signer: None,
    };
    let Some(runner) = runner else {
        return Ok(result);
    };
    let scratch = scratch_dir()?;
    let signature_file = scratch.path().join("signature.asc");
    std::fs::write(&signature_file, signature)
        .map_err(|source| io_error(&signature_file, &source))?;
    let program = config.gpg_program.as_deref().unwrap_or(DEFAULT_PROGRAM);
    let mut args = vec![
        "--batch".to_owned(),
        "--no-tty".to_owned(),
        "--disable-dirmngr".to_owned(),
        "--no-auto-key-retrieve".to_owned(),
    ];
    let imported = !config.public_key_files.is_empty();
    let home = scratch.path().join("home");
    if imported {
        let import = import_keys(runner, program, &home, &config.public_key_files);
        if let Err(error) = import {
            stop_agent(runner, &home);
            return if is_problem(&error, PROGRAM_UNAVAILABLE) {
                Ok(result)
            } else {
                Err(error)
            };
        }
        args.extend(["--homedir".to_owned(), path_arg(&home)]);
        if let Some(keyring) = config.gnupg_home.as_deref().and_then(keyring_of) {
            args.extend(["--keyring".to_owned(), path_arg(&keyring)]);
        }
    } else if let Some(home) = &config.gnupg_home {
        args.extend(["--homedir".to_owned(), path_arg(home)]);
    }
    args.extend([
        "--status-fd=1".to_owned(),
        "--verify".to_owned(),
        path_arg(&signature_file),
        "-".to_owned(),
    ]);
    let run = execute(runner, program, args, scratch.path(), payload.to_vec());
    if imported {
        stop_agent(runner, &home);
    }
    match run {
        Ok(outcome) => {
            apply_verification(&mut result, &statuses(&outcome.stdout));
            Ok(result)
        }
        Err(error) if is_problem(&error, PROGRAM_UNAVAILABLE) => Ok(result),
        Err(error) => Err(error),
    }
}

/// Sets the status, the key and the signer of `result` from the status lines
/// of `gpg --verify`. A signature that is expired, or made by an expired or
/// revoked key, is not good: `git` reports such a signature as `X`, `Y` or `R`.
fn apply_verification(result: &mut CommitSignature, statuses: &[Status]) {
    result.status = SignatureStatus::Bad;
    for (keyword, text) in statuses {
        let mut fields = text.split_whitespace();
        match keyword.as_str() {
            "GOODSIG" => {
                result.status = SignatureStatus::Good;
                let key_id = fields.next();
                result.key = result
                    .key
                    .take()
                    .or_else(|| key_id.map(str::to_ascii_uppercase));
                result.signer = text
                    .split_once(' ')
                    .map(|(_, user)| user.trim().to_owned())
                    .filter(|user| !user.is_empty());
            }
            "BADSIG" | "EXPSIG" | "EXPKEYSIG" | "REVKEYSIG" => result.status = SignatureStatus::Bad,
            "VALIDSIG" => result.key = fields.next().map(str::to_ascii_uppercase),
            "ERRSIG" => {
                let fields: Vec<&str> = text.split_whitespace().collect();
                result.status = match fields.get(5).copied() {
                    Some("9") => SignatureStatus::UnknownKey,
                    Some("4") => SignatureStatus::Unsupported,
                    _ => SignatureStatus::Bad,
                };
                result.key = fields.last().map(|key| key.to_ascii_uppercase());
            }
            _ => {}
        }
    }
}

fn sign_with_agent(
    runner: Runner<'_>,
    payload: &[u8],
    config: &SigningConfig,
) -> Result<Signature> {
    let Some(key) = key_spec(config) else {
        return Err(no_key(
            "no OpenPGP signing key is configured; set `user.signingKey`",
        ));
    };
    let mut args = vec!["--batch".to_owned(), "--no-tty".to_owned()];
    if let Some(home) = &config.gnupg_home {
        args.extend(["--homedir".to_owned(), path_arg(home)]);
    }
    if let Some(file) = &config.passphrase_file {
        args.extend(passphrase_args(file)?);
    }
    let scratch = scratch_dir()?;
    run_sign(
        runner,
        program_of(config),
        args,
        Some(key),
        scratch.path(),
        payload,
        KeySource::Agent,
    )
}

fn sign_with_key_file(
    runner: Runner<'_>,
    payload: &[u8],
    config: &SigningConfig,
) -> Result<Option<Signature>> {
    let Some(file) = &config.key_file else {
        return Ok(None);
    };
    let program = program_of(config);
    let scratch = scratch_dir()?;
    let home = scratch.path().join("home");
    let mut args = vec![
        "--batch".to_owned(),
        "--no-tty".to_owned(),
        "--homedir".to_owned(),
        path_arg(&home),
        "--pinentry-mode".to_owned(),
        "loopback".to_owned(),
    ];
    if let Some(passphrase) = &config.passphrase_file {
        args.extend(["--passphrase-file".to_owned(), path_arg(passphrase)]);
        read_passphrase(passphrase)?;
    }
    let outcome = import_keys(runner, program, &home, std::slice::from_ref(file)).and_then(|()| {
        run_sign(
            runner,
            program,
            args,
            key_spec(config),
            scratch.path(),
            payload,
            KeySource::KeyFile,
        )
    });
    stop_agent(runner, &home);
    outcome.map(Some)
}

fn run_sign(
    runner: Runner<'_>,
    program: &str,
    mut args: Vec<String>,
    key: Option<&str>,
    cwd: &Path,
    payload: &[u8],
    source: KeySource,
) -> Result<Signature> {
    args.push("--status-fd=2".to_owned());
    args.extend(["--detach-sign".to_owned(), "--armor".to_owned()]);
    if let Some(key) = key {
        args.extend(["--local-user".to_owned(), key.to_owned()]);
    }
    let outcome = execute(runner, program, args, cwd, payload.to_vec())?;
    let status = statuses(&outcome.stderr);
    let created = status.iter().find(|(keyword, _)| keyword == "SIG_CREATED");
    let armored = armored_lf(&outcome.stdout);
    match created {
        Some((_, text)) if outcome.exit_code == Some(0) && armored.starts_with(ARMOR_HEADER) => {
            Ok(Signature {
                format: SignatureFormat::Openpgp,
                armored,
                key: text
                    .split_whitespace()
                    .last()
                    .unwrap_or_default()
                    .to_ascii_uppercase(),
                source,
            })
        }
        _ => Err(sign_failure(program, key, &outcome, &status)),
    }
}

/// The error for a run of GnuPG that created no signature.
fn sign_failure(
    program: &str,
    key: Option<&str>,
    outcome: &ProgramOutcome,
    status: &[Status],
) -> nurt_contracts::error::Error {
    let has = |keyword: &str| status.iter().any(|(found, _)| found == keyword);
    if has("INV_SGNR") || has("NO_SECKEY") {
        return no_key(format!(
            "no OpenPGP secret key matches `{}`",
            key.unwrap_or("the default key")
        ));
    }
    let bad_passphrase = status.iter().any(|(keyword, text)| {
        keyword == "FAILURE"
            && text
                .split_whitespace()
                .nth(1)
                .and_then(|code| code.parse::<u32>().ok())
                .is_some_and(|code| code & GPG_ERROR_CODE_MASK == GPG_ERR_BAD_PASSPHRASE)
    });
    if bad_passphrase
        || has("BAD_PASSPHRASE")
        || has("MISSING_PASSPHRASE")
        || has("NEED_PASSPHRASE")
    {
        return key_locked(
            "the OpenPGP secret key is protected by a passphrase that was not available; unlock it in `gpg-agent` or set NURT_GPG_PASSPHRASE_FILE",
        );
    }
    let detail = String::from_utf8_lossy(&outcome.stderr)
        .lines()
        .rev()
        .find(|line| !line.starts_with(STATUS_PREFIX) && !line.trim().is_empty())
        .map_or_else(String::new, |line| format!(": {}", line.trim()));
    program_failed(format!(
        "`{program}` created no signature (exit code {}){detail}",
        outcome
            .exit_code
            .map_or_else(|| "none".to_owned(), |code| code.to_string())
    ))
}

/// Imports the key files into a new GnuPG home directory `home`.
fn import_keys(runner: Runner<'_>, program: &str, home: &Path, files: &[PathBuf]) -> Result<()> {
    std::fs::create_dir(home).map_err(|source| io_error(home, &source))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(home, std::fs::Permissions::from_mode(0o700))
            .map_err(|source| io_error(home, &source))?;
    }
    let mut args = vec![
        "--batch".to_owned(),
        "--no-tty".to_owned(),
        "--homedir".to_owned(),
        path_arg(home),
        "--status-fd=2".to_owned(),
        "--import".to_owned(),
    ];
    for file in files {
        let metadata = std::fs::metadata(file).map_err(|source| io_error(file, &source))?;
        if !metadata.is_file() {
            return Err(with_path(
                no_key(format!("`{}` is not a key file", file.display())),
                "key_file",
                file,
            ));
        }
        args.push(path_arg(file));
    }
    let outcome = execute(runner, program, args, home, Vec::new())?;
    let imported = statuses(&outcome.stderr)
        .iter()
        .any(|(keyword, _)| keyword == "IMPORT_OK");
    if imported {
        Ok(())
    } else {
        Err(no_key(format!(
            "no OpenPGP key could be read from {}",
            files
                .iter()
                .map(|file| format!("`{}`", file.display()))
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }
}

/// Stops the `gpg-agent` that serves `home`; failures are not reported,
/// because the agent exits with its home directory in any case.
fn stop_agent(runner: Runner<'_>, home: &Path) {
    let args = vec![
        "--homedir".to_owned(),
        path_arg(home),
        "--kill".to_owned(),
        "gpg-agent".to_owned(),
    ];
    drop(execute(runner, "gpgconf", args, home, Vec::new()));
}

fn execute(
    runner: Runner<'_>,
    program: &str,
    args: Vec<String>,
    cwd: &Path,
    stdin: Vec<u8>,
) -> Result<ProgramOutcome> {
    let mut spec = ProgramSpec::new(program, args, cwd);
    spec.stdin = ProgramInput::Bytes(stdin);
    spec.timeout_ms = Some(TIMEOUT_MS);
    spec.env = vec![EnvVar {
        name: "LC_ALL".to_owned(),
        value: Some("C".to_owned()),
    }];
    let outcome = runner
        .host
        .run_program(&spec, runner.cancel)
        .map_err(|error| program_unavailable(format!("cannot run `{program}`: {error}")))?;
    if outcome.cancelled || outcome.timed_out {
        return Err(program_failed(format!(
            "`{program}` was stopped before it finished"
        )));
    }
    Ok(outcome)
}

fn statuses(output: &[u8]) -> Vec<Status> {
    String::from_utf8_lossy(output)
        .lines()
        .filter_map(|line| line.strip_prefix(STATUS_PREFIX))
        .map(|rest| match rest.split_once(' ') {
            Some((keyword, text)) => (keyword.to_owned(), text.to_owned()),
            None => (rest.to_owned(), String::new()),
        })
        .collect()
}

/// The armored signature with LF line endings and a final line feed.
fn armored_lf(output: &[u8]) -> Vec<u8> {
    let mut armored: Vec<u8> = output
        .iter()
        .copied()
        .filter(|byte| *byte != b'\r')
        .collect();
    if !armored.is_empty() && !armored.ends_with(b"\n") {
        armored.push(b'\n');
    }
    armored
}

fn program_of(config: &SigningConfig) -> &str {
    config.gpg_program.as_deref().unwrap_or(DEFAULT_PROGRAM)
}

/// The key to sign with: `user.signingKey`, otherwise the identity of the
/// committer or tagger, as Git does.
fn key_spec(config: &SigningConfig) -> Option<&str> {
    config
        .signing_key
        .as_deref()
        .or(config.default_identity.as_deref())
}

fn passphrase_args(file: &Path) -> Result<Vec<String>> {
    read_passphrase(file)?;
    Ok(vec![
        "--pinentry-mode".to_owned(),
        "loopback".to_owned(),
        "--passphrase-file".to_owned(),
        path_arg(file),
    ])
}

/// The public keyring of a GnuPG home directory, when it has one.
fn keyring_of(home: &Path) -> Option<PathBuf> {
    ["pubring.kbx", "pubring.gpg"]
        .iter()
        .map(|name| home.join(name))
        .find(|path| path.is_file())
}

fn scratch_dir() -> Result<tempfile::TempDir> {
    tempfile::Builder::new()
        .prefix("nurt-gnupg-")
        .tempdir()
        .map_err(|source| io_error(&std::env::temp_dir(), &source))
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
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
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use nurt_contracts::stage::CancelToken;

    use super::fixtures::*;
    use super::*;
    use crate::send_pack::testing::ScriptHost;

    const USER: &str = "Test Key <key@example.test>";

    fn status_lines(lines: &[(&str, &str)]) -> Vec<Status> {
        lines
            .iter()
            .map(|(keyword, text)| ((*keyword).to_owned(), (*text).to_owned()))
            .collect()
    }

    #[test]
    fn expired_and_revoked_keys_do_not_verify_as_good() {
        let user = "ABCD1234 Test Key <key@example.test>";
        for keyword in ["EXPSIG", "EXPKEYSIG", "REVKEYSIG", "BADSIG"] {
            let mut result = CommitSignature {
                status: SignatureStatus::Unsupported,
                format: Some(SignatureFormat::Openpgp),
                key: None,
                signer: None,
            };
            apply_verification(&mut result, &status_lines(&[(keyword, user)]));
            assert_eq!(result.status, SignatureStatus::Bad, "{keyword}");
        }
        let mut good = CommitSignature {
            status: SignatureStatus::Unsupported,
            format: Some(SignatureFormat::Openpgp),
            key: None,
            signer: None,
        };
        apply_verification(&mut good, &status_lines(&[("GOODSIG", user)]));
        assert_eq!(good.status, SignatureStatus::Good);
    }

    /// Runs `gpg` with `home` as its home directory, as the tests need a key
    /// that only GnuPG can generate.
    #[allow(
        clippy::disallowed_methods,
        reason = "the tests generate a key with the installed GnuPG, outside the program service"
    )]
    fn gpg(home: &Path, args: &[&str]) -> std::process::Output {
        let output = Command::new("gpg")
            .arg("--homedir")
            .arg(home)
            .args(["--batch", "--no-tty", "--pinentry-mode", "loopback"])
            .args(args)
            .output()
            .expect("gpg is installed");
        assert!(
            output.status.success(),
            "gpg {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn private_dir(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::create_dir(&path).expect("directory");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).expect("mode");
        }
        path
    }

    /// A GnuPG home directory holding an Ed25519 key of [`USER`] protected by
    /// a passphrase, and the armored secret key exported from it.
    struct Generated {
        home: PathBuf,
        secret: PathBuf,
        fingerprint: String,
    }

    fn generate(dir: &Path, passphrase: &str) -> Generated {
        let home = private_dir(dir, "generated");
        gpg(
            &home,
            &[
                "--passphrase",
                passphrase,
                "--quick-generate-key",
                USER,
                "ed25519",
                "sign",
                "never",
            ],
        );
        let listing = gpg(&home, &["--with-colons", "--list-secret-keys"]);
        let fingerprint = String::from_utf8_lossy(&listing.stdout)
            .lines()
            .find_map(|line| line.strip_prefix("fpr:::::::::"))
            .map(|rest| rest.trim_end_matches(':').to_owned())
            .expect("fingerprint");
        let secret = dir.join("secret.asc");
        let exported = gpg(
            &home,
            &[
                "--passphrase",
                passphrase,
                "--armor",
                "--export-secret-keys",
            ],
        );
        std::fs::write(&secret, exported.stdout).expect("write");
        Generated {
            home,
            secret,
            fingerprint,
        }
    }

    #[allow(
        clippy::disallowed_methods,
        reason = "the tests stop the agent they started with the installed GnuPG"
    )]
    fn stop_generated_agent(home: &Path) {
        drop(
            Command::new("gpgconf")
                .arg("--homedir")
                .arg(home)
                .args(["--kill", "gpg-agent"])
                .output(),
        );
    }

    fn key_file_config(key: &Generated) -> SigningConfig {
        let mut config = SigningConfig::new(SignatureFormat::Openpgp, CREATED);
        config.openpgp_sources = vec![OpenPgpSource::KeyFile];
        config.key_file = Some(key.secret.clone());
        config.signing_key = Some("key@example.test".to_owned());
        config
    }

    fn run<T>(work: impl FnOnce(Option<Runner<'_>>) -> T) -> T {
        let host = ScriptHost::default();
        let cancel = CancelToken::new();
        work(Some(Runner {
            host: &host,
            cancel: &cancel,
        }))
    }

    fn public_file(dir: &Path, name: &str, armored: &str) -> VerifyConfig {
        let path = dir.join(name);
        std::fs::write(&path, armored).expect("write");
        VerifyConfig {
            public_key_files: vec![path],
            ..VerifyConfig::default()
        }
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn verifies_signatures_made_by_gnupg() {
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
            let config = public_file(dir.path(), "public.asc", public);
            let result = run(|runner| verify(runner, PAYLOAD, signature.as_bytes(), &config))
                .expect("verify");
            assert_eq!(result.status, SignatureStatus::Good);
            assert_eq!(result.key.as_deref(), Some(fingerprint));
            assert_eq!(result.signer.as_deref(), Some(signer));
            let changed = run(|runner| verify(runner, b"other", signature.as_bytes(), &config))
                .expect("verify");
            assert_eq!(changed.status, SignatureStatus::Bad);
        }
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn reports_signature_of_an_unknown_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = VerifyConfig {
            gnupg_home: Some(private_dir(dir.path(), "empty")),
            ..VerifyConfig::default()
        };
        let result = run(|runner| verify(runner, PAYLOAD, ED25519_SIGNATURE.as_bytes(), &config))
            .expect("verify");
        assert_eq!(result.status, SignatureStatus::UnknownKey);
        assert_eq!(result.key.as_deref(), Some(ED25519_FINGERPRINT));
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn reports_garbage_as_a_bad_signature() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = public_file(dir.path(), "public.asc", ED25519_PUBLIC);
        let result =
            run(|runner| verify(runner, PAYLOAD, b"not a signature", &config)).expect("verify");
        assert_eq!(result.status, SignatureStatus::Bad);
    }

    #[test]
    fn reports_signatures_as_unsupported_without_a_program() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = public_file(dir.path(), "public.asc", ED25519_PUBLIC);
        let without_host =
            verify(None, PAYLOAD, ED25519_SIGNATURE.as_bytes(), &config).expect("verify");
        assert_eq!(without_host.status, SignatureStatus::Unsupported);
        config.gpg_program = Some("nurt-no-such-gpg".to_owned());
        let missing = run(|runner| verify(runner, PAYLOAD, ED25519_SIGNATURE.as_bytes(), &config))
            .expect("verify");
        assert_eq!(missing.status, SignatureStatus::Unsupported);
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn signs_with_a_key_file_and_verifies_the_signature() {
        let dir = tempfile::tempdir().expect("tempdir");
        let key = generate(dir.path(), "");
        let signature =
            run(|runner| sign(runner, PAYLOAD, &key_file_config(&key))).expect("signed");
        assert!(
            signature
                .armored
                .starts_with(b"-----BEGIN PGP SIGNATURE-----\n")
        );
        assert!(
            signature
                .armored
                .ends_with(b"-----END PGP SIGNATURE-----\n")
        );
        assert_eq!(signature.key, key.fingerprint);
        assert_eq!(signature.source, KeySource::KeyFile);
        let config = VerifyConfig {
            public_key_files: vec![key.secret.clone()],
            ..VerifyConfig::default()
        };
        let result =
            run(|runner| verify(runner, PAYLOAD, &signature.armored, &config)).expect("verify");
        assert_eq!(result.status, SignatureStatus::Good);
        assert_eq!(result.key.as_deref(), Some(key.fingerprint.as_str()));
        assert_eq!(result.signer.as_deref(), Some(USER));
        stop_generated_agent(&key.home);
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn signs_with_the_agent_of_a_gnupg_home() {
        let dir = tempfile::tempdir().expect("tempdir");
        let key = generate(dir.path(), "");
        let mut config = SigningConfig::new(SignatureFormat::Openpgp, CREATED);
        config.openpgp_sources = vec![OpenPgpSource::Agent];
        config.gnupg_home = Some(key.home.clone());
        config.default_identity = Some(USER.to_owned());
        let signature = run(|runner| sign(runner, PAYLOAD, &config)).expect("signed");
        assert_eq!(signature.key, key.fingerprint);
        assert_eq!(signature.source, KeySource::Agent);
        config.signing_key = Some("someone@example.test".to_owned());
        let error = run(|runner| sign(runner, PAYLOAD, &config)).expect_err("no key");
        assert_eq!(error.data().problem.as_deref(), Some("no_key"));
        stop_generated_agent(&key.home);
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn reports_key_file_without_matching_user_as_missing_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let key = generate(dir.path(), "");
        let mut config = key_file_config(&key);
        config.signing_key = Some("someone@example.test".to_owned());
        let error = run(|runner| sign(runner, PAYLOAD, &config)).expect_err("no match");
        assert_eq!(error.data().problem.as_deref(), Some("no_key"));
        config.signing_key = None;
        assert!(run(|runner| sign(runner, PAYLOAD, &config)).is_ok());
        stop_generated_agent(&key.home);
    }

    #[cfg(unix)]
    #[test]
    fn unlocks_a_key_file_with_a_passphrase_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let key = generate(dir.path(), "correct horse");
        let mut config = key_file_config(&key);
        let error = run(|runner| sign(runner, PAYLOAD, &config)).expect_err("locked");
        assert_eq!(error.data().problem.as_deref(), Some("key_locked"));
        let passphrase = dir.path().join("passphrase");
        std::fs::write(&passphrase, "wrong\n").expect("write");
        std::fs::set_permissions(&passphrase, std::fs::Permissions::from_mode(0o600))
            .expect("mode");
        config.passphrase_file = Some(passphrase.clone());
        let error = run(|runner| sign(runner, PAYLOAD, &config)).expect_err("wrong passphrase");
        assert_eq!(error.data().problem.as_deref(), Some("key_locked"));
        std::fs::write(&passphrase, "correct horse\n").expect("write");
        let signature = run(|runner| sign(runner, PAYLOAD, &config)).expect("signed");
        assert_eq!(signature.format, SignatureFormat::Openpgp);
        stop_generated_agent(&key.home);
    }

    #[test]
    fn refuses_to_sign_when_the_program_cannot_run() {
        let mut config = SigningConfig::new(SignatureFormat::Openpgp, CREATED);
        config.signing_key = Some("key@example.test".to_owned());
        let error = sign(None, PAYLOAD, &config).expect_err("no program service");
        assert_eq!(error.data().problem.as_deref(), Some("program_unavailable"));
        config.gpg_program = Some("nurt-no-such-gpg".to_owned());
        let error = run(|runner| sign(runner, PAYLOAD, &config)).expect_err("missing program");
        assert_eq!(error.data().problem.as_deref(), Some("program_unavailable"));
    }

    #[cfg(unix)] // the test writes POSIX absolute paths, which Windows reads as relative
    #[test]
    fn reports_a_failing_program_as_failed() {
        let mut config = SigningConfig::new(SignatureFormat::Openpgp, CREATED);
        config.signing_key = Some("key@example.test".to_owned());
        config.gpg_program = Some("false".to_owned());
        let error = run(|runner| sign(runner, PAYLOAD, &config)).expect_err("failure");
        assert_eq!(error.data().problem.as_deref(), Some("program_failed"));
    }
}
