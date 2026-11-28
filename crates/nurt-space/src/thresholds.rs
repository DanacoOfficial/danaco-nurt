// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-space::thresholds
// Responsibility:   Reserve, soft threshold, margin and hysteresis.
// Dependencies:     nurt_contracts
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Reserve, soft threshold, margin and hysteresis.
//!
//! All thresholds are derived from the capacity `T` of a volume, which is the
//! account quota when that is smaller than the file system. With the default
//! configuration the reserve `R` is 15 % of `T` clamped to 5–50 GiB, the soft
//! threshold `S` is `R + max(5 % T, 2 GiB)` but no more than `R + 20 GiB`, the
//! margin `m` is `min(1 % T, 1 GiB)` and the hysteresis `h` is 1 % of `T`.

use nurt_contracts::config::{Percent, SizeOrPercent, SpaceConfig};
use nurt_contracts::journal::{Thresholds, VolumeState};

const GIB: u64 = 1 << 30;

/// Hundredths of a percent in a whole.
const HUNDREDTHS_IN_WHOLE: u128 = 10_000;

/// Upper bound of the reserve of a quota-limited account, as a share of the
/// quota.
const QUOTA_RESERVE_SHARE: Percent = Percent::from_percent(15);

/// Default distance between the reserve and the soft threshold, as a share of
/// the capacity, with its lower and upper bounds.
const SOFT_GAP_SHARE: Percent = Percent::from_percent(5);
const SOFT_GAP_MIN: u64 = 2 * GIB;
const SOFT_GAP_MAX: u64 = 20 * GIB;

/// Default margin as a share of the capacity, with its upper bound.
const MARGIN_SHARE: Percent = Percent::from_percent(1);
const MARGIN_MAX: u64 = GIB;

/// Hysteresis as a share of the capacity.
const HYSTERESIS_SHARE: Percent = Percent::from_percent(1);

/// Share of `total`, rounded down.
pub(crate) fn share(total: u64, percent: Percent) -> u64 {
    let product = u128::from(total) * u128::from(percent.hundredths()) / HUNDREDTHS_IN_WHOLE;
    u64::try_from(product).unwrap_or(u64::MAX)
}

/// Bounds `value` to `min..=max`; when the bounds are inverted, `max` wins so
/// that a misconfigured lower bound never raises the reserve above the upper.
fn bound(value: u64, min: u64, max: u64) -> u64 {
    value.max(min).min(max)
}

fn size_or_share(setting: SizeOrPercent, total: u64) -> u64 {
    match setting {
        SizeOrPercent::Size(size) => size.bytes(),
        SizeOrPercent::Percent(percent) => share(total, percent),
    }
}

fn reserve(config: &SpaceConfig, total: u64, quota_limited: bool) -> u64 {
    let configured = match config.reserve {
        SizeOrPercent::Size(size) => size.bytes(),
        SizeOrPercent::Percent(percent) => bound(
            share(total, percent),
            config.reserve_min.bytes(),
            config.reserve_max.bytes(),
        ),
    };
    if quota_limited {
        configured.min(share(total, QUOTA_RESERVE_SHARE))
    } else {
        configured
    }
}

fn default_soft(reserve: u64, total: u64) -> u64 {
    let gap = bound(share(total, SOFT_GAP_SHARE), SOFT_GAP_MIN, SOFT_GAP_MAX);
    reserve.saturating_add(gap)
}

/// Thresholds of a volume of capacity `total`.
///
/// `quota_limited` is set when an account quota applies to the volume; the
/// reserve is then no more than 15 % of `total`. A configured soft threshold
/// at or below the reserve is replaced by the default, so that `S > R` holds.
pub(crate) fn resolve(config: &SpaceConfig, total: u64, quota_limited: bool) -> Thresholds {
    let reserve = reserve(config, total, quota_limited);
    let soft = config
        .soft
        .map(|setting| size_or_share(setting, total))
        .filter(|soft| *soft > reserve)
        .unwrap_or_else(|| default_soft(reserve, total));
    let margin = config.margin.map_or_else(
        || share(total, MARGIN_SHARE).min(MARGIN_MAX),
        |setting| size_or_share(setting, total),
    );
    Thresholds {
        reserve,
        soft,
        margin,
        hysteresis: share(total, HYSTERESIS_SHARE),
    }
}

