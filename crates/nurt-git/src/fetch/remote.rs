// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Resolution of the remote of a fetch: its address, refspecs, tag mode and
//! pruning, from the request and the `remote.<name>.*` configuration.

use std::num::NonZeroU32;
use std::path::Path;

use bstr::ByteSlice;
use gix::protocol::fetch::refmap::SpecIndex;
use gix::protocol::fetch::{RefMap, Shallow, Tags};
use gix::protocol::handshake::Ref;
use gix::refspec::RefSpec;
use gix::refspec::instruction::Fetch;
use nurt_contracts::error::{Error, ErrorCode, Result};

use super::{FetchRequest, TagMode, head};
use crate::repo::GitRepo;
use crate::repo::error::{quote, quote_path};

/// Refspec of the tags of a remote.
pub(crate) const TAG_SPEC: &str = "refs/tags/*:refs/tags/*";

/// Refspec of an address fetched without a refspec: its `HEAD`.
const HEAD_SPEC: &str = "HEAD";

/// Largest number of configured remotes listed in an error.
const MAX_LISTED: usize = 8;

/// The remote of a fetch with everything the transfer needs.
#[derive(Debug, Clone)]
pub(crate) struct Resolved {
    /// Name of the configured remote; `None` for an address.
    pub(crate) name: Option<String>,
    /// Address as configured or given, shown in `FETCH_HEAD`.
    pub(crate) url: String,
    /// Address used for the connection: relative local paths are resolved
    /// against the root of the repository.
    pub(crate) connect_url: String,
    /// Explicit refspecs: those of the request or of the configuration.
    pub(crate) specs: Vec<RefSpec>,
    /// Implicit refspecs: the tag refspec when tags are followed or fetched.
    pub(crate) extra_specs: Vec<RefSpec>,
    /// Tag handling of the transfer.
    pub(crate) tags: Tags,
    /// Remove stale remote-tracking references.
    pub(crate) prune: bool,
    /// The refspecs come from the request, not from the configuration.
    pub(crate) explicit: bool,
}

/// Resolves the remote of `request` in `repo`.
///
/// # Errors
///
/// `E_GIT_REF` with problem `unknown_remote` for a name that is neither a
/// configured remote nor an address, and `invalid_refspec` for a refspec
/// that cannot be parsed.
pub(crate) fn resolve(repo: &GitRepo, request: &FetchRequest) -> Result<Resolved> {
    let configured = repo.config_string(&format!("remote.{}.url", request.remote));
    let (name, url) = match configured {
        Some(url) => (Some(request.remote.clone()), url),
        None if is_address(&request.remote) => (None, request.remote.clone()),
        None => return Err(unknown_remote(repo, &request.remote)),
    };
    let explicit = !request.refspecs.is_empty();
    let texts: Vec<String> = if explicit {
        request.refspecs.clone()
    } else if let Some(name) = &name {
        configured_values(repo, &format!("remote.{name}.fetch"))
    } else {
        vec![HEAD_SPEC.to_owned()]
    };
    let specs = texts
        .iter()
        .map(|text| parse_spec(text))
        .collect::<Result<Vec<_>>>()?;
    let mode = request
        .tags
        .unwrap_or_else(|| configured_tags(repo, name.as_deref()));
    let follows = specs.iter().any(has_destination);
    let tags = match mode {
        TagMode::All => Tags::All,
        TagMode::Auto if follows => Tags::Included,
        TagMode::Auto | TagMode::None => Tags::None,
    };
    let extra_specs = match tags {
        Tags::All | Tags::Included => vec![parse_spec(TAG_SPEC)?],
        Tags::None => Vec::new(),
    };
    let prune = request
        .prune
        .unwrap_or_else(|| configured_prune(repo, name.as_deref()));
    Ok(Resolved {
        connect_url: connect_address(repo.root(), &url),
        name,
        url,
        specs,
        extra_specs,
        tags,
        prune,
        explicit,
    })
}

/// Every value of a multi-valued configuration key, in file order.
fn configured_values(repo: &GitRepo, key: &str) -> Vec<String> {
    repo.config()
        .strings(key)
        .unwrap_or_default()
        .into_iter()
        .map(|value| value.to_str_lossy().into_owned())
        .collect()
}

