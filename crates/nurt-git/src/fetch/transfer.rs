// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The network part of a fetch: connection, handshake, listing of the remote
//! references, negotiation and the write of the received pack.
//!
//! The transport comes from [`nurt_git_transport::connect`], so no program
//! is started for any address. The protocol library writes the received
//! pack into the object store; it would also write the `shallow` file, so it
//! is given a copy of that file in a scratch directory of the object store
//! and the new content is returned for a journaled write. References are
//! never written here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use bstr::ByteSlice;
use gix::error::ResultExt;
use gix::protocol::fetch::refmap::SpecIndex;
use gix::protocol::fetch::{Arguments, RefMap, Shallow, Tags, negotiate};
use gix::protocol::handshake::Ref;
use gix::protocol::transport::Protocol;
use gix::protocol::transport::client::blocking_io::Transport;
use gix::refspec::RefSpec;
use nurt_contracts::error::Result;
use nurt_contracts::report::{Notable, NotableEntry};
use nurt_contracts::stage::ExecCtx;
use nurt_git_transport::connect::{self, Direction, TransportOptions, UrlRewrites};
use nurt_git_transport::credentials::{CredentialOptions, Credentials, EnvLookup, bearer_token};
use nurt_git_transport::http::{HttpNote, HttpSettings};
use nurt_git_transport::ssh::{HostKeyAddition, SshOptions};

use super::error;

/// Interval at which cancellation and the time limit are checked during a
/// transfer.
const WATCH_INTERVAL: Duration = Duration::from_millis(50);

/// Name of the scratch directories of the `shallow` file in the object
/// store.
const SCRATCH_PREFIX: &str = "nurt-shallow-";

/// Name of the `shallow` file in a scratch directory.
const SHALLOW_FILE: &str = "shallow";

/// Number of scratch directories created by this process, for unique names.
static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Address and settings of one transfer.
pub(crate) struct Network {
    /// Address used for the connection, before the rewrite rules.
    pub(crate) url: String,
    options: TransportOptions,
    /// Credential lookup of the address.
    pub(crate) credentials: Credentials,
    /// Unusual settings, such as disabled certificate verification.
    pub(crate) notable: Vec<NotableEntry>,
}

impl Network {
    /// Settings for `url` from the effective configuration `config` and the
    /// execution context.
    ///
    /// # Errors
    ///
    /// `E_NETWORK` with problem `proxy` or `tls` for invalid HTTP settings.
    pub(crate) fn new(
        config: &gix::config::File,
        url: &str,
        remote: Option<&str>,
        accept_host_key: Option<String>,
        retry: &str,
        ctx: &ExecCtx,
    ) -> Result<Self> {
        let env = env_lookup(ctx);
        let rewrites = UrlRewrites::from_config(config);
        let rewritten = rewrites.rewrite(url, Direction::Fetch);
        let mut notable = Vec::new();
        let http = if rewritten.starts_with("http://") || rewritten.starts_with("https://") {
            let mut settings = HttpSettings::resolve(config, &rewritten, remote, &*env)
                .map_err(|failure| error::network(&failure))?;
            if settings.bearer.is_none() {
                settings.bearer = bearer_token(&*env, &rewritten);
            }
            notable.extend(settings.notes.iter().map(http_note));
            settings
        } else {
            HttpSettings::default()
        };
        let mut ssh = SshOptions::from_ctx(ctx);
        ssh.accept_host_key = accept_host_key;
        ssh.retry_command = Some(retry.to_owned());
        let mut options = TransportOptions::new(http)
            .with_rewrites(rewrites)
            .with_ssh(ssh);
        options.protocol = protocol_version(config);
        let credentials = Credentials::new(config, &rewritten, CredentialOptions::new(env));
        Ok(Self {
            url: url.to_owned(),
            options,
            credentials,
            notable,
        })
    }
}

impl Network {
    /// Keys of new SSH hosts accepted by the connections of this transfer,
    /// which the command records in its transaction.
    pub(crate) fn host_key_additions(&self) -> Vec<HostKeyAddition> {
        self.options
            .ssh
            .as_ref()
            .map(|ssh| ssh.host_keys.take())
            .unwrap_or_default()
    }
}