/// State of a volume with `free` bytes available to ordinary programs.
pub(crate) fn state(free: u64, thresholds: &Thresholds) -> VolumeState {
    if u128::from(free) * 2 < u128::from(thresholds.reserve) {
        VolumeState::Emergency
    } else if free < thresholds.reserve {
        VolumeState::Hard
    } else if free < thresholds.soft {
        VolumeState::Soft
    } else {
        VolumeState::Normal
    }
}

/// Whether `extra` more journal bytes leave at least reserve plus margin
/// free.
pub(crate) fn leaves_reserve(free: u64, extra: u64, thresholds: &Thresholds) -> bool {
    u128::from(free)
        >= u128::from(extra) + u128::from(thresholds.reserve) + u128::from(thresholds.margin)
}

/// Free space that background reclaim aims for in `state`: `S + h` in the
/// soft state, `R + m` in the hard and emergency states, none otherwise.
pub(crate) fn reclaim_target(state: VolumeState, thresholds: &Thresholds) -> Option<u64> {
    match state {
        VolumeState::Normal => None,
        VolumeState::Soft => Some(thresholds.soft.saturating_add(thresholds.hysteresis)),
        VolumeState::Hard | VolumeState::Emergency => {
            Some(thresholds.reserve.saturating_add(thresholds.margin))
        }
    }
}

#[cfg(test)]
mod tests {
    use nurt_contracts::config::SizeSetting;

    use super::*;

    const GB: u64 = 1_000_000_000;
    const TB: u64 = 1_000 * GB;

    fn defaults(total: u64) -> Thresholds {
        resolve(&SpaceConfig::default(), total, false)
    }

    #[test]
    fn thresholds_defaults_one_terabyte_disk_clamps_reserve_to_upper_bound() {
        let thresholds = defaults(TB);
        assert_eq!(thresholds.reserve, 50 * GIB);
        assert_eq!(thresholds.soft, 70 * GIB);
        assert_eq!(thresholds.margin, GIB);
        assert_eq!(thresholds.hysteresis, 10 * GB);
    }

    #[test]
    fn thresholds_defaults_four_terabyte_disk_keeps_upper_bounds() {
        let thresholds = defaults(4 * TB);
        assert_eq!(thresholds.reserve, 50 * GIB);
        assert_eq!(thresholds.soft, 70 * GIB);
        assert_eq!(thresholds.margin, GIB);
        assert_eq!(thresholds.hysteresis, 40 * GB);
    }

    #[test]
    fn thresholds_defaults_five_hundred_gigabyte_disk_clamps_fifteen_percent() {
        let thresholds = defaults(500 * GB);
        assert_eq!(thresholds.reserve, 50 * GIB);
        assert_eq!(thresholds.soft, 50 * GIB + 20 * GIB);
        assert_eq!(thresholds.margin, GIB);
        assert_eq!(thresholds.hysteresis, 5 * GB);
    }

    #[test]
    fn thresholds_defaults_small_disk_raises_reserve_to_lower_bound() {
        let thresholds = defaults(32 * GB);
        assert_eq!(thresholds.reserve, 5 * GIB);
        assert_eq!(thresholds.soft, 5 * GIB + 2 * GIB);
        assert_eq!(thresholds.margin, 320_000_000);
        assert_eq!(thresholds.hysteresis, 320_000_000);
    }

    #[test]
    fn thresholds_defaults_mid_size_disk_uses_fifteen_percent() {
        let thresholds = defaults(100 * GB);
        assert_eq!(thresholds.reserve, 15 * GB);
        assert_eq!(thresholds.soft, 15 * GB + 5 * GB);
        assert_eq!(thresholds.margin, GB);
    }

    #[test]
    fn thresholds_defaults_quota_caps_reserve_at_fifteen_percent_of_quota() {
        let quota = 10 * GB;
        let thresholds = resolve(&SpaceConfig::default(), quota, true);
        assert_eq!(thresholds.reserve, 1_500_000_000);
        assert!(thresholds.soft > thresholds.reserve);
        let unlimited = resolve(&SpaceConfig::default(), quota, false);
        assert_eq!(unlimited.reserve, 5 * GIB);
    }

    #[test]
    fn thresholds_defaults_quota_keeps_smaller_configured_reserve() {
        let mut config = SpaceConfig::default();
        config.reserve = SizeOrPercent::Size(SizeSetting::from_mib(100));
        let thresholds = resolve(&config, 10 * GB, true);
        assert_eq!(thresholds.reserve, 100 << 20);
    }

