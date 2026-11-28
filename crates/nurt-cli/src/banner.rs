// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-cli::banner
// Responsibility:   The start banner of an interactive session.
// Dependencies:     nurt_contracts, nurt_core
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! The start banner of an interactive session.
//!
//! The banner shows the mark of the product in its two colours, silver for the
//! left chevron and gold for the right one, with the text of the product
//! beside it. The drawing is made of half blocks on a UTF-8 terminal and of
//! 7-bit ASCII on any other terminal. The colour is chosen per cell from the
//! role of the cell, so a row never depends on a column of the drawing. A
//! session whose styling is disabled gets the glyphs only.

use nurt_contracts::product::{self, BannerDrawing, BannerLine, BannerRole};

use crate::theme::Theme;

/// Width in columns assumed for a terminal whose size cannot be read.
const DEFAULT_WIDTH: usize = 80;

/// Empty columns left of the banner of a session, as in the terminal of the brand book: the
/// drawing does not touch the edge of the window.
pub const MARGIN_COLUMNS: usize = 3;

/// Empty lines above and below the banner of a session.
pub const MARGIN_ROWS: usize = 1;

/// Escape sequence that ends every colour and weight.
const RESET: &str = "\x1b[0m";

/// Escape sequence of the bold weight.
const BOLD: &str = "\x1b[1m";

/// Whether the terminal described by `variable` shows UTF-8.
///
/// On Windows that is Windows Terminal (`WT_SESSION`), a console whose output
/// code page is UTF-8, or a console that processes the escape sequences (the
/// process turns that on at the start); elsewhere it is the locale.
#[must_use]
pub fn utf8_terminal(variable: &impl Fn(&str) -> Option<String>) -> bool {
    if cfg!(windows) {
        windows_utf8(
            variable,
            nurt_core::system::console_output_is_utf8()
                || nurt_core::system::console_virtual_terminal_is_on(),
        )
    } else {
        locale_is_utf8(variable)
    }
}

/// Whether the Windows terminal shows UTF-8: Windows Terminal, or a console
/// that shows it (`console_is_utf8`). The standard library writes the text of a
/// console as UTF-16, so a console of the current Windows shows UTF-8 text
/// whatever its code page is; a console that does not process the escape
/// sequences is an old one and gets the ASCII drawing.
fn windows_utf8(variable: &impl Fn(&str) -> Option<String>, console_is_utf8: bool) -> bool {
    variable("WT_SESSION").is_some() || console_is_utf8
}

/// Whether the character set of the locale is UTF-8. The first of `LC_ALL`,
/// `LC_CTYPE` and `LANG` that is set and not empty decides, as the locale
/// rules of the system do.
fn locale_is_utf8(variable: &impl Fn(&str) -> Option<String>) -> bool {
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .into_iter()
        .find_map(|name| variable(name).filter(|value| !value.is_empty()))
        .is_some_and(|locale| {
            let locale = locale.to_ascii_lowercase();
            locale.contains("utf-8") || locale.contains("utf8")
        })
}

/// The drawing for a terminal: half blocks with UTF-8, ASCII otherwise.
#[must_use]
pub fn drawing_for(utf8: bool) -> BannerDrawing {
    if utf8 {
        BannerDrawing::HalfBlocks
    } else {
        BannerDrawing::Ascii
    }
}

/// The width of the terminal in columns, or a default when it cannot be read.
#[must_use]
pub fn terminal_width() -> usize {
    crossterm::terminal::size().map_or(DEFAULT_WIDTH, |(columns, _)| usize::from(columns))
}

/// The escape sequence that starts a role, or `None` for plain text.
fn start_of(role: BannerRole, theme: Theme) -> Option<String> {
    match role {
        BannerRole::Plain => None,
        BannerRole::Silver => Some(theme.silver_escape()),
        BannerRole::Gold => Some(theme.gold_escape()),
        BannerRole::Name => Some(BOLD.to_owned()),
    }
}

/// Paints one line. Spaces keep the colour that is active, and a line that
/// has a coloured cell starts with the escape sequence of its first one, so
/// that the indentation and the gaps never open or close a colour.
fn paint_line(line: &BannerLine, theme: Theme) -> String {
    let mut out = String::new();
    let mut active = BannerRole::Plain;
    if let Some((_, role)) = line.iter().find(|(glyph, _)| *glyph != ' ')
        && let Some(start) = start_of(*role, theme)
    {
        out.push_str(&start);
        active = *role;
    }
    for (glyph, role) in line {
        if *glyph != ' ' && *role != active {
            if active != BannerRole::Plain {
                out.push_str(RESET);
            }
            if let Some(start) = start_of(*role, theme) {
                out.push_str(&start);
            }
            active = *role;
        }
        out.push(*glyph);
    }
    if active != BannerRole::Plain {
        out.push_str(RESET);
    }
    out
}