/// Variables of the execution context, read by the credential sources.
fn env_lookup(ctx: &ExecCtx) -> EnvLookup {
    let variables: HashMap<String, String> = ctx
        .host()
        .env_vars()
        .into_iter()
        .map(|(name, value)| {
            (
                name.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        })
        .collect();
    Arc::new(move |name: &str| variables.get(name).cloned())
}

/// Report entry of an HTTP setting.
fn http_note(note: &HttpNote) -> NotableEntry {
    match note {
        HttpNote::TlsVerificationDisabled => {
            let mut entry = NotableEntry::new(Notable::TlsVerificationDisabled);
            entry.detail = Some("http.sslVerify is false".to_owned());
            entry
        }
        HttpNote::OptionIgnored { key } => {
            let mut entry = NotableEntry::new(Notable::HttpOptionIgnored);
            entry.detail = Some(key.clone());
            entry
        }
    }
}

/// Protocol version of `protocol.version`; version 2 unless 0 or 1 is
/// configured.
fn protocol_version(config: &gix::config::File) -> Protocol {
    match config.integer("protocol.version") {
        Ok(Some(0 | 1)) => Protocol::V1,
        _ => Protocol::V2,
    }
}

/// Feature naming the client in the protocol.
fn agent() -> gix::protocol::command::Feature {
    (
        "agent",
        Some(format!("git/nurt-{}", env!("CARGO_PKG_VERSION"))),
    )
}

/// An open connection after the handshake, with the references of the
/// remote.
pub(crate) struct Session {
    transport: Box<dyn Transport>,
    handshake: gix::protocol::Handshake,
    remote_refs: Vec<Ref>,
}

/// Result of receiving a pack.
#[derive(Debug, Clone, Default)]
pub(crate) struct Received {
    /// Objects in the received pack.
    pub(crate) objects: u64,
    /// Size of the received pack in bytes.
    pub(crate) bytes: u64,
    /// A pack was written into the object store.
    pub(crate) pack_written: bool,
    /// New content of the `shallow` file when it changed; empty when the
    /// file is removed.
    pub(crate) shallow: Option<Vec<u8>>,
}

impl Received {
    /// Adds the counts of a later transfer of the same operation.
    pub(crate) fn absorb(&mut self, later: &Received) {
        self.objects += later.objects;
        self.bytes += later.bytes;
        self.pack_written |= later.pack_written;
    }
}

/// Annotated tags that point into the received history but whose tag object
/// did not arrive with the pack: the protocol sends tag objects on request
/// only, so they are fetched in a second transfer, as `git` does.
pub(crate) fn missing_tag_objects(repo: &gix::Repository, ref_map: &RefMap) -> Vec<String> {
    ref_map
        .mappings
        .iter()
        .filter(|mapping| matches!(mapping.spec_index, SpecIndex::Implicit(_)))
        .filter_map(|mapping| {
            let local = mapping.local.as_ref()?.to_str_lossy().into_owned();
            let remote = super::update::remote_ref(&mapping.remote)?;
            let missing = remote.id != remote.peeled
                && repo.has_object(remote.peeled)
                && !repo.has_object(remote.id)
                && repo
                    .try_find_reference(local.as_str())
                    .ok()
                    .flatten()
                    .is_none();
            missing.then_some(remote.name)
        })
        .collect()
}

/// Fetches the tag objects of the tags `names` in a second transfer.
///
/// # Errors
///
/// The errors of [`Session::open`] and [`Session::receive`].
pub(crate) fn fetch_tag_objects(
    network: &Network,
    repo: &gix::Repository,
    names: &[String],
    ctx: &ExecCtx,
) -> Result<Received> {
    let specs = names
        .iter()
        .map(|name| super::remote::parse_spec(&format!("+{name}:{name}")))
        .collect::<Result<Vec<_>>>()?;
    let session = Session::open(network, ctx)?;
    let ref_map = session.map(specs, Vec::new())?;
    session.receive(repo, &ref_map, &Shallow::NoChange, Tags::None, network, ctx)
}

impl Session {
    /// Connects, performs the handshake and lists the references of the
    /// remote.
    ///
    /// # Errors
    ///
    /// The errors of [`connect::connect`] and the errors of the transfer.
    pub(crate) fn open(network: &Network, ctx: &ExecCtx) -> Result<Self> {
        ctx.checkpoint()?;
        ctx.progress("connect", 0, None);
        let mut transport = connect::connect(&network.url, Direction::Fetch, &network.options)?;
        let mut progress = gix::progress::Discard;
        let mut handshake = gix::protocol::handshake(
            &mut transport,
            gix::protocol::transport::Service::UploadPack,
            network.credentials.authenticate(),
            Vec::new(),
            &mut progress,
        )
        .map_err(|failure| error::transfer(network, &failure, "the handshake"))?;
        let remote_refs = match handshake.refs.take() {
            Some(refs) => refs,
            None => {
                let context = gix::protocol::fetch::refmap::init::Context {
                    fetch_refspecs: Vec::new(),
                    extra_refspecs: Vec::new(),
                };
                let listing = handshake
                    .prepare_lsrefs_or_extract_refmap(agent(), false, context)
                    .map_err(|failure| error::transfer(network, &failure, "the listing"))?;
                listing
                    .fetch_blocking(gix::progress::Discard, &mut transport, false)
                    .map_err(|failure| error::transfer(network, &failure, "the listing"))?
                    .remote_refs
            }
        };
        Ok(Self {
            transport,
            handshake,
            remote_refs,
        })
    }

    /// References advertised by the remote.
    pub(crate) fn remote_refs(&self) -> &[Ref] {
        &self.remote_refs
    }

    /// Mapping of the remote references by `specs` and the implicit `extra`
    /// refspecs.
    ///
    /// # Errors
    ///
    /// `E_GIT_PROTOCOL` when the mappings are inconsistent or the remote
    /// announces an unknown object format.
    pub(crate) fn map(&self, specs: Vec<RefSpec>, extra: Vec<RefSpec>) -> Result<RefMap> {
        RefMap::from_refs(
            self.remote_refs.clone(),
            &self.handshake.capabilities,
            gix::protocol::fetch::refmap::init::Context {
                fetch_refspecs: specs,
                extra_refspecs: extra,
            },
        )
        .map_err(|failure| {
            error::protocol(
                "",
                &failure.to_string(),
                "the mapping of the remote references",
            )
        })
    }

    /// Negotiates and receives the objects of `ref_map` into the object
    /// store of `repo`.
    ///
    /// # Errors
    ///
    /// `E_GIT_UNSUPPORTED` for a remote with another object format, the
    /// errors of the transfer, and `E_CANCELLED` or `E_TIMEOUT` when the
    /// transfer was interrupted.
    pub(crate) fn receive(
        mut self,
        repo: &gix::Repository,
        ref_map: &RefMap,
        shallow: &Shallow,
        tags: Tags,
        network: &Network,
        ctx: &ExecCtx,
    ) -> Result<Received> {
        if ref_map.object_hash != repo.object_hash() {
            return Err(error::object_format(
                &network.url,
                ref_map.object_hash,
                repo.object_hash(),
            ));
        }
        ctx.checkpoint()?;
        ctx.progress("receive", 0, None);
        let objects_dir = repo.objects.store_ref().path().to_path_buf();
        let shallow_file = repo
            .shallow_file()
            .map_err(|failure| error::read_error("locate the shallow file", &failure))?;
        let scratch = Scratch::new(&objects_dir, &shallow_file)?;
        let interrupt = AtomicBool::new(false);
        let exchange = Exchange {
            repo,
            ref_map,
            shallow,
            tags,
            shallow_file: scratch.file(),
            pack_dir: objects_dir.join("pack"),
        };
        let outcome = watched(ctx, &interrupt, || exchange.run(&mut self, &interrupt));
        let changed = scratch.changed();
        drop(scratch);
        if interrupt.load(Ordering::Relaxed) {
            return Err(error::interrupted(ctx));
        }
        let mut received =
            outcome.map_err(|failure| error::transfer(network, &failure, "the fetch"))?;
        received.shallow = changed?;
        ctx.progress("receive", received.objects, Some(received.objects));
        Ok(received)
    }
}

/// One negotiation and pack transfer.
struct Exchange<'a> {
    repo: &'a gix::Repository,
    ref_map: &'a RefMap,
    shallow: &'a Shallow,
    tags: Tags,
    shallow_file: PathBuf,
    pack_dir: PathBuf,
}

