// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Space figures of a macOS volume.
//!
//! Free space is the space that ordinary programs see: the value of
//! `NSURLVolumeAvailableCapacityKey` (`kCFURLVolumeAvailableCapacityKey`),
//! which excludes purgeable space, with `statfs.f_bavail × f_bsize` as the
//! fallback. Purgeable space is reported for information only, as the
//! difference between `NSURLVolumeAvailableCapacityForImportantUsageKey` and
//! the available capacity; the system decides when it is released, so it is
//! never counted as free.

use nurt_contracts::fs::SpaceInfo;

/// `Q_GETQUOTA` of `sys/quota.h`.
pub const Q_GETQUOTA: i32 = 0x0300;
/// `USRQUOTA` of `sys/quota.h`.
pub const USRQUOTA: i32 = 0;
/// `SUBCMDSHIFT` of `sys/quota.h`.
const SUBCMDSHIFT: u32 = 8;
/// `SUBCMDMASK` of `sys/quota.h`.
const SUBCMDMASK: i32 = 0x00ff;

/// Raw space figures of a volume.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpaceFigures {
    /// `statfs.f_bsize`.
    pub block_size: u64,
    /// `statfs.f_blocks`.
    pub blocks: u64,
    /// `statfs.f_bavail`.
    pub blocks_available: u64,
    /// `kCFURLVolumeTotalCapacityKey`, when readable.
    pub total_capacity: Option<u64>,
    /// `kCFURLVolumeAvailableCapacityKey`, when readable.
    pub available_capacity: Option<u64>,
    /// `kCFURLVolumeAvailableCapacityForImportantUsageKey`, when readable.
    pub important_capacity: Option<u64>,
    /// Account quota of the volume, when enforced.
    pub quota: Option<QuotaFigures>,
}

/// Byte quota of the account (`struct dqblk`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QuotaFigures {
    /// `dqb_bhardlimit`: limit in bytes; zero means no limit.
    pub hard_limit: u64,
    /// `dqb_curbytes`: bytes in use.
    pub used: u64,
}

/// Space figures of the contract from raw figures.
///
/// `total` and `available` are capped by an enforced quota;
/// `quota_remaining` is the hard limit minus the bytes in use;
/// `purgeable_system` is the important-usage capacity minus the available
/// capacity, or `None` when the important-usage capacity is unknown.
///
/// # Examples
///
/// ```
/// use nurt_fs::macos::{SpaceFigures, space_info};
///
/// let figures = SpaceFigures {
///     block_size: 4096,
///     blocks: 1000,
///     blocks_available: 100,
///     available_capacity: Some(409_600),
///     important_capacity: Some(1_000_000),
///     ..SpaceFigures::default()
/// };
/// let info = space_info(&figures);
/// assert_eq!(info.available, 409_600);
/// assert_eq!(info.purgeable_system, Some(590_400));
/// ```
#[must_use]
pub fn space_info(figures: &SpaceFigures) -> SpaceInfo {
    let total = figures
        .total_capacity
        .unwrap_or_else(|| figures.blocks.saturating_mul(figures.block_size));
    let available = figures
        .available_capacity
        .unwrap_or_else(|| figures.blocks_available.saturating_mul(figures.block_size));
    let quota_remaining = figures
        .quota
        .filter(|quota| quota.hard_limit > 0)
        .map(|quota| quota.hard_limit.saturating_sub(quota.used));
    let hard_limit = figures
        .quota
        .map(|quota| quota.hard_limit)
        .filter(|&limit| limit > 0);
    SpaceInfo {
        total: hard_limit.map_or(total, |limit| total.min(limit)),
        available: quota_remaining.map_or(available, |remaining| available.min(remaining)),
        quota_remaining,
        quota_limit: hard_limit,
        purgeable_system: figures
            .important_capacity
            .map(|important| important.saturating_sub(available)),
    }
}

