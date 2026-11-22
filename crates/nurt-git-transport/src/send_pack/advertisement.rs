// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Reference advertisement of a `receive-pack` service.

use bstr::{BStr, BString, ByteSlice};
use gix::ObjectId;
use gix_transport::client::blocking_io::{ReadlineBufRead, Transport};
use gix_transport::{Protocol, Service};
use nurt_contracts::error::{Error, ErrorCode, Result};

use super::error::{protocol_error, response_too_large, transport_error};

/// Name of the placeholder line advertised by a repository without
/// references.
const EMPTY_PLACEHOLDER: &[u8] = b"capabilities^{}";

/// Largest number of reference and `.have` lines accepted in an advertisement.
const MAX_ADVERTISED_REFS: usize = 1_000_000;

/// Largest total size of the advertisement lines, in bytes.
const MAX_ADVERTISEMENT_BYTES: usize = 128 * 1024 * 1024;

/// A reference advertised by the remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvertisedRef {
    /// Full reference name, for example `refs/heads/main`.
    pub name: BString,
    /// Object the reference points to.
    pub id: ObjectId,
}

/// The references and capabilities announced by `receive-pack`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Advertisement {
    refs: Vec<AdvertisedRef>,
    capabilities: Vec<(String, Option<String>)>,
    extra_haves: Vec<ObjectId>,
}

impl Advertisement {
    /// Builds an advertisement from its parts.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_git_transport::send_pack::{AdvertisedRef, Advertisement};
    ///
    /// let advertisement = Advertisement::new(
    ///     Vec::new(),
    ///     vec![("report-status".to_owned(), None), ("agent".to_owned(), Some("git/2.53".to_owned()))],
    /// );
    /// assert!(advertisement.supports("report-status"));
    /// assert_eq!(advertisement.capability("agent"), Some("git/2.53"));
    /// ```
    #[must_use]
    pub fn new(refs: Vec<AdvertisedRef>, capabilities: Vec<(String, Option<String>)>) -> Self {
        Self {
            refs,
            capabilities,
            extra_haves: Vec::new(),
        }
    }

    /// Advertised references in the order of the advertisement.
    #[must_use]
    pub fn refs(&self) -> &[AdvertisedRef] {
        &self.refs
    }

    /// Objects the remote announced with `.have` lines (alternates).
    #[must_use]
    pub fn extra_haves(&self) -> &[ObjectId] {
        &self.extra_haves
    }

    /// Value of the reference `name` on the remote.
    #[must_use]
    pub fn find(&self, name: &BStr) -> Option<ObjectId> {
        self.refs
            .iter()
            .find(|advertised| advertised.name == name)
            .map(|advertised| advertised.id)
    }

    /// Whether the remote announced capability `name`.
    #[must_use]
    pub fn supports(&self, name: &str) -> bool {
        self.capabilities.iter().any(|(known, _)| known == name)
    }

    /// Value of capability `name`, for example the agent string.
    #[must_use]
    pub fn capability(&self, name: &str) -> Option<&str> {
        self.capabilities
            .iter()
            .find(|(known, _)| known == name)
            .and_then(|(_, value)| value.as_deref())
    }

    /// Object format of the remote, SHA-1 by default.
    ///
    /// # Errors
    ///
    /// Fails with `E_GIT_PROTOCOL` when the announced format is unknown.
    pub fn object_format(&self) -> Result<gix::hash::Kind> {
        match self.capability("object-format") {
            None | Some("sha1") => Ok(gix::hash::Kind::Sha1),
            Some("sha256") => Ok(gix::hash::Kind::Sha256),
            Some(other) => Err(protocol_error(
                "unknown_object_format",
                format!("the remote uses the unknown object format `{other}`"),
            )),
        }
    }

    fn push_line(&mut self, line: &[u8]) -> Result<()> {
        let line = line.trim_end_with(|c| c == '\n');
        let Some((id, name)) = line.split_once_str(b" ") else {
            return Err(malformed(line));
        };
        if id == b"shallow" {
            return Ok(());
        }
        let id = ObjectId::from_hex(id).map_err(|_| malformed(line))?;
        if self.refs.len() + self.extra_haves.len() >= MAX_ADVERTISED_REFS {
            return Err(response_too_large("references", MAX_ADVERTISED_REFS));
        }
        if name == EMPTY_PLACEHOLDER {
            return Ok(());
        }
        if name == b".have" {
            self.extra_haves.push(id);
            return Ok(());
        }
        self.refs.push(AdvertisedRef {
            name: name.into(),
            id,
        });
        Ok(())
    }
}