impl Exchange<'_> {
    fn run(&self, session: &mut Session, interrupt: &AtomicBool) -> gix::Result<Received> {
        let repo = self.repo;
        let mut graph = repo.revision_graph(None);
        let mut negotiation = Negotiation {
            repo,
            graph: &mut graph,
            negotiator: gix::negotiate::Algorithm::Consecutive.into_negotiator(),
            ref_map: self.ref_map,
            shallow: self.shallow,
            tags: self.tags,
            interrupt,
        };
        let mut received = Received::default();
        let options = gix::odb::pack::bundle::write::Options {
            thread_limit: None,
            iteration_mode: gix::odb::pack::data::input::Mode::Verify,
            index_version: gix::odb::pack::index::Version::V2,
            alloc_limit_bytes: None,
            compression: gix::zlib::Compression::default(),
        };
        let pack_dir = &self.pack_dir;
        gix::protocol::fetch(
            &mut negotiation,
            |reader, progress, should_interrupt| -> gix::Result<bool> {
                let written = gix::odb::pack::Bundle::write_to_directory(
                    reader,
                    Some(pack_dir),
                    progress,
                    should_interrupt,
                    Some(repo.objects.clone()),
                    repo.object_hash(),
                    options,
                )
                .or_raise(|| gix::error::message("failed to write the received pack"))?;
                received.objects = u64::from(written.index.num_objects);
                received.bytes = written
                    .data_path
                    .as_ref()
                    .and_then(|path| std::fs::metadata(path).ok())
                    .map_or(0, |meta| meta.len());
                received.pack_written = written.data_path.is_some();
                if let Some(keep) = &written.keep_path {
                    std::fs::remove_file(keep).ok();
                }
                Ok(true)
            },
            gix::progress::Discard,
            interrupt,
            gix::protocol::fetch::Context {
                handshake: &mut session.handshake,
                transport: &mut session.transport,
                user_agent: agent(),
                trace_packetlines: false,
            },
            gix::protocol::fetch::Options {
                shallow_file: self.shallow_file.clone(),
                shallow: self.shallow,
                tags: self.tags,
                reject_shallow_remote: false,
            },
        )?;
        if session.handshake.server_protocol_version == Protocol::V2 {
            gix::protocol::indicate_end_of_interaction(&mut session.transport, false).ok();
        }
        Ok(received)
    }
}