/// The banner as it is written to the terminal, without a final line break.
/// With `theme` of `None` the lines carry the glyphs only.
#[must_use]
pub fn render(version: &str, drawing: BannerDrawing, width: usize, theme: Option<Theme>) -> String {
    product::banner_lines(version, drawing, Some(width))
        .iter()
        .map(|line| match theme {
            Some(theme) => paint_line(line, theme),
            None => line.iter().map(|(glyph, _)| *glyph).collect(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The banner `text` moved [`MARGIN_COLUMNS`] columns right, with [`MARGIN_ROWS`] empty lines
/// above and below it; empty lines stay empty.
#[must_use]
pub fn with_margin(text: &str) -> String {
    let pad = " ".repeat(MARGIN_COLUMNS);
    let body = text
        .lines()
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("{pad}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let rows = "\n".repeat(MARGIN_ROWS);
    format!("{rows}{body}{rows}")
}

/// The banner for the terminal of this process: the drawing follows the
/// character set, the colours follow `theme` when `colour` is true, and the
/// layout follows the width of the terminal less the margin.
#[must_use]
pub fn session_banner(
    version: &str,
    variable: &impl Fn(&str) -> Option<String>,
    colour: bool,
    theme: Theme,
) -> String {
    let width = terminal_width();
    with_margin(&render(
        version,
        drawing_for(utf8_terminal(variable)),
        width.saturating_sub(MARGIN_COLUMNS),
        colour.then_some(theme),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSION: &str = "1.2.3";

    fn environment(
        pairs: &'static [(&'static str, &'static str)],
    ) -> impl Fn(&str) -> Option<String> {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn locale_with_utf8_selects_half_blocks() {
        for locale in ["en_US.UTF-8", "pl_PL.utf8", "C.UTF-8", "en_US.utf-8"] {
            let value = locale.to_owned();
            let variable = move |name: &str| (name == "LANG").then(|| value.clone());
            assert!(locale_is_utf8(&variable), "{locale}");
        }
    }

    #[test]
    fn locale_without_utf8_selects_ascii() {
        assert!(!locale_is_utf8(&environment(&[("LANG", "C")])));
        assert!(!locale_is_utf8(&environment(&[(
            "LANG",
            "pl_PL.ISO-8859-2"
        )])));
        assert!(!locale_is_utf8(&environment(&[])));
        assert_eq!(drawing_for(false), BannerDrawing::Ascii);
        assert_eq!(drawing_for(true), BannerDrawing::HalfBlocks);
    }

    #[test]
    fn locale_variables_are_read_in_the_order_of_the_system() {
        let utf8_all = environment(&[("LC_ALL", "en_US.UTF-8"), ("LANG", "C")]);
        assert!(locale_is_utf8(&utf8_all));
        let c_all = environment(&[("LC_ALL", "C"), ("LANG", "en_US.UTF-8")]);
        assert!(!locale_is_utf8(&c_all));
        let ctype = environment(&[("LC_CTYPE", "en_US.UTF-8"), ("LANG", "C")]);
        assert!(locale_is_utf8(&ctype));
        let empty_all = environment(&[("LC_ALL", ""), ("LANG", "en_US.UTF-8")]);
        assert!(locale_is_utf8(&empty_all));
    }

    #[test]
    fn windows_terminal_or_a_utf8_console_selects_half_blocks() {
        assert!(windows_utf8(&environment(&[("WT_SESSION", "x")]), false));
        assert!(windows_utf8(&environment(&[]), true));
        assert!(!windows_utf8(&environment(&[]), false));
    }

    #[test]
    fn every_drawing_row_starts_with_the_escape_of_its_first_colour() {
        for (theme, silver, gold) in [
            (
                Theme::new(true),
                "\x1b[38;2;221;225;230m",
                "\x1b[38;2;201;161;90m",
            ),
            (Theme::new(false), "\x1b[38;5;254m", "\x1b[38;5;179m"),
        ] {
            for drawing in [BannerDrawing::HalfBlocks, BannerDrawing::Ascii] {
                let painted = render(VERSION, drawing, 200, Some(theme));
                let (rows, silver_first) = if drawing == BannerDrawing::HalfBlocks {
                    (product::BANNER_BLOCKS.len(), 11)
                } else {
                    (product::BANNER_ASCII.len(), 10)
                };
                let lines: Vec<&str> = painted.lines().collect();
                assert_eq!(lines.len(), rows);
                for (index, line) in lines.iter().enumerate() {
                    let first = if index < silver_first { silver } else { gold };
                    assert!(line.starts_with(first), "{drawing:?} {index} {line:?}");
                    if !product::BANNER_TEXT_ROWS.contains(&index) {
                        assert!(line.ends_with(RESET), "{line:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_row_with_both_chevrons_switches_from_silver_to_gold() {
        let theme = Theme::new(true);
        let painted = render(VERSION, BannerDrawing::HalfBlocks, 200, Some(theme));
        let row = painted.lines().nth(4).expect("row 4");
        let silver = theme.silver_escape();
        let gold = theme.gold_escape();
        assert!(row.starts_with(&format!("{silver} \u{2584}")), "{row:?}");
        assert!(row.contains(&format!("{RESET}{gold}\u{2584}")), "{row:?}");
        assert!(row.ends_with(RESET));
    }

    #[test]
    fn the_name_is_bold_and_the_other_text_is_plain() {
        let theme = Theme::new(true);
        let painted = render(VERSION, BannerDrawing::HalfBlocks, 200, Some(theme));
        let lines: Vec<&str> = painted.lines().collect();
        assert!(
            lines[3].contains(&format!("{RESET}{BOLD}Danaco Nurt{RESET}")),
            "{:?}",
            lines[3]
        );
        let tagline = lines[5];
        assert!(
            tagline.contains(&format!("{RESET}Command shell and language")),
            "{tagline:?}"
        );
        assert!(tagline.ends_with("undone"), "{tagline:?}");
        assert!(
            lines[7].contains(&format!("{RESET}nurt {VERSION}")),
            "{:?}",
            lines[7]
        );
    }

    #[test]
    fn styling_disabled_leaves_the_glyphs_only() {
        for drawing in [BannerDrawing::HalfBlocks, BannerDrawing::Ascii] {
            let plain = render(VERSION, drawing, 200, None);
            assert!(!plain.contains('\x1b'));
            let lines = product::banner_lines(VERSION, drawing, Some(200));
            let expected: Vec<String> = lines
                .iter()
                .map(|line| line.iter().map(|(glyph, _)| *glyph).collect())
                .collect();
            assert_eq!(plain, expected.join("\n"));
        }
        assert_eq!(
            render(VERSION, BannerDrawing::HalfBlocks, 200, None),
            product::banner(VERSION)
        );
    }

    #[test]
    fn the_session_banner_keeps_a_margin_from_the_edge() {
        let plain = render(VERSION, BannerDrawing::HalfBlocks, 200, None);
        let framed = with_margin(&plain);
        let lines: Vec<&str> = framed.split('\n').collect();
        assert_eq!(lines.len(), plain.lines().count() + 2 * MARGIN_ROWS);
        assert_eq!(lines[0], "");
        assert_eq!(*lines.last().unwrap(), "");
        let pad = " ".repeat(MARGIN_COLUMNS);
        for (line, original) in lines[MARGIN_ROWS..lines.len() - MARGIN_ROWS]
            .iter()
            .zip(plain.lines())
        {
            if original.is_empty() {
                assert_eq!(*line, "");
            } else {
                assert_eq!(*line, format!("{pad}{original}"));
            }
        }
    }

    #[test]
    fn the_ascii_drawing_has_only_seven_bit_characters() {
        let painted = render(VERSION, BannerDrawing::Ascii, 200, Some(Theme::new(true)));
        assert!(painted.is_ascii());
        let plain = render(VERSION, BannerDrawing::Ascii, 200, None);
        assert!(plain.is_ascii());
    }

    #[test]
    fn a_narrow_terminal_gets_the_text_below_the_drawing() {
        let plain = render(VERSION, BannerDrawing::HalfBlocks, 80, None);
        let lines: Vec<&str> = plain.lines().collect();
        assert_eq!(lines.len(), product::BANNER_BLOCKS.len() + 1 + 5);
        assert_eq!(lines[product::BANNER_BLOCKS.len()], "");
        assert_eq!(lines[product::BANNER_BLOCKS.len() + 1], "Danaco Nurt");
        assert_eq!(*lines.last().unwrap(), product::COPYRIGHT_NOTICE);
    }

    #[test]
    fn a_terminal_narrower_than_the_drawing_gets_the_text_only() {
        let painted = render(
            VERSION,
            BannerDrawing::HalfBlocks,
            20,
            Some(Theme::new(false)),
        );
        let lines: Vec<&str> = painted.lines().collect();
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[0], format!("{BOLD}Danaco Nurt{RESET}"));
        assert_eq!(lines[2], format!("nurt {VERSION}"));
        assert!(!painted.contains('\u{2588}'));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
