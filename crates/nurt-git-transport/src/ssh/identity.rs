// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Client keys: private key files and the SSH agent.
//!
//! The order of attempts follows OpenSSH: keys held by the agent come first
//! (with `IdentitiesOnly`, only those that correspond to a configured key
//! file), then key files the agent does not hold. Keys protected by a
//! passphrase are used through the agent or with the passphrase read from
//! the file named by `NURT_SSH_KEY_PASSPHRASE_FILE`; no terminal prompt is
//! shown. Security keys (`sk-*` algorithms) need a hardware token and are
//! not supported.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use nurt_contracts::error::{Error, ErrorCode, Result};
use russh::keys::agent::client::{AgentClient, AgentStream};
use russh::keys::{PrivateKey, PublicKey};

use super::config::AgentSetting;

/// Agent connection with the stream type erased.
pub(crate) type Agent = AgentClient<Box<dyn AgentStream + Send + Unpin + 'static>>;

/// Named pipe of the OpenSSH agent on Windows.
#[cfg(windows)]
const WINDOWS_AGENT_PIPE: &str = r"\\.\pipe\openssh-ssh-agent";

/// Permission bits that must be clear on the passphrase file.
#[cfg(unix)]
const GROUP_OTHER_BITS: u32 = 0o077;

/// A configured private key file.
#[derive(Debug, Clone)]
pub(crate) struct KeyFile {
    pub(crate) path: PathBuf,
    pub(crate) public: Option<PublicKey>,
    pub(crate) state: KeyState,
}

/// Usability of a key file.
#[derive(Debug, Clone)]
pub(crate) enum KeyState {
    /// The private key is available.
    Ready(Arc<PrivateKey>),
    /// The key is protected by a passphrase that is not available.
    Locked,
    /// The key cannot be used; the text says why.
    Unsupported(String),
}

/// A key to offer to the server.
#[derive(Debug, Clone)]
pub(crate) enum Credential {
    /// A key held by the agent.
    Agent { key: PublicKey, comment: String },
    /// A key read from a file.
    File { path: PathBuf, key: Arc<PrivateKey> },
}

impl Credential {
    /// Public key offered to the server.
    pub(crate) fn public_key(&self) -> &PublicKey {
        match self {
            Self::Agent { key, .. } => key,
            Self::File { key, .. } => key.public_key(),
        }
    }

    /// Description of the source without secrets, for error details.
    pub(crate) fn describe(&self) -> String {
        let fingerprint = super::known_hosts::fingerprint(self.public_key());
        match self {
            Self::Agent { comment, .. } if comment.is_empty() => format!("agent {fingerprint}"),
            Self::Agent { comment, .. } => format!("agent {fingerprint} ({comment})"),
            Self::File { path, .. } => format!("{} {fingerprint}", path.display()),
        }
    }
}

/// Keys to offer and keys that could not be used.
#[derive(Debug, Clone, Default)]
pub(crate) struct Identities {
    pub(crate) credentials: Vec<Credential>,
    pub(crate) locked: Vec<PathBuf>,
    pub(crate) unsupported: Vec<(PathBuf, String)>,
}

/// Reads a key file; `None` when the file does not exist.
pub(crate) fn load_key_file(path: &Path, passphrase: Option<&str>) -> Option<KeyFile> {
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let mut public_path = path.as_os_str().to_owned();
    public_path.push(".pub");
    let public = russh::keys::load_public_key(PathBuf::from(public_path)).ok();
    let state = match russh::keys::decode_secret_key(&text, None) {
        Ok(key) => KeyState::Ready(Arc::new(key)),
        Err(russh::keys::Error::KeyIsEncrypted) => match passphrase {
            Some(passphrase) => russh::keys::decode_secret_key(&text, Some(passphrase))
                .map_or(KeyState::Locked, |key| KeyState::Ready(Arc::new(key))),
            None => KeyState::Locked,
        },
        Err(err) => KeyState::Unsupported(err.to_string()),
    };
    let public = public.or_else(|| match &state {
        KeyState::Ready(key) => Some(key.public_key().clone()),
        _ => None,
    });
    let state = match &public {
        Some(key) if key.algorithm().as_str().starts_with("sk-") => {
            KeyState::Unsupported("security keys need a hardware token".to_owned())
        }
        _ => state,
    };
    Some(KeyFile {
        path: path.to_path_buf(),
        public,
        state,
    })
}

