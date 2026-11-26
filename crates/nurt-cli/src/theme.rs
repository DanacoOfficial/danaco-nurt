// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Colours of the identity in the terminal.
//!
//! The mark has two colours, silver `#C3C8CE` for the step back and gold
//! `#D4AE6E` for the step forward; the blue of the product has the cyan
//! `#4FD1F5`. A terminal that reports 24-bit colour shows exactly these
//! values. Any other terminal gets the nearest colours of the 256-colour
//! palette.

use reedline::Color;

const SILVER: (u8, u8, u8) = (0xC3, 0xC8, 0xCE);
const GOLD: (u8, u8, u8) = (0xD4, 0xAE, 0x6E);
const CYAN: (u8, u8, u8) = (0x4F, 0xD1, 0xF5);
const SILVER_INDEX: u8 = 251;
const GOLD_INDEX: u8 = 179;
const CYAN_INDEX: u8 = 81;

/// Terminal programs that show 24-bit colour and do not always say so with
/// the variable `COLORTERM`.
const TRUECOLOR_PROGRAMS: [&str; 5] = ["vscode", "iTerm.app", "WezTerm", "ghostty", "Hyper"];

/// The colours of the identity for one terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    truecolor: bool,
}

impl Theme {
    /// The theme of the terminal of this process, from its environment. On
    /// Windows the terminal shows 24-bit colour.
    #[must_use]
    pub fn detect() -> Self {
        Self::from_environment(|name| std::env::var(name).ok())
    }

    /// The theme for the environment given by `variable`.
    #[must_use]
    pub fn from_environment(variable: impl Fn(&str) -> Option<String>) -> Self {
        Self::new(cfg!(windows) || reports_truecolor(&variable))
    }

    /// The theme with 24-bit colour or with the 256-colour palette.
    #[must_use]
    pub fn new(truecolor: bool) -> Self {
        Self { truecolor }
    }

    /// Silver, the step back; the first two letters of the banner.
    #[must_use]
    pub fn silver(self) -> Color {
        self.colour(SILVER, SILVER_INDEX)
    }

    /// Gold, the step forward; the last two letters of the banner.
    #[must_use]
    pub fn gold(self) -> Color {
        self.colour(GOLD, GOLD_INDEX)
    }

    /// Cyan of the blue of the product.
    #[must_use]
    pub fn cyan(self) -> Color {
        self.colour(CYAN, CYAN_INDEX)
    }

    /// The escape sequence that selects silver as the foreground colour.
    #[must_use]
    pub fn silver_escape(self) -> String {
        self.escape(SILVER, SILVER_INDEX)
    }

    /// The escape sequence that selects gold as the foreground colour.
    #[must_use]
    pub fn gold_escape(self) -> String {
        self.escape(GOLD, GOLD_INDEX)
    }

    fn colour(self, rgb: (u8, u8, u8), index: u8) -> Color {
        if self.truecolor {
            Color::Rgb(rgb.0, rgb.1, rgb.2)
        } else {
            Color::Fixed(index)
        }
    }

    fn escape(self, rgb: (u8, u8, u8), index: u8) -> String {
        if self.truecolor {
            format!("\x1b[38;2;{};{};{}m", rgb.0, rgb.1, rgb.2)
        } else {
            format!("\x1b[38;5;{index}m")
        }
    }
}

