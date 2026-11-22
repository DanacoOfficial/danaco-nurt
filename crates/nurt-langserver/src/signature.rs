// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Signature of the command whose arguments the cursor writes.

use lsp_types::{
    Documentation, ParameterInformation, ParameterLabel, Position, SignatureHelp,
    SignatureInformation,
};

use crate::assist::Request;
use crate::catalog::{self, FullSignature};
use crate::context::{self, Active};
use crate::position::Lines;

/// Returns the signature of the command at `position` with its active
/// parameter: the labelled parameter being written, or the positional
/// parameter of the argument's position.
pub(crate) fn help(request: Request<'_>, position: Position) -> Option<SignatureHelp> {
    let lines = Lines::new(request.text);
    let offset = lines.offset(position, request.encoding);
    let tokens = context::tokens(request.text, request.variant);
    let call = context::call(request.text, &tokens, offset, request.variant)?;
    let spec = request.commands.get(&call.command)?;
    let FullSignature { label, params } = catalog::full_signature(spec, request.variant);
    let active = match &call.active {
        Active::Named(name) => params.iter().position(|(_, param)| param.name == *name),
        Active::Positional(index) => {
            let positional = spec.positional();
            positional.get(*index).and_then(|wanted| {
                params
                    .iter()
                    .position(|(_, param)| param.name == wanted.name)
            })
        }
    };
    let active = active.and_then(|index| u32::try_from(index).ok());
    let measure = |end: usize| {
        request
            .encoding
            .measure(label.get(..end).unwrap_or_default())
    };
    let parameters = params
        .iter()
        .map(|(range, param)| ParameterInformation {
            label: ParameterLabel::LabelOffsets([measure(range.start), measure(range.end)]),
            documentation: Some(Documentation::String(format!(
                "{} ({})",
                param.summary,
                catalog::param_detail(param)
            ))),
        })
        .collect();
    let signature = SignatureInformation {
        label,
        documentation: Some(Documentation::String(spec.summary.clone())),
        parameters: Some(parameters),
        active_parameter: active,
    };
    Some(SignatureHelp {
        signatures: vec![signature],
        active_signature: Some(0),
        active_parameter: active,
    })
}

#[cfg(test)]
mod tests {
    use nurt_syntax::parse::Variant;

    use super::*;
    use crate::catalog::Commands;
    use crate::catalog::tests::sample;
    use crate::position::Encoding;

    fn signature_at(text: &str, variant: Variant, commands: &Commands) -> Option<SignatureHelp> {
        let request = Request {
            text,
            variant,
            encoding: Encoding::Utf16,
            commands,
            help: None,
            diagnostics: &[],
        };
        let character = u32::try_from(text.chars().count()).unwrap();
        help(request, Position::new(0, character))
    }

    #[test]
    fn shows_the_signature_with_the_positional_parameter() {
        let commands = sample();
        let found = signature_at("sort ", Variant::A, &commands).unwrap();
        let signature = &found.signatures[0];
        assert_eq!(signature.label, "sort <by> [order: <enum(asc|desc)>]");
        assert_eq!(found.active_parameter, Some(0));
        let Some(parameters) = &signature.parameters else {
            panic!("expected parameters")
        };
        assert_eq!(parameters[1].label, ParameterLabel::LabelOffsets([10, 35]));
        assert_eq!(
            signature_at("sort size ", Variant::A, &commands)
                .unwrap()
                .active_parameter,
            None
        );
    }

    #[test]
    fn shows_the_labelled_parameter_being_written() {
        let commands = sample();
        let named = signature_at("sort(size, order: ", Variant::B, &commands).unwrap();
        assert_eq!(
            named.signatures[0].label,
            "sort(<by>, [order: <enum(asc|desc)>])"
        );
        assert_eq!(named.active_parameter, Some(1));
        let unknown = signature_at("sort size limit: ", Variant::A, &commands).unwrap();
        assert_eq!(unknown.active_parameter, None);
    }

    #[test]
    fn shows_nothing_outside_the_arguments_of_a_known_command() {
        let commands = sample();
        assert!(signature_at("sort", Variant::A, &commands).is_none());
        assert!(signature_at("frobnicate ", Variant::A, &commands).is_none());
        assert!(signature_at("count |> ", Variant::A, &commands).is_none());
    }
}