/// Tag mode of `remote.<name>.tagOpt`.
fn configured_tags(repo: &GitRepo, name: Option<&str>) -> TagMode {
    let option = name.and_then(|name| repo.config_string(&format!("remote.{name}.tagOpt")));
    match option.as_deref() {
        Some("--no-tags") => TagMode::None,
        Some("--tags") => TagMode::All,
        _ => TagMode::Auto,
    }
}

/// `remote.<name>.prune`, then `fetch.prune`.
fn configured_prune(repo: &GitRepo, name: Option<&str>) -> bool {
    name.and_then(|name| repo.config_bool(&format!("remote.{name}.prune")))
        .or_else(|| repo.config_bool("fetch.prune"))
        .unwrap_or(false)
}

/// Whether `text` is an address rather than the name of a remote: a URL, an
/// scp-like address or a path.
pub(crate) fn is_address(text: &str) -> bool {
    text.contains("://")
        || text.starts_with('.')
        || text.starts_with('/')
        || text.contains('\\')
        || Path::new(text).is_absolute()
        || scp_like(text)
}

/// Whether `text` has the form `[user@]host:path` with no `/` before the
/// colon.
fn scp_like(text: &str) -> bool {
    match text.find(':') {
        Some(colon) => colon > 1 && !text[..colon].contains('/'),
        None => false,
    }
}

/// Address used for the connection: a relative local path is taken relative
/// to the repository root, as `git` takes it relative to the directory it
/// runs in.
pub(crate) fn connect_address(root: &Path, url: &str) -> String {
    if url.contains("://") || scp_like(url) || Path::new(url).is_absolute() {
        return url.to_owned();
    }
    root.join(url).to_string_lossy().into_owned()
}

/// Parses a fetch refspec.
///
/// # Errors
///
/// `E_GIT_REF` with problem `invalid_refspec`.
pub(crate) fn parse_spec(text: &str) -> Result<RefSpec> {
    gix::refspec::parse(text.as_bytes().as_bstr(), gix::refspec::parse::Operation::Fetch)
        .map(|spec| spec.to_owned())
        .map_err(|failure| {
            Error::new(
                ErrorCode::GitRef,
                format!("invalid refspec `{text}`: {failure}; use <src>:<dst>, for example +refs/heads/*:refs/remotes/origin/*"),
            )
            .with_problem("invalid_refspec")
            .with_found(text.to_owned())
        })
}

/// Whether a refspec stores what it fetches in a local reference.
fn has_destination(spec: &RefSpec) -> bool {
    matches!(
        spec.to_ref().instruction(),
        gix::refspec::Instruction::Fetch(Fetch::AndUpdate { .. })
    )
}

/// Source of a refspec without a pattern, when it must match a remote
/// reference.
fn exact_source(spec: &RefSpec) -> Option<String> {
    let source = match spec.to_ref().instruction() {
        gix::refspec::Instruction::Fetch(Fetch::Only { src } | Fetch::AndUpdate { src, .. }) => src,
        _ => return None,
    };
    let text = source.to_str_lossy().into_owned();
    let is_id = gix::ObjectId::from_hex(source).is_ok();
    (!text.contains('*') && !is_id).then_some(text)
}

/// Checks that every refspec without a pattern matched a remote reference.
///
/// # Errors
///
/// `E_GIT_REF` with problem `no_match`, as `git` fails with
/// `couldn't find remote ref`.
pub(crate) fn check_mapping(ref_map: &RefMap, resolved: &Resolved) -> Result<()> {
    for (position, spec) in ref_map.refspecs.iter().enumerate() {
        let Some(source) = exact_source(spec) else {
            continue;
        };
        let matched = ref_map
            .mappings
            .iter()
            .any(|mapping| mapping.spec_index == SpecIndex::ExplicitInRemote(position));
        if !matched && (resolved.explicit || resolved.name.is_none() || source != HEAD_SPEC) {
            return Err(Error::new(
                ErrorCode::GitRef,
                format!(
                    "the remote {} has no reference `{source}`; list its references with git.branch remote: true after a fetch",
                    head::anonymize(&resolved.url)
                ),
            )
            .with_problem("no_match")
            .with_found(source));
        }
    }
    Ok(())
}

