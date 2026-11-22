// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Time zones of schedules.

use jiff::tz::{self, TimeZone};
use nurt_contracts::error::{Candidate, Error};

use crate::errors::invalid;

/// Value of `timezone` that stands for the time zone of the device.
pub const LOCAL: &str = "local";

const MAX_CANDIDATES: usize = 3;
const MAX_DISTANCE: usize = 3;

/// Resolves a time zone name; `local` is replaced by `host_zone`.
///
/// The returned name is the IANA name saved in the definition, so that a later
/// change of the zone of the device does not move the runs of a schedule.
pub(crate) fn resolve(name: &str, host_zone: &str) -> Result<(String, TimeZone), Error> {
    let wanted = if name.eq_ignore_ascii_case(LOCAL) {
        host_zone
    } else {
        name
    };
    if wanted.is_empty() {
        return Ok(("UTC".to_owned(), TimeZone::UTC));
    }
    match TimeZone::get(wanted) {
        Ok(zone) => Ok((zone.iana_name().unwrap_or(wanted).to_owned(), zone)),
        Err(_) => Err(unknown(wanted)),
    }
}

/// Time zone of a saved IANA name.
pub(crate) fn get(name: &str) -> Result<TimeZone, Error> {
    TimeZone::get(name).map_err(|_| unknown(name))
}

fn unknown(name: &str) -> Error {
    let mut error = invalid(format!(
        "unknown time zone `{name}`; give an IANA name such as `Europe/Warsaw` or `UTC`"
    ))
    .with_found(name);
    for (rank, candidate) in (1u32..).zip(candidates(name)) {
        error = error.with_candidate(Candidate {
            rank,
            reference: None,
            program: format!("timezone: \"{candidate}\""),
            why: Some("similar time zone name".to_owned()),
        });
    }
    error
}

fn candidates(name: &str) -> Vec<String> {
    let wanted = name.to_ascii_lowercase();
    let mut scored: Vec<(usize, String)> = tz::db()
        .available()
        .filter_map(|known| {
            let known = known.as_str().to_owned();
            let lower = known.to_ascii_lowercase();
            let city = lower.rsplit('/').next().unwrap_or(&lower).to_owned();
            let distance = edit_distance(&wanted, &lower).min(edit_distance(&wanted, &city));
            (distance <= MAX_DISTANCE || lower.contains(&wanted)).then_some((distance, known))
        })
        .collect();
    scored.sort();
    scored
        .into_iter()
        .take(MAX_CANDIDATES)
        .map(|(_, known)| known)
        .collect()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, left) in a.chars().enumerate() {
        let mut current = vec![i + 1];
        for (j, right) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(left != *right);
            current.push(substitution.min(previous[j + 1] + 1).min(current[j] + 1));
        }
        previous = current;
    }
    previous.last().copied().unwrap_or(0)
}
