// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! The authorizer of every connection.
//!
//! SQLite calls the authorizer while it prepares a statement, with each
//! action the statement performs. The authorizer refuses `ATTACH` and
//! `DETACH`, including the database that `VACUUM` attaches internally, the
//! pragmas that point SQLite to directories outside the database, the
//! function `load_extension` and, while the statements of a change are
//! prepared, statements that begin or end a transaction. Each refusal is
//! recorded, so that the failure of the statement reports its reason
//! instead of the generic `not authorized` of SQLite.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use rusqlite::Connection;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};

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
    /// A statement that starts or ends the transaction of a change.
    Transaction(&'static str),
}

/// Decisions of the authorizer of one connection.
#[derive(Debug)]
pub(super) struct Policy {
    writing: bool,
    statements: AtomicBool,
    denial: Mutex<Option<Denial>>,
}

impl Policy {
    fn new(writing: bool) -> Self {
        Self {
            writing,
            statements: AtomicBool::new(false),
            denial: Mutex::new(None),
        }
    }

    /// Installs the authorizer on `connection`; `writing` refuses
    /// transaction control in the statements of the call.
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

    /// Marks the statements of the call as being prepared until the scope
    /// ends. Statements that `sql` runs itself, such as `BEGIN IMMEDIATE`
    /// and `COMMIT`, run outside this scope.
    pub(super) fn statements(&self) -> StatementScope<'_> {
        self.statements.store(true, Ordering::Release);
        StatementScope { policy: self }
    }

    /// The last refusal, which is then cleared.
    pub(super) fn take_denial(&self) -> Option<Denial> {
        self.denial
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
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
            AuthAction::Transaction { operation }
                if self.writing && self.statements.load(Ordering::Acquire) =>
            {
                Some(Denial::Transaction(transaction_keyword(operation)))
            }
            _ => None,
        }
    }
}

/// Scope in which the statements of the call are prepared.
#[derive(Debug)]
pub(super) struct StatementScope<'p> {
    policy: &'p Policy,
}

impl Drop for StatementScope<'_> {
    fn drop(&mut self) {
        self.policy.statements.store(false, Ordering::Release);
    }
}

/// Keyword of a transaction statement. SQLite reports `BEGIN`, `COMMIT`
/// and `ROLLBACK`; `rusqlite` names only `BEGIN` and `ROLLBACK`, so any
/// other operation is a `COMMIT` (also written `END`).
fn transaction_keyword(operation: &TransactionOperation) -> &'static str {
    match operation {
        TransactionOperation::Begin => "BEGIN",
        TransactionOperation::Rollback => "ROLLBACK",
        _ => "COMMIT",
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
    fn refuses_transaction_control_in_statements_of_a_change() {
        let writing = Policy::new(true);
        let begin = AuthAction::Transaction {
            operation: TransactionOperation::Begin,
        };
        assert_eq!(decide(&writing, begin), (Authorization::Allow, None));
        {
            let _scope = writing.statements();
            assert_eq!(
                decide(&writing, begin),
                (Authorization::Deny, Some(Denial::Transaction("BEGIN")))
            );
            let commit = AuthAction::Transaction {
                operation: TransactionOperation::Unknown,
            };
            assert_eq!(
                decide(&writing, commit),
                (Authorization::Deny, Some(Denial::Transaction("COMMIT")))
            );
            let savepoint = AuthAction::Savepoint {
                operation: TransactionOperation::Rollback,
                savepoint_name: "s1",
            };
            assert_eq!(decide(&writing, savepoint), (Authorization::Allow, None));
        }
        assert_eq!(decide(&writing, begin), (Authorization::Allow, None));
        let reading = Policy::new(false);
        let _scope = reading.statements();
        assert_eq!(decide(&reading, begin), (Authorization::Allow, None));
    }

    #[test]
    fn names_rollback_statements() {
        assert_eq!(
            transaction_keyword(&TransactionOperation::Rollback),
            "ROLLBACK"
        );
    }
}
