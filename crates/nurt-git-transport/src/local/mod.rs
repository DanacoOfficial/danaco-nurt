// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! In-process transport for local repositories.
//!
//! For `file://` addresses and local paths `git` runs `git-upload-pack` as a
//! program. [`LocalTransport`] runs the server side of the protocol in a
//! thread of the calling process instead and talks to it over in-memory
//! pipes, so fetch and clone from local repositories go through the same
//! negotiation, shallow handling and pack reception as remote transfers.

mod objects;
pub mod receive;
#[cfg(test)]
mod testrepo;
mod upload_pack;

use std::borrow::Cow;
use std::fmt;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;

use bstr::{BStr, ByteSlice};
use gix_transport::client::blocking_io::{RequestWriter, SetServiceResponse};
use gix_transport::client::git::ConnectMode;
use gix_transport::client::git::blocking_io::Connection;
use gix_transport::client::{self, MessageKind, TransportWithoutIO, WriteMode};
use gix_transport::{Protocol, Service};
use gix_utils::io::pipe;

/// Writes buffered between the client and the server thread.
const PIPE_WRITES_IN_FLIGHT: usize = 64;

/// Failure to open a local repository as the source of a transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalError {
    /// The path is not a Git repository.
    NotRepository {
        /// Path of the address.
        path: PathBuf,
        /// Reason reported by the repository reader.
        reason: String,
    },
}

impl fmt::Display for LocalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRepository { path, reason } => {
                write!(f, "'{}' is not a git repository: {reason}", path.display())
            }
        }
    }
}

impl std::error::Error for LocalError {}

/// Path of the repository addressed by a `file://` address or a local path.
#[must_use]
pub fn repository_path(url: &gix_url::Url) -> PathBuf {
    gix::path::from_bstring(url.path.clone())
        .unwrap_or_else(|_| PathBuf::from(url.path.to_str_lossy().into_owned()))
}

/// Opening options of the source repository: no environment variables, no
/// configuration outside the repository and no external programs.
fn open_options() -> gix::open::Options {
    gix::open::Options::isolated()
}

/// Transport to a local repository whose upload-pack service runs in a
/// thread of this process.
pub struct LocalTransport {
    url: gix_url::Url,
    path: PathBuf,
    repo: gix::ThreadSafeRepository,
    version: Protocol,
    connection: Option<Connection<pipe::Reader, pipe::Writer>>,
    server: Option<JoinHandle<()>>,
}

impl fmt::Debug for LocalTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalTransport")
            .field("path", &self.path)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// Opens the repository at the path of `url` as the source of a fetch.
///
/// # Errors
///
/// Fails with [`LocalError::NotRepository`] when the path is not a readable
/// Git repository.
pub fn connect(url: gix_url::Url, version: Protocol) -> Result<LocalTransport, LocalError> {
    let path = repository_path(&url);
    let repo = gix::ThreadSafeRepository::open_opts(&path, open_options()).map_err(|e| {
        LocalError::NotRepository {
            path: path.clone(),
            reason: e.to_string(),
        }
    })?;
    Ok(LocalTransport {
        url,
        path,
        repo,
        version,
        connection: None,
        server: None,
    })
}

impl LocalTransport {
    /// Path of the source repository.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn stop(&mut self) {
        self.connection = None;
        if let Some(server) = self.server.take() {
            server.join().ok();
        }
    }
}

impl Drop for LocalTransport {
    fn drop(&mut self) {
        self.stop();
    }
}

impl TransportWithoutIO for LocalTransport {
    fn to_url(&self) -> Cow<'_, BStr> {
        Cow::Owned(self.url.to_bstring())
    }

    fn connection_persists_across_multiple_requests(&self) -> bool {
        true
    }

    fn configure(&mut self, _config: &dyn std::any::Any) -> gix::error::Result {
        Ok(())
    }
}

