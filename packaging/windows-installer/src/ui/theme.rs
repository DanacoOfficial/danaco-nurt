// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/ui/theme.rs
// Responsibility:   Tokens of the wizard: colours, geometry and motion of the design, the embedded faces, and the colours of the high-contrast mode.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tokens of the wizard.
//!
//! The values are those of section 4 of the design (`design/installer/installer.md`), which
//! are derived from the production identity 1.0.0 (`branding/tokens.css`). A value that is
//! not in the identity or in the design is not allowed in the wizard.

use eframe::egui::{
    Color32, Context, FontData, FontDefinitions, FontFamily, FontId, FontTweak,
    epaint::text::VariationCoords,
};

use crate::sys::SystemColors;

/// Colours of the wizard.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Palette {
    /// Ground of the window and of the main area.
    pub(crate) graphite: Color32,
    /// Rail, cards, fields.
    pub(crate) graphite_soft: Color32,
    /// Ground around the window, log.
    pub(crate) black_deep: Color32,
    /// Passed steps, the left chevron, the indicator of the prompt.
    pub(crate) silver: Color32,
    /// The step in progress, the primary button, selected markers, commands.
    pub(crate) gold: Color32,
    /// The border of the label NEW.
    pub(crate) gold_deep: Color32,
    /// Strings in the log, as in the typed line of nurt.
    pub(crate) cyan: Color32,
    /// Dividers and borders that carry no meaning.
    pub(crate) line: Color32,
    /// The edge of every control.
    pub(crate) control_edge: Color32,
    /// Titles and labels.
    pub(crate) text_strong: Color32,
    /// Running text.
    pub(crate) text_body: Color32,
    /// Leads and notes.
    pub(crate) text_soft: Color32,
    /// Captions and the version line.
    pub(crate) text_muted: Color32,
    /// Errors.
    pub(crate) alert: Color32,
    /// Hover of the primary button.
    pub(crate) gold_hover: Color32,
    /// Hover of a choice.
    pub(crate) choice_hover: Color32,
    /// Text on the primary button.
    pub(crate) on_gold: Color32,
    /// High-contrast mode is on: the colours are those of the system.
    pub(crate) high_contrast: bool,
}

const fn rgb(value: u32) -> Color32 {
    Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

impl Palette {
    /// The colours of the identity.
    pub(crate) const fn identity() -> Self {
        Self {
            graphite: rgb(0x0B0C0E),
            graphite_soft: rgb(0x15171A),
            black_deep: rgb(0x050506),
            silver: rgb(0xDDE1E6),
            gold: rgb(0xC9A15A),
            gold_deep: rgb(0xA3722A),
            cyan: rgb(0x4FD1F5),
            line: rgb(0x23262B),
            control_edge: rgb(0x5E6571),
            text_strong: rgb(0xF6F7F8),
            text_body: rgb(0xC9CED5),
            text_soft: rgb(0x9AA1AA),
            text_muted: rgb(0x7C8390),
            alert: rgb(0xE2897D),
            gold_hover: rgb(0xD6AE66),
            choice_hover: rgb(0x191C20),
            on_gold: rgb(0x0B0C0E),
            high_contrast: false,
        }
    }

    /// The colours of the system in the high-contrast mode: Window, WindowText, Highlight and ButtonFace.
    pub(crate) fn high_contrast(system: SystemColors) -> Self {
        let c = |value: [u8; 3]| Color32::from_rgb(value[0], value[1], value[2]);
        let (window, text, highlight, face, gray) = (
            c(system.window),
            c(system.window_text),
            c(system.highlight),
            c(system.button_face),
            c(system.gray_text),
        );
        Self {
            graphite: window,
            graphite_soft: window,
            black_deep: window,
            silver: text,
            gold: highlight,
            gold_deep: highlight,
            cyan: text,
            line: gray,
            control_edge: text,
            text_strong: text,
            text_body: text,
            text_soft: text,
            text_muted: gray,
            alert: text,
            gold_hover: highlight,
            choice_hover: face,
            on_gold: c(system.highlight_text),
            high_contrast: true,
        }
    }

    /// The palette that the system calls for.
    pub(crate) fn current() -> Self {
        crate::sys::high_contrast().map_or_else(Self::identity, Self::high_contrast)
    }
}

/// Width of the window in logical points.
pub(crate) const WINDOW_W: f32 = 960.0;
/// Height of the window in logical points.
pub(crate) const WINDOW_H: f32 = 600.0;
/// Width of the rail.
pub(crate) const RAIL_W: f32 = 248.0;
/// Height of the caption bar.
pub(crate) const CAPTION_H: f32 = 44.0;
/// Height of the footer.
pub(crate) const FOOTER_H: f32 = 72.0;
/// Margin of the content.
pub(crate) const MARGIN: f32 = 48.0;
/// Cut of the buttons, cards and dialogs: width.
pub(crate) const CUT_X: f32 = 12.0;
/// Cut of the buttons, cards and dialogs: height (slope 3 : 4).
pub(crate) const CUT_Y: f32 = 9.0;

/// Time of a hover or a press, in seconds.
pub(crate) const MOTION_FAST: f32 = 0.120;
/// Time of a dialog or a scrim, in seconds.
pub(crate) const MOTION_BASE: f32 = 0.200;
/// Time of the change of a screen, in seconds.
pub(crate) const MOTION_SLOW: f32 = 0.320;

/// The identity of the face of the text.
pub(crate) fn sans_family() -> FontFamily {
    FontFamily::Name("geist".into())
}

/// The identity of the monospaced face.
pub(crate) fn mono_family() -> FontFamily {
    FontFamily::Name("geist-mono".into())
}

/// Sans face of a size.
pub(crate) fn sans(size: f32) -> FontId {
    FontId::new(size, sans_family())
}

/// Monospaced face of a size.
pub(crate) fn mono(size: f32) -> FontId {
    FontId::new(size, mono_family())
}

/// The variation of the weight of a variable face.
pub(crate) fn weight(value: f32) -> VariationCoords {
    VariationCoords::new([(b"wght", value)])
}

/// Registers the faces Geist and Geist Mono (SIL Open Font License 1.1, subsets embedded in the file).
pub(crate) fn install_fonts(context: &Context) {
    let mut definitions = FontDefinitions::empty();
    let tweak = FontTweak {
        coords: weight(400.0),
        ..FontTweak::default()
    };
    let mut regular = FontData::from_static(include_bytes!("../../assets/fonts/Geist-subset.ttf"));
    regular.tweak = tweak.clone();
    let mut monospaced =
        FontData::from_static(include_bytes!("../../assets/fonts/GeistMono-subset.ttf"));
    monospaced.tweak = tweak;
    definitions
        .font_data
        .insert("geist".to_string(), std::sync::Arc::new(regular));
    definitions
        .font_data
        .insert("geist-mono".to_string(), std::sync::Arc::new(monospaced));
    definitions
        .families
        .insert(sans_family(), vec!["geist".to_string()]);
    definitions
        .families
        .insert(mono_family(), vec!["geist-mono".to_string()]);
    definitions
        .families
        .insert(FontFamily::Proportional, vec!["geist".to_string()]);
    definitions
        .families
        .insert(FontFamily::Monospace, vec!["geist-mono".to_string()]);
    context.set_fonts(definitions);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