/// Runs `work` while a watcher sets `interrupt` on cancellation or when the
/// time limit of the context is reached.
fn watched<T>(ctx: &ExecCtx, interrupt: &AtomicBool, work: impl FnOnce() -> T) -> T {
    let done = AtomicBool::new(false);
    let watch = ctx.clone();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                let expired = watch.remaining().is_some_and(|left| left.is_zero());
                if watch.cancel().is_cancelled() || expired {
                    interrupt.store(true, Ordering::Relaxed);
                    return;
                }
                std::thread::sleep(WATCH_INTERVAL);
            }
        });
        // Stops the watcher when `work` panics as well, so that the scope can
        // join it while the panic unwinds.
        let _stop = StopWatch(&done);
        work()
    })
}

/// Tells the watcher of [`watched`] to end when dropped.
struct StopWatch<'a>(&'a AtomicBool);

impl Drop for StopWatch<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Negotiation of the objects to send, with the algorithm `git` uses by
/// default.
struct Negotiation<'a, 'r> {
    repo: &'r gix::Repository,
    graph: &'a mut gix::negotiate::Graph<'r, 'r>,
    negotiator: Box<dyn gix::negotiate::Negotiator>,
    ref_map: &'a RefMap,
    shallow: &'a Shallow,
    tags: Tags,
    interrupt: &'a AtomicBool,
}

impl Negotiation<'_, '_> {
    /// Fails once the watcher has asked for the end of the transfer, so
    /// that the negotiation rounds stop as well as the pack write.
    fn check_interrupt(&self) -> gix::Result<()> {
        if self.interrupt.load(Ordering::Relaxed) {
            return Err(gix::error::message("the transfer was interrupted").into());
        }
        Ok(())
    }
}

