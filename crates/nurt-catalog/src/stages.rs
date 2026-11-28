// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-catalog::stages
// Responsibility:   Name of the record kind list entry that stands for every kind.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

use std::sync::Arc;

use nurt_contracts::error::Result;
use nurt_contracts::spec::{CatalogIssue, IoSpec};
use nurt_contracts::stage::{InputKinds, StageDescriptor, StageImpl, StageRegistrar};
use nurt_contracts::value::RecordKind;

use crate::error::{invalid, not_implemented, unsupported_input};
use crate::registry::Registry;

/// Name of the record kind list entry that stands for every kind.
const ANY_KIND: &str = "any";

/// An implementation with the descriptor it was registered under.
#[derive(Clone)]
pub(crate) struct RegisteredImpl {
    descriptor: StageDescriptor,
    implementation: Arc<dyn StageImpl>,
}

/// Input record kinds that the implementations of one command accept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedKinds {
    /// Record kinds accepted in the stage form; a single `any` entry when one
    /// implementation accepts every kind.
    pub kinds: Vec<String>,
    /// An implementation can start a pipeline.
    pub source: bool,
}

/// Returns `true` when two descriptors of one command cannot be told apart by
/// the registry: both are sources, or they accept a common input record kind.
///
/// Descriptors of different commands never overlap.
///
/// # Examples
///
/// ```
/// use nurt_catalog::descriptors_overlap;
/// use nurt_contracts::stage::{InputKinds, StageDescriptor};
///
/// let source = StageDescriptor {
///     command: "find".to_owned(),
///     source: true,
///     input: InputKinds::None,
/// };
/// assert!(descriptors_overlap(&source, &source.clone()));
/// ```
#[must_use]
pub fn descriptors_overlap(left: &StageDescriptor, right: &StageDescriptor) -> bool {
    left.command == right.command && overlaps(left, right)
}

fn overlaps(left: &StageDescriptor, right: &StageDescriptor) -> bool {
    if left.source && right.source {
        return true;
    }
    match (&left.input, &right.input) {
        (InputKinds::None, _) | (_, InputKinds::None) => false,
        (InputKinds::Any, _) | (_, InputKinds::Any) => true,
        (InputKinds::Only(own), InputKinds::Only(other)) => {
            own.iter().any(|kind| other.contains(kind))
        }
        _ => false,
    }
}

impl Registry {
    /// Registers a command implementation.
    ///
    /// The command and the accepted input kinds come from
    /// [`StageImpl::descriptor`]. One command can have several
    /// implementations when their input kinds are disjoint. The command does
    /// not have to be in the catalog yet; [`Registry::validate_implementations`]
    /// reports implementations without a command.
    ///
    /// The method is the registration step of the stage registry. An
    /// implementation of the registration trait of the stage contract
    /// delegates to it.
    ///
    /// # Errors
    ///
    /// Returns `E_CATALOG` when an implementation of the command already
    /// accepts one of the input kinds, or already is a source.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::Arc;
    /// use nurt_catalog::Registry;
    /// use nurt_contracts::stage::{InputKinds, StageDescriptor, StageImpl};
    /// use nurt_contracts::value::RecordKind;
    ///
    /// struct Count;
    ///
    /// impl StageImpl for Count {
    ///     fn descriptor(&self) -> StageDescriptor {
    ///         StageDescriptor {
    ///             command: "count".to_owned(),
    ///             source: false,
    ///             input: InputKinds::Any,
    ///         }
    ///     }
    /// }
    ///
    /// let mut registry = Registry::load_embedded()?;
    /// registry.register_impl(Arc::new(Count))?;
    /// assert!(registry.resolve("count", RecordKind::Entry).is_ok());
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn register_impl(&mut self, implementation: Arc<dyn StageImpl>) -> Result<()> {
        let descriptor = implementation.descriptor();
        let clash = self.impls.iter().any(|known| {
            known.descriptor.command == descriptor.command
                && overlaps(&known.descriptor, &descriptor)
        });
        if clash {
            let issue = CatalogIssue::new(
                Some(descriptor.command.clone()),
                None,
                "another implementation accepts the same input",
            );
            return Err(invalid("implementation", &[issue]));
        }
        self.impls.push(RegisteredImpl {
            descriptor,
            implementation,
        });
        Ok(())
    }

