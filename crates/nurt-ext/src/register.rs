// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Registration of the commands of an extension.
//!
//! An extension contributes a command group named after the extension. The
//! catalog part (signatures) goes to the command registry with
//! [`register`]; the implementations go through the stage registrar of the
//! contract ([`register_stages`]), which [`register`] also calls.

use std::collections::BTreeSet;
use std::sync::Arc;

use nurt_catalog::Registry;
use nurt_contracts::error::Result;
use nurt_contracts::kinds::{Category, EffectKind, Implementation, Reversibility};
use nurt_contracts::spec::{IoSpec, StageSpec};
use nurt_contracts::stage::{StageImpl, StageRegistrar};

use crate::manifest::{Manifest, StageEntry};
use crate::stage::{ANY_KIND, ExtRuntime, ExtensionStage, parse_kind};
use crate::store::InstalledExt;

/// Names an extension may not take: groups, commands and command prefixes
/// of the registry.
#[must_use]
pub fn core_names(registry: &Registry) -> BTreeSet<String> {
    let groups = registry.groups().map(|group| group.name.clone());
    let commands = registry.commands().flat_map(|spec| {
        let prefix = spec.name.split('.').next().unwrap_or_default().to_owned();
        [spec.name.clone(), prefix]
    });
    groups.chain(commands).collect()
}

fn io_list(kinds: &[String]) -> Result<Vec<IoSpec>> {
    kinds
        .iter()
        .map(|kind| {
            if kind == ANY_KIND {
                Ok(IoSpec::Any)
            } else {
                parse_kind(kind).map(|kind| IoSpec::Record(kind.as_str().to_owned()))
            }
        })
        .collect()
}

/// Catalog specification of one command of an extension.
///
/// # Errors
///
/// Fails with `E_CATALOG` for an unknown record kind.
pub fn stage_spec(manifest: &Manifest, entry: &StageEntry) -> Result<StageSpec> {
    let mut input = io_list(&entry.input)?;
    if entry.source {
        input.insert(0, IoSpec::None);
    }
    let mut output = io_list(&entry.output)?;
    if output.is_empty() {
        output.push(IoSpec::Any);
    }
    let (category, effect, reversibility) = if entry.writes {
        (
            Category::Change,
            EffectKind::Local,
            Reversibility::Reversible,
        )
    } else if entry.network {
        (Category::Source, EffectKind::External, Reversibility::Read)
    } else if entry.input.is_empty() {
        (Category::Source, EffectKind::Read, Reversibility::Read)
    } else {
        (Category::Transform, EffectKind::Read, Reversibility::Read)
    };
    let summary = entry.summary.clone().unwrap_or_else(|| {
        format!(
            "Command `{}` of the `{}` extension.",
            entry.name, manifest.ext.name
        )
    });
    Ok(StageSpec {
        name: format!("{}.{}", manifest.ext.name, entry.name),
        group: String::new(),
        category,
        reversibility,
        effect,
        implementation: Implementation::Builtin,
        input,
        output,
        summary,
        reversibility_note: None,
        effects: Vec::new(),
        waits: false,
        os: Vec::new(),
        external: None,
        fallback: None,
        required_groups: Vec::new(),
        params: entry.params.clone(),
        forms: Vec::new(),
    })
}

/// Group summary of an extension.
#[must_use]
pub fn group_summary(manifest: &Manifest) -> String {
    match manifest.ext.description.as_deref() {
        Some(text) if text.trim_end().ends_with('.') => text.trim_end().to_owned(),
        Some(text) => format!("{}.", text.trim_end()),
        None => format!("Commands of the `{}` extension.", manifest.ext.name),
    }
}

/// Adds the stage implementations of `extension`, run by `runtime`, to
/// `registrar`.
///
/// # Errors
///
/// Fails with `E_CATALOG` for an unknown record kind and with the error of
/// the registrar.
pub fn register_stages(
    extension: &InstalledExt,
    runtime: &Arc<dyn ExtRuntime>,
    registrar: &mut dyn StageRegistrar,
) -> Result<()> {
    for entry in &extension.manifest.stage {
        let stage: Arc<dyn StageImpl> =
            Arc::new(ExtensionStage::new(extension, entry, Arc::clone(runtime))?);
        registrar.register(stage)?;
    }
    Ok(())
}

