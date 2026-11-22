// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Resolution of effective limits from configuration levels.
//!
//! The configuration loader merges the seven levels and records the level
//! that set every key ([`Resolved`]); trust of the project file is decided
//! there as well. The resolver turns that result into the effective limits
//! of one volume:
//!
//! * every setting is resolved on its own; the value comes from the highest
//!   level that sets it, and `journal.volume."<mount>".*` takes precedence
//!   over the general key of the same level;
//! * limits set by `journal.limits` for a session form the run-time level;
//! * a value from a project file is raised to at least
//!   [`PROJECT_FLOOR_MAX_OPS`] and [`PROJECT_FLOOR_MAX_AGE`];
//! * a value below `journal.floor.*` is raised to the floor, whatever level
//!   set it; `unlimited` satisfies every floor;
//! * a percentage size budget is converted to bytes for the volume and
//!   clamped to `journal.max_size_min..=journal.max_size_max`.

use std::collections::BTreeMap;
use std::io::Read;
use std::sync::Arc;

use nurt_contracts::config::{
    AgeLimit, ConfigLayer, ConfigLevel, ConfigNoteKind, JournalConfig, JournalFloor,
    PROJECT_FLOOR_MAX_AGE, PROJECT_FLOOR_MAX_OPS, Percent, Resolved, SettingParseError, SizeLimit,
    StepLimit, resolve as config_resolve,
};
use nurt_contracts::error::{Error, ErrorCode, Result};
use nurt_contracts::fs::{AbsPath, Fs, VolumeInfo};
use nurt_contracts::journal::{EffectiveRetention, JournalNotable, Sourced};
use serde::{Deserialize, Serialize};

use super::limits::{SESSION_DIR, sessions_from_contents};
use crate::store::RetentionLimits;

/// Key of the step limit.
pub(crate) const MAX_OPS_KEY: &str = "journal.max_ops";
/// Key of the age limit.
pub(crate) const MAX_AGE_KEY: &str = "journal.max_age";
const OVERSIZE_KEY: &str = "journal.oversize";
const AGE_CLOCK_KEY: &str = "journal.age_clock";
const MAX_OP_SIZE_KEY: &str = "journal.max_op_size";
const FLOOR_MAX_OPS_KEY: &str = "journal.floor.max_ops";
const FLOOR_MAX_AGE_KEY: &str = "journal.floor.max_age";

/// Hundredths of a percent in a whole.
const HUNDREDTHS_IN_WHOLE: u128 = 10_000;

/// Values of the three limits set together, for example by one call of
/// `journal.limits`; `None` leaves the lower level in force.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LimitValues {
    /// Step limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) steps: Option<StepLimit>,
    /// Age limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) age: Option<AgeLimit>,
    /// Size budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) size: Option<SizeLimit>,
}

impl LimitValues {
    /// Whether no limit is set.
    pub(crate) fn is_empty(&self) -> bool {
        self.steps.is_none() && self.age.is_none() && self.size.is_none()
    }

    /// Replaces the limits that `newer` sets.
    pub(crate) fn overlay(&mut self, newer: &Self) {
        self.steps = newer.steps.or(self.steps);
        self.age = newer.age.or(self.age);
        self.size = newer.size.or(self.size);
    }

    /// Removes the limits that `other` sets.
    fn clear_set_by(&mut self, other: &Self) {
        if other.steps.is_some() {
            self.steps = None;
        }
        if other.age.is_some() {
            self.age = None;
        }
        if other.size.is_some() {
            self.size = None;
        }
    }
}

/// Limits of the run-time level: values for all volumes and values for
/// single volumes, keyed by mount point in the language form.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RuntimeLimits {
    /// Limits of every volume.
    #[serde(default)]
    pub(crate) all: LimitValues,
    /// Limits of single volumes; they take precedence over [`Self::all`].
    #[serde(default)]
    pub(crate) volumes: BTreeMap<String, LimitValues>,
}

impl RuntimeLimits {
    /// Sets limits for one volume, or for all volumes when `mount` is
    /// `None`. A value for all volumes replaces the same limit set earlier
    /// for single volumes.
    pub(crate) fn set(&mut self, mount: Option<&str>, values: &LimitValues) {
        match mount {
            Some(mount) => self
                .volumes
                .entry(mount.to_owned())
                .or_default()
                .overlay(values),
            None => {
                self.all.overlay(values);
                for volume in self.volumes.values_mut() {
                    volume.clear_set_by(values);
                }
                self.volumes.retain(|_, volume| !volume.is_empty());
            }
        }
    }

    /// Whether no limit is set.
    pub(crate) fn is_empty(&self) -> bool {
        self.all.is_empty() && self.volumes.values().all(LimitValues::is_empty)
    }

    fn volume(&self, mount: &str) -> LimitValues {
        self.volumes.get(mount).copied().unwrap_or_default()
    }
}

/// Facts about one volume needed to resolve its limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResolutionInput<'a> {
    /// Mount point in the language form, as written in
    /// `journal.volume."<mount>"`.
    pub(crate) mount: &'a str,
    /// Capacity of the volume in bytes.
    pub(crate) capacity: u64,
    /// Free bytes including the bytes held by the journal (`F + J`).
    pub(crate) free_with_journal: u64,
}

