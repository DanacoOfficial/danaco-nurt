// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/textedit.rs
// Responsibility:   Pure text operations of the installation: PATH lists, Windows Terminal settings, PowerShell profile blocks, configuration file, versions.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Text operations of the installation.
//!
//! Nothing here touches the system: the functions take the text of a file or of a
//! registry value and return the new text, so that they are tested without Windows.

/// Name of the profile of Windows Terminal.
pub(crate) const PROFILE_NAME: &str = "Danaco Nurt";

/// GUID of the profile of Windows Terminal.
pub(crate) const PROFILE_GUID: &str = "{b7e0c4a2-5d1f-4c6e-9a38-2f6d0a71c5e4}";

/// GUID of the profile "Windows PowerShell" of Windows Terminal.
pub(crate) const POWERSHELL_GUID: &str = "{61c54bbd-c2c6-5271-96e7-009a87ff44bf}";

/// Name of the colour scheme of the profile.
pub(crate) const SCHEME_NAME: &str = "Danaco Nurt";

const BLOCK_BEGIN: &str = "# >>> Danaco Nurt completion >>>";
const BLOCK_END: &str = "# <<< Danaco Nurt completion <<<";
const DEFAULT_KEY: &str = "defaultProfile";
const INDENT: &str = "    ";

/// Normal form of a `PATH` entry for comparison.
fn normal(entry: &str) -> String {
    entry
        .trim()
        .trim_end_matches(['\\', '/'])
        .to_ascii_lowercase()
}

/// Returns the list with the entry appended, or `None` when it is present.
pub(crate) fn path_add(current: &str, entry: &str) -> Option<String> {
    let wanted = normal(entry);
    if current.split(';').any(|item| normal(item) == wanted) {
        return None;
    }
    let trimmed = current.trim_end_matches(';');
    Some(if trimmed.is_empty() {
        entry.to_string()
    } else {
        format!("{trimmed};{entry}")
    })
}

/// Returns the list without the entry, or `None` when it is absent.
pub(crate) fn path_remove(current: &str, entry: &str) -> Option<String> {
    let wanted = normal(entry);
    let items: Vec<&str> = current.split(';').collect();
    if !items.iter().any(|item| normal(item) == wanted) {
        return None;
    }
    let kept: Vec<&str> = items
        .into_iter()
        .filter(|item| normal(item) != wanted && !item.is_empty())
        .collect();
    Some(kept.join(";"))
}

/// A member of the top-level object of a JSON text with comments.
struct Member {
    /// Offset of the opening quote of the key.
    key_start: usize,
    /// Text of the key between its quotes.
    key: String,
    /// Offset of the first byte of the value.
    value_start: usize,
    /// Offset after the last byte of the value.
    value_end: usize,
    /// Offset after the comma that follows the value, or `value_end` when there is none.
    after: usize,
}

/// The top-level object of a JSON text with comments.
struct TopObject {
    /// Offset of the opening brace.
    open: usize,
    /// The members in the order of the text.
    members: Vec<Member>,
}

/// Offset after the whitespace and the comments that start at `from`.
fn skip_trivia(bytes: &[u8], from: usize) -> usize {
    let mut at = from;
    loop {
        match bytes.get(at) {
            Some(b' ' | b'\t' | b'\r' | b'\n') => at += 1,
            Some(b'/') if bytes.get(at + 1) == Some(&b'/') => {
                while bytes.get(at).is_some_and(|byte| *byte != b'\n') {
                    at += 1;
                }
            }
            Some(b'/') if bytes.get(at + 1) == Some(&b'*') => {
                at += 2;
                while at < bytes.len() && !(bytes[at] == b'*' && bytes.get(at + 1) == Some(&b'/')) {
                    at += 1;
                }
                at = (at + 2).min(bytes.len());
            }
            _ => return at,
        }
    }
}

