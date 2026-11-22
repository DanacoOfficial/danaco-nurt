// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Choice of the integration step once the fetched heads are known.

use super::settings::{FfPolicy, RebaseKind, Strategy};

/// Relation between `HEAD` and the fetched heads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Relation {
    /// The current branch has no commit yet.
    pub unborn: bool,
    /// Exactly one head was fetched and `HEAD` is its ancestor or equal to
    /// it.
    pub can_fast_forward: bool,
    /// Every fetched head is an ancestor of `HEAD` or equal to it.
    pub up_to_date: bool,
    /// Number of fetched heads to integrate.
    pub heads: usize,
}

/// Integration step of a pull.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Check out the fetched head on the unborn branch (`initial pull`).
    InitialCheckout,
    /// Merge the fetched heads with the given fast-forward policy; covers
    /// the fast-forward and the up-to-date cases, as `git pull` runs
    /// `git merge` for them.
    Merge(FfPolicy),
    /// Rebase the local commits onto the fetched head.
    Rebase(RebaseKind),
}

/// Reason why a pull integrates nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The histories diverged and no mode was chosen.
    Diverged,
    /// The histories diverged and only a fast-forward is allowed.
    NotFastForward,
    /// Several heads cannot be checked out on an unborn branch.
    MultipleIntoUnborn,
    /// Several heads cannot be the base of a rebase.
    RebaseMultiple,
    /// Several heads cannot be fast-forwarded to.
    FastForwardMultiple,
}