/// Resolver of the effective limits of the current configuration.
///
/// Pure: the result depends only on the merged configuration, the run-time
/// limits and the facts of the volume.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RetentionResolver<'a> {
    resolved: &'a Resolved,
    runtime: Option<&'a RuntimeLimits>,
}

/// One value that may set a setting, with its precedence.
#[derive(Clone, Copy)]
struct Candidate<T> {
    value: T,
    level: ConfigLevel,
    volume: bool,
}

impl<'a> RetentionResolver<'a> {
    /// Resolver over a merged configuration and the run-time limits.
    pub(crate) fn new(resolved: &'a Resolved, runtime: Option<&'a RuntimeLimits>) -> Self {
        Self { resolved, runtime }
    }

    /// Journal section of the merged configuration.
    pub(crate) fn journal(&self) -> &'a JournalConfig {
        &self.resolved.config.journal
    }

    /// Effective limits of one volume with their sources.
    pub(crate) fn resolve(&self, input: &ResolutionInput<'_>) -> EffectiveRetention {
        let journal = self.journal();
        let configured = journal.volume.get(input.mount);
        let runtime_all = self.runtime.map(|limits| limits.all).unwrap_or_default();
        let runtime_volume = self
            .runtime
            .map(|limits| limits.volume(input.mount))
            .unwrap_or_default();

        let mut max_ops = self.pick(
            input.mount,
            "max_ops",
            journal.max_ops,
            configured.and_then(|volume| volume.max_ops),
            [runtime_all.steps, runtime_volume.steps],
        );
        let mut max_age = self.pick(
            input.mount,
            "max_age",
            journal.max_age,
            configured.and_then(|volume| volume.max_age),
            [runtime_all.age, runtime_volume.age],
        );
        let max_size = self.pick(
            input.mount,
            "max_size",
            journal.max_size,
            None,
            [runtime_all.size, runtime_volume.size],
        );

        let project_ops = (max_ops.source == ConfigLevel::Project)
            .then_some((PROJECT_FLOOR_MAX_OPS, ConfigLevel::Project));
        raise(&mut max_ops, project_ops, steps_below);
        raise(&mut max_ops, self.floor_steps(), steps_below);
        let project_age = (max_age.source == ConfigLevel::Project)
            .then_some((PROJECT_FLOOR_MAX_AGE, ConfigLevel::Project));
        raise(&mut max_age, project_age, age_below);
        raise(&mut max_age, self.floor_age(), age_below);

        let max_size_bytes = budget_bytes(max_size.value, input, journal);
        EffectiveRetention {
            max_ops,
            max_age,
            max_size,
            max_size_bytes,
            oversize: self.general(OVERSIZE_KEY, journal.oversize),
            age_clock: self.general(AGE_CLOCK_KEY, journal.age_clock),
            max_op_size: self.general(MAX_OP_SIZE_KEY, journal.max_op_size),
        }
    }

    /// Protection floor in force and the level that set it; when the two
    /// floors come from different levels, the higher level is reported.
    pub(crate) fn floor(&self) -> Option<Sourced<JournalFloor>> {
        let source = [
            self.floor_steps().map(|(_, level)| level),
            self.floor_age().map(|(_, level)| level),
        ]
        .into_iter()
        .flatten()
        .max()?;
        Some(Sourced {
            value: self.journal().floor.clone(),
            source,
            raised_to_floor: false,
        })
    }

    /// Notes of `journal.status`: ignored project settings, once per file,
    /// and limits raised to a floor.
    pub(crate) fn notes(&self, effective: &EffectiveRetention) -> Vec<JournalNotable> {
        let mut notes: Vec<JournalNotable> = Vec::new();
        for note in &self.resolved.notes {
            let notable = match note.kind {
                ConfigNoteKind::JournalConfigIgnored => JournalNotable::JournalConfigIgnored {
                    file: note.origin.clone(),
                },
                ConfigNoteKind::TrustConfigIgnored => JournalNotable::TrustConfigIgnored {
                    file: note.origin.clone(),
                },
                _ => continue,
            };
            if !notes.contains(&notable) {
                notes.push(notable);
            }
        }
        for (raised, key) in [
            (effective.max_ops.raised_to_floor, MAX_OPS_KEY),
            (effective.max_age.raised_to_floor, MAX_AGE_KEY),
        ] {
            if raised {
                notes.push(JournalNotable::RaisedToFloor {
                    setting: key.to_owned(),
                });
            }
        }
        notes
    }

    fn general<T: Copy>(&self, key: &str, value: T) -> Sourced<T> {
        Sourced {
            value,
            source: self.resolved.source(key),
            raised_to_floor: false,
        }
    }

    fn pick<T: Copy>(
        &self,
        mount: &str,
        field: &str,
        general: T,
        configured: Option<T>,
        runtime: [Option<T>; 2],
    ) -> Sourced<T> {
        let mut best = Candidate {
            value: general,
            level: self.resolved.source(&format!("journal.{field}")),
            volume: false,
        };
        let [runtime_all, runtime_volume] = runtime;
        let others = [
            configured.map(|value| Candidate {
                value,
                level: self.resolved.source(&volume_key(mount, field)),
                volume: true,
            }),
            runtime_all.map(|value| Candidate {
                value,
                level: ConfigLevel::Runtime,
                volume: false,
            }),
            runtime_volume.map(|value| Candidate {
                value,
                level: ConfigLevel::Runtime,
                volume: true,
            }),
        ];
        for candidate in others.into_iter().flatten() {
            if (candidate.level, candidate.volume) >= (best.level, best.volume) {
                best = candidate;
            }
        }
        Sourced {
            value: best.value,
            source: best.level,
            raised_to_floor: false,
        }
    }

    fn floor_steps(&self) -> Option<(StepLimit, ConfigLevel)> {
        self.journal()
            .floor
            .max_ops
            .map(|floor| (floor, self.resolved.source(FLOOR_MAX_OPS_KEY)))
    }

    fn floor_age(&self) -> Option<(AgeLimit, ConfigLevel)> {
        self.journal()
            .floor
            .max_age
            .map(|floor| (floor, self.resolved.source(FLOOR_MAX_AGE_KEY)))
    }
}

