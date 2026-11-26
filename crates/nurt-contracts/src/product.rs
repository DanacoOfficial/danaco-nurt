// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Product texts of Danaco Nurt.
//!
//! Every place that names or describes the product derives its text from
//! this module: the description of the package that builds the `nurt`
//! executable, `nurt -V` and `nurt --version`, the first line of
//! `nurt --help`, the first line of the `help` command, the banner of an
//! interactive session and the `NAME` section of the manual page. The texts
//! contain only ASCII characters; the version is passed in by the caller from
//! the package metadata. The drawing of the mark in half blocks is the one
//! part of the banner that is not ASCII; the banner has a second drawing in
//! 7-bit ASCII for terminals without UTF-8.

/// Name of the product.
pub const PRODUCT_NAME: &str = "Danaco Nurt";

/// Name of the executable.
pub const PROGRAM_NAME: &str = "nurt";

/// Canonical description of the product, one sentence without a final period.
///
/// The `description` of the package that builds the `nurt` executable is
/// exactly this sentence, and the first paragraph of `README.md` contains it.
pub const TAGLINE: &str = "Command shell and language where every change can be undone";

/// Copyright line printed by `nurt --version` and by the session banner.
pub const COPYRIGHT_NOTICE: &str =
    "Copyright (c) 2026 Danaco Holding Group Sp. z o.o. All rights reserved.";

/// License line printed by `nurt --version`.
pub const LICENSE_NOTICE: &str = "License: Apache-2.0";

/// Last line of the session banner.
pub const BANNER_HINT: &str = "Type 'help' to get started.";

/// Mark of the product drawn with half blocks, the main drawing of the session
/// banner: 26 columns and 15 rows, that is 26 x 30 pixels. Rows have no trailing
/// space.
pub const BANNER_BLOCKS: [&str; 15] = [
    "           ▄██",
    "        ▄▄████",
    "      ▄██████▀",
    "   ▄██████▀",
    " ▄█████▀▀   ▄▄▄",
    "██████▄     ████▄",
    " ▀██████▄   ▀██████▄",
    "   ▀▀██████▄  ▀██████▄▄",
    "      ▀██████▄   ▀██████▄",
    "         ▀████     ▀██████",
    "           ▀▀▀   ▄▄█████▀",
    "               ▄██████▀",
    "            ▄██████▀",
    "            ████▀▀",
    "            ██▀",
];

/// Role of every cell of [`BANNER_BLOCKS`]: `s` silver, `g` gold, a space for an
/// empty cell. Each row is as long as its row of the drawing.
pub const BANNER_BLOCKS_ROLES: [&str; 15] = [
    "           sss",
    "        ssssss",
    "      ssssssss",
    "   ssssssss",
    " ssssssss   ggg",
    "sssssss     ggggg",
    " ssssssss   gggggggg",
    "   sssssssss  ggggggggg",
    "      ssssssss   gggggggg",
    "         sssss     ggggggg",
    "           sss   gggggggg",
    "               gggggggg",
    "            gggggggg",
    "            gggggg",
    "            ggg",
];

/// Mark of the product drawn with 7-bit ASCII, the drawing of the session banner
/// for a terminal without UTF-8: 24 columns and 14 rows. Rows have no trailing
/// space.
pub const BANNER_ASCII: [&str; 14] = [
    "          +#+",
    "       :+###+",
    "    :+######:",
    "  +######+",
    "+#####+:   +#+:",
    "######+    +####+",
    " :+######: :+#####+:",
    "    :+#####+: :######+:",
    "       +####+    +######",
    "         :+#+   :+#####+",
    "              +######+",
    "           :######+:",
    "           +###+:",
    "           +#+",
];

/// Role of every cell of [`BANNER_ASCII`], with the same meaning as the roles of
/// [`BANNER_BLOCKS`].
pub const BANNER_ASCII_ROLES: [&str; 14] = [
    "          sss",
    "       ssssss",
    "    sssssssss",
    "  ssssssss",
    "ssssssss   gggg",
    "sssssss    gggggg",
    " sssssssss ggggggggg",
    "    sssssssss ggggggggg",
    "       ssssss    ggggggg",
    "         ssss   gggggggg",
    "              gggggggg",
    "           ggggggggg",
    "           gggggg",
    "           ggg",
];

/// Number of columns of [`BANNER_BLOCKS`].
pub const BANNER_BLOCKS_WIDTH: usize = 26;

/// Number of columns of [`BANNER_ASCII`].
pub const BANNER_ASCII_WIDTH: usize = 24;

/// Number of empty columns between the drawing and the text of the banner.
pub const BANNER_GAP: usize = 3;