/// Offset after the string literal whose opening quote is at `open`.
fn string_end(bytes: &[u8], open: usize) -> Option<usize> {
    let mut at = open + 1;
    while let Some(byte) = bytes.get(at) {
        match byte {
            b'\\' => at += 2,
            b'"' => return Some(at + 1),
            _ => at += 1,
        }
    }
    None
}

/// Offset after the value that starts at `start`.
fn value_end(bytes: &[u8], start: usize) -> Option<usize> {
    match bytes.get(start)? {
        b'"' => string_end(bytes, start),
        b'{' | b'[' => {
            let mut depth = 0_usize;
            let mut at = start;
            while at < bytes.len() {
                match bytes[at] {
                    b'"' => at = string_end(bytes, at)?,
                    b'/' if matches!(bytes.get(at + 1), Some(b'/' | b'*')) => {
                        at = skip_trivia(bytes, at)
                    }
                    b'{' | b'[' => {
                        depth += 1;
                        at += 1;
                    }
                    b'}' | b']' => {
                        depth = depth.checked_sub(1)?;
                        at += 1;
                        if depth == 0 {
                            return Some(at);
                        }
                    }
                    _ => at += 1,
                }
            }
            None
        }
        _ => {
            let mut at = start;
            while bytes.get(at).is_some_and(|byte| {
                !matches!(
                    byte,
                    b',' | b'}' | b']' | b' ' | b'\t' | b'\r' | b'\n' | b'/'
                )
            }) {
                at += 1;
            }
            Some(at)
        }
    }
}

/// Reads the members of the top-level object; `None` when the text is not such an object.
fn top_object(text: &str) -> Option<TopObject> {
    let bytes = text.as_bytes();
    let open = skip_trivia(
        bytes,
        if text.starts_with('\u{feff}') {
            '\u{feff}'.len_utf8()
        } else {
            0
        },
    );
    if bytes.get(open) != Some(&b'{') {
        return None;
    }
    let mut members = Vec::new();
    let mut at = skip_trivia(bytes, open + 1);
    loop {
        match bytes.get(at)? {
            b'}' => return Some(TopObject { open, members }),
            b'"' => {}
            _ => return None,
        }
        let key_end = string_end(bytes, at)?;
        let key = text.get(at + 1..key_end - 1)?.to_string();
        let colon = skip_trivia(bytes, key_end);
        if bytes.get(colon) != Some(&b':') {
            return None;
        }
        let value_start = skip_trivia(bytes, colon + 1);
        let end = value_end(bytes, value_start)?;
        let next = skip_trivia(bytes, end);
        let after = if bytes.get(next) == Some(&b',') {
            next + 1
        } else {
            end
        };
        members.push(Member {
            key_start: at,
            key,
            value_start,
            value_end: end,
            after,
        });
        match bytes.get(next)? {
            b',' => at = skip_trivia(bytes, next + 1),
            b'}' => return Some(TopObject { open, members }),
            _ => return None,
        }
    }
}