/// Raises `sourced` to `minimum` when it is below it; the level of the
/// minimum becomes the source.
fn raise<T: Copy>(
    sourced: &mut Sourced<T>,
    minimum: Option<(T, ConfigLevel)>,
    below: fn(T, T) -> bool,
) {
    if let Some((minimum, level)) = minimum
        && below(sourced.value, minimum)
    {
        sourced.value = minimum;
        sourced.source = level;
        sourced.raised_to_floor = true;
    }
}

fn steps_below(value: StepLimit, minimum: StepLimit) -> bool {
    match (value, minimum) {
        (StepLimit::Unlimited, _) => false,
        (StepLimit::Limited(_), StepLimit::Unlimited) => true,
        (StepLimit::Limited(value), StepLimit::Limited(minimum)) => value < minimum,
    }
}

fn age_below(value: AgeLimit, minimum: AgeLimit) -> bool {
    match (value, minimum) {
        (AgeLimit::Unlimited, _) => false,
        (AgeLimit::Limited(_), AgeLimit::Unlimited) => true,
        (AgeLimit::Limited(value), AgeLimit::Limited(minimum)) => value < minimum,
    }
}

/// Size budget in bytes of a volume; `None` when unlimited.
///
/// Percentages are clamped to `max_size_min..=max_size_max`; when the
/// configured minimum exceeds the maximum, the maximum wins, so that a
/// misconfiguration never enlarges the journal.
pub(crate) fn budget_bytes(
    limit: SizeLimit,
    input: &ResolutionInput<'_>,
    journal: &JournalConfig,
) -> Option<u64> {
    let clamp = |bytes: u64| {
        bytes
            .max(journal.max_size_min.bytes())
            .min(journal.max_size_max.bytes())
    };
    match limit {
        SizeLimit::Bytes(size) => Some(size.bytes()),
        SizeLimit::OfCapacity(share) => Some(clamp(share_of(input.capacity, share))),
        SizeLimit::OfFree(share) => Some(clamp(share_of(input.free_with_journal, share))),
        SizeLimit::Unlimited => None,
    }
}

fn share_of(base: u64, share: Percent) -> u64 {
    let bytes = u128::from(base) * u128::from(share.hundredths()) / HUNDREDTHS_IN_WHOLE;
    u64::try_from(bytes).unwrap_or(u64::MAX)
}

/// Checks limits requested for a volume of `capacity` bytes against the
/// accepted ranges: steps `1..=100000`, age `10s..=365d`, size at least
/// 1 MiB and at most the capacity, `1%..=90%` of the capacity or
/// `1%free..=100%free`; `unlimited` is always accepted.
///
/// # Errors
///
/// Returns `E_INVALID_VALUE` naming the value and the accepted range.
pub(crate) fn validate(values: &LimitValues, capacity: u64) -> Result<()> {
    if let Some(steps) = values.steps {
        reparse::<StepLimit>(&steps.to_string())?;
    }
    if let Some(age) = values.age {
        reparse::<AgeLimit>(&age.to_string())?;
    }
    if let Some(size) = values.size {
        reparse::<SizeLimit>(&size.to_string())?;
        if let SizeLimit::Bytes(bytes) = size
            && bytes.bytes() > capacity
        {
            return Err(Error::new(
                ErrorCode::InvalidValue,
                format!(
                    "size budget {bytes} is larger than the volume capacity of {capacity} bytes; use a smaller size or a percentage"
                ),
            ));
        }
    }
    Ok(())
}

fn reparse<T: std::str::FromStr<Err = SettingParseError>>(text: &str) -> Result<T> {
    text.parse::<T>()
        .map_err(|error| Error::new(ErrorCode::InvalidValue, error.message))
}

/// Dotted key of a setting of one volume, quoted as the configuration
/// loader records it.
fn volume_key(mount: &str, field: &str) -> String {
    format!("journal.volume.{}.{field}", key_segment(mount))
}