impl client::blocking_io::Transport for LocalTransport {
    fn handshake<'a>(
        &mut self,
        service: Service,
        extra_parameters: &'a [(&'a str, Option<&'a str>)],
    ) -> Result<SetServiceResponse<'_>, client::Error> {
        if service != Service::UploadPack {
            return Err(client::Error::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "the in-process transport serves fetches only",
            )));
        }
        self.stop();
        let (client_writer, server_reader) = pipe::unidirectional(PIPE_WRITES_IN_FLIGHT);
        let (server_writer, client_reader) = pipe::unidirectional(PIPE_WRITES_IN_FLIGHT);
        let repo = self.repo.clone();
        let server = std::thread::Builder::new()
            .name("nurt-git-upload-pack".to_owned())
            .spawn(move || {
                let repo = repo.to_thread_local();
                upload_pack::serve(&repo, server_reader, server_writer).ok();
            })
            .map_err(client::Error::Io)?;
        self.server = Some(server);
        let path = gix::path::into_bstr(self.path.as_path())
            .map_err(|e| {
                client::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    e.to_string(),
                ))
            })?
            .into_owned();
        let connection = self.connection.insert(Connection::new(
            client_reader,
            client_writer,
            self.version,
            path,
            None::<(&str, Option<u16>)>,
            ConnectMode::Process,
            false,
        ));
        connection.handshake(service, extra_parameters)
    }

    fn request(
        &mut self,
        write_mode: WriteMode,
        on_into_read: MessageKind,
        trace: bool,
    ) -> Result<RequestWriter<'_>, client::Error> {
        self.connection
            .as_mut()
            .ok_or(client::Error::MissingHandshake)?
            .request(write_mode, on_into_read, trace)
    }
}

#[cfg(test)]
mod tests {
    use super::testrepo::TestRepo;
    use super::*;
    use gix::ObjectId;
    use gix::remote::fetch::{Shallow, Status};
    use gix_transport::client::blocking_io::Transport as _;
    use std::num::NonZeroU32;
    use std::sync::atomic::AtomicBool;

    fn url_of(repo: &TestRepo) -> gix_url::Url {
        gix_url::parse(
            gix::path::into_bstr(repo.path())
                .expect("UTF-8 path")
                .as_ref(),
        )
        .expect("valid path")
    }

    fn fetch(
        client: &TestRepo,
        source: &TestRepo,
        shallow: Option<u32>,
    ) -> gix::remote::fetch::Outcome {
        let url = url_of(source);
        let remote = client
            .repo
            .remote_at(url.clone())
            .expect("remote")
            .with_refspecs(
                ["+refs/heads/*:refs/remotes/origin/*"],
                gix::remote::Direction::Fetch,
            )
            .expect("refspec");
        let transport = connect(url, Protocol::V2).expect("source opened");
        let prepare = remote
            .to_connection_with_transport(transport)
            .prepare_fetch(gix::progress::Discard, Default::default())
            .expect("references listed");
        let prepare = match shallow.and_then(NonZeroU32::new) {
            Some(depth) => prepare.with_shallow(Shallow::DepthAtRemote(depth)),
            None => prepare,
        };
        prepare
            .receive(gix::progress::Discard, &AtomicBool::new(false))
            .expect("pack received")
    }

    fn tracking(client: &TestRepo, branch: &str) -> Option<ObjectId> {
        client
            .repo
            .find_reference(format!("refs/remotes/origin/{branch}").as_str())
            .ok()
            .and_then(|r| r.target().try_id().map(ToOwned::to_owned))
    }

    fn received_objects(outcome: &gix::remote::fetch::Outcome) -> u32 {
        match &outcome.status {
            Status::Change {
                write_pack_bundle, ..
            } => write_pack_bundle.index.num_objects,
            Status::NoPackReceived { .. } => 0,
        }
    }

    #[test]
    fn fetches_all_branches_into_an_empty_repository() {
        let source = TestRepo::init();
        let first = source.commit("main", &[("a.txt", "one\n")], &[], "first");
        let second = source.commit(
            "main",
            &[("a.txt", "two\n"), ("b.txt", "b\n")],
            &[first],
            "second",
        );
        let topic = source.commit("topic", &[("c.txt", "c\n")], &[first], "topic");
        let client = TestRepo::init();
        let outcome = fetch(&client, &source, None);
        assert_eq!(tracking(&client, "main"), Some(second));
        assert_eq!(tracking(&client, "topic"), Some(topic));
        assert!(client.repo.find_commit(first).is_ok());
        assert_eq!(
            received_objects(&outcome),
            10,
            "three commits, three trees, four blobs"
        );
    }