/// Rows of the drawing, counted from zero, on which the lines of the banner
/// text stand when the text is beside the drawing.
pub const BANNER_TEXT_ROWS: [usize; 5] = [3, 5, 7, 9, 11];

/// Role of a cell of the banner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerRole {
    /// Text without colour.
    Plain,
    /// The left chevron of the mark, the step back.
    Silver,
    /// The right chevron of the mark, the step forward.
    Gold,
    /// The name of the product, set bold where styling is enabled.
    Name,
}

/// Drawing of the mark in the banner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerDrawing {
    /// Half blocks, for a UTF-8 terminal.
    HalfBlocks,
    /// 7-bit ASCII, for any other terminal.
    Ascii,
}

/// One line of the banner: its characters, each with its role.
pub type BannerLine = Vec<(char, BannerRole)>;

impl BannerDrawing {
    /// The rows of the drawing and the roles of their cells.
    fn rows(self) -> (&'static [&'static str], &'static [&'static str]) {
        match self {
            Self::HalfBlocks => (&BANNER_BLOCKS, &BANNER_BLOCKS_ROLES),
            Self::Ascii => (&BANNER_ASCII, &BANNER_ASCII_ROLES),
        }
    }

    /// The number of columns of the drawing.
    #[must_use]
    pub const fn width(self) -> usize {
        match self {
            Self::HalfBlocks => BANNER_BLOCKS_WIDTH,
            Self::Ascii => BANNER_ASCII_WIDTH,
        }
    }
}

/// Returns the output of `nurt -V`, the program name and the version.
///
/// # Examples
///
/// ```
/// assert_eq!(nurt_contracts::product::version_line("0.1.0"), "nurt 0.1.0");
/// ```
#[must_use]
pub fn version_line(version: &str) -> String {
    format!("{PROGRAM_NAME} {version}")
}

/// Returns the output of `nurt --version` as three lines without a final
/// line break.
///
/// The argument `build` holds the commit and the date of the build. A build
/// without repository data, for example from a source package, passes
/// `None`, and the first line then has no parenthesized part.
///
/// # Examples
///
/// ```
/// let text = nurt_contracts::product::long_version("0.1.0", Some(("1a2b3c4", "2026-11-22")));
/// assert!(text.starts_with("nurt 0.1.0 (1a2b3c4 2026-11-22)\n"));
/// ```
#[must_use]
pub fn long_version(version: &str, build: Option<(&str, &str)>) -> String {
    let first = match build {
        Some((commit, date)) => format!("{} ({commit} {date})", version_line(version)),
        None => version_line(version),
    };
    format!("{first}\n{COPYRIGHT_NOTICE}\n{LICENSE_NOTICE}")
}

/// Returns the first line of `nurt --help`, which is the canonical sentence
/// with a final period.
#[must_use]
pub fn help_summary() -> String {
    format!("{TAGLINE}.")
}

/// Returns the first line of the `help` command of a session, which joins
/// the product name, the version and the canonical sentence.
///
/// # Examples
///
/// ```
/// let line = nurt_contracts::product::session_help_heading("0.1.0");
/// assert!(line.starts_with("Danaco Nurt 0.1.0 - command shell"));
/// ```
#[must_use]
pub fn session_help_heading(version: &str) -> String {
    format!("{PRODUCT_NAME} {version} - {}", lowercase_first(TAGLINE))
}

/// Returns the lines of the banner text: the name of the product, the
/// canonical sentence, the version line, the hint and the copyright line.
#[must_use]
pub fn banner_text(version: &str) -> [String; 5] {
    [
        PRODUCT_NAME.to_owned(),
        TAGLINE.to_owned(),
        version_line(version),
        BANNER_HINT.to_owned(),
        COPYRIGHT_NOTICE.to_owned(),
    ]
}