/// Writes one key segment as TOML does: bare when it consists of ASCII
/// letters, digits, `_` and `-`, otherwise quoted with `\` and `"` escaped.
fn key_segment(segment: &str) -> String {
    let bare = !segment.is_empty()
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if bare {
        segment.to_owned()
    } else {
        let escaped = segment.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    }
}

/// Effective retention limits for the journal store, resolved from the
/// configuration layers of the instance and the limits of the sessions kept
/// in the state directory.
///
/// The budget of a percentage of free space is computed from the space
/// available to the account, without the bytes held by the journal, so it is
/// never larger than the budget reported by `journal.status`. Unreadable
/// session files are skipped; the expiry pass removes them.
pub struct ConfiguredLimits {
    resolved: Resolved,
    fs: Arc<dyn Fs>,
    sessions: AbsPath,
}

impl ConfiguredLimits {
    /// Limits of the configuration `layers`; the session limits are read
    /// from `state_dir` at every call.
    ///
    /// # Errors
    ///
    /// Returns `E_CONFIG` naming the key, level and file of an invalid value.
    pub fn new(
        layers: &[ConfigLayer],
        project_trusted: bool,
        fs: Arc<dyn Fs>,
        state_dir: &AbsPath,
    ) -> Result<Self> {
        let resolved = config_resolve(layers, project_trusted)
            .map_err(|error| Error::new(ErrorCode::Config, error.to_string()))?;
        let sessions = state_dir.join_name(SESSION_DIR).map_err(Error::from)?;
        Ok(Self {
            resolved,
            fs,
            sessions,
        })
    }

    fn session_limits(&self) -> Option<RuntimeLimits> {
        let entries = self.fs.read_dir(&self.sessions).ok()?;
        let mut contents = Vec::new();
        for entry in entries.flatten() {
            let Some(path) = entry
                .name
                .to_str()
                .and_then(|name| self.sessions.join_name(name).ok())
            else {
                continue;
            };
            let mut content = Vec::new();
            if let Ok(mut handle) = self.fs.open_read(&path)
                && handle.read_to_end(&mut content).is_ok()
            {
                contents.push(content);
            }
        }
        sessions_from_contents(&contents)
    }
}

impl RetentionLimits for ConfiguredLimits {
    fn effective(&self, volume: &VolumeInfo) -> EffectiveRetention {
        let (capacity, free) = self
            .fs
            .space(&volume.id)
            .map_or((0, 0), |space| (space.total, space.available));
        let runtime = self.session_limits();
        let mount = volume.mount.to_language_string();
        RetentionResolver::new(&self.resolved, runtime.as_ref()).resolve(&ResolutionInput {
            mount: &mount,
            capacity,
            free_with_journal: free,
        })
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::config::{
        AgeClock, ByteLimit, ConfigLayer, DurationSetting, Oversize, SizeSetting, env_layer,
        resolve,
    };
    use nurt_fs::mem::{MemFs, VolumeSpec};
    use serde_json::{Value, json};

    use super::*;

    const GIB: u64 = 1 << 30;
    const MOUNT: &str = "/mnt/data";

    /// Levels that come from configuration sources, weakest first.
    const SOURCE_LEVELS: [ConfigLevel; 6] = [
        ConfigLevel::System,
        ConfigLevel::User,
        ConfigLevel::Project,
        ConfigLevel::ConfigFile,
        ConfigLevel::Environment,
        ConfigLevel::Runtime,
    ];

    fn layer(level: ConfigLevel, values: Value) -> ConfigLayer {
        let values = values.as_object().cloned().unwrap_or_default();
        ConfigLayer::new(level, format!("{level}.toml"), values)
    }

    fn merged(layers: &[ConfigLayer], project_trusted: bool) -> Resolved {
        resolve(layers, project_trusted).unwrap()
    }

    /// Volume of 500 GiB with 200 GiB free including the journal.
    fn volume() -> ResolutionInput<'static> {
        ResolutionInput {
            mount: MOUNT,
            capacity: 500 * GIB,
            free_with_journal: 200 * GIB,
        }
    }

    fn effective(resolved: &Resolved, runtime: Option<&RuntimeLimits>) -> EffectiveRetention {
        RetentionResolver::new(resolved, runtime).resolve(&volume())
    }

    fn minutes(count: u64) -> AgeLimit {
        AgeLimit::Limited(DurationSetting::from_secs(count * 60))
    }

    #[test]
    fn builtin_defaults_apply_without_layers() {
        let resolved = merged(&[], false);
        let limits = effective(&resolved, None);
        assert_eq!(limits.max_ops.value, StepLimit::Limited(30));
        assert_eq!(limits.max_age.value, minutes(30));
        assert_eq!(limits.max_size.value, SizeLimit::Unlimited);
        assert_eq!(limits.max_size_bytes, None);
        assert_eq!(limits.oversize.value, Oversize::Skip);
        assert_eq!(limits.age_clock.value, AgeClock::Wall);
        assert_eq!(limits.max_op_size.value, ByteLimit::Unlimited);
        for source in [
            limits.max_ops.source,
            limits.max_age.source,
            limits.max_size.source,
            limits.oversize.source,
            limits.age_clock.source,
            limits.max_op_size.source,
        ] {
            assert_eq!(source, ConfigLevel::Builtin);
        }
        assert!(!limits.max_ops.raised_to_floor && !limits.max_age.raised_to_floor);
    }

