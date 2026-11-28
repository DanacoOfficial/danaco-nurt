// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-types::pattern
// Responsibility:   Compilation of regular expressions with a bounded cache.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Compilation of regular expressions with a bounded cache.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::value::Regex;

/// Compiled expressions kept by one evaluator; the cache is emptied when it
/// reaches this size.
const CACHE_CAPACITY: usize = 256;

/// Cache of compiled regular expressions, shared by the threads that use
/// one evaluator.
#[derive(Debug, Default)]
pub(crate) struct RegexCache(Mutex<HashMap<Regex, Arc<regex::Regex>>>);

impl RegexCache {
    /// The compiled form of `regex`.
    pub(crate) fn compile(&self, regex: &Regex) -> Result<Arc<regex::Regex>> {
        if let Some(compiled) = self.lock().get(regex) {
            return Ok(Arc::clone(compiled));
        }
        let compiled = Arc::new(build(regex)?);
        let mut cache = self.lock();
        if cache.len() >= CACHE_CAPACITY {
            cache.clear();
        }
        cache.insert(regex.clone(), Arc::clone(&compiled));
        Ok(compiled)
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<Regex, Arc<regex::Regex>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Compiles `regex` in the dialect of the `regex` library with its flags.
pub(crate) fn build(regex: &Regex) -> Result<regex::Regex> {
    regex::RegexBuilder::new(&regex.pattern)
        .case_insensitive(regex.flags.case_insensitive)
        .multi_line(regex.flags.multi_line)
        .dot_matches_new_line(regex.flags.dot_matches_new_line)
        .ignore_whitespace(regex.flags.extended)
        .build()
        .map_err(|error| {
            let reason = error.to_string();
            let reason = reason.lines().last().unwrap_or("").trim();
            let reason = reason.strip_prefix("error: ").unwrap_or(reason);
            Error::new(
                ErrorCode::Regex,
                format!("invalid regular expression {regex}: {reason}"),
            )
            .with_found(regex.to_string())
            .with_help("regex")
        })
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