/// Returns the lines of the banner of an interactive session for a drawing
/// and a terminal width in columns; `None` means that the width is not
/// limited.
///
/// Wide enough for the drawing, the gap and the longest text line, the text
/// stands beside the drawing on the rows of [`BANNER_TEXT_ROWS`]. A narrower
/// terminal gets the text below the drawing after one empty line, and a
/// terminal narrower than the drawing gets the text alone. Lines have no
/// trailing space.
#[must_use]
pub fn banner_lines(
    version: &str,
    drawing: BannerDrawing,
    width: Option<usize>,
) -> Vec<BannerLine> {
    let text = banner_text(version);
    let text_width = text
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    let drawing_width = drawing.width();
    let text_line = |line: &str, index: usize| -> BannerLine {
        let role = if index == 0 {
            BannerRole::Name
        } else {
            BannerRole::Plain
        };
        line.chars().map(|glyph| (glyph, role)).collect()
    };
    let beside = width.is_none_or(|columns| columns >= drawing_width + BANNER_GAP + text_width);
    if beside {
        let (glyphs, roles) = drawing.rows();
        return glyphs
            .iter()
            .zip(roles)
            .enumerate()
            .map(|(row, (glyphs, roles))| {
                let mut line: BannerLine = glyphs
                    .chars()
                    .zip(roles.chars())
                    .map(|(glyph, role)| (glyph, role_of(role)))
                    .collect();
                if let Some(index) = BANNER_TEXT_ROWS.iter().position(|at| *at == row) {
                    while line.len() < drawing_width + BANNER_GAP {
                        line.push((' ', BannerRole::Plain));
                    }
                    line.extend(text_line(&text[index], index));
                }
                line
            })
            .collect();
    }
    let mut lines: Vec<BannerLine> = Vec::new();
    if width.is_some_and(|columns| columns >= drawing_width) {
        let (glyphs, roles) = drawing.rows();
        lines.extend(glyphs.iter().zip(roles).map(|(glyphs, roles)| {
            glyphs
                .chars()
                .zip(roles.chars())
                .map(|(glyph, role)| (glyph, role_of(role)))
                .collect()
        }));
        lines.push(Vec::new());
    }
    lines.extend(
        text.iter()
            .enumerate()
            .map(|(index, line)| text_line(line, index)),
    );
    lines
}

/// The role that a letter of a role row stands for.
fn role_of(letter: char) -> BannerRole {
    match letter {
        's' => BannerRole::Silver,
        'g' => BannerRole::Gold,
        _ => BannerRole::Plain,
    }
}