    /// Every level sets a distinct value of all three dimensions; the
    /// highest present level decides each of them.
    #[test]
    fn highest_present_level_decides_every_dimension() {
        for top in 0..SOURCE_LEVELS.len() {
            let layers: Vec<ConfigLayer> = (0..=top)
                .map(|index| {
                    layer(
                        SOURCE_LEVELS[index],
                        json!({ "journal": {
                            "max_ops": 20 + index,
                            "max_age": format!("{}m", 10 + index),
                            "max_size": format!("{}gib", 1 + index),
                        }}),
                    )
                })
                .collect();
            let resolved = merged(&layers, true);
            let limits = effective(&resolved, None);
            let level = SOURCE_LEVELS[top];
            let offset = u32::try_from(top).unwrap();
            assert_eq!(limits.max_ops.value, StepLimit::Limited(20 + offset));
            assert_eq!(limits.max_age.value, minutes(10 + u64::from(offset)));
            assert_eq!(
                limits.max_size.value,
                SizeLimit::Bytes(SizeSetting::from_gib(1 + u64::from(offset)))
            );
            assert_eq!(limits.max_size_bytes, Some((1 + u64::from(offset)) * GIB));
            assert_eq!(limits.max_ops.source, level);
            assert_eq!(limits.max_age.source, level);
            assert_eq!(limits.max_size.source, level);
        }
    }

    /// Seven levels times three dimensions: a dimension set at one level
    /// takes its value and source from that level, the other dimensions keep
    /// the built-in defaults.
    #[test]
    fn each_level_sets_each_dimension_independently() {
        let dimensions: [(&str, Value); 3] = [
            ("max_ops", json!(45)),
            ("max_age", json!("2h")),
            ("max_size", json!("7gib")),
        ];
        for level in SOURCE_LEVELS {
            for (field, value) in &dimensions {
                let layers = [layer(level, json!({ "journal": { *field: value } }))];
                let resolved = merged(&layers, true);
                let limits = effective(&resolved, None);
                let sources = [
                    ("max_ops", limits.max_ops.source),
                    ("max_age", limits.max_age.source),
                    ("max_size", limits.max_size.source),
                ];
                for (name, source) in sources {
                    let expected = if name == *field {
                        level
                    } else {
                        ConfigLevel::Builtin
                    };
                    assert_eq!(source, expected, "{name} set by {field} at {level}");
                }
                match *field {
                    "max_ops" => assert_eq!(limits.max_ops.value, StepLimit::Limited(45)),
                    "max_age" => assert_eq!(limits.max_age.value, minutes(120)),
                    _ => assert_eq!(limits.max_size_bytes, Some(7 * GIB)),
                }
            }
        }
    }

    #[test]
    fn session_limits_take_the_runtime_level() {
        let layers = [layer(
            ConfigLevel::Environment,
            json!({ "journal": { "max_ops": 50, "max_age": "1h", "max_size": "9gib" } }),
        )];
        let resolved = merged(&layers, false);
        let mut runtime = RuntimeLimits::default();
        runtime.set(
            None,
            &LimitValues {
                steps: Some(StepLimit::Limited(12)),
                age: None,
                size: Some(SizeLimit::Unlimited),
            },
        );
        let limits = effective(&resolved, Some(&runtime));
        assert_eq!(limits.max_ops.value, StepLimit::Limited(12));
        assert_eq!(limits.max_ops.source, ConfigLevel::Runtime);
        assert_eq!(limits.max_age.value, minutes(60));
        assert_eq!(limits.max_age.source, ConfigLevel::Environment);
        assert_eq!(limits.max_size_bytes, None);
        assert_eq!(limits.max_size.source, ConfigLevel::Runtime);
    }

    #[test]
    fn environment_variables_resolve_at_their_level() {
        let environment = env_layer(
            [
                ("NURT_JOURNAL_MAX_OPS", "75"),
                ("NURT_JOURNAL_MAX_SIZE", "10%"),
            ],
            ':',
        )
        .unwrap();
        let resolved = merged(&[environment], false);
        let limits = effective(&resolved, None);
        assert_eq!(limits.max_ops.value, StepLimit::Limited(75));
        assert_eq!(limits.max_ops.source, ConfigLevel::Environment);
        assert_eq!(limits.max_size_bytes, Some(50 * GIB));
    }

    #[test]
    fn volume_override_wins_within_its_level() {
        let layers = [layer(
            ConfigLevel::User,
            json!({ "journal": {
                "max_ops": 50,
                "volume": { MOUNT: { "max_ops": 40, "max_age": "2h" } },
            }}),
        )];
        let resolved = merged(&layers, false);
        let limits = effective(&resolved, None);
        assert_eq!(limits.max_ops.value, StepLimit::Limited(40));
        assert_eq!(limits.max_ops.source, ConfigLevel::User);
        assert_eq!(limits.max_age.value, minutes(120));

        let other = RetentionResolver::new(&resolved, None).resolve(&ResolutionInput {
            mount: "/home",
            ..volume()
        });
        assert_eq!(other.max_ops.value, StepLimit::Limited(50));
    }