/// Orders the keys as OpenSSH does.
pub(crate) fn plan(
    files: Vec<KeyFile>,
    agent_keys: Vec<(PublicKey, String)>,
    identities_only: bool,
) -> Identities {
    let mut identities = Identities::default();
    let mut files: Vec<Option<KeyFile>> = files.into_iter().map(Some).collect();
    for (key, comment) in agent_keys {
        let matching = files.iter_mut().find(|file| {
            file.as_ref()
                .and_then(|file| file.public.as_ref())
                .is_some_and(|public| public.key_data() == key.key_data())
        });
        match matching {
            Some(slot) => {
                *slot = None;
                identities
                    .credentials
                    .push(Credential::Agent { key, comment });
            }
            None if !identities_only => identities
                .credentials
                .push(Credential::Agent { key, comment }),
            None => {}
        }
    }
    for file in files.into_iter().flatten() {
        match file.state {
            KeyState::Ready(key) => identities.credentials.push(Credential::File {
                path: file.path,
                key,
            }),
            KeyState::Locked => identities.locked.push(file.path),
            KeyState::Unsupported(reason) => identities.unsupported.push((file.path, reason)),
        }
    }
    identities
}

/// Reads the first line of the passphrase file.
///
/// On Unix the account of the file must be the account of the home
/// directory, and the group and other accounts must have no access. Fails
/// with `E_GIT_AUTH` and problem `passphrase_file` when the file
/// cannot be read or other accounts can access it.
pub(crate) fn read_passphrase_file(path: &Path, home: &Path) -> Result<String> {
    let failure = |reason: String| {
        Error::new(
            ErrorCode::GitAuth,
            format!("passphrase file `{}` {reason}", path.display()),
        )
        .with_problem("passphrase_file")
    };
    let metadata =
        std::fs::metadata(path).map_err(|err| failure(format!("cannot be read: {err}")))?;
    check_permissions(path, &metadata, home).map_err(|reason| {
        failure(reason).with_fix(format!("run [\"chmod\", \"600\", \"{}\"]", path.display()))
    })?;
    let text =
        std::fs::read_to_string(path).map_err(|err| failure(format!("cannot be read: {err}")))?;
    Ok(text.lines().next().unwrap_or_default().to_owned())
}

