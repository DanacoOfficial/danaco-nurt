// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/logfile.rs
// Responsibility:   Log file of the setup: lines with a UTC time, kept in the temporary directory or in the file given with --log.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Log file of the setup.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static LOG: Mutex<Option<(PathBuf, File)>> = Mutex::new(None);

/// Civil date and time (UTC) of a number of seconds since the Unix epoch.
fn civil(seconds: u64) -> (u64, u64, u64, u64, u64, u64) {
    let days = seconds / 86_400;
    let rest = seconds % 86_400;
    // Days since 0000-03-01 (algorithm of Howard Hinnant).
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day, rest / 3600, rest % 3600 / 60, rest % 60)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Date of today, `YYYYMMDD`.
pub(crate) fn today() -> String {
    let (year, month, day, ..) = civil(now());
    format!("{year:04}{month:02}{day:02}")
}

/// UTC time of now, ISO 8601 with the designation `Z`.
pub(crate) fn timestamp() -> String {
    let (year, month, day, hour, minute, second) = civil(now());
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Opens the log; returns its path. Without a path the file `nurt-setup.log` of the temporary directory is used.
pub(crate) fn open(path: Option<PathBuf>) -> PathBuf {
    let path = path.unwrap_or_else(|| std::env::temp_dir().join("nurt-setup.log"));
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path)
        && let Ok(mut guard) = LOG.lock()
    {
        *guard = Some((path.clone(), file));
    }
    path
}

/// Appends a line.
pub(crate) fn line(text: &str) {
    let (year, month, day, hour, minute, second) = civil(now());
    if let Ok(mut guard) = LOG.lock()
        && let Some((_, file)) = guard.as_mut()
    {
        let _ = writeln!(
            file,
            "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z {text}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::civil;

    #[test]
    fn converts_epoch_seconds() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(1_795_824_000), (2026, 11, 28, 0, 0, 0));
        assert_eq!(civil(951_782_400 + 86_399), (2000, 2, 29, 23, 59, 59));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
