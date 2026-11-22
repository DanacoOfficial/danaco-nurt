// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Persistent environment variables, read through the `registry:` provider.

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{FsError, FsErrorKind};
use nurt_contracts::record::{EnvironmentScope, RegistryType};
use nurt_contracts::value::Value;

use super::persistent_location;
use crate::registry::path::Address;
use crate::registry::{RegistryItem, RegistryProvider};

/// Text variables of `scope` in the order of the registry; values of other
/// types are skipped and `REG_EXPAND_SZ` values are not expanded. A missing
/// key yields no variables.
///
/// # Errors
///
/// Fails with `E_PERMISSION` when the key may not be read and `E_IO` for
/// other errors of the registry.
pub(crate) fn read_persistent(
    provider: &RegistryProvider,
    scope: EnvironmentScope,
) -> Result<Vec<(String, String)>> {
    let Some(location) = persistent_location(scope) else {
        return Ok(Vec::new());
    };
    let Ok(Address::Key(key)) = Address::from_provider_key(&location) else {
        return Ok(Vec::new());
    };
    let items = match provider.list(&key) {
        Ok(items) => items,
        Err(error) if error.kind == FsErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(registry_error(&error, scope)),
    };
    Ok(items.into_iter().filter_map(text_variable).collect())
}

/// Name and text of a `REG_SZ` or `REG_EXPAND_SZ` value; `None` for keys and
/// values of other types.
fn text_variable(item: RegistryItem) -> Option<(String, String)> {
    let RegistryItem::Value(value) = item else {
        return None;
    };
    match (value.kind, value.value) {
        (RegistryType::String | RegistryType::ExpandString, Value::Text(text)) => {
            Some((value.name, text))
        }
        _ => None,
    }
}

fn registry_error(error: &FsError, scope: EnvironmentScope) -> Error {
    let code = match error.kind {
        FsErrorKind::PermissionDenied | FsErrorKind::Privilege => ErrorCode::Permission,
        _ => ErrorCode::Io,
    };
    let failure = Error::new(
        code,
        format!(
            "the persistent `{scope}` variables could not be read ({})",
            error.kind.as_str()
        ),
    );
    match error.os_code {
        Some(os_code) => failure.with_detail("os_error", serde_json::json!(os_code)),
        None => failure,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nurt_contracts::record::{EnvironmentScope, RegistryView};

    use super::read_persistent;
    use crate::environment::persistent_location;
    use crate::registry::RegistryProvider;
    use crate::registry::api::RegistryApi;
    use crate::registry::mem::MemRegistry;
    use crate::registry::path::{Address, KeyPath};
    use crate::registry::value::{REG_EXPAND_SZ, RawValue};

    const VIEW: RegistryView = RegistryView::Bits64;

    fn key(scope: EnvironmentScope) -> KeyPath {
        let location = persistent_location(scope).unwrap();
        Address::from_provider_key(&location).unwrap().key().clone()
    }

    /// Creates `key` with every missing key above it.
    fn create(registry: &MemRegistry, key: &KeyPath) {
        for depth in 1..=key.keys().len() {
            registry
                .create_key(VIEW, &key.prefix(depth), "", false)
                .unwrap();
        }
    }

    fn expandable(text: &str) -> RawValue {
        let bytes = text
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        RawValue::new(REG_EXPAND_SZ, bytes)
    }

    fn provider(registry: MemRegistry) -> RegistryProvider {
        RegistryProvider::new(Arc::new(registry))
    }

    #[test]
    fn lists_text_values_in_registry_order_without_expanding() {
        let registry = MemRegistry::new();
        let user = key(EnvironmentScope::User);
        create(&registry, &user);
        registry
            .set_value(VIEW, &user, "TOOLS", &RawValue::string(r"C:\Tools"))
            .unwrap();
        registry
            .set_value(VIEW, &user, "COUNT", &RawValue::dword(1))
            .unwrap();
        registry
            .set_value(VIEW, &user, "TEMP", &expandable(r"%USERPROFILE%\Temp"))
            .unwrap();
        registry
            .set_value(VIEW, &user, "LIST", &RawValue::multi_string(&["a", "b"]))
            .unwrap();
        create(&registry, &user.child("Volatile"));

        let variables = read_persistent(&provider(registry), EnvironmentScope::User).unwrap();
        assert_eq!(
            variables,
            [
                ("TOOLS".to_owned(), r"C:\Tools".to_owned()),
                ("TEMP".to_owned(), r"%USERPROFILE%\Temp".to_owned()),
            ]
        );
    }

    #[test]
    fn missing_key_and_process_scope_yield_no_variables() {
        let provider = provider(MemRegistry::new());
        assert!(
            read_persistent(&provider, EnvironmentScope::Machine)
                .unwrap()
                .is_empty()
        );
        assert!(
            read_persistent(&provider, EnvironmentScope::Process)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn denied_key_is_permission_error() {
        let registry = MemRegistry::new();
        let machine = key(EnvironmentScope::Machine);
        create(&registry, &machine);
        registry.deny(VIEW, &machine).unwrap();

        let error = read_persistent(&provider(registry), EnvironmentScope::Machine).unwrap_err();
        assert_eq!(error.code(), nurt_contracts::error::ErrorCode::Permission);
    }
}