/// Shallow boundary requested by `depth`.
///
/// # Errors
///
/// `E_INVALID_VALUE` for a depth of 0.
pub(crate) fn shallow_request(repo: &GitRepo, depth: Option<u32>) -> Result<Shallow> {
    match depth {
        None => Ok(Shallow::NoChange),
        Some(depth) => NonZeroU32::new(depth)
            .map(Shallow::DepthAtRemote)
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::InvalidValue,
                    format!(
                        "depth must be at least 1; omit depth for the whole history of {}",
                        quote_path(repo.root())
                    ),
                )
                .with_problem("invalid_depth")
                .with_found("0")
            }),
    }
}

/// Error for a name that is neither a configured remote nor an address.
fn unknown_remote(repo: &GitRepo, name: &str) -> Error {
    let mut remotes: Vec<String> = repo
        .gix()
        .remote_names()
        .into_iter()
        .map(|remote| remote.to_string())
        .collect();
    remotes.truncate(MAX_LISTED);
    let mut error = Error::new(
        ErrorCode::GitRef,
        format!("`{name}` is neither a configured remote nor a repository address; pass the name of a remote or a URL"),
    )
    .with_problem("unknown_remote")
    .with_found(name.to_owned());
    for remote in &remotes {
        error = error.with_sample(remote.clone()).with_fix(format!(
            "{} {} remote: {}",
            super::COMMAND,
            quote_path(repo.root()),
            quote(remote)
        ));
    }
    error
}

/// Target of the symbolic `HEAD` advertised by the remote.
pub(crate) fn head_symref(refs: &[Ref]) -> Option<String> {
    refs.iter().find_map(|reference| match reference {
        Ref::Symbolic {
            full_ref_name,
            target,
            ..
        } if full_ref_name == "HEAD" => Some(target.to_str_lossy().into_owned()),
        _ => None,
    })
}

/// `refs/remotes/<remote>/HEAD` to create after a fetch of the configured
/// refspecs, as `git` does for `remote.<name>.followRemoteHEAD` unset or
/// `create`: the reference does not exist yet, the remote `HEAD` names a
/// branch, and the remote-tracking branch of that branch was fetched.
pub(crate) fn new_remote_head(
    repo: &GitRepo,
    resolved: &Resolved,
    target: Option<&str>,
    changes: &[super::RefChange],
) -> Option<(String, String)> {
    let name = resolved.name.as_deref().filter(|_| !resolved.explicit)?;
    let setting = repo.config_string(&format!("remote.{name}.followRemoteHEAD"));
    if !matches!(setting.as_deref(), None | Some("create")) {
        return None;
    }
    let branch = target?.strip_prefix(crate::repo::records::HEADS)?;
    let link = format!("refs/remotes/{name}/HEAD");
    let tracking = format!("refs/remotes/{name}/{branch}");
    let exists = repo
        .gix()
        .try_find_reference(link.as_str())
        .ok()
        .flatten()
        .is_some();
    let fetched = changes.iter().any(|change| change.name == tracking);
    (!exists && fetched).then_some((link, tracking))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_told_apart_from_remote_names() {
        assert!(is_address("https://example.com/r.git"));
        assert!(is_address("git@example.com:org/r.git"));
        assert!(is_address("../upstream"));
        assert!(is_address("/srv/repo.git"));
        assert!(!is_address("origin"));
        assert!(!is_address("team/mirror"));
    }

    #[test]
    fn relative_paths_are_resolved_against_the_root() {
        let root = Path::new("/work/app");
        assert_eq!(
            connect_address(root, "../upstream"),
            Path::new("/work/app").join("../upstream").to_string_lossy()
        );
        assert_eq!(
            connect_address(root, "https://example.com/r"),
            "https://example.com/r"
        );
        assert_eq!(connect_address(root, "git@host:r"), "git@host:r");
    }

    #[test]
    fn invalid_refspec_is_reported() {
        let error = parse_spec("refs/heads/*:refs/remotes/*/x/*").unwrap_err();
        assert_eq!(error.code, ErrorCode::GitRef);
        assert_eq!(error.problem.as_deref(), Some("invalid_refspec"));
    }

    #[test]
    fn exact_sources_exclude_patterns_and_ids() {
        let spec = parse_spec("refs/heads/main:refs/remotes/origin/main").unwrap();
        assert_eq!(exact_source(&spec).as_deref(), Some("refs/heads/main"));
        assert!(exact_source(&parse_spec("+refs/heads/*:refs/remotes/o/*").unwrap()).is_none());
        assert!(has_destination(&parse_spec(TAG_SPEC).unwrap()));
        assert!(!has_destination(&parse_spec(HEAD_SPEC).unwrap()));
    }
}