/// Chooses the integration step as `git pull` 2.53 does.
///
/// A fast-forward-only policy takes precedence over a rebase setting; a
/// rebase that can be done by a fast-forward is a fast-forward merge.
///
/// # Errors
///
/// [`Refusal`] when the pull must stop after the transfer.
///
/// # Examples
///
/// ```
/// use nurt_git_ops::pull::{Action, FfPolicy, RebaseKind, Refusal, Relation, Strategy, decide};
///
/// let strategy = Strategy {
///     rebase: RebaseKind::No,
///     ff: None,
///     rebase_unspecified: true,
///     autostash: false,
/// };
/// let diverged = Relation { unborn: false, can_fast_forward: false, up_to_date: false, heads: 1 };
/// assert_eq!(decide(&strategy, &diverged), Err(Refusal::Diverged));
/// let behind = Relation { can_fast_forward: true, ..diverged };
/// assert_eq!(decide(&strategy, &behind), Ok(Action::Merge(FfPolicy::Allow)));
/// ```
pub fn decide(strategy: &Strategy, relation: &Relation) -> Result<Action, Refusal> {
    if relation.unborn {
        return if relation.heads > 1 {
            Err(Refusal::MultipleIntoUnborn)
        } else {
            Ok(Action::InitialCheckout)
        };
    }
    if relation.heads > 1 {
        if strategy.rebase != RebaseKind::No {
            return Err(Refusal::RebaseMultiple);
        }
        if strategy.ff == Some(FfPolicy::Only) {
            return Err(Refusal::FastForwardMultiple);
        }
    }
    let diverged = !relation.can_fast_forward && !relation.up_to_date;
    let mut rebase = strategy.rebase;
    if strategy.ff == Some(FfPolicy::Only) {
        if diverged {
            return Err(Refusal::NotFastForward);
        }
        rebase = RebaseKind::No;
    }
    if strategy.ff.is_none() && strategy.rebase_unspecified && diverged {
        return Err(Refusal::Diverged);
    }
    if rebase == RebaseKind::No {
        return Ok(Action::Merge(strategy.ff.unwrap_or(FfPolicy::Allow)));
    }
    if relation.can_fast_forward {
        Ok(Action::Merge(FfPolicy::Only))
    } else {
        Ok(Action::Rebase(rebase))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn strategy(rebase: RebaseKind, ff: Option<FfPolicy>, unspecified: bool) -> Strategy {
        Strategy {
            rebase,
            ff,
            rebase_unspecified: unspecified,
            autostash: false,
        }
    }

    const UNSPECIFIED: Strategy = strategy(RebaseKind::No, None, true);

    const fn relation(can_fast_forward: bool, up_to_date: bool) -> Relation {
        Relation {
            unborn: false,
            can_fast_forward,
            up_to_date,
            heads: 1,
        }
    }

    const DIVERGED: Relation = relation(false, false);
    const BEHIND: Relation = relation(true, false);
    const AHEAD: Relation = relation(false, true);
    const EQUAL: Relation = relation(true, true);

    #[test]
    fn fast_forwards_without_configuration() {
        assert_eq!(
            decide(&UNSPECIFIED, &BEHIND),
            Ok(Action::Merge(FfPolicy::Allow))
        );
        assert_eq!(
            decide(&UNSPECIFIED, &EQUAL),
            Ok(Action::Merge(FfPolicy::Allow))
        );
        assert_eq!(
            decide(&UNSPECIFIED, &AHEAD),
            Ok(Action::Merge(FfPolicy::Allow))
        );
    }

    #[test]
    fn refuses_diverged_histories_without_a_mode() {
        assert_eq!(decide(&UNSPECIFIED, &DIVERGED), Err(Refusal::Diverged));
    }

    #[test]
    fn merges_diverged_histories_when_pull_ff_is_set() {
        for ff in [FfPolicy::Allow, FfPolicy::Never] {
            let configured = strategy(RebaseKind::No, Some(ff), true);
            assert_eq!(decide(&configured, &DIVERGED), Ok(Action::Merge(ff)));
        }
        let merge = strategy(RebaseKind::No, None, false);
        assert_eq!(
            decide(&merge, &DIVERGED),
            Ok(Action::Merge(FfPolicy::Allow))
        );
    }

    #[test]
    fn fast_forward_only_wins_over_rebase() {
        let only = strategy(RebaseKind::Plain, Some(FfPolicy::Only), false);
        assert_eq!(decide(&only, &DIVERGED), Err(Refusal::NotFastForward));
        assert_eq!(decide(&only, &BEHIND), Ok(Action::Merge(FfPolicy::Only)));
        assert_eq!(decide(&only, &AHEAD), Ok(Action::Merge(FfPolicy::Only)));
    }

    #[test]
    fn rebases_unless_a_fast_forward_suffices() {
        let rebase = strategy(RebaseKind::Plain, Some(FfPolicy::Never), false);
        assert_eq!(
            decide(&rebase, &DIVERGED),
            Ok(Action::Rebase(RebaseKind::Plain))
        );
        assert_eq!(decide(&rebase, &BEHIND), Ok(Action::Merge(FfPolicy::Only)));
        assert_eq!(decide(&rebase, &EQUAL), Ok(Action::Merge(FfPolicy::Only)));
        assert_eq!(
            decide(&rebase, &AHEAD),
            Ok(Action::Rebase(RebaseKind::Plain))
        );
        let merges = strategy(RebaseKind::Merges, None, false);
        assert_eq!(
            decide(&merges, &DIVERGED),
            Ok(Action::Rebase(RebaseKind::Merges))
        );
    }

    #[test]
    fn checks_out_the_head_on_an_unborn_branch() {
        let unborn = Relation {
            unborn: true,
            ..DIVERGED
        };
        assert_eq!(decide(&UNSPECIFIED, &unborn), Ok(Action::InitialCheckout));
        let several = Relation { heads: 2, ..unborn };
        assert_eq!(
            decide(&UNSPECIFIED, &several),
            Err(Refusal::MultipleIntoUnborn)
        );
    }

    #[test]
    fn refuses_several_heads_where_git_does() {
        let several = Relation {
            heads: 2,
            ..DIVERGED
        };
        let rebase = strategy(RebaseKind::Plain, None, false);
        assert_eq!(decide(&rebase, &several), Err(Refusal::RebaseMultiple));
        let only = strategy(RebaseKind::No, Some(FfPolicy::Only), false);
        assert_eq!(decide(&only, &several), Err(Refusal::FastForwardMultiple));
        let merge = strategy(RebaseKind::No, None, false);
        assert_eq!(decide(&merge, &several), Ok(Action::Merge(FfPolicy::Allow)));
    }
}