/// Registers the commands of `extension` in `registry`: the command group
/// with the signatures, then the implementations run by `runtime`. Returns
/// the full command names.
///
/// # Errors
///
/// Fails with `E_CATALOG` when the commands do not pass the checks of the
/// catalog or the extension name is taken; nothing is registered then.
pub fn register(
    registry: &mut Registry,
    extension: &InstalledExt,
    runtime: Arc<dyn ExtRuntime>,
) -> Result<Vec<String>> {
    let manifest = &extension.manifest;
    let specs = manifest
        .stage
        .iter()
        .map(|entry| stage_spec(manifest, entry))
        .collect::<Result<Vec<_>>>()?;
    let mut stages: Vec<Arc<dyn StageImpl>> = Vec::new();
    register_stages(extension, &runtime, &mut stages)?;
    registry.register_extension(&manifest.ext.name, &group_summary(manifest), specs)?;
    for stage in stages {
        registry.register_impl(stage)?;
    }
    Ok(manifest.commands())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nurt_catalog::Registry;
    use nurt_contracts::error::{ErrorCode, Result};
    use nurt_contracts::spec::IoSpec;
    use nurt_contracts::stage::ExecCtx;
    use nurt_contracts::value::RecordKind;

    use super::{core_names, group_summary, register, stage_spec};
    use crate::manifest::{HOST_API, Manifest};
    use crate::stage::{ExtRuntime, GuestCall, GuestOutput};
    use crate::store::InstalledExt;

    const MANIFEST: &str = r#"
[ext]
name = "acme"
version = "1.0.0"
api = "nurt:ext@1.0"
kind = "wasm"
wasm = "acme.wasm"
description = "Checks of acme files"

[[stage]]
name = "lint"
summary = "Reports problems of acme files."
input = ["entry"]
output = ["finding"]

[[stage.param]]
name = "strict"
type = "bool"
summary = "Reports style problems too."

[[stage]]
name = "fix"
input = ["entry"]
writes = true

[[stage]]
name = "make"
source = true

[permissions]
write = ["${project}"]
"#;

    struct Idle;

    impl ExtRuntime for Idle {
        fn run(&self, _call: GuestCall<'_>, _ctx: &ExecCtx) -> Result<GuestOutput> {
            Ok(GuestOutput::default())
        }
    }

    fn extension(text: &str) -> InstalledExt {
        InstalledExt {
            manifest: Manifest::parse(text).unwrap(),
            dir: "/ext/acme".into(),
        }
    }

    #[test]
    fn specifications_follow_the_stage_entries() {
        let extension = extension(MANIFEST);
        let manifest = &extension.manifest;
        let lint = stage_spec(manifest, &manifest.stage[0]).unwrap();
        assert_eq!(lint.name, "acme.lint");
        assert_eq!(lint.input, [IoSpec::Record("entry".to_owned())]);
        assert_eq!(lint.output, [IoSpec::Record("finding".to_owned())]);
        assert_eq!(lint.params.len(), 1);
        assert!(lint.check().is_empty(), "{:?}", lint.check());
        let fix = stage_spec(manifest, &manifest.stage[1]).unwrap();
        assert_eq!(fix.output, [IoSpec::Any]);
        assert!(fix.check().is_empty(), "{:?}", fix.check());
        let make = stage_spec(manifest, &manifest.stage[2]).unwrap();
        assert_eq!(make.input, [IoSpec::None]);
        assert!(make.summary.ends_with('.'));
        assert_eq!(group_summary(manifest), "Checks of acme files.");
    }

    #[test]
    fn extensions_register_their_commands_and_implementations() {
        let mut registry = Registry::load_embedded().unwrap();
        let core = core_names(&registry);
        assert!(core.contains("git") && core.contains("sort"));
        let extension = extension(MANIFEST);
        extension.manifest.validate(HOST_API, &core).unwrap();
        let commands = register(&mut registry, &extension, Arc::new(Idle)).unwrap();
        assert_eq!(commands, ["acme.lint", "acme.fix", "acme.make"]);
        assert!(registry.command("acme.lint").is_some());
        assert!(registry.resolve("acme.lint", RecordKind::Entry).is_ok());
        assert!(registry.resolve_source("acme.make").is_ok());
        assert!(registry.validate_implementations().is_empty());

        let again = register(&mut registry, &extension, Arc::new(Idle)).unwrap_err();
        assert_eq!(again.code(), ErrorCode::Catalog);
    }

    #[test]
    fn unknown_kinds_are_rejected_before_registration() {
        let mut registry = Registry::load_embedded().unwrap();
        let extension =
            extension(&MANIFEST.replace("output = [\"finding\"]", "output = [\"nothing\"]"));
        let error = register(&mut registry, &extension, Arc::new(Idle)).unwrap_err();
        assert_eq!(error.code(), ErrorCode::Catalog);
        assert!(registry.group("acme").is_none());
    }
}