    #[test]
    fn general_key_of_higher_level_overrides_volume_override() {
        let layers = [
            layer(
                ConfigLevel::User,
                json!({ "journal": { "volume": { MOUNT: { "max_ops": 40 } } } }),
            ),
            layer(
                ConfigLevel::ConfigFile,
                json!({ "journal": { "max_ops": 60 } }),
            ),
        ];
        let resolved = merged(&layers, false);
        let limits = effective(&resolved, None);
        assert_eq!(limits.max_ops.value, StepLimit::Limited(60));
        assert_eq!(limits.max_ops.source, ConfigLevel::ConfigFile);
    }

    #[test]
    fn session_volume_limit_wins_over_session_limit_for_all_volumes() {
        let resolved = merged(&[], false);
        let mut runtime = RuntimeLimits::default();
        let steps = |count| LimitValues {
            steps: Some(StepLimit::Limited(count)),
            ..LimitValues::default()
        };
        runtime.set(None, &steps(20));
        runtime.set(Some(MOUNT), &steps(15));
        assert_eq!(
            effective(&resolved, Some(&runtime)).max_ops.value,
            StepLimit::Limited(15)
        );

        runtime.set(None, &steps(25));
        assert!(runtime.volumes.is_empty());
        assert_eq!(
            effective(&resolved, Some(&runtime)).max_ops.value,
            StepLimit::Limited(25)
        );
    }

    #[test]
    fn untrusted_project_limits_are_ignored_with_a_note() {
        let project = layer(
            ConfigLevel::Project,
            json!({ "journal": { "max_ops": 1, "max_age": "10s" } }),
        );
        let resolved = merged(&[project], false);
        let resolver = RetentionResolver::new(&resolved, None);
        let limits = resolver.resolve(&volume());
        assert_eq!(limits.max_ops.value, StepLimit::Limited(30));
        assert_eq!(limits.max_ops.source, ConfigLevel::Builtin);
        assert_eq!(
            resolver.notes(&limits),
            vec![JournalNotable::JournalConfigIgnored {
                file: "project.toml".to_owned()
            }]
        );
    }

    #[test]
    fn project_cannot_trust_itself() {
        let project = layer(
            ConfigLevel::Project,
            json!({
                "scope": { "trusted_projects": ["/home/user/app"] },
                "service": { "mode": "system" },
                "journal": { "max_ops": 12 },
            }),
        );
        let resolved = merged(&[project], false);
        let resolver = RetentionResolver::new(&resolved, None);
        let limits = resolver.resolve(&volume());
        assert_eq!(limits.max_ops.source, ConfigLevel::Builtin);
        assert_eq!(
            resolver.notes(&limits),
            vec![
                JournalNotable::TrustConfigIgnored {
                    file: "project.toml".to_owned()
                },
                JournalNotable::JournalConfigIgnored {
                    file: "project.toml".to_owned()
                },
            ]
        );
    }

    #[test]
    fn trusted_project_limits_apply() {
        let project = layer(
            ConfigLevel::Project,
            json!({ "journal": { "max_ops": 40 } }),
        );
        let resolved = merged(&[project], true);
        let resolver = RetentionResolver::new(&resolved, None);
        let limits = resolver.resolve(&volume());
        assert_eq!(limits.max_ops.value, StepLimit::Limited(40));
        assert_eq!(limits.max_ops.source, ConfigLevel::Project);
        assert!(resolver.notes(&limits).is_empty());
    }

    #[test]
    fn trusted_project_values_are_raised_to_the_project_minimum() {
        let project = layer(
            ConfigLevel::Project,
            json!({ "journal": { "max_ops": 3, "max_age": "1m" } }),
        );
        let resolved = merged(&[project], true);
        let resolver = RetentionResolver::new(&resolved, None);
        let limits = resolver.resolve(&volume());
        assert_eq!(limits.max_ops.value, PROJECT_FLOOR_MAX_OPS);
        assert_eq!(limits.max_age.value, PROJECT_FLOOR_MAX_AGE);
        assert_eq!(limits.max_ops.source, ConfigLevel::Project);
        assert!(limits.max_ops.raised_to_floor && limits.max_age.raised_to_floor);
        assert_eq!(resolver.floor(), None);
        assert_eq!(
            resolver.notes(&limits),
            vec![
                JournalNotable::RaisedToFloor {
                    setting: MAX_OPS_KEY.to_owned()
                },
                JournalNotable::RaisedToFloor {
                    setting: MAX_AGE_KEY.to_owned()
                },
            ]
        );
    }

    #[test]
    fn system_floor_raises_lower_values_of_every_level() {
        let system = layer(
            ConfigLevel::System,
            json!({ "journal": { "floor": { "max_ops": 20, "max_age": "10m" } } }),
        );
        for level in &SOURCE_LEVELS[1..] {
            let lower = layer(
                *level,
                json!({ "journal": { "max_ops": 12, "max_age": "5m" } }),
            );
            let resolved = merged(&[system.clone(), lower], true);
            let limits = effective(&resolved, None);
            assert_eq!(limits.max_ops.value, StepLimit::Limited(20), "{level}");
            assert_eq!(limits.max_ops.source, ConfigLevel::System);
            assert!(limits.max_ops.raised_to_floor);
            assert_eq!(limits.max_age.value, minutes(10));
            assert!(limits.max_age.raised_to_floor);
        }
    }