impl gix::protocol::fetch::Negotiate for Negotiation<'_, '_> {
    fn mark_complete_and_common_ref(&mut self) -> gix::Result<negotiate::Action> {
        self.check_interrupt()?;
        negotiate::mark_complete_and_common_ref(
            &self.repo.objects,
            &self.repo.refs,
            || -> gix::Result<std::iter::Empty<(gix::refs::file::Store, gix::OdbHandle)>> {
                Ok(std::iter::empty())
            },
            self.negotiator.as_mut(),
            &mut *self.graph,
            self.ref_map,
            self.shallow,
            negotiate::make_refmapping_ignore_predicate(self.tags, self.ref_map),
        )
    }

    fn add_wants(&mut self, arguments: &mut Arguments, remote_ref_target_known: &[bool]) -> bool {
        negotiate::add_wants(
            &self.repo.objects,
            arguments,
            self.ref_map,
            remote_ref_target_known,
            self.shallow,
            negotiate::make_refmapping_ignore_predicate(self.tags, self.ref_map),
        )
    }

    fn one_round(
        &mut self,
        state: &mut negotiate::one_round::State,
        arguments: &mut Arguments,
        previous_response: Option<&gix::protocol::fetch::Response>,
    ) -> gix::Result<(negotiate::Round, bool)> {
        self.check_interrupt()?;
        negotiate::one_round(
            self.negotiator.as_mut(),
            &mut *self.graph,
            state,
            arguments,
            previous_response,
        )
    }
}

/// Scratch directory of the object store holding a copy of the `shallow`
/// file for the protocol library.
struct Scratch {
    dir: PathBuf,
    before: Option<Vec<u8>>,
}

impl Scratch {
    fn new(objects_dir: &Path, shallow_file: &Path) -> Result<Self> {
        let serial = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = objects_dir.join(format!("{SCRATCH_PREFIX}{}-{serial}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|failure| crate::repo::error::io(&dir, &failure))?;
        // The directory is removed by `Drop` from here on, also when the copy
        // of the shallow file fails.
        let mut scratch = Self { dir, before: None };
        scratch.before = std::fs::read(shallow_file).ok();
        if let Some(content) = &scratch.before {
            let copy = scratch.file();
            std::fs::write(&copy, content)
                .map_err(|failure| crate::repo::error::io(&copy, &failure))?;
        }
        Ok(scratch)
    }

    fn file(&self) -> PathBuf {
        self.dir.join(SHALLOW_FILE)
    }

    /// New content of the file when it differs from the copy; empty when it
    /// was removed.
    fn changed(&self) -> Result<Option<Vec<u8>>> {
        let after = match std::fs::read(self.file()) {
            Ok(content) => Some(content),
            Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => None,
            Err(failure) => return Err(crate::repo::error::io(&self.file(), &failure)),
        };
        Ok((after != self.before).then(|| after.unwrap_or_default()))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

impl std::fmt::Debug for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Network")
            .field("url", &super::head::anonymize(&self.url))
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_follows_configuration() {
        let config: gix::config::File = "[protocol]\n\tversion = 1\n".parse().unwrap();
        assert_eq!(protocol_version(&config), Protocol::V1);
        let config: gix::config::File = "".parse().unwrap();
        assert_eq!(protocol_version(&config), Protocol::V2);
    }

    #[test]
    fn scratch_reports_only_changed_shallow_files() {
        let dir = tempfile::tempdir().unwrap();
        let shallow = dir.path().join("shallow");
        std::fs::write(&shallow, b"a\n").unwrap();
        let scratch = Scratch::new(dir.path(), &shallow).unwrap();
        assert_eq!(scratch.changed().unwrap(), None);
        std::fs::write(scratch.file(), b"b\n").unwrap();
        assert_eq!(scratch.changed().unwrap(), Some(b"b\n".to_vec()));
        std::fs::remove_file(scratch.file()).unwrap();
        assert_eq!(scratch.changed().unwrap(), Some(Vec::new()));
        let path = scratch.dir.clone();
        drop(scratch);
        assert!(!path.exists());
    }

    #[test]
    fn watcher_ends_when_the_work_panics() {
        let ctx = crate::testutil::test_ctx();
        let interrupt = AtomicBool::new(false);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            watched(&ctx, &interrupt, || -> () { panic!("work failed") })
        }));
        assert!(outcome.is_err());
        assert!(!interrupt.load(Ordering::Relaxed));
    }
}
