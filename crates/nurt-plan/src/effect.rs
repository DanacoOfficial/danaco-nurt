// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Effect class and reversibility of stages, pipelines and plans.

use nurt_contracts::bound::{BoundExpr, BoundStage};
use nurt_contracts::kinds::{EffectKind, Reversibility};
use nurt_contracts::spec::StageSpec;

use crate::model::PlannedStage;

/// Strength of an effect in the declaration order of [`EffectKind`].
fn effect_rank(effect: EffectKind) -> u8 {
    match effect {
        EffectKind::Read => 0,
        EffectKind::Local => 1,
        EffectKind::External => 2,
        EffectKind::Process => 3,
    }
}

/// Weakness of reversibility: a higher rank is harder to reverse.
fn reversibility_rank(reversibility: Reversibility) -> u8 {
    match reversibility {
        Reversibility::Read => 0,
        Reversibility::Reversible => 1,
        Reversibility::ReversibleIfPreserved => 2,
        Reversibility::Irreversible => 3,
    }
}

/// Effect class accumulated over stages: the strongest effect and the
/// weakest reversibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EffectClass {
    pub(crate) effect: EffectKind,
    pub(crate) reversibility: Reversibility,
}

impl EffectClass {
    pub(crate) const READ: Self = Self {
        effect: EffectKind::Read,
        reversibility: Reversibility::Read,
    };

    pub(crate) fn join(self, effect: EffectKind, reversibility: Reversibility) -> Self {
        Self {
            effect: if effect_rank(effect) > effect_rank(self.effect) {
                effect
            } else {
                self.effect
            },
            reversibility: if reversibility_rank(reversibility)
                > reversibility_rank(self.reversibility)
            {
                reversibility
            } else {
                self.reversibility
            },
        }
    }

    /// Class of planned stages, including the pipelines of `verify` stages.
    pub(crate) fn of_stages(stages: &[PlannedStage]) -> Self {
        stages.iter().fold(Self::READ, |class, stage| {
            let class = class.join(stage.stage.effect, stage.reversibility);
            match stage.check.as_deref() {
                Some(check) => class.join(check.effect, check.reversibility),
                None => class,
            }
        })
    }
}

/// Reversibility used when the catalog does not describe the effect of the
/// stage.
fn default_reversibility(effect: EffectKind) -> Reversibility {
    match effect {
        EffectKind::Read => Reversibility::Read,
        EffectKind::Local => Reversibility::Reversible,
        EffectKind::External => Reversibility::Irreversible,
        EffectKind::Process => Reversibility::ReversibleIfPreserved,
    }
}

/// Returns `true` when the bound arguments select the catalog action
/// `action`: an enumeration argument has that value, or the parameter of
/// that name was given a value.
fn selects_action(stage: &BoundStage, action: &str) -> bool {
    stage.args.iter().any(|arg| {
        let enum_value = matches!(&arg.value, BoundExpr::Enum { v } if v == action);
        let given = arg.name == action && !arg.default && arg.value != BoundExpr::Null;
        enum_value || given
    })
}