    #[test]
    fn floor_applies_to_session_limits() {
        let host = layer(
            ConfigLevel::ConfigFile,
            json!({ "journal": { "floor": { "max_ops": 15 } } }),
        );
        let resolved = merged(&[host], false);
        let mut runtime = RuntimeLimits::default();
        runtime.set(
            None,
            &LimitValues {
                steps: Some(StepLimit::Limited(1)),
                ..LimitValues::default()
            },
        );
        let resolver = RetentionResolver::new(&resolved, Some(&runtime));
        let limits = resolver.resolve(&volume());
        assert_eq!(limits.max_ops.value, StepLimit::Limited(15));
        assert_eq!(limits.max_ops.source, ConfigLevel::ConfigFile);
        assert_eq!(
            resolver.floor().map(|floor| floor.source),
            Some(ConfigLevel::ConfigFile)
        );
    }

    #[test]
    fn floor_raises_builtin_defaults_and_keeps_higher_values() {
        let system = layer(
            ConfigLevel::System,
            json!({ "journal": { "floor": { "max_ops": 50 } } }),
        );
        let resolved = merged(&[system.clone()], false);
        let limits = effective(&resolved, None);
        assert_eq!(limits.max_ops.value, StepLimit::Limited(50));
        assert!(limits.max_ops.raised_to_floor);
        assert!(!limits.max_age.raised_to_floor);

        let user = layer(ConfigLevel::User, json!({ "journal": { "max_ops": 80 } }));
        let resolved = merged(&[system, user], false);
        let limits = effective(&resolved, None);
        assert_eq!(limits.max_ops.value, StepLimit::Limited(80));
        assert_eq!(limits.max_ops.source, ConfigLevel::User);
        assert!(!limits.max_ops.raised_to_floor);
    }

    #[test]
    fn unlimited_satisfies_every_floor() {
        let layers = [
            layer(
                ConfigLevel::System,
                json!({ "journal": { "floor": { "max_ops": 100, "max_age": "1d" } } }),
            ),
            layer(
                ConfigLevel::User,
                json!({ "journal": { "max_ops": "unlimited", "max_age": "unlimited" } }),
            ),
        ];
        let resolved = merged(&layers, false);
        let limits = effective(&resolved, None);
        assert_eq!(limits.max_ops.value, StepLimit::Unlimited);
        assert_eq!(limits.max_age.value, AgeLimit::Unlimited);
        assert!(!limits.max_ops.raised_to_floor && !limits.max_age.raised_to_floor);
    }

    #[test]
    fn unlimited_floor_raises_every_limited_value() {
        let layers = [layer(
            ConfigLevel::System,
            json!({ "journal": { "floor": { "max_ops": "unlimited", "max_age": "unlimited" } } }),
        )];
        let resolved = merged(&layers, false);
        let limits = effective(&resolved, None);
        assert_eq!(limits.max_ops.value, StepLimit::Unlimited);
        assert_eq!(limits.max_age.value, AgeLimit::Unlimited);
    }

    #[test]
    fn size_forms_resolve_to_bytes_with_clamping() {
        let budget = |text: &str, capacity: u64, free: u64| {
            let layers = [layer(
                ConfigLevel::User,
                json!({ "journal": { "max_size": text } }),
            )];
            let resolved = merged(&layers, false);
            RetentionResolver::new(&resolved, None)
                .resolve(&ResolutionInput {
                    mount: MOUNT,
                    capacity: capacity * GIB,
                    free_with_journal: free * GIB,
                })
                .max_size_bytes
        };
        assert_eq!(budget("20gib", 500, 200), Some(20 * GIB));
        assert_eq!(budget("1%", 500, 200), Some(5 * GIB));
        assert_eq!(budget("10%", 500, 200), Some(50 * GIB));
        assert_eq!(budget("90%", 500, 200), Some(50 * GIB));
        assert_eq!(budget("1%", 100, 50), Some(2 * GIB));
        assert_eq!(budget("10%free", 500, 200), Some(20 * GIB));
        assert_eq!(budget("100%free", 500, 200), Some(50 * GIB));
        assert_eq!(budget("1%free", 500, 100), Some(2 * GIB));
        assert_eq!(budget("unlimited", 500, 200), None);
    }

    #[test]
    fn misconfigured_clamp_keeps_the_upper_bound() {
        let layers = [layer(
            ConfigLevel::User,
            json!({ "journal": { "max_size": "1%", "max_size_min": "8gib", "max_size_max": "4gib" } }),
        )];
        let resolved = merged(&layers, false);
        assert_eq!(effective(&resolved, None).max_size_bytes, Some(4 * GIB));
    }