/// Whether the environment says that the terminal shows 24-bit colour.
fn reports_truecolor(variable: &impl Fn(&str) -> Option<String>) -> bool {
    let colorterm = variable("COLORTERM")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if colorterm == "truecolor" || colorterm == "24bit" {
        return true;
    }
    if variable("WT_SESSION").is_some() {
        return true;
    }
    let program = variable("TERM_PROGRAM").unwrap_or_default();
    TRUECOLOR_PROGRAMS.contains(&program.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RGB value of an entry of the 256-colour palette from 16 upwards.
    fn palette(index: u8) -> (i32, i32, i32) {
        const LEVELS: [i32; 6] = [0, 95, 135, 175, 215, 255];
        if index >= 232 {
            let grey = 8 + 10 * (i32::from(index) - 232);
            return (grey, grey, grey);
        }
        let cube = i32::from(index) - 16;
        let level = |part: i32| LEVELS[usize::try_from(part).unwrap_or(0)];
        (level(cube / 36), level(cube / 6 % 6), level(cube % 6))
    }

    #[test]
    fn palette_indexes_are_the_nearest_entries_to_the_brand_colours() {
        for (rgb, index) in [
            (SILVER, SILVER_INDEX),
            (GOLD, GOLD_INDEX),
            (CYAN, CYAN_INDEX),
        ] {
            let distance = |candidate: u8| {
                let (r, g, b) = palette(candidate);
                let (tr, tg, tb) = (i32::from(rgb.0), i32::from(rgb.1), i32::from(rgb.2));
                (r - tr).pow(2) + (g - tg).pow(2) + (b - tb).pow(2)
            };
            let nearest = (16..=255_u8).min_by_key(|candidate| distance(*candidate));
            assert_eq!(Some(index), nearest, "{rgb:?}");
        }
    }

    fn environment<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn truecolor_is_reported_by_colorterm_by_windows_terminal_and_by_known_programs() {
        assert!(reports_truecolor(&environment(&[(
            "COLORTERM",
            "truecolor"
        )])));
        assert!(reports_truecolor(&environment(&[("COLORTERM", "24BIT")])));
        assert!(reports_truecolor(&environment(&[("WT_SESSION", "1")])));
        assert!(reports_truecolor(&environment(&[(
            "TERM_PROGRAM",
            "vscode"
        )])));
        assert!(!reports_truecolor(&environment(&[(
            "TERM",
            "xterm-256color"
        )])));
        assert!(!reports_truecolor(&environment(&[])));
    }

    #[test]
    fn theme_of_an_environment_follows_the_variables() {
        let truecolor = Theme::from_environment(|name| {
            (name == "COLORTERM").then(|| "truecolor".to_owned())
        });
        assert_eq!(truecolor, Theme::new(true));
        let plain = Theme::from_environment(|_| None);
        assert_eq!(plain, Theme::new(cfg!(windows)));
        let unknown = Theme::from_environment(|name| {
            (name == "TERM_PROGRAM").then(|| "Apple_Terminal".to_owned())
        });
        assert_eq!(unknown, Theme::new(cfg!(windows)));
    }

    #[test]
    fn colorterm_is_compared_without_regard_to_case_and_exactly() {
        let with = |value: &str| {
            let value = value.to_owned();
            move |name: &str| (name == "COLORTERM").then(|| value.clone())
        };
        assert!(reports_truecolor(&with("TrueColor")));
        assert!(reports_truecolor(&with("24bit")));
        assert!(!reports_truecolor(&with("yes")));
        assert!(!reports_truecolor(&with("")));
    }

    #[test]
    fn every_known_program_reports_truecolor() {
        for program in TRUECOLOR_PROGRAMS {
            assert!(
                reports_truecolor(&environment(&[("TERM_PROGRAM", program)])),
                "{program}"
            );
        }
        assert!(!reports_truecolor(&environment(&[("TERM_PROGRAM", "vscode2")])));
    }

    #[test]
    fn silver_and_cyan_have_exact_and_palette_forms() {
        assert_eq!(Theme::new(true).cyan(), Color::Rgb(0x4F, 0xD1, 0xF5));
        assert_eq!(Theme::new(true).silver_escape(), "\x1b[38;2;195;200;206m");
        assert_eq!(Theme::new(false).silver_escape(), "\x1b[38;5;251m");
    }

    #[test]
    fn truecolor_theme_uses_the_exact_colours_of_the_mark() {
        let theme = Theme::new(true);
        assert_eq!(theme.silver(), Color::Rgb(0xC3, 0xC8, 0xCE));
        assert_eq!(theme.gold(), Color::Rgb(0xD4, 0xAE, 0x6E));
        assert_eq!(theme.gold_escape(), "\x1b[38;2;212;174;110m");
        assert_eq!(theme.silver_escape(), "\x1b[38;2;195;200;206m");
    }

    #[test]
    fn other_terminals_get_the_nearest_colours_of_the_256_palette() {
        let theme = Theme::new(false);
        assert_eq!(theme.silver(), Color::Fixed(251));
        assert_eq!(theme.gold(), Color::Fixed(179));
        assert_eq!(theme.cyan(), Color::Fixed(81));
        assert_eq!(theme.gold_escape(), "\x1b[38;5;179m");
    }
}
