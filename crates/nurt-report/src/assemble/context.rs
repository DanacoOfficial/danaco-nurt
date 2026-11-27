// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-report::assemble::context
// Responsibility:   Configuration and host facts used to assemble reports.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Configuration and host facts used to assemble reports.

use nurt_contracts::config::{ReportConfig, ScopeConfig};

use crate::rules::PathContext;
use crate::rules::paths::normalize_root;

/// Configuration of the report rules: the `report.*` keys, the project roots
/// of `scope.project_roots` and the host facts used to classify paths.
///
/// A context is built once per engine and shared by every report it
/// assembles.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReportContext {
    config: ReportConfig,
    project_roots: Vec<String>,
    paths: PathContext,
}

impl ReportContext {
    /// A context with the given `report.*` configuration and host facts and
    /// no project roots.
    #[must_use]
    pub fn new(config: ReportConfig, paths: PathContext) -> Self {
        Self {
            config,
            project_roots: Vec::new(),
            paths,
        }
    }

    /// Sets the project roots; targets outside every root are reported as
    /// `outside_project`. Roots are converted to `/` separators and empty
    /// roots are dropped.
    #[must_use]
    pub fn with_project_roots<I, S>(mut self, roots: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.project_roots = roots
            .into_iter()
            .filter_map(|root| normalize_root(root.as_ref()))
            .collect();
        self
    }

    /// Sets the project roots from the `[scope]` section.
    #[must_use]
    pub fn with_scope(self, scope: &ScopeConfig) -> Self {
        self.with_project_roots(&scope.project_roots)
    }

    /// The `report.*` configuration.
    #[must_use]
    pub fn config(&self) -> &ReportConfig {
        &self.config
    }

    /// Project roots in canonical form.
    #[must_use]
    pub fn project_roots(&self) -> &[String] {
        &self.project_roots
    }

    /// Host facts used to classify paths.
    #[must_use]
    pub fn paths(&self) -> &PathContext {
        &self.paths
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
