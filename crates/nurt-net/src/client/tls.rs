// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Trusted certificates and TLS settings.

use std::path::Path;

use nurt_contracts::error::{Error, ErrorCode, Result};
use ureq::tls::{Certificate, PemItem, RootCerts, TlsConfig};

/// Root certificates: the certificate store of the operating system and the
/// certificates of `ca_file`.
///
/// When neither gives a certificate, the roots compiled into the TLS library
/// are used.
pub(crate) fn load_roots(ca_file: Option<&Path>) -> Result<RootCerts> {
    let mut certs: Vec<Certificate<'static>> = rustls_native_certs::load_native_certs()
        .certs
        .iter()
        .map(|der| Certificate::from_der(der.as_ref()).to_owned())
        .collect();
    if let Some(path) = ca_file {
        certs.extend(read_ca_file(path)?);
    }
    if certs.is_empty() {
        return Ok(RootCerts::WebPki);
    }
    Ok(RootCerts::new_with_certs(&certs))
}

fn read_ca_file(path: &Path) -> Result<Vec<Certificate<'static>>> {
    let invalid = |reason: String| {
        Error::new(
            ErrorCode::Config,
            format!(
                "the certificate file `{}` cannot be used: {reason}",
                path.display()
            ),
        )
    };
    let pem = std::fs::read(path).map_err(|error| invalid(error.to_string()))?;
    let mut certs = Vec::new();
    for item in ureq::tls::parse_pem(&pem) {
        if let PemItem::Certificate(cert) = item.map_err(|error| invalid(error.to_string()))? {
            certs.push(cert);
        }
    }
    if certs.is_empty() {
        return Err(invalid("it holds no PEM certificate".to_owned()));
    }
    Ok(certs)
}

/// TLS settings of one connection.
pub(crate) fn tls_config(roots: &RootCerts, verify: bool) -> TlsConfig {
    TlsConfig::builder()
        .root_certs(roots.clone())
        .disable_verification(!verify)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_missing_and_empty_certificate_files() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.pem");
        assert_eq!(
            load_roots(Some(&missing)).unwrap_err().code(),
            ErrorCode::Config
        );
        let empty = dir.path().join("empty.pem");
        std::fs::write(&empty, "not a certificate\n").unwrap();
        assert_eq!(
            load_roots(Some(&empty)).unwrap_err().code(),
            ErrorCode::Config
        );
    }
}
