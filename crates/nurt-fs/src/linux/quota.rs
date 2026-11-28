// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-fs::linux::quota
// Responsibility:   Account quota figures from the result of `quotactl(Q_GETQUOTA)`.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Account quota figures from the result of `quotactl(Q_GETQUOTA)`.
//!
//! Block limits of `struct if_dqblk` are counted in quota blocks of
//! `QIF_DQBLKSIZE` bytes; the current usage `dqb_curspace` is counted in
//! bytes (`quotactl(2)`).

/// Size of a quota block in bytes (`QIF_DQBLKSIZE`).
pub const QUOTA_BLOCK_SIZE: u64 = 1024;
/// `dqb_valid` bit: block limits are valid (`QIF_BLIMITS`).
pub const QIF_BLIMITS: u32 = 1;
/// `dqb_valid` bit: current space usage is valid (`QIF_SPACE`).
pub const QIF_SPACE: u32 = 2;

/// Space quota of an account on a volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaFigures {
    /// Effective limit in bytes.
    pub limit: u64,
    /// Bytes the account may still allocate; zero when the usage exceeds
    /// the limit.
    pub remaining: u64,
}

/// Quota figures from the fields of `struct if_dqblk`.
///
/// The effective limit is the soft block limit when it is set, otherwise
/// the hard block limit: writes beyond the soft limit fail once the grace
/// period ends, so ordinary programs can rely only on the space below it.
/// Returns `None` when no block limit is set or when `valid` lacks the
/// block limit or space usage bits.
///
/// # Examples
///
/// ```
/// use nurt_fs::linux::{QIF_BLIMITS, QIF_SPACE, QuotaFigures, quota_figures};
///
/// // Soft limit of 1000 blocks (1 024 000 bytes), 24 000 bytes in use.
/// let figures = quota_figures(QIF_BLIMITS | QIF_SPACE, 1000, 2000, 24_000);
/// assert_eq!(figures, Some(QuotaFigures { limit: 1_024_000, remaining: 1_000_000 }));
/// ```
#[must_use]
pub fn quota_figures(
    valid: u32,
    block_soft_limit: u64,
    block_hard_limit: u64,
    current_space: u64,
) -> Option<QuotaFigures> {
    let required = QIF_BLIMITS | QIF_SPACE;
    if valid & required != required {
        return None;
    }
    let blocks = match (block_soft_limit, block_hard_limit) {
        (0, 0) => return None,
        (0, hard) => hard,
        (soft, 0) => soft,
        (soft, hard) => soft.min(hard),
    };
    let limit = blocks.saturating_mul(QUOTA_BLOCK_SIZE);
    Some(QuotaFigures {
        limit,
        remaining: limit.saturating_sub(current_space),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: u32 = QIF_BLIMITS | QIF_SPACE;

    #[test]
    fn converts_quota_blocks_to_bytes() {
        let cases = [
            ((0, 0, 0), None),
            (
                (100, 0, 0),
                Some(QuotaFigures {
                    limit: 102_400,
                    remaining: 102_400,
                }),
            ),
            (
                (0, 100, 2_400),
                Some(QuotaFigures {
                    limit: 102_400,
                    remaining: 100_000,
                }),
            ),
            (
                (50, 100, 1_000),
                Some(QuotaFigures {
                    limit: 51_200,
                    remaining: 50_200,
                }),
            ),
            (
                (100, 50, 0),
                Some(QuotaFigures {
                    limit: 51_200,
                    remaining: 51_200,
                }),
            ),
            (
                (10, 20, 20_000),
                Some(QuotaFigures {
                    limit: 10_240,
                    remaining: 0,
                }),
            ),
            (
                (u64::MAX, 0, 1),
                Some(QuotaFigures {
                    limit: u64::MAX,
                    remaining: u64::MAX - 1,
                }),
            ),
        ];
        for ((soft, hard, used), expected) in cases {
            assert_eq!(
                quota_figures(VALID, soft, hard, used),
                expected,
                "soft {soft}, hard {hard}, used {used}"
            );
        }
    }

    #[test]
    fn ignores_result_without_valid_limits_or_usage() {
        assert_eq!(quota_figures(0, 10, 20, 0), None);
        assert_eq!(quota_figures(QIF_BLIMITS, 10, 20, 0), None);
        assert_eq!(quota_figures(QIF_SPACE, 10, 20, 0), None);
        assert!(quota_figures(VALID | 0x3c, 10, 20, 0).is_some());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