#[cfg(unix)]
fn check_permissions(
    _path: &Path,
    metadata: &std::fs::Metadata,
    home: &Path,
) -> std::result::Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    if metadata.mode() & GROUP_OTHER_BITS != 0 {
        return Err("is accessible to other accounts; allow access to the owner only".to_owned());
    }
    let owner = std::fs::metadata(home).map(|home| home.uid()).ok();
    if owner.is_some_and(|owner| owner != metadata.uid()) {
        return Err("belongs to another account".to_owned());
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_permissions(
    _path: &Path,
    _metadata: &std::fs::Metadata,
    _home: &Path,
) -> std::result::Result<(), String> {
    Ok(())
}

/// Connects to the agent selected by `IdentityAgent`; `None` when no agent
/// is reachable.
pub(crate) async fn connect_agent(
    setting: &AgentSetting,
    auth_sock: Option<&Path>,
) -> Option<Agent> {
    let path = match setting {
        AgentSetting::Disabled => return None,
        AgentSetting::Path(path) => Some(path.as_path()),
        AgentSetting::Default => auth_sock,
    };
    connect_platform_agent(path).await
}

#[cfg(unix)]
async fn connect_platform_agent(path: Option<&Path>) -> Option<Agent> {
    let client = AgentClient::connect_uds(path?).await.ok()?;
    Some(client.dynamic())
}

#[cfg(windows)]
async fn connect_platform_agent(path: Option<&Path>) -> Option<Agent> {
    let pipe = path.map_or_else(
        || std::ffi::OsString::from(WINDOWS_AGENT_PIPE),
        |path| path.as_os_str().to_owned(),
    );
    if let Ok(client) = AgentClient::connect_named_pipe(&pipe).await {
        return Some(client.dynamic());
    }
    if path.is_some() {
        return None;
    }
    AgentClient::connect_pageant()
        .await
        .ok()
        .map(AgentClient::dynamic)
}

#[cfg(not(any(unix, windows)))]
async fn connect_platform_agent(_path: Option<&Path>) -> Option<Agent> {
    None
}

/// Public keys held by the agent; certificates are skipped.
pub(crate) async fn agent_keys(agent: &mut Agent) -> Vec<(PublicKey, String)> {
    agent
        .request_identities()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|identity| match identity {
            russh::keys::agent::AgentIdentity::PublicKey { key, comment } => Some((key, comment)),
            russh::keys::agent::AgentIdentity::Certificate { .. } => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::ssh_key::LineEnding;
    use russh::keys::ssh_key::private::Ed25519Keypair;

    fn private(seed: u8) -> PrivateKey {
        PrivateKey::from(Ed25519Keypair::from_seed(&[seed; 32]))
    }

    fn write_key(dir: &Path, name: &str, seed: u8, passphrase: Option<&str>) -> PathBuf {
        let mut key = private(seed);
        if let Some(passphrase) = passphrase {
            key = key
                .encrypt(&mut russh::keys::key::safe_rng(), passphrase)
                .unwrap();
        }
        let path = dir.join(name);
        std::fs::write(&path, key.to_openssh(LineEnding::LF).unwrap().as_bytes()).unwrap();
        path
    }

    #[test]
    fn plain_key_file_is_ready() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_key(dir.path(), "id_ed25519", 1, None);
        let file = load_key_file(&path, None).unwrap();
        assert!(matches!(file.state, KeyState::Ready(_)));
        assert_eq!(
            file.public.unwrap().key_data(),
            private(1).public_key().key_data()
        );
    }

    #[test]
    fn missing_key_file_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_key_file(&dir.path().join("id_rsa"), None).is_none());
    }

    #[test]
    fn encrypted_key_needs_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_key(dir.path(), "id_ed25519", 2, Some("secret words"));
        let public = private(2).public_key().to_openssh().unwrap();
        std::fs::write(dir.path().join("id_ed25519.pub"), public).unwrap();
        let locked = load_key_file(&path, None).unwrap();
        assert!(matches!(locked.state, KeyState::Locked));
        assert!(locked.public.is_some());
        let wrong = load_key_file(&path, Some("other")).unwrap();
        assert!(matches!(wrong.state, KeyState::Locked));
        let ready = load_key_file(&path, Some("secret words")).unwrap();
        assert!(matches!(ready.state, KeyState::Ready(_)));
    }

    #[test]
    fn garbage_key_file_is_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("id_ecdsa");
        std::fs::write(&path, "not a key").unwrap();
        assert!(matches!(
            load_key_file(&path, None).unwrap().state,
            KeyState::Unsupported(_)
        ));
    }

    fn ready_file(seed: u8) -> KeyFile {
        let key = private(seed);
        KeyFile {
            path: PathBuf::from(format!("/keys/{seed}")),
            public: Some(key.public_key().clone()),
            state: KeyState::Ready(Arc::new(key)),
        }
    }

    #[test]
    fn agent_keys_come_first_and_replace_matching_files() {
        let agent = vec![
            (private(9).public_key().clone(), "other".to_owned()),
            (private(1).public_key().clone(), "laptop".to_owned()),
        ];
        let identities = plan(vec![ready_file(1), ready_file(2)], agent, false);
        let order: Vec<String> = identities
            .credentials
            .iter()
            .map(|credential| match credential {
                Credential::Agent { comment, .. } => format!("agent:{comment}"),
                Credential::File { path, .. } => path.display().to_string(),
            })
            .collect();
        assert_eq!(order, ["agent:other", "agent:laptop", "/keys/2"]);
    }

    #[test]
    fn identities_only_keeps_only_configured_agent_keys() {
        let agent = vec![
            (private(9).public_key().clone(), "other".to_owned()),
            (private(1).public_key().clone(), "laptop".to_owned()),
        ];
        let identities = plan(vec![ready_file(1)], agent, true);
        assert_eq!(identities.credentials.len(), 1);
        assert!(
            matches!(&identities.credentials[0], Credential::Agent { comment, .. } if comment == "laptop")
        );
    }

    #[test]
    fn locked_and_unsupported_files_are_reported() {
        let locked = KeyFile {
            path: PathBuf::from("/keys/locked"),
            public: None,
            state: KeyState::Locked,
        };
        let token = KeyFile {
            path: PathBuf::from("/keys/token"),
            public: None,
            state: KeyState::Unsupported("security keys need a hardware token".into()),
        };
        let identities = plan(vec![locked, token], Vec::new(), false);
        assert!(identities.credentials.is_empty());
        assert_eq!(identities.locked, [PathBuf::from("/keys/locked")]);
        assert_eq!(identities.unsupported.len(), 1);
    }

    #[test]
    fn locked_file_held_by_agent_is_used_through_agent() {
        let locked = KeyFile {
            path: PathBuf::from("/keys/locked"),
            public: Some(private(3).public_key().clone()),
            state: KeyState::Locked,
        };
        let identities = plan(
            vec![locked],
            vec![(private(3).public_key().clone(), String::new())],
            true,
        );
        assert_eq!(identities.credentials.len(), 1);
        assert!(identities.locked.is_empty());
        assert!(
            identities.credentials[0]
                .describe()
                .starts_with("agent SHA256:")
        );
    }

    #[cfg(unix)]
    #[test]
    fn passphrase_file_must_be_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pass");
        std::fs::write(&path, "secret words\nignored\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let error = read_passphrase_file(&path, dir.path()).unwrap_err();
        assert_eq!(error.code(), ErrorCode::GitAuth);
        assert!(!error.data().fix.is_empty());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            read_passphrase_file(&path, dir.path()).unwrap(),
            "secret words"
        );
    }

    #[test]
    fn missing_passphrase_file_is_an_auth_error() {
        let dir = tempfile::tempdir().unwrap();
        let error = read_passphrase_file(&dir.path().join("absent"), dir.path()).unwrap_err();
        assert_eq!(error.data().problem.as_deref(), Some("passphrase_file"));
    }
}