/// Line ending that a text uses: CRLF when it holds one, otherwise LF.
fn line_ending(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

/// Sets `defaultProfile` in the text of `settings.json` of Windows Terminal.
///
/// The file is JSON with comments, so the text is edited and not parsed: the value of the
/// key of the top-level object is replaced, or, when the key is absent, a line with the key
/// is inserted after the opening brace. Comments, trailing commas, line endings and the byte
/// order mark stay as they are. Returns `None` when the text is not an object.
pub(crate) fn set_default_profile(settings: &str, guid: &str) -> Option<String> {
    let object = top_object(settings)?;
    if let Some(member) = object
        .members
        .iter()
        .find(|member| member.key == DEFAULT_KEY)
    {
        return Some(format!(
            "{}\"{guid}\"{}",
            &settings[..member.value_start],
            &settings[member.value_end..]
        ));
    }
    let eol = line_ending(settings);
    let line = if object.members.is_empty() {
        format!("{eol}{INDENT}\"{DEFAULT_KEY}\": \"{guid}\"{eol}")
    } else {
        format!("{eol}{INDENT}\"{DEFAULT_KEY}\": \"{guid}\",")
    };
    Some(format!(
        "{}{line}{}",
        &settings[..=object.open],
        &settings[object.open + 1..]
    ))
}

/// Removes the line that `set_default_profile` inserted for `guid`; `None` when there is no such line.
pub(crate) fn remove_default_profile(settings: &str, guid: &str) -> Option<String> {
    let object = top_object(settings)?;
    let member = object
        .members
        .iter()
        .find(|member| member.key == DEFAULT_KEY)?;
    if !matches!(
        &settings[object.open + 1..member.key_start],
        "\n    " | "\r\n    "
    ) {
        return None;
    }
    if settings.get(member.value_start..member.value_end)? != format!("\"{guid}\"") {
        return None;
    }
    Some(format!(
        "{}{}",
        &settings[..=object.open],
        &settings[member.after..]
    ))
}

/// Returns the value of `defaultProfile` in the text of `settings.json`.
pub(crate) fn default_profile(settings: &str) -> Option<String> {
    let object = top_object(settings)?;
    let member = object
        .members
        .iter()
        .find(|member| member.key == DEFAULT_KEY)?;
    let value = settings.get(member.value_start..member.value_end)?;
    value
        .strip_prefix('"')?
        .strip_suffix('"')
        .map(str::to_string)
}

/// PowerShell expression that yields `text`. Text of ASCII characters is one single-quoted
/// literal; any other character is written as `[char]` code units, so that the profile, which
/// Windows PowerShell 5.1 reads as ANSI when it has no byte order mark, never depends on its encoding.
fn powershell_string(text: &str) -> String {
    if text.is_ascii() {
        return format!("'{}'", text.replace('\'', "''"));
    }
    let mut parts = vec!["''".to_string()];
    let mut run = String::new();
    for character in text.chars() {
        if character.is_ascii() {
            run.push(character);
            continue;
        }
        if !run.is_empty() {
            parts.push(format!("'{}'", run.replace('\'', "''")));
            run.clear();
        }
        let mut units = [0_u16; 2];
        for unit in character.encode_utf16(&mut units) {
            parts.push(format!("[char]0x{unit:04X}"));
        }
    }
    if !run.is_empty() {
        parts.push(format!("'{}'", run.replace('\'', "''")));
    }
    format!("({})", parts.join(" + "))
}

/// Returns the text of a PowerShell profile with the block that loads the completion.
///
/// The block goes to the end of the profile, after one empty line. `profile_without_block`
/// undoes the addition exactly, including the byte order mark and the line endings.
pub(crate) fn profile_with_block(profile: &str, script: &str) -> String {
    let without = profile_without_block(profile);
    let eol = line_ending(&without);
    let path = powershell_string(script);
    let block = format!(
        "{BLOCK_BEGIN}{eol}if (Test-Path -LiteralPath {path}) {{ . {path} }}{eol}{BLOCK_END}"
    );
    if without.trim_start_matches('\u{feff}').is_empty() {
        format!("{without}{block}{eol}")
    } else if without.ends_with('\n') {
        format!("{without}{eol}{block}{eol}")
    } else {
        format!("{without}{eol}{eol}{block}")
    }
}

/// Offset of `at` moved back over the line ending that precedes it; `at` when none precedes.
fn before_line_ending(text: &str, at: usize) -> usize {
    if text[..at].ends_with("\r\n") {
        at - 2
    } else if text[..at].ends_with('\n') {
        at - 1
    } else {
        at
    }
}

/// Returns the text of a PowerShell profile without the block of the completion.
pub(crate) fn profile_without_block(profile: &str) -> String {
    let mut text = profile.to_string();
    while let Some(next) = remove_one_block(&text) {
        text = next;
    }
    text
}

/// Removes the first block of the completion; `None` when there is none.
fn remove_one_block(profile: &str) -> Option<String> {
    let begin = profile.find(BLOCK_BEGIN)?;
    if !profile[..begin]
        .rsplit('\n')
        .next()
        .is_some_and(|before| before.trim().is_empty() || before == "\u{feff}")
    {
        return None;
    }
    let mut end = begin + profile[begin..].find(BLOCK_END)? + BLOCK_END.len();
    let mut start = begin;
    if profile[end..].starts_with("\r\n") {
        end += 2;
        start = before_line_ending(profile, start);
    } else if profile[end..].starts_with('\n') {
        end += 1;
        start = before_line_ending(profile, start);
    } else if end == profile.len() {
        start = before_line_ending(profile, before_line_ending(profile, start));
    }
    Some(format!("{}{}", &profile[..start], &profile[end..]))
}

/// Text of the fragment of Windows Terminal with the profile of the program.
///
/// `exe` is the path of `nurt.exe` and `icon` the path of `nurt.ico`. The profile starts
/// `nurt.exe` without arguments, which opens the interactive session with its start banner.
pub(crate) fn terminal_fragment(exe: &str, icon: &str) -> String {
    let command = json_string(&format!("\"{exe}\""));
    let icon = json_string(icon);
    format!(
        "{{\n  \"profiles\": [\n    {{\n      \"name\": \"{PROFILE_NAME}\",\n      \"guid\": \"{PROFILE_GUID}\",\n      \"commandline\": {command},\n      \"icon\": {icon},\n      \"startingDirectory\": \"%USERPROFILE%\",\n      \"colorScheme\": \"{SCHEME_NAME}\",\n      \"cursorShape\": \"underscore\",\n      \"tabTitle\": \"{PROFILE_NAME}\",\n      \"suppressApplicationTitle\": true\n    }}\n  ],\n  \"schemes\": [\n{SCHEME}\n  ]\n}}\n"
    )
}

/// The colour scheme of the profile, drawn from the tones of the identity 1.0.0: the ground is
/// graphite, the text is silver, the cursor and the yellow are gold, cyan is the blue of the pipe,
/// red is the colour of alert of the setup, and the hues that the identity does not have are
/// tones of silver and gold (success is silver, as in the log of the setup).
const SCHEME: &str = concat!(
    "    {\n",
    "      \"name\": \"Danaco Nurt\",\n",
    "      \"background\": \"#0B0C0E\",\n",
    "      \"foreground\": \"#DDE1E6\",\n",
    "      \"cursorColor\": \"#C9A15A\",\n",
    "      \"selectionBackground\": \"#3A3F47\",\n",
    "      \"black\": \"#0B0C0E\",\n",
    "      \"red\": \"#E2897D\",\n",
    "      \"green\": \"#C9CED5\",\n",
    "      \"yellow\": \"#C9A15A\",\n",
    "      \"blue\": \"#9AA1AA\",\n",
    "      \"purple\": \"#A3722A\",\n",
    "      \"cyan\": \"#4FD1F5\",\n",
    "      \"white\": \"#C9CED5\",\n",
    "      \"brightBlack\": \"#7C8390\",\n",
    "      \"brightRed\": \"#E2897D\",\n",
    "      \"brightGreen\": \"#F6F7F8\",\n",
    "      \"brightYellow\": \"#D9BC85\",\n",
    "      \"brightBlue\": \"#DDE1E6\",\n",
    "      \"brightPurple\": \"#D6AE66\",\n",
    "      \"brightCyan\": \"#4FD1F5\",\n",
    "      \"brightWhite\": \"#F6F7F8\"\n",
    "    }"
);

/// JSON string literal of a text.
fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", control as u32))
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Text of the first configuration file `nurt.toml`.
pub(crate) fn config_toml(config: &crate::model::ConfigChoices, version: &str) -> String {
    let language = if config.language == "pl" { "pl" } else { "en" };
    let color = match config.color.as_str() {
        "always" | "never" => config.color.as_str(),
        _ => "auto",
    };
    let syntax = match config.syntax.as_str() {
        "b" | "c" => config.syntax.as_str(),
        _ => "a",
    };
    format!(
        "# Configuration of Danaco Nurt, written by the setup of version {version}.\n# Every key is optional; the complete list is in docs/site/configuration.md.\n\n[syntax]\nvariant = \"{syntax}\"\n\n[output]\nlanguage = \"{language}\"\ncolor = \"{color}\"\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = "C:\\Users\\user\\AppData\\Local\\Programs\\Nurt\\completions\\_nurt.ps1";

    #[test]
    fn path_add_appends_once() {
        assert_eq!(
            path_add("C:\\a;C:\\b", "C:\\n\\bin").as_deref(),
            Some("C:\\a;C:\\b;C:\\n\\bin")
        );
        assert_eq!(path_add("C:\\a;C:\\N\\BIN\\", "c:\\n\\bin"), None);
        assert_eq!(path_add("", "C:\\n").as_deref(), Some("C:\\n"));
        assert_eq!(path_add("C:\\a;", "C:\\n").as_deref(), Some("C:\\a;C:\\n"));
    }

    #[test]
    fn path_remove_drops_the_entry_only() {
        assert_eq!(
            path_remove("C:\\a;C:\\n\\bin;C:\\b", "C:\\n\\bin").as_deref(),
            Some("C:\\a;C:\\b")
        );
        assert_eq!(
            path_remove("%USERPROFILE%\\x;C:\\n", "C:\\n").as_deref(),
            Some("%USERPROFILE%\\x")
        );
        assert_eq!(path_remove("C:\\a", "C:\\n"), None);
    }

    #[test]
    fn default_profile_is_replaced_and_comments_stay() {
        let with = "{\n    // \"defaultProfile\": \"{x}\",\n    /* \"defaultProfile\": \"{y}\" */\n    \"defaultProfile\": \"{old}\",\n    \"profiles\": {}\n}";
        let replaced = set_default_profile(with, PROFILE_GUID).unwrap_or_default();
        assert_eq!(replaced, with.replace("{old}", PROFILE_GUID));
        assert_eq!(default_profile(with).as_deref(), Some("{old}"));
        assert_eq!(default_profile(&replaced).as_deref(), Some(PROFILE_GUID));
    }

    #[test]
    fn default_profile_is_inserted_when_the_key_is_absent() {
        let without = "{\n    \"profiles\": {}\n}";
        let inserted = set_default_profile(without, PROFILE_GUID).unwrap_or_default();
        assert_eq!(
            inserted,
            format!("{{\n    \"defaultProfile\": \"{PROFILE_GUID}\",\n    \"profiles\": {{}}\n}}")
        );
        assert_eq!(
            remove_default_profile(&inserted, PROFILE_GUID).as_deref(),
            Some(without)
        );
        assert!(set_default_profile("not json", PROFILE_GUID).is_none());
        assert!(set_default_profile("[1, 2]", PROFILE_GUID).is_none());
        assert!(set_default_profile("", PROFILE_GUID).is_none());
    }

    #[test]
    fn default_profile_ignores_keys_in_comments_strings_and_nested_objects() {
        let text = "{\n    // \"defaultProfile\": \"{c}\"\n    \"note\": \"\\\"defaultProfile\\\": \\\"{s}\\\"\",\n    \"profiles\": { \"defaultProfile\": \"{nested}\" }\n}";
        assert_eq!(default_profile(text), None);
        let inserted = set_default_profile(text, PROFILE_GUID).unwrap_or_default();
        assert_eq!(default_profile(&inserted).as_deref(), Some(PROFILE_GUID));
        assert!(inserted.contains("\"defaultProfile\": \"{nested}\""));
        assert!(inserted.contains("// \"defaultProfile\": \"{c}\""));
        assert_eq!(
            remove_default_profile(&inserted, PROFILE_GUID).as_deref(),
            Some(text)
        );
    }

    #[test]
    fn default_profile_keeps_trailing_commas_bom_and_crlf() {
        let text = "\u{feff}// header\r\n{\r\n    \"$schema\": \"x\", // note\r\n    \"profiles\": { \"list\": [ { \"name\": \"a\", }, ], },\r\n}\r\n";
        let inserted = set_default_profile(text, PROFILE_GUID).unwrap_or_default();
        assert!(inserted.starts_with("\u{feff}// header\r\n{\r\n    \"defaultProfile\": "));
        assert!(!inserted.replace("\r\n", "").contains('\n'));
        assert_eq!(default_profile(&inserted).as_deref(), Some(PROFILE_GUID));
        assert_eq!(
            remove_default_profile(&inserted, PROFILE_GUID).as_deref(),
            Some(text)
        );
        let replaced = set_default_profile(&inserted, POWERSHELL_GUID).unwrap_or_default();
        assert_eq!(replaced, inserted.replace(PROFILE_GUID, POWERSHELL_GUID));
    }

    #[test]
    fn default_profile_handles_empty_objects_and_other_value_kinds() {
        let inserted = set_default_profile("{}", PROFILE_GUID).unwrap_or_default();
        assert_eq!(
            inserted,
            format!("{{\n    \"defaultProfile\": \"{PROFILE_GUID}\"\n}}")
        );
        assert_eq!(default_profile(&inserted).as_deref(), Some(PROFILE_GUID));
        let null = "{ \"defaultProfile\": null, \"x\": 1 }";
        assert_eq!(default_profile(null), None);
        let expected = format!("{{ \"defaultProfile\": \"{PROFILE_GUID}\", \"x\": 1 }}");
        assert_eq!(
            set_default_profile(null, PROFILE_GUID).as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(
            remove_default_profile("{ \"defaultProfile\": \"{other}\" }", PROFILE_GUID),
            None
        );
    }

    #[test]
    fn profile_block_round_trips_byte_for_byte() {
        let samples = [
            "",
            "\u{feff}",
            "\n",
            "Set-Alias ll ls\n",
            "Set-Alias ll ls",
            "Set-Alias ll ls\r\n",
            "\u{feff}Set-Alias ll ls\r\nfunction f { 1 }\r\n",
            "\u{feff}Set-Alias ll ls\r\nfunction f { 1 }",
            "a\n\n\n",
            "a\r\n\r\n",
            "# comment only",
        ];
        for original in samples {
            let added = profile_with_block(original, SCRIPT);
            assert!(
                added.contains(BLOCK_BEGIN) && added.contains(BLOCK_END),
                "{original:?}"
            );
            assert_eq!(profile_without_block(&added), original, "{original:?}");
            assert_eq!(profile_with_block(&added, SCRIPT), added, "{original:?}");
        }
    }

    #[test]
    fn profile_block_follows_the_line_endings_of_the_profile() {
        let added = profile_with_block("\u{feff}Set-Alias ll ls\r\n", SCRIPT);
        assert!(!added.replace("\r\n", "").contains('\n'));
        let added = profile_with_block("Set-Alias ll ls\n", SCRIPT);
        assert!(!added.contains('\r'));
    }

    #[test]
    fn profile_block_is_replaced_when_the_script_moves() {
        let first = profile_with_block("x\n", "C:\\one\\_nurt.ps1");
        let second = profile_with_block(&first, "C:\\two\\_nurt.ps1");
        assert!(second.contains("C:\\two") && !second.contains("C:\\one"));
        assert_eq!(profile_without_block(&second), "x\n");
    }

    #[test]
    fn profile_block_keeps_text_added_after_it() {
        let added = profile_with_block("x\n", SCRIPT);
        let edited = format!("{added}Set-Alias a b\n");
        assert_eq!(profile_without_block(&edited), "x\nSet-Alias a b\n");
    }

    #[test]
    fn profile_block_leaves_marker_inside_a_line_alone() {
        let text = format!("Write-Host '{BLOCK_BEGIN}'\n{BLOCK_END}\n");
        assert_eq!(profile_without_block(&text), text);
    }

    #[test]
    fn profile_block_quotes_the_path_for_powershell() {
        let ascii = profile_with_block("", "C:\\it's\\_nurt.ps1");
        assert!(
            ascii.contains("-LiteralPath 'C:\\it''s\\_nurt.ps1') { . 'C:\\it''s\\_nurt.ps1' }")
        );
        let wide = profile_with_block("", "C:\\U\u{17c}ytkownik \u{1F600}\\_nurt.ps1");
        assert!(wide.is_ascii());
        assert!(wide.contains("('' + 'C:\\U' + [char]0x017C + 'ytkownik ' + [char]0xD83D + [char]0xDE00 + '\\_nurt.ps1')"));
    }

    #[test]
    fn fragment_is_valid_json_with_the_profile_and_the_scheme() {
        let exe = "C:\\Users\\user b\\Programs\\Nurt\\bin\\nurt.exe";
        let icon = "C:\\Users\\user b\\Programs\\Nurt\\nurt.ico";
        let text = terminal_fragment(exe, icon);
        let value: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        let profiles = value["profiles"].as_array().cloned().unwrap_or_default();
        assert_eq!(profiles.len(), 1);
        let profile = &profiles[0];
        assert_eq!(profile["name"], PROFILE_NAME);
        assert_eq!(profile["guid"], PROFILE_GUID);
        assert_eq!(profile["commandline"], format!("\"{exe}\""));
        assert_eq!(profile["icon"], icon);
        assert_eq!(profile["colorScheme"], SCHEME_NAME);
        assert_eq!(profile["cursorShape"], "underscore");
        assert_eq!(profile["tabTitle"], PROFILE_NAME);
        assert_eq!(profile["suppressApplicationTitle"], true);
        let schemes = value["schemes"].as_array().cloned().unwrap_or_default();
        assert_eq!(schemes.len(), 1);
        assert_eq!(schemes[0]["name"], SCHEME_NAME);
    }

    #[test]
    fn fragment_escapes_unusual_paths() {
        let exe = "C:\\U\u{17c}ytkownik \"x\"\\nurt.exe";
        let value: serde_json::Value =
            serde_json::from_str(&terminal_fragment(exe, "C:\\i.ico")).unwrap_or_default();
        assert_eq!(value["profiles"][0]["commandline"], format!("\"{exe}\""));
    }

    #[test]
    fn scheme_uses_the_tones_of_the_identity() {
        let value: serde_json::Value =
            serde_json::from_str(&terminal_fragment("C:\\n.exe", "C:\\n.ico")).unwrap_or_default();
        let scheme = &value["schemes"][0];
        assert_eq!(scheme["background"], "#0B0C0E", "graphite");
        assert_eq!(scheme["foreground"], "#DDE1E6", "silver 221;225;230");
        assert_eq!(scheme["cursorColor"], "#C9A15A", "gold");
        assert_eq!(scheme["yellow"], "#C9A15A", "gold");
        assert_eq!(scheme["cyan"], "#4FD1F5", "the pipe");
        assert_eq!(scheme["brightCyan"], "#4FD1F5", "the pipe");
        assert_eq!(scheme["brightBlack"], "#7C8390", "caption");
        let names = [
            "background",
            "foreground",
            "cursorColor",
            "selectionBackground",
            "black",
            "red",
            "green",
            "yellow",
            "blue",
            "purple",
            "cyan",
            "white",
            "brightBlack",
            "brightRed",
            "brightGreen",
            "brightYellow",
            "brightBlue",
            "brightPurple",
            "brightCyan",
            "brightWhite",
        ];
        for name in names {
            let colour = scheme[name].as_str().unwrap_or_default();
            assert!(
                colour.len() == 7
                    && colour.starts_with('#')
                    && colour[1..].chars().all(|c| c.is_ascii_hexdigit()),
                "{name}: {colour}"
            );
        }
    }

    #[test]
    fn configuration_is_complete_and_clamped() {
        let config = crate::model::ConfigChoices {
            language: "pl".into(),
            color: "rainbow".into(),
            ..crate::model::ConfigChoices::default()
        };
        let text = config_toml(&config, "0.1.2");
        assert!(
            text.contains("language = \"pl\"")
                && text.contains("color = \"auto\"")
                && text.contains("variant = \"a\"")
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
