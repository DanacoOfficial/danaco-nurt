// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Translation of the common-command list in script mode.

use std::path::PathBuf;

use nurt_correct::bash::{Mode, TranslateContext, translate_with};

/// Number of lines of the list that translate without an error. The line
/// `tail -n +2` is not among them: it asks for the lines from the second on,
/// which no Nurt parameter says.
const MINIMUM_TRANSLATED: usize = 221;

const SCRIPT: TranslateContext<'static> = TranslateContext {
    base: None,
    lets: &[],
    mode: Mode::Script,
};

#[test]
fn common_commands_translate_in_script_mode() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/d1/migrate/common-500/bash.txt");
    let text = std::fs::read_to_string(path).expect("the list of common commands");
    let mut translated = 0;
    let mut failed = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        match translate_with(line, &SCRIPT) {
            Some(Ok(_)) => translated += 1,
            Some(Err(error)) => failed.push(format!("{line}\n    {}", error.message)),
            None => failed.push(format!("{line}\n    not a habit")),
        }
    }
    assert!(
        translated >= MINIMUM_TRANSLATED,
        "{translated} translated, {} failed:\n{}",
        failed.len(),
        failed.join("\n")
    );
}
