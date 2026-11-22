// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Modes for the tests of output capture, decoding and filters.
//!
//! | Mode | Arguments | Behaviour |
//! |---|---|---|
//! | `emit-file` | path, chunk bytes, milliseconds | writes the file to standard output in chunks, pausing between them |
//! | `emit-encoding` | `utf-8`, `utf-16le`, `utf-16be`, `cp1250` or `cp852` | writes [`SAMPLE`] and a line feed in the encoding |
//! | `emit-binary` | byte count | writes bytes of all values, zero included |
//! | `interleave` | count | writes `out <n>` to standard output and `err <n>` to standard error in turn |

use std::io::Write;
use std::process::ExitCode;
use std::time::Duration;

/// Exit code for missing or malformed arguments.
const BAD_ARGUMENTS: u8 = 65;

/// Text written by `emit-encoding`: a Polish pangram.
const SAMPLE: &str = "Za\u{17c}\u{f3}\u{142}\u{107} g\u{119}\u{15b}l\u{105} ja\u{17a}\u{144}";

/// [`SAMPLE`] in code page 852, which `encoding_rs` does not encode.
const SAMPLE_CP852: [u8; 17] = [
    0x5a, 0x61, 0xbe, 0xa2, 0x88, 0x86, 0x20, 0x67, 0xa9, 0x98, 0x6c, 0xa5, 0x20, 0x6a, 0x61, 0xab,
    0xe4,
];

/// Multiplier and increment of the byte sequence of `emit-binary`.
const BINARY_STEP: (usize, usize) = (31, 7);

/// Runs `mode` when it belongs to this area.
pub(crate) fn run(mode: &str, args: &[String]) -> Option<ExitCode> {
    let done = match mode {
        "emit-file" => emit_file(args),
        "emit-encoding" => args.first().and_then(|name| emit_encoding(name)),
        "emit-binary" => count(args).and_then(emit_binary),
        "interleave" => count(args).and_then(interleave),
        _ => return None,
    };
    Some(done.map_or(ExitCode::from(BAD_ARGUMENTS), |()| ExitCode::SUCCESS))
}

fn count(args: &[String]) -> Option<usize> {
    args.first()?.parse().ok()
}

fn emit_file(args: &[String]) -> Option<()> {
    let content = std::fs::read(args.first()?).ok()?;
    let chunk: usize = args.get(1)?.parse().ok().filter(|chunk| *chunk > 0)?;
    let pause = Duration::from_millis(args.get(2)?.parse().ok()?);
    let mut stdout = std::io::stdout().lock();
    for part in content.chunks(chunk) {
        stdout.write_all(part).ok()?;
        stdout.flush().ok()?;
        std::thread::sleep(pause);
    }
    Some(())
}

fn emit_encoding(name: &str) -> Option<()> {
    let line = format!("{SAMPLE}\n");
    let bytes: Vec<u8> = match name {
        "utf-8" => line.into_bytes(),
        "utf-16le" => line.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        "utf-16be" => line.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        "cp1250" => encoding_rs::WINDOWS_1250.encode(&line).0.into_owned(),
        "cp852" => SAMPLE_CP852.iter().copied().chain(*b"\n").collect(),
        _ => return None,
    };
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&bytes).ok()?;
    stdout.flush().ok()
}

fn emit_binary(length: usize) -> Option<()> {
    let (multiplier, increment) = BINARY_STEP;
    let bytes: Vec<u8> = (0..length)
        .map(|index| ((index * multiplier + increment) % 256) as u8)
        .collect();
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&bytes).ok()?;
    stdout.flush().ok()
}

fn interleave(count: usize) -> Option<()> {
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    for index in 0..count {
        writeln!(stdout, "out {index}").ok()?;
        stdout.flush().ok()?;
        writeln!(stderr, "err {index}").ok()?;
        stderr.flush().ok()?;
    }
    Some(())
}