    #[test]
    fn configured_sizes_are_used_verbatim() {
        let mut config = SpaceConfig::default();
        config.reserve = SizeOrPercent::Size(SizeSetting::from_gib(1));
        config.soft = Some(SizeOrPercent::Size(SizeSetting::from_gib(3)));
        config.margin = Some(SizeOrPercent::Size(SizeSetting::from_mib(10)));
        let thresholds = resolve(&config, TB, false);
        assert_eq!(thresholds.reserve, GIB);
        assert_eq!(thresholds.soft, 3 * GIB);
        assert_eq!(thresholds.margin, 10 << 20);
    }

    #[test]
    fn configured_percentages_apply_to_capacity() {
        let mut config = SpaceConfig::default();
        config.soft = Some(SizeOrPercent::Percent(Percent::from_percent(30)));
        config.margin = Some(SizeOrPercent::Percent(Percent::from_percent(2)));
        let thresholds = resolve(&config, 100 * GB, false);
        assert_eq!(thresholds.soft, 30 * GB);
        assert_eq!(thresholds.margin, 2 * GB);
    }

    #[test]
    fn soft_threshold_at_or_below_reserve_falls_back_to_default() {
        let mut config = SpaceConfig::default();
        config.soft = Some(SizeOrPercent::Percent(Percent::from_percent(10)));
        let thresholds = resolve(&config, 100 * GB, false);
        assert_eq!(thresholds.soft, 20 * GB);
    }

    #[test]
    fn inverted_reserve_bounds_use_the_upper_bound() {
        let mut config = SpaceConfig::default();
        config.reserve_min = SizeSetting::from_gib(60);
        config.reserve_max = SizeSetting::from_gib(10);
        assert_eq!(resolve(&config, TB, false).reserve, 10 * GIB);
    }

    #[test]
    fn empty_volume_still_has_soft_above_reserve() {
        let thresholds = defaults(0);
        assert_eq!(thresholds.reserve, 5 * GIB);
        assert!(thresholds.soft > thresholds.reserve);
        assert_eq!(thresholds.margin, 0);
    }

    #[test]
    fn volume_state_transitions() {
        let thresholds = Thresholds {
            reserve: 100,
            soft: 200,
            margin: 10,
            hysteresis: 5,
        };
        assert_eq!(state(u64::MAX, &thresholds), VolumeState::Normal);
        assert_eq!(state(200, &thresholds), VolumeState::Normal);
        assert_eq!(state(199, &thresholds), VolumeState::Soft);
        assert_eq!(state(100, &thresholds), VolumeState::Soft);
        assert_eq!(state(99, &thresholds), VolumeState::Hard);
        assert_eq!(state(50, &thresholds), VolumeState::Hard);
        assert_eq!(state(49, &thresholds), VolumeState::Emergency);
        assert_eq!(state(0, &thresholds), VolumeState::Emergency);
    }

    #[test]
    fn volume_state_with_odd_reserve_compares_half_exactly() {
        let thresholds = Thresholds {
            reserve: 101,
            soft: 200,
            margin: 0,
            hysteresis: 0,
        };
        assert_eq!(state(51, &thresholds), VolumeState::Hard);
        assert_eq!(state(50, &thresholds), VolumeState::Emergency);
    }

    #[test]
    fn leaves_reserve_holds_exactly_at_reserve_plus_margin() {
        let thresholds = Thresholds {
            reserve: 100,
            soft: 200,
            margin: 10,
            hysteresis: 5,
        };
        assert!(leaves_reserve(150, 40, &thresholds));
        assert!(!leaves_reserve(150, 41, &thresholds));
        assert!(!leaves_reserve(0, u64::MAX, &thresholds));
        assert!(leaves_reserve(u64::MAX, 0, &thresholds));
    }

    #[test]
    fn reclaim_target_depends_on_state() {
        let thresholds = Thresholds {
            reserve: 100,
            soft: 200,
            margin: 10,
            hysteresis: 5,
        };
        assert_eq!(reclaim_target(VolumeState::Normal, &thresholds), None);
        assert_eq!(reclaim_target(VolumeState::Soft, &thresholds), Some(205));
        assert_eq!(reclaim_target(VolumeState::Hard, &thresholds), Some(110));
        assert_eq!(
            reclaim_target(VolumeState::Emergency, &thresholds),
            Some(110)
        );
    }

    #[test]
    fn share_saturates_instead_of_overflowing() {
        assert_eq!(share(u64::MAX, Percent::from_percent(100)), u64::MAX);
        assert_eq!(share(1_000, Percent::from_percent(15)), 150);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
