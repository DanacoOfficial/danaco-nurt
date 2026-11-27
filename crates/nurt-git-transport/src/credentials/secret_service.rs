// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-transport::credentials::secret_service
// Responsibility:   Credentials in the Secret Service of the desktop session, stored with the.
// Dependencies:     zbus
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Credentials in the Secret Service of the desktop session, stored with the
//! attributes used by `git-credential-libsecret`: schema `org.git.Password`,
//! `protocol`, `server`, `port`, `object` (the path) and `user`.
//!
//! The session is opened with the `plain` algorithm over the private session
//! bus. A locked collection needs an unlock prompt of the desktop, which is
//! never shown: the lookup reports the store as locked instead.

use std::collections::HashMap;

use zbus::wire::{OwnedObjectPath, OwnedVariant, Variant};
use zbus::{Connection, Proxy};

use super::Query;

const DESTINATION: &str = "org.freedesktop.secrets";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SERVICE_INTERFACE: &str = "org.freedesktop.Secret.Service";
const ITEM_INTERFACE: &str = "org.freedesktop.Secret.Item";
const COLLECTION_INTERFACE: &str = "org.freedesktop.Secret.Collection";
const DEFAULT_COLLECTION: &str = "/org/freedesktop/secrets/aliases/default";
const SCHEMA: &str = "org.git.Password";
const CONTENT_TYPE: &str = "text/plain; charset=utf8";
/// Object path meaning "no prompt needed".
const NO_PROMPT: &str = "/";

/// Secret structure of the protocol: session, parameters, value, content type.
type SecretValue = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

/// Result of a lookup.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Lookup {
    /// A matching unlocked item.
    Found { username: String, password: String },
    /// No matching item.
    NotFound,
    /// Matching items exist only in locked collections.
    Locked,
}

/// Attributes identifying the credentials of `query`; `user` is included
/// when known.
pub(crate) fn attributes(query: &Query, user: Option<&str>) -> HashMap<String, String> {
    let mut attributes = HashMap::from([
        ("xdg:schema".to_owned(), SCHEMA.to_owned()),
        ("protocol".to_owned(), query.protocol.clone()),
        ("server".to_owned(), query.host_name().to_owned()),
    ]);
    if let Some(port) = query.port() {
        attributes.insert("port".to_owned(), port.to_string());
    }
    if let Some(path) = &query.path {
        attributes.insert("object".to_owned(), path.clone());
    }
    if let Some(user) = user {
        attributes.insert("user".to_owned(), user.to_owned());
    }
    attributes
}

async fn open(connection: &Connection) -> zbus::Result<(Proxy<'_>, OwnedObjectPath)> {
    let service = Proxy::new(connection, DESTINATION, SERVICE_PATH, SERVICE_INTERFACE).await?;
    let (_, session): (OwnedVariant, OwnedObjectPath) = service
        .call("OpenSession", &("plain", Variant::from("")))
        .await?;
    Ok((service, session))
}

async fn search(
    service: &Proxy<'_>,
    attributes: &HashMap<String, String>,
) -> zbus::Result<(Vec<OwnedObjectPath>, Vec<OwnedObjectPath>)> {
    service.call("SearchItems", attributes).await
}

async fn get_async(query: &Query) -> zbus::Result<Lookup> {
    let connection = Connection::session().await?;
    let (service, session) = open(&connection).await?;
    let (unlocked, locked) =
        search(&service, &attributes(query, query.username.as_deref())).await?;
    let Some(item_path) = unlocked.into_iter().next() else {
        return Ok(if locked.is_empty() {
            Lookup::NotFound
        } else {
            Lookup::Locked
        });
    };
    let item = Proxy::new(&connection, DESTINATION, item_path, ITEM_INTERFACE).await?;
    let stored: HashMap<String, String> = item.get_property("Attributes").await?;
    let (_, _, value, _): SecretValue = item.call("GetSecret", &session).await?;
    let username = match stored.get("user") {
        Some(user) => user.clone(),
        None => return Ok(Lookup::NotFound),
    };
    Ok(Lookup::Found {
        username,
        password: String::from_utf8_lossy(&value).into_owned(),
    })
}

async fn store_async(query: &Query, username: &str, password: &str) -> zbus::Result<bool> {
    let connection = Connection::session().await?;
    let (_, session) = open(&connection).await?;
    let collection = Proxy::new(
        &connection,
        DESTINATION,
        DEFAULT_COLLECTION,
        COLLECTION_INTERFACE,
    )
    .await?;
    let label = format!(
        "Git: {}://{}/{}",
        query.protocol,
        query.host,
        query.path.as_deref().unwrap_or_default()
    );
    let properties: HashMap<&str, Variant<'_>> = HashMap::from([
        (
            "org.freedesktop.Secret.Item.Label",
            Variant::from(label.as_str()),
        ),
        (
            "org.freedesktop.Secret.Item.Attributes",
            Variant::from(attributes(query, Some(username))),
        ),
    ]);
    let secret: SecretValue = (
        session,
        Vec::new(),
        password.as_bytes().to_vec(),
        CONTENT_TYPE.to_owned(),
    );
    let (_, prompt): (OwnedObjectPath, OwnedObjectPath) = collection
        .call("CreateItem", &(properties, secret, true))
        .await?;
    Ok(prompt.as_str() == NO_PROMPT)
}

async fn erase_async(query: &Query, username: &str) -> zbus::Result<()> {
    let connection = Connection::session().await?;
    let (service, _) = open(&connection).await?;
    let (unlocked, _) = search(&service, &attributes(query, Some(username))).await?;
    for path in unlocked {
        let item = Proxy::new(&connection, DESTINATION, path, ITEM_INTERFACE).await?;
        let _prompt: OwnedObjectPath = item.call("Delete", &()).await?;
    }
    Ok(())
}

/// Looks up the credentials of `query`.
pub(crate) fn get(query: &Query) -> Result<Lookup, String> {
    zbus::block_on(get_async(query)).map_err(|e| e.to_string())
}

/// Stores credentials for `query`; `Ok(false)` when the collection is locked.
pub(crate) fn store(query: &Query, username: &str, password: &str) -> Result<bool, String> {
    zbus::block_on(store_async(query, username, password)).map_err(|e| e.to_string())
}

/// Deletes the unlocked items of `query` and `username`.
pub(crate) fn erase(query: &Query, username: &str) -> Result<(), String> {
    zbus::block_on(erase_async(query, username)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_attributes_of_the_libsecret_helper() {
        let query = Query {
            protocol: "https".to_owned(),
            host: "example.com:8443".to_owned(),
            path: Some("org/r.git".to_owned()),
            username: None,
        };
        let attributes = attributes(&query, Some("bob"));
        assert_eq!(
            attributes.get("xdg:schema").map(String::as_str),
            Some(SCHEMA)
        );
        assert_eq!(
            attributes.get("server").map(String::as_str),
            Some("example.com")
        );
        assert_eq!(attributes.get("port").map(String::as_str), Some("8443"));
        assert_eq!(
            attributes.get("object").map(String::as_str),
            Some("org/r.git")
        );
        assert_eq!(attributes.get("user").map(String::as_str), Some("bob"));
        let without_user = super::attributes(
            &Query {
                path: None,
                ..query
            },
            None,
        );
        assert!(!without_user.contains_key("user") && !without_user.contains_key("object"));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