    #[test]
    fn validation_accepts_values_within_ranges() {
        let values = LimitValues {
            steps: Some(StepLimit::Limited(100_000)),
            age: Some(AgeLimit::Limited(DurationSetting::from_secs(10))),
            size: Some(SizeLimit::Bytes(SizeSetting::from_mib(1))),
        };
        assert!(validate(&values, GIB).is_ok());
        let unlimited = LimitValues {
            steps: Some(StepLimit::Unlimited),
            age: Some(AgeLimit::Unlimited),
            size: Some(SizeLimit::Unlimited),
        };
        assert!(validate(&unlimited, GIB).is_ok());
        let shares = LimitValues {
            size: Some(SizeLimit::OfFree(Percent::from_percent(100))),
            ..LimitValues::default()
        };
        assert!(validate(&shares, GIB).is_ok());
    }

    #[test]
    fn validation_rejects_values_out_of_range() {
        let rejected = [
            LimitValues {
                steps: Some(StepLimit::Limited(0)),
                ..LimitValues::default()
            },
            LimitValues {
                steps: Some(StepLimit::Limited(StepLimit::MAX + 1)),
                ..LimitValues::default()
            },
            LimitValues {
                age: Some(AgeLimit::Limited(DurationSetting::from_secs(5))),
                ..LimitValues::default()
            },
            LimitValues {
                age: Some(AgeLimit::Limited(DurationSetting::from_secs(
                    366 * 24 * 3600,
                ))),
                ..LimitValues::default()
            },
            LimitValues {
                size: Some(SizeLimit::Bytes(SizeSetting::from_bytes(512 * 1024))),
                ..LimitValues::default()
            },
            LimitValues {
                size: Some(SizeLimit::Bytes(SizeSetting::from_gib(2))),
                ..LimitValues::default()
            },
            LimitValues {
                size: Some(SizeLimit::OfCapacity(Percent::from_percent(95))),
                ..LimitValues::default()
            },
            LimitValues {
                size: Some(SizeLimit::OfFree(Percent::from_percent(0))),
                ..LimitValues::default()
            },
        ];
        for values in rejected {
            let error = validate(&values, GIB).unwrap_err();
            assert_eq!(error.code(), ErrorCode::InvalidValue, "{values:?}");
        }
    }

    #[test]
    fn volume_keys_are_quoted_like_the_loader() {
        assert_eq!(
            volume_key("/mnt/data", "max_ops"),
            "journal.volume.\"/mnt/data\".max_ops"
        );
        assert_eq!(
            volume_key("data-1", "max_age"),
            "journal.volume.data-1.max_age"
        );
        assert_eq!(
            volume_key("C:/a\"b\\", "max_ops"),
            "journal.volume.\"C:/a\\\"b\\\\\".max_ops"
        );
    }

    #[test]
    fn configured_limits_combine_configuration_and_session_files() {
        let root = AbsPath::new("/").unwrap();
        let fs = MemFs::builder()
            .volume(VolumeSpec::posix(root).with_capacity(500 * GIB))
            .build()
            .unwrap();
        let state = AbsPath::new("/state").unwrap();
        let sessions = state.join_name(SESSION_DIR).unwrap();
        fs.seed_dir(&state).unwrap();
        fs.seed_dir(&sessions).unwrap();
        let fs = Arc::new(fs);
        let layers = [layer(
            ConfigLevel::User,
            json!({ "journal": { "max_age": "1h", "max_size": "1gib" } }),
        )];
        let limits = ConfiguredLimits::new(&layers, false, fs.clone(), &state).unwrap();
        let volume = fs.volume(&state).unwrap();

        let before = limits.effective(&volume);
        assert_eq!(before.max_ops.source, ConfigLevel::Builtin);
        assert_eq!(before.max_age.value, minutes(60));
        assert_eq!(before.max_size_bytes, Some(GIB));

        let session = br#"{"session":"s1","changed":0,"touched":0,"limits":{"all":{"steps":12}}}"#;
        fs.seed_file(&sessions.join_name("a.json").unwrap(), session)
            .unwrap();
        fs.seed_file(&sessions.join_name("b.json").unwrap(), b"not json")
            .unwrap();
        let after = limits.effective(&volume);
        assert_eq!(after.max_ops.value, StepLimit::Limited(12));
        assert_eq!(after.max_ops.source, ConfigLevel::Runtime);
    }

    #[test]
    fn configured_limits_reject_an_invalid_configuration() {
        let fs = Arc::new(MemFs::new().unwrap());
        let layers = [layer(
            ConfigLevel::User,
            json!({ "journal": { "max_ops": 0 } }),
        )];
        let state = AbsPath::new("/state").unwrap();
        let error = ConfiguredLimits::new(&layers, false, fs, &state)
            .err()
            .unwrap();
        assert_eq!(error.code(), ErrorCode::Config);
    }

    #[test]
    fn runtime_limits_round_trip_through_json() {
        let mut runtime = RuntimeLimits::default();
        runtime.set(
            Some(MOUNT),
            &LimitValues {
                steps: Some(StepLimit::Unlimited),
                age: Some(minutes(15)),
                size: Some(SizeLimit::OfFree(Percent::from_percent(50))),
            },
        );
        let text = serde_json::to_string(&runtime).unwrap();
        let back: RuntimeLimits = serde_json::from_str(&text).unwrap();
        assert_eq!(back, runtime);
        assert!(!back.is_empty());
        assert!(RuntimeLimits::default().is_empty());
    }
}