/// Reads the reference advertisement of the `receive-pack` service.
///
/// Push uses protocol version 0; version 2 has no push command.
///
/// # Errors
///
/// Fails with `E_GIT_AUTH` when the remote requires credentials,
/// `E_NETWORK` when the connection fails and `E_GIT_PROTOCOL` when the
/// advertisement is malformed.
pub fn discover(transport: &mut dyn Transport) -> Result<Advertisement> {
    let response = transport
        .handshake(Service::ReceivePack, &[])
        .map_err(|error| transport_error(&error))?;
    if response.actual_protocol == Protocol::V2 {
        return Err(protocol_error(
            "protocol_v2_push",
            "the remote answered the push request with protocol version 2",
        ));
    }
    let mut advertisement = Advertisement {
        capabilities: response
            .capabilities
            .iter()
            .map(|capability| {
                (
                    capability.name().to_str_lossy().into_owned(),
                    capability
                        .value()
                        .map(|value| value.to_str_lossy().into_owned()),
                )
            })
            .collect(),
        ..Advertisement::default()
    };
    if let Some(mut refs) = response.refs {
        read_ref_lines(refs.as_mut(), &mut advertisement)?;
    }
    Ok(advertisement)
}

fn read_ref_lines(refs: &mut dyn ReadlineBufRead, advertisement: &mut Advertisement) -> Result<()> {
    let mut line = String::new();
    let mut total = 0_usize;
    loop {
        line.clear();
        let read = refs.readline_str(&mut line).map_err(|error| {
            Error::new(
                ErrorCode::Network,
                format!("reading the reference advertisement failed: {error}"),
            )
        })?;
        if read == 0 {
            return Ok(());
        }
        total = total.saturating_add(read);
        if total > MAX_ADVERTISEMENT_BYTES {
            return Err(response_too_large(
                "bytes of reference advertisement",
                MAX_ADVERTISEMENT_BYTES,
            ));
        }
        advertisement.push_line(line.as_bytes())?;
    }
}

fn malformed(line: &[u8]) -> Error {
    protocol_error(
        "malformed_advertisement",
        format!(
            "the remote sent a malformed reference line `{}`",
            super::error::clean_remote_text(&line.to_str_lossy())
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "1111111111111111111111111111111111111111";

    #[test]
    fn parses_refs_alternates_and_placeholders() {
        let mut advertisement = Advertisement::default();
        advertisement
            .push_line(format!("{ID} refs/heads/main\n").as_bytes())
            .unwrap();
        advertisement
            .push_line(format!("{ID} .have\n").as_bytes())
            .unwrap();
        advertisement
            .push_line(format!("{ID} capabilities^{{}}").as_bytes())
            .unwrap();
        advertisement
            .push_line(format!("shallow {ID}").as_bytes())
            .unwrap();
        assert_eq!(advertisement.refs().len(), 1);
        assert_eq!(advertisement.extra_haves().len(), 1);
        assert_eq!(
            advertisement.find(b"refs/heads/main".as_bstr()),
            ObjectId::from_hex(ID.as_bytes()).ok()
        );
        assert_eq!(advertisement.find(b"refs/heads/x".as_bstr()), None);
    }

    #[test]
    fn rejects_malformed_lines() {
        let mut advertisement = Advertisement::default();
        let error = advertisement.push_line(b"nospace").unwrap_err();
        assert_eq!(error.code(), ErrorCode::GitProtocol);
        assert!(advertisement.push_line(b"zz refs/heads/main").is_err());
    }

    #[test]
    fn reports_object_format() {
        let sha256 = Advertisement::new(
            Vec::new(),
            vec![("object-format".to_owned(), Some("sha256".to_owned()))],
        );
        assert_eq!(sha256.object_format().unwrap(), gix::hash::Kind::Sha256);
        assert_eq!(
            Advertisement::default().object_format().unwrap(),
            gix::hash::Kind::Sha1
        );
        let unknown = Advertisement::new(
            Vec::new(),
            vec![("object-format".to_owned(), Some("md5".to_owned()))],
        );
        assert_eq!(
            unknown.object_format().unwrap_err().code(),
            ErrorCode::GitProtocol
        );
    }
}