/// Reversibility of a bound stage according to the catalog.
///
/// The effect recorded by binding already reflects the selected action; the
/// reversibility comes from the action of the catalog with that effect, then
/// from the command, then from the effect itself.
pub(crate) fn stage_reversibility(stage: &BoundStage, spec: &StageSpec) -> Reversibility {
    if stage.effect == EffectKind::Read && !stage.phase.is_effect() {
        return Reversibility::Read;
    }
    let action = spec
        .effects
        .iter()
        .find(|action| action.effect == stage.effect && selects_action(stage, &action.action));
    if let Some(action) = action {
        return action.reversibility;
    }
    if spec.effect == stage.effect {
        return spec.reversibility;
    }
    default_reversibility(stage.effect)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "tests fail by panicking"
    )]

    use nurt_contracts::bound::{BoundArg, Phase};
    use nurt_contracts::kinds::{Category, Implementation};
    use nurt_contracts::spec::{ActionEffect, IoSpec};

    use super::*;

    fn spec(effect: EffectKind, reversibility: Reversibility) -> StageSpec {
        StageSpec {
            name: "http.request".to_owned(),
            group: String::new(),
            category: Category::Change,
            reversibility,
            effect,
            implementation: Implementation::Builtin,
            input: vec![IoSpec::None],
            output: vec![IoSpec::Any],
            summary: "Sends a request.".to_owned(),
            reversibility_note: None,
            effects: vec![ActionEffect {
                action: "post".to_owned(),
                effect: EffectKind::External,
                reversibility: Reversibility::Irreversible,
                note: None,
            }],
            waits: false,
            os: Vec::new(),
            external: None,
            fallback: None,
            required_groups: Vec::new(),
            params: Vec::new(),
            forms: Vec::new(),
        }
    }

    fn stage(effect: EffectKind, phase: Phase, args: Vec<BoundArg>) -> BoundStage {
        BoundStage {
            name: "http.request".to_owned(),
            effect,
            phase,
            waits: false,
            args,
            span: None,
        }
    }

    #[test]
    fn join_keeps_strongest_effect_and_weakest_reversibility() {
        let class = EffectClass::READ
            .join(EffectKind::Process, Reversibility::Reversible)
            .join(EffectKind::Local, Reversibility::Irreversible);
        assert_eq!(class.effect, EffectKind::Process);
        assert_eq!(class.reversibility, Reversibility::Irreversible);
        let external = EffectClass::READ.join(EffectKind::External, Reversibility::Read);
        assert_eq!(
            external.join(EffectKind::Local, Reversibility::ReversibleIfPreserved),
            EffectClass {
                effect: EffectKind::External,
                reversibility: Reversibility::ReversibleIfPreserved,
            }
        );
    }

    #[test]
    fn reading_stage_is_read() {
        let spec = spec(EffectKind::Read, Reversibility::Read);
        let stage = stage(EffectKind::Read, Phase::Source, Vec::new());
        assert_eq!(stage_reversibility(&stage, &spec), Reversibility::Read);
    }

    #[test]
    fn selected_action_gives_its_reversibility() {
        let spec = spec(EffectKind::Read, Reversibility::Read);
        let method = BoundArg::given(
            "method",
            BoundExpr::Enum {
                v: "post".to_owned(),
            },
        );
        let stage = stage(EffectKind::External, Phase::Mutate, vec![method]);
        assert_eq!(
            stage_reversibility(&stage, &spec),
            Reversibility::Irreversible
        );
    }

    #[test]
    fn command_reversibility_applies_to_its_own_effect() {
        let spec = spec(EffectKind::Local, Reversibility::ReversibleIfPreserved);
        let stage = stage(EffectKind::Local, Phase::Mutate, Vec::new());
        assert_eq!(
            stage_reversibility(&stage, &spec),
            Reversibility::ReversibleIfPreserved
        );
    }

    #[test]
    fn unknown_effect_falls_back_to_effect_default() {
        let spec = spec(EffectKind::Read, Reversibility::Read);
        let process = stage(EffectKind::Process, Phase::Run, Vec::new());
        assert_eq!(
            stage_reversibility(&process, &spec),
            Reversibility::ReversibleIfPreserved
        );
        let local = stage(EffectKind::Local, Phase::Mutate, Vec::new());
        assert_eq!(
            stage_reversibility(&local, &spec),
            Reversibility::Reversible
        );
        let external = stage(EffectKind::External, Phase::Mutate, Vec::new());
        assert_eq!(
            stage_reversibility(&external, &spec),
            Reversibility::Irreversible
        );
    }

    #[test]
    fn given_parameter_selects_action_of_its_name() {
        let mut spec = spec(EffectKind::Read, Reversibility::Read);
        spec.effects[0].action = "save".to_owned();
        spec.effects[0].effect = EffectKind::Local;
        spec.effects[0].reversibility = Reversibility::Reversible;
        let save = BoundArg::given(
            "save",
            BoundExpr::Path {
                v: "/tmp/page.html".to_owned(),
            },
        );
        let stage = stage(EffectKind::Local, Phase::Mutate, vec![save]);
        assert_eq!(
            stage_reversibility(&stage, &spec),
            Reversibility::Reversible
        );
    }
}
