// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The authorizer of every connection.
//!
//! SQLite calls the authorizer while it prepares a statement, with each
//! action the statement performs. A connection opened for reading refuses
//! `ATTACH` and `DETACH`, the pragmas that point SQLite to directories
//! outside the database and the function `load_extension`; each refusal is
//! recorded, so that the failure of the statement reports its reason instead
//! of the generic `not authorized` of SQLite. A connection opened for
//! writing allows all of them: the call changes files, and the journal
//! preserves them before the change.

use std::sync::{Arc, Mutex, PoisonError};

use rusqlite::Connection;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};

/// Pragmas that make SQLite create or use files outside the database
/// directory.
const DENIED_PRAGMAS: &[&str] = &["temp_store_directory", "data_store_directory"];

/// SQL function that loads a shared library into the connection.
const EXTENSION_FUNCTION: &str = "load_extension";

/// Why a statement is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Denial {
    /// `ATTACH` or `DETACH`.
    Attach(&'static str),
    /// A pragma from [`DENIED_PRAGMAS`].
    Pragma(String),
    /// A call of `load_extension`.
    Extension,
}

/// Decisions of the authorizer of one connection.
#[derive(Debug)]
pub(super) struct Policy {
    writing: bool,
    denial: Mutex<Option<Denial>>,
    effects: Mutex<Effects>,
}

/// Actions that a connection opened for writing performed and whose effect
/// the journal cannot undo.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Effects {
    /// A statement called `load_extension`.
    pub(super) extension: bool,
    /// Pragmas that set a value of the whole process, by name.
    pub(super) process_pragmas: Vec<String>,
}

impl Policy {
    fn new(writing: bool) -> Self {
        Self {
            writing,
            denial: Mutex::new(None),
            effects: Mutex::new(Effects::default()),
        }
    }

    /// Installs the authorizer on `connection`; `writing` allows every
    /// action.
    pub(super) fn install(
        connection: &mut Connection,
        writing: bool,
    ) -> rusqlite::Result<Arc<Self>> {
        let policy = Arc::new(Self::new(writing));
        let hook = Arc::clone(&policy);
        connection.authorizer(Some(move |context: AuthContext<'_>| {
            hook.authorize(&context.action)
        }))?;
        Ok(policy)
    }

    /// The last refusal, which is then cleared.
    pub(super) fn take_denial(&self) -> Option<Denial> {
        self.denial
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// Effects recorded so far.
    pub(super) fn effects(&self) -> Effects {
        self.effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, action: &AuthAction<'_>) {
        let mut effects = self.effects.lock().unwrap_or_else(PoisonError::into_inner);
        match action {
            AuthAction::Function { function_name }
                if function_name.eq_ignore_ascii_case(EXTENSION_FUNCTION) =>
            {
                effects.extension = true;
            }
            AuthAction::Pragma {
                pragma_name,
                pragma_value: Some(_),
            } => {
                if let Some(denied) = DENIED_PRAGMAS
                    .iter()
                    .find(|denied| pragma_name.eq_ignore_ascii_case(denied))
                    && !effects.process_pragmas.iter().any(|seen| seen == denied)
                {
                    effects.process_pragmas.push((*denied).to_owned());
                }
            }
            _ => {}
        }
    }

    fn authorize(&self, action: &AuthAction<'_>) -> Authorization {
        match self.denial_of(action) {
            Some(denial) => {
                *self.denial.lock().unwrap_or_else(PoisonError::into_inner) = Some(denial);
                Authorization::Deny
            }
            None => Authorization::Allow,
        }
    }

    fn denial_of(&self, action: &AuthAction<'_>) -> Option<Denial> {
        if self.writing {
            self.record(action);
            return None;
        }
        match action {
            AuthAction::Attach { .. } => Some(Denial::Attach("ATTACH")),
            AuthAction::Detach { .. } => Some(Denial::Attach("DETACH")),
            AuthAction::Pragma { pragma_name, .. } => DENIED_PRAGMAS
                .iter()
                .find(|denied| pragma_name.eq_ignore_ascii_case(denied))
                .map(|denied| Denial::Pragma((*denied).to_owned())),
            AuthAction::Function { function_name }
                if function_name.eq_ignore_ascii_case(EXTENSION_FUNCTION) =>
            {
                Some(Denial::Extension)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decide(policy: &Policy, action: AuthAction<'_>) -> (Authorization, Option<Denial>) {
        (policy.authorize(&action), policy.take_denial())
    }

    #[test]
    fn refuses_attach_and_detach() {
        let policy = Policy::new(false);
        assert_eq!(
            decide(
                &policy,
                AuthAction::Attach {
                    filename: "other.db"
                }
            ),
            (Authorization::Deny, Some(Denial::Attach("ATTACH")))
        );
        assert_eq!(
            decide(
                &policy,
                AuthAction::Detach {
                    database_name: "other"
                }
            ),
            (Authorization::Deny, Some(Denial::Attach("DETACH")))
        );
    }

    #[test]
    fn refuses_directory_pragmas_in_any_case() {
        let policy = Policy::new(false);
        let action = AuthAction::Pragma {
            pragma_name: "TEMP_STORE_DIRECTORY",
            pragma_value: Some("/tmp"),
        };
        assert_eq!(
            decide(&policy, action),
            (
                Authorization::Deny,
                Some(Denial::Pragma("temp_store_directory".to_owned()))
            )
        );
        let allowed = AuthAction::Pragma {
            pragma_name: "journal_mode",
            pragma_value: Some("wal"),
        };
        assert_eq!(decide(&policy, allowed), (Authorization::Allow, None));
    }

    #[test]
    fn refuses_extension_loading_only() {
        let policy = Policy::new(false);
        let load = AuthAction::Function {
            function_name: "load_extension",
        };
        assert_eq!(
            decide(&policy, load),
            (Authorization::Deny, Some(Denial::Extension))
        );
        let upper = AuthAction::Function {
            function_name: "upper",
        };
        assert_eq!(decide(&policy, upper), (Authorization::Allow, None));
    }

    #[test]
    fn records_extension_calls_and_process_pragmas_while_writing() {
        let policy = Policy::new(true);
        let quoted_in_another_case = AuthAction::Function {
            function_name: "Load_Extension",
        };
        assert_eq!(
            decide(&policy, quoted_in_another_case).0,
            Authorization::Allow
        );
        let pragma = AuthAction::Pragma {
            pragma_name: "TEMP_STORE_DIRECTORY",
            pragma_value: Some("/tmp"),
        };
        decide(&policy, pragma);
        let read = AuthAction::Pragma {
            pragma_name: "data_store_directory",
            pragma_value: None,
        };
        decide(&policy, read);
        let effects = policy.effects();
        assert!(effects.extension);
        assert_eq!(effects.process_pragmas, ["temp_store_directory"]);
    }

    #[test]
    fn allows_every_action_while_writing() {
        let policy = Policy::new(true);
        let load = AuthAction::Function {
            function_name: "load_extension",
        };
        assert_eq!(decide(&policy, load), (Authorization::Allow, None));
        let attach = AuthAction::Attach {
            filename: "other.db",
        };
        assert_eq!(decide(&policy, attach), (Authorization::Allow, None));
        let pragma = AuthAction::Pragma {
            pragma_name: "temp_store_directory",
            pragma_value: Some("/tmp"),
        };
        assert_eq!(decide(&policy, pragma), (Authorization::Allow, None));
    }
}