/// Command word of `quotactl`: `QCMD(cmd, type)` of `sys/quota.h`.
///
/// # Examples
///
/// ```
/// use nurt_fs::macos::{Q_GETQUOTA, USRQUOTA, quota_command};
///
/// assert_eq!(quota_command(Q_GETQUOTA, USRQUOTA), 0x30000);
/// ```
#[must_use]
pub fn quota_command(command: i32, quota_type: i32) -> i32 {
    (command << SUBCMDSHIFT) | (quota_type & SUBCMDMASK)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: u64 = 4096;

    fn statfs_only(blocks: u64, available: u64) -> SpaceFigures {
        SpaceFigures {
            block_size: BLOCK,
            blocks,
            blocks_available: available,
            ..SpaceFigures::default()
        }
    }

    #[test]
    fn falls_back_to_statfs_without_capacity_keys() {
        let info = space_info(&statfs_only(1_000, 250));
        assert_eq!(info.total, 1_000 * BLOCK);
        assert_eq!(info.available, 250 * BLOCK);
        assert_eq!(info.quota_remaining, None);
        assert_eq!(info.purgeable_system, None);
    }

    #[test]
    fn prefers_capacity_keys_over_statfs() {
        let figures = SpaceFigures {
            total_capacity: Some(5_000_000),
            available_capacity: Some(1_200_000),
            ..statfs_only(1_000, 250)
        };
        let info = space_info(&figures);
        assert_eq!(info.total, 5_000_000);
        assert_eq!(info.available, 1_200_000);
    }

    #[test]
    fn reports_purgeable_as_important_minus_available() {
        let figures = SpaceFigures {
            available_capacity: Some(10_000),
            important_capacity: Some(25_000),
            ..statfs_only(1_000, 250)
        };
        let info = space_info(&figures);
        assert_eq!(info.purgeable_system, Some(15_000));
        assert_eq!(info.available, 10_000);
    }

    #[test]
    fn never_reports_negative_purgeable_space() {
        let figures = SpaceFigures {
            available_capacity: Some(30_000),
            important_capacity: Some(25_000),
            ..statfs_only(1_000, 250)
        };
        assert_eq!(space_info(&figures).purgeable_system, Some(0));
    }

    #[test]
    fn caps_available_and_total_by_enforced_quota() {
        let figures = SpaceFigures {
            quota: Some(QuotaFigures {
                hard_limit: 100 * BLOCK,
                used: 40 * BLOCK,
            }),
            ..statfs_only(1_000, 250)
        };
        let info = space_info(&figures);
        assert_eq!(info.quota_remaining, Some(60 * BLOCK));
        assert_eq!(info.quota_limit, Some(100 * BLOCK));
        assert_eq!(info.available, 60 * BLOCK);
        assert_eq!(info.total, 100 * BLOCK);
    }

    #[test]
    fn ignores_quota_without_hard_limit() {
        let figures = SpaceFigures {
            quota: Some(QuotaFigures {
                hard_limit: 0,
                used: 40 * BLOCK,
            }),
            ..statfs_only(1_000, 250)
        };
        let info = space_info(&figures);
        assert_eq!(info.quota_remaining, None);
        assert_eq!(info.quota_limit, None);
        assert_eq!(info.available, 250 * BLOCK);
        assert_eq!(info.total, 1_000 * BLOCK);
    }

    #[test]
    fn reports_exhausted_quota_as_zero_remaining() {
        let figures = SpaceFigures {
            quota: Some(QuotaFigures {
                hard_limit: 10,
                used: 50,
            }),
            ..statfs_only(1_000, 250)
        };
        let info = space_info(&figures);
        assert_eq!(info.quota_remaining, Some(0));
        assert_eq!(info.available, 0);
    }

    #[test]
    fn saturates_block_products() {
        let info = space_info(&SpaceFigures {
            block_size: u64::MAX,
            blocks: 2,
            blocks_available: 2,
            ..SpaceFigures::default()
        });
        assert_eq!(info.total, u64::MAX);
        assert_eq!(info.available, u64::MAX);
    }

    #[test]
    fn builds_quota_command_word() {
        assert_eq!(quota_command(Q_GETQUOTA, USRQUOTA), 0x0003_0000);
        assert_eq!(quota_command(Q_GETQUOTA, 1), 0x0003_0001);
    }
}