    #[test]
    fn incremental_fetch_sends_only_new_objects() {
        let source = TestRepo::init();
        let first = source.commit(
            "main",
            &[("a.txt", "one\n"), ("b.txt", "b\n")],
            &[],
            "first",
        );
        let client = TestRepo::init();
        fetch(&client, &source, None);
        let second = source.commit(
            "main",
            &[("a.txt", "two\n"), ("b.txt", "b\n")],
            &[first],
            "second",
        );
        let outcome = fetch(&client, &source, None);
        assert_eq!(tracking(&client, "main"), Some(second));
        assert_eq!(
            received_objects(&outcome),
            3,
            "commit, tree and changed blob"
        );
    }

    #[test]
    fn up_to_date_fetch_receives_no_pack() {
        let source = TestRepo::init();
        source.commit("main", &[("a.txt", "one\n")], &[], "first");
        let client = TestRepo::init();
        fetch(&client, &source, None);
        assert_eq!(received_objects(&fetch(&client, &source, None)), 0);
    }

    #[test]
    fn shallow_fetch_stops_at_the_requested_depth() {
        let source = TestRepo::init();
        let first = source.commit("main", &[("a.txt", "one\n")], &[], "first");
        let second = source.commit("main", &[("a.txt", "two\n")], &[first], "second");
        let client = TestRepo::init();
        fetch(&client, &source, Some(1));
        assert_eq!(tracking(&client, "main"), Some(second));
        assert!(client.repo.find_commit(first).is_err());
        let shallow = client
            .repo
            .shallow_commits()
            .expect("readable")
            .expect("shallow");
        assert_eq!(shallow.iter().copied().collect::<Vec<_>>(), [second]);
    }

    #[test]
    fn annotated_tags_on_fetched_commits_follow() {
        let source = TestRepo::init();
        let first = source.commit("main", &[("a.txt", "one\n")], &[], "first");
        let tag = source.tag("v1", first);
        let client = TestRepo::init();
        fetch(&client, &source, None);
        let local = client
            .repo
            .find_reference("refs/tags/v1")
            .expect("tag followed");
        assert_eq!(local.target().try_id().map(ToOwned::to_owned), Some(tag));
    }

    #[test]
    fn lists_unborn_head_of_an_empty_repository() {
        let source = TestRepo::init();
        let client = TestRepo::init();
        let remote = client
            .repo
            .remote_at(url_of(&source))
            .expect("remote")
            .with_refspecs(
                ["+refs/heads/*:refs/remotes/origin/*"],
                gix::remote::Direction::Fetch,
            )
            .expect("refspec");
        let transport = connect(url_of(&source), Protocol::V2).expect("source opened");
        let prepare = remote
            .to_connection_with_transport(transport)
            .prepare_fetch(
                gix::progress::Discard,
                gix::remote::ref_map::Options {
                    prefix_from_spec_as_filter_on_remote: false,
                    ..Default::default()
                },
            )
            .expect("references listed");
        let unborn = prepare.ref_map().remote_refs.iter().any(|r| {
            matches!(r, gix::protocol::handshake::Ref::Unborn { full_ref_name, target }
                if full_ref_name == "HEAD" && target == "refs/heads/main")
        });
        assert!(unborn);
    }

    #[test]
    fn refuses_push_and_missing_repositories() {
        let source = TestRepo::init();
        let mut transport = connect(url_of(&source), Protocol::V2).expect("source opened");
        assert!(transport.handshake(Service::ReceivePack, &[]).is_err());
        assert_eq!(transport.path(), source.path());
        let missing = source.path().join("missing");
        let url = gix_url::parse(
            gix::path::into_bstr(missing.as_path())
                .expect("UTF-8 path")
                .as_ref(),
        )
        .expect("valid path");
        let error = connect(url, Protocol::V2).expect_err("missing");
        assert!(error.to_string().contains("is not a git repository"));
    }
}