/// Returns the banner of an interactive session without a final line break
/// and without colour: the mark in half blocks with the text beside it.
///
/// The banner is written only to an interactive terminal; scripts, pipes and
/// `serve --stdio` receive nothing.
#[must_use]
pub fn banner(version: &str) -> String {
    banner_lines(version, BannerDrawing::HalfBlocks, None)
        .iter()
        .map(|line| line.iter().map(|(glyph, _)| *glyph).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Returns the line of the `NAME` section of the manual page, which joins
/// the program name and the canonical sentence.
#[must_use]
pub fn man_name() -> String {
    format!("{PROGRAM_NAME} - {}", lowercase_first(TAGLINE))
}

/// Returns the text with its first character in lower case; the product texts
/// are ASCII, so one character is one byte.
fn lowercase_first(text: &str) -> String {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => first.to_ascii_lowercase().to_string() + characters.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_TEXTS: &[&str] = &[
        PRODUCT_NAME,
        PROGRAM_NAME,
        TAGLINE,
        COPYRIGHT_NOTICE,
        LICENSE_NOTICE,
        BANNER_HINT,
    ];

    const BANNER_TEXT: &str = concat!(
        "           ▄██\n",
        "        ▄▄████\n",
        "      ▄██████▀\n",
        "   ▄██████▀                  Danaco Nurt\n",
        " ▄█████▀▀   ▄▄▄\n",
        "██████▄     ████▄            Command shell and language where every change can be undone\n",
        " ▀██████▄   ▀██████▄\n",
        "   ▀▀██████▄  ▀██████▄▄      nurt 1.2.3\n",
        "      ▀██████▄   ▀██████▄\n",
        "         ▀████     ▀██████   Type 'help' to get started.\n",
        "           ▀▀▀   ▄▄█████▀\n",
        "               ▄██████▀      Copyright (c) 2026 Danaco Holding Group Sp. z o.o. All rights reserved.\n",
        "            ▄██████▀\n",
        "            ████▀▀\n",
        "            ██▀"
    );

    #[test]
    fn canonical_sentence_is_exact() {
        assert_eq!(
            TAGLINE,
            "Command shell and language where every change can be undone"
        );
    }

    #[test]
    fn version_texts_are_exact() {
        assert_eq!(version_line("1.2.3"), "nurt 1.2.3");
        assert_eq!(
            long_version("1.2.3", Some(("0123abc", "2026-11-22"))),
            "nurt 1.2.3 (0123abc 2026-11-22)\n\
             Copyright (c) 2026 Danaco Holding Group Sp. z o.o. All rights reserved.\n\
             License: Apache-2.0"
        );
        assert_eq!(
            long_version("1.2.3", None),
            "nurt 1.2.3\n\
             Copyright (c) 2026 Danaco Holding Group Sp. z o.o. All rights reserved.\n\
             License: Apache-2.0"
        );
    }

    #[test]
    fn help_lines_are_exact() {
        assert_eq!(
            help_summary(),
            "Command shell and language where every change can be undone."
        );
        assert_eq!(
            session_help_heading("1.2.3"),
            "Danaco Nurt 1.2.3 - command shell and language where every change can be undone"
        );
    }

    #[test]
    fn banner_is_exact() {
        assert_eq!(banner("1.2.3"), BANNER_TEXT);
    }

    /// Characters of a drawing with the roles of their cells, as a grid.
    fn grid(drawing: BannerDrawing) -> Vec<Vec<(char, char)>> {
        let (glyphs, roles) = drawing.rows();
        let width = drawing.width();
        glyphs
            .iter()
            .zip(roles)
            .map(|(glyphs, roles)| {
                let mut glyphs: Vec<char> = glyphs.chars().collect();
                let mut roles: Vec<char> = roles.chars().collect();
                glyphs.resize(width, ' ');
                roles.resize(width, ' ');
                glyphs.into_iter().zip(roles).collect()
            })
            .collect()
    }

    const DRAWINGS: [BannerDrawing; 2] = [BannerDrawing::HalfBlocks, BannerDrawing::Ascii];

    #[test]
    fn drawings_have_the_documented_size_and_no_trailing_space() {
        assert_eq!(BANNER_BLOCKS.len(), 15);
        assert_eq!(BANNER_ASCII.len(), 14);
        for drawing in DRAWINGS {
            let (glyphs, _) = drawing.rows();
            let widest = glyphs.iter().map(|row| row.chars().count()).max();
            assert_eq!(widest, Some(drawing.width()));
            for row in glyphs {
                assert_eq!(*row, row.trim_end(), "{row}");
            }
        }
        assert!(BANNER_ASCII.iter().all(|row| row.is_ascii()));
    }

    #[test]
    fn role_rows_are_as_long_as_glyph_rows_and_every_glyph_has_a_role() {
        for drawing in DRAWINGS {
            let (glyphs, roles) = drawing.rows();
            assert_eq!(glyphs.len(), roles.len());
            for (glyph_row, role_row) in glyphs.iter().zip(roles) {
                assert_eq!(glyph_row.chars().count(), role_row.chars().count());
                for (glyph, role) in glyph_row.chars().zip(role_row.chars()) {
                    if glyph == ' ' {
                        assert_eq!(role, ' ', "{glyph_row}");
                    } else {
                        assert!(role == 's' || role == 'g', "{glyph_row} {role_row}");
                    }
                }
            }
        }
    }

    /// The character that a half block becomes when the drawing is turned by
    /// 180 degrees.
    fn turned(glyph: char) -> char {
        match glyph {
            '\u{2580}' => '\u{2584}',
            '\u{2584}' => '\u{2580}',
            other => other,
        }
    }

    #[test]
    fn turning_a_drawing_by_180_degrees_with_the_roles_exchanged_gives_the_same_drawing() {
        for drawing in DRAWINGS {
            let cells = grid(drawing);
            let (height, width) = (cells.len(), drawing.width());
            for (row, line) in cells.iter().enumerate() {
                for (column, (glyph, role)) in line.iter().enumerate() {
                    let (other, other_role) = cells[height - 1 - row][width - 1 - column];
                    let expected = match role {
                        's' => 'g',
                        'g' => 's',
                        _ => ' ',
                    };
                    assert_eq!(turned(*glyph), other, "{drawing:?} {row} {column}");
                    assert_eq!(expected, other_role, "{drawing:?} {row} {column}");
                }
            }
        }
    }

    #[test]
    fn each_chevron_is_one_connected_shape() {
        for drawing in DRAWINGS {
            let cells = grid(drawing);
            for role in ['s', 'g'] {
                let members: Vec<(usize, usize)> = cells
                    .iter()
                    .enumerate()
                    .flat_map(|(row, line)| {
                        line.iter()
                            .enumerate()
                            .filter(move |(_, (_, own))| *own == role)
                            .map(move |(column, _)| (row, column))
                    })
                    .collect();
                let mut seen = vec![members[0]];
                let mut open = vec![members[0]];
                while let Some((row, column)) = open.pop() {
                    let next: Vec<(usize, usize)> = members
                        .iter()
                        .copied()
                        .filter(|(r, c)| {
                            r.abs_diff(row) + c.abs_diff(column) == 1 && !seen.contains(&(*r, *c))
                        })
                        .collect();
                    seen.extend(next.iter().copied());
                    open.extend(next);
                }
                assert_eq!(seen.len(), members.len(), "{drawing:?} {role}");
            }
        }
    }

    #[test]
    fn chevrons_are_separated_by_a_clear_cell_in_every_row_where_both_occur() {
        for drawing in DRAWINGS {
            for line in grid(drawing) {
                let last_silver = line.iter().rposition(|(_, role)| *role == 's');
                let first_gold = line.iter().position(|(_, role)| *role == 'g');
                if let (Some(silver), Some(gold)) = (last_silver, first_gold) {
                    assert!(gold > silver + 1, "{drawing:?} {line:?}");
                }
            }
        }
    }

    /// The text of a line of the banner.
    fn plain(line: &BannerLine) -> String {
        line.iter().map(|(glyph, _)| *glyph).collect()
    }

    #[test]
    fn text_stands_beside_the_drawing_on_a_wide_terminal() {
        let longest = banner_text("1.2.3")
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        for drawing in DRAWINGS {
            let lines = banner_lines(
                "1.2.3",
                drawing,
                Some(drawing.width() + BANNER_GAP + longest),
            );
            let text = banner_text("1.2.3");
            assert_eq!(lines.len(), drawing.rows().0.len());
            for (index, row) in BANNER_TEXT_ROWS.iter().enumerate() {
                let line = plain(&lines[*row]);
                let start = drawing.width() + BANNER_GAP;
                assert_eq!(line.chars().skip(start).collect::<String>(), text[index]);
                assert!(line.chars().take(start).any(|glyph| glyph != ' '));
            }
            assert_eq!(
                lines[BANNER_TEXT_ROWS[0]].last().map(|cell| cell.1),
                Some(BannerRole::Name)
            );
            assert!(
                lines
                    .iter()
                    .all(|line| plain(line) == plain(line).trim_end())
            );
        }
    }

    #[test]
    fn text_goes_below_the_drawing_on_a_terminal_that_is_too_narrow_for_both() {
        let width = BANNER_BLOCKS_WIDTH + BANNER_GAP + 70;
        let lines = banner_lines("1.2.3", BannerDrawing::HalfBlocks, Some(width));
        assert_eq!(lines.len(), BANNER_BLOCKS.len() + 1 + 5);
        assert!(lines[BANNER_BLOCKS.len()].is_empty());
        assert_eq!(plain(&lines[BANNER_BLOCKS.len() + 1]), "Danaco Nurt");
        assert_eq!(plain(&lines[BANNER_BLOCKS.len() + 3]), "nurt 1.2.3");
        let at_drawing = banner_lines(
            "1.2.3",
            BannerDrawing::HalfBlocks,
            Some(BANNER_BLOCKS_WIDTH),
        );
        assert_eq!(at_drawing.len(), lines.len());
    }

    #[test]
    fn drawing_is_left_out_on_a_terminal_narrower_than_the_drawing() {
        let lines = banner_lines(
            "1.2.3",
            BannerDrawing::HalfBlocks,
            Some(BANNER_BLOCKS_WIDTH - 1),
        );
        let text: Vec<String> = lines.iter().map(plain).collect();
        assert_eq!(text, banner_text("1.2.3"));
        assert_eq!(lines[0][0].1, BannerRole::Name);
        assert_eq!(lines[1][0].1, BannerRole::Plain);
    }

    #[test]
    fn banner_text_is_built_from_the_product_constants() {
        let text = banner_text("1.2.3");
        assert_eq!(text[0], PRODUCT_NAME);
        assert_eq!(text[1], TAGLINE);
        assert_eq!(text[2], version_line("1.2.3"));
        assert_eq!(text[3], BANNER_HINT);
        assert_eq!(text[4], COPYRIGHT_NOTICE);
    }

    #[test]
    fn manual_page_name_is_exact() {
        assert_eq!(
            man_name(),
            "nurt - command shell and language where every change can be undone"
        );
    }

    #[test]
    fn texts_are_ascii_without_audience_suffix() {
        let derived = [
            version_line("1.2.3"),
            long_version("1.2.3", Some(("0123abc", "2026-11-22"))),
            help_summary(),
            session_help_heading("1.2.3"),
            banner_text("1.2.3").join("\n"),
            man_name(),
        ];
        for text in ALL_TEXTS
            .iter()
            .copied()
            .chain(derived.iter().map(String::as_str))
        {
            assert!(text.is_ascii(), "{text:?} is not ASCII");
            assert!(
                !text.contains("people and AI agents"),
                "{text:?} carries the audience suffix"
            );
        }
    }

    #[test]
    fn lowercase_first_handles_empty_text() {
        assert_eq!(lowercase_first(""), "");
        assert_eq!(lowercase_first("Ab"), "ab");
    }
}