    /// Returns the descriptors of the implementations of a command, in
    /// registration order.
    pub fn implementations<'a>(
        &'a self,
        command: &'a str,
    ) -> impl Iterator<Item = &'a StageDescriptor> {
        self.impls
            .iter()
            .filter(move |known| known.descriptor.command == command)
            .map(|known| &known.descriptor)
    }

    /// Returns the input kinds that the implementations of a command accept.
    #[must_use]
    pub fn accepted_kinds(&self, command: &str) -> AcceptedKinds {
        let mut kinds: Vec<String> = Vec::new();
        let mut source = false;
        let mut any = false;
        for descriptor in self.implementations(command) {
            source |= descriptor.source;
            match &descriptor.input {
                InputKinds::Any => any = true,
                InputKinds::Only(list) => {
                    for kind in list {
                        if !kinds.iter().any(|known| known == kind.as_str()) {
                            kinds.push(kind.as_str().to_owned());
                        }
                    }
                }
                _ => {}
            }
        }
        if any {
            kinds = vec![ANY_KIND.to_owned()];
        }
        AcceptedKinds { kinds, source }
    }

    /// Selects the implementation of a command for an input record kind.
    ///
    /// # Errors
    ///
    /// Returns `E_UNKNOWN_COMMAND` when the catalog has no such command,
    /// `E_TYPE` with the accepted kinds in `expected` when no implementation
    /// accepts the kind, and `E_INTERNAL` when the command has no
    /// implementation at all.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_catalog::Registry;
    /// use nurt_contracts::error::ErrorCode;
    /// use nurt_contracts::value::RecordKind;
    ///
    /// let registry = Registry::load_embedded()?;
    /// let error = registry.resolve("remvoe", RecordKind::Entry).err().expect("unknown");
    /// assert_eq!(error.code(), ErrorCode::UnknownCommand);
    /// # Ok::<(), nurt_contracts::error::Error>(())
    /// ```
    pub fn resolve(&self, command: &str, kind: RecordKind) -> Result<Arc<dyn StageImpl>> {
        self.require_command(command)?;
        let found = self.impls.iter().find(|known| {
            known.descriptor.command == command && known.descriptor.input.accepts(kind)
        });
        match found {
            Some(known) => Ok(Arc::clone(&known.implementation)),
            None => Err(self.mismatch(command, Some(kind.as_str()))),
        }
    }

    /// Selects the implementation of a command that starts a pipeline.
    ///
    /// # Errors
    ///
    /// Returns `E_UNKNOWN_COMMAND` when the catalog has no such command,
    /// `E_TYPE` when no implementation is a source, and `E_INTERNAL` when the
    /// command has no implementation at all.
    pub fn resolve_source(&self, command: &str) -> Result<Arc<dyn StageImpl>> {
        self.require_command(command)?;
        let found = self
            .impls
            .iter()
            .find(|known| known.descriptor.command == command && known.descriptor.source);
        match found {
            Some(known) => Ok(Arc::clone(&known.implementation)),
            None => Err(self.mismatch(command, None)),
        }
    }

    fn mismatch(&self, command: &str, found: Option<&str>) -> nurt_contracts::error::Error {
        if self.implementations(command).next().is_none() {
            return not_implemented(command);
        }
        let accepted = self.accepted_kinds(command);
        unsupported_input(command, found, &accepted.kinds)
    }

    /// Checks the registered implementations against the catalog.
    ///
    /// Every implementation must belong to a command of the catalog, a source
    /// implementation needs a catalog input `none`, and the accepted input
    /// kinds must be among the catalog input kinds. The list is empty when
    /// all implementations are consistent.
    #[must_use]
    pub fn validate_implementations(&self) -> Vec<CatalogIssue> {
        let mut issues = Vec::new();
        for known in &self.impls {
            let descriptor = &known.descriptor;
            let issue = |message: String| {
                CatalogIssue::new(Some(descriptor.command.clone()), None, message)
            };
            let Some(spec) = self.command(&descriptor.command) else {
                issues.push(issue(
                    "implementation has no command in the catalog".to_owned(),
                ));
                continue;
            };
            if descriptor.source && !spec.input.contains(&IoSpec::None) {
                issues.push(issue(
                    "source implementation, but the catalog input has no `none`".to_owned(),
                ));
            }
            let catalog_any = spec.input.contains(&IoSpec::Any);
            match &descriptor.input {
                InputKinds::None if !descriptor.source => {
                    issues.push(issue(
                        "implementation accepts no input and is not a source".to_owned(),
                    ));
                }
                InputKinds::Any if !catalog_any => {
                    issues.push(issue(
                        "implementation accepts any kind, the catalog input does not".to_owned(),
                    ));
                }
                InputKinds::Only(kinds) => {
                    for kind in kinds {
                        let listed = spec
                            .input
                            .contains(&IoSpec::Record(kind.as_str().to_owned()));
                        if !catalog_any && !listed {
                            issues.push(issue(format!(
                                "input kind `{}` is not in the catalog input",
                                kind.as_str()
                            )));
                        }
                    }
                }
                _ => {}
            }
        }
        issues
    }

    /// Returns the names of the catalog commands that have no implementation,
    /// in catalog order.
    #[must_use]
    pub fn unimplemented_commands(&self) -> Vec<&str> {
        self.commands()
            .map(|spec| spec.name.as_str())
            .filter(|name| self.implementations(name).next().is_none())
            .collect()
    }
}

/// Registers the implementation as [`Registry::register_impl`] does, so a
/// command crate can add its stages through `register(&mut registry)`.
impl StageRegistrar for Registry {
    fn register(&mut self, stage: Arc<dyn StageImpl>) -> Result<()> {
        self.register_impl(stage)
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
