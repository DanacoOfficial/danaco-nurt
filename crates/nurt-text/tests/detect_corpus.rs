// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-text/tests/detect_corpus.rs
// Responsibility:   Detection accuracy on a corpus of Polish and Western European texts in.
// Dependencies:     oem_cp
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Detection accuracy on a corpus of Polish and Western European texts in
//! every supported encoding and on binary content.
//!
//! Samples of 200 bytes or more must be detected correctly in at least 99%
//! of cases. Shorter samples must be detected as text.

use nurt_text::detect::{DetectOptions, detect};
use oem_cp::code_table::ENCODING_TABLE_CP852;

const LONG_SAMPLE: usize = 200;
const REQUIRED_ACCURACY_PERCENT: usize = 99;

/// Polish prose and source code with Polish comments; every text contains
/// letters that differ between `windows-1250` and `iso-8859-2`.
const POLISH: [&str; 5] = [
    "Wczoraj wieczorem pojechali\u{15b}my nad jezioro, \u{17c}eby obejrze\u{107} zach\u{f3}d s\u{142}o\u{144}ca. \
     Woda by\u{142}a spokojna, a nad brzegiem ko\u{142}ysa\u{142}y si\u{119} trzciny. Babcia opowiada\u{142}a \
     o czasach, gdy w tej okolicy sta\u{142} m\u{142}yn i ka\u{17c}dy gospodarz przywozi\u{142} zbo\u{17c}e \
     do mielenia. Wr\u{f3}cili\u{15b}my p\u{f3}\u{17a}no, zm\u{119}czeni, ale szcz\u{119}\u{15b}liwi.",
    "Zgodnie z regulaminem ka\u{17c}dy uczestnik szkolenia otrzymuje za\u{15b}wiadczenie \
     o uko\u{144}czeniu kursu. Warunkiem jest obecno\u{15b}\u{107} na co najmniej dziewi\u{119}\u{107}dziesi\u{119}ciu \
     procentach zaj\u{119}\u{107} oraz zdanie egzaminu ko\u{144}cowego. Odwo\u{142}ania nale\u{17c}y sk\u{142}ada\u{107} \
     w sekretariacie w ci\u{105}gu czternastu dni od og\u{142}oszenia wynik\u{f3}w.",
    "\u{179}r\u{f3}d\u{142}em problemu okaza\u{142}a si\u{119} \u{17a}le skonfigurowana us\u{142}uga sieciowa. Po \
     zmianie ustawie\u{144} serwer odpowiada\u{142} szybciej, a liczba b\u{142}\u{119}d\u{f3}w spad\u{142}a \
     prawie do zera. Nast\u{119}pnym krokiem b\u{119}dzie przeniesienie bazy danych na \
     nowy dysk i sprawdzenie, czy kopie zapasowe daj\u{105} si\u{119} odtworzy\u{107}.",
    "\u{15a}limak powoli wspina\u{142} si\u{119} po \u{142}odydze, zostawiaj\u{105}c za sob\u{105} l\u{15b}ni\u{105}cy \u{15b}lad. \
     Dzieci przygl\u{105}da\u{142}y mu si\u{119} z ciekawo\u{15b}ci\u{105} i liczy\u{142}y, ile czasu zajmie mu \
     dotarcie do li\u{15b}cia. Gdy wreszcie dotar\u{142} na szczyt, wszyscy klaskali, \
     a najm\u{142}odszy ch\u{142}opiec og\u{142}osi\u{142}, \u{17c}e to najszybszy \u{15b}limak w ogrodzie.",
    "/* Obliczanie \u{15b}redniej warto\u{15b}ci pomiar\u{f3}w z czujnik\u{f3}w temperatury. */\n\
     double srednia(const double *pomiary, size_t liczba) {\n\
     \x20   /* Pusta tablica oznacza brak pomiar\u{f3}w, wi\u{119}c zwracamy zero. */\n\
     \x20   if (liczba == 0) return 0.0;\n\
     \x20   double suma = 0.0; /* Suma wszystkich warto\u{15b}ci. */\n\
     \x20   for (size_t i = 0; i < liczba; ++i) suma += pomiary[i];\n\
     \x20   return suma / liczba; /* Wynik w stopniach Celsjusza. */\n\
     }\n",
];

/// German, French and Spanish prose for `windows-1252`.
const WESTERN: [&str; 3] = [
    "Die Gr\u{f6}\u{df}e des Gartens \u{fc}berraschte die G\u{e4}ste. Hinter dem Haus wuchsen \
     \u{c4}pfel, Birnen und Pflaumen, und am Zaun bl\u{fc}hten Rosen in allen Farben. \
     Der Gro\u{df}vater erz\u{e4}hlte, dass er die B\u{e4}ume vor f\u{fc}nfzig Jahren gepflanzt \
     hatte, als das Grundst\u{fc}ck noch eine Wiese war.",
    "L'\u{e9}t\u{e9} dernier, nous avons visit\u{e9} un petit village pr\u{e8}s de la c\u{f4}te. Les \
     maisons \u{e9}taient peintes en blanc et les fen\u{ea}tres d\u{e9}cor\u{e9}es de fleurs. Le \
     soir, nous mangions des cr\u{ea}pes au caf\u{e9} de la place, o\u{f9} le propri\u{e9}taire \
     racontait des histoires de p\u{ea}cheurs.",
    "El se\u{f1}or Garc\u{ed}a abri\u{f3} la puerta de la habitaci\u{f3}n y mir\u{f3} por la ventana. \
     En el jard\u{ed}n, los ni\u{f1}os jugaban junto a la fuente mientras su madre le\u{ed}a \
     una canci\u{f3}n antigua. Despu\u{e9}s de la cena, todos salieron a caminar por la \
     orilla del r\u{ed}o hasta que se ocult\u{f3} el sol.",
];

struct Sample {
    name: String,
    bytes: Vec<u8>,
    expected: Option<&'static str>,
}

fn utf16(text: &str, little_endian: bool, bom: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    for unit in std::iter::once(0xfeff)
        .filter(|_| bom)
        .chain(text.encode_utf16())
    {
        if little_endian {
            bytes.extend_from_slice(&unit.to_le_bytes());
        } else {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
    }
    bytes
}

fn polish_variants(index: usize, text: &str) -> Vec<Sample> {
    let cp852 = oem_cp::encode_string_lossy(text, &ENCODING_TABLE_CP852);
    let mut utf8_bom = b"\xef\xbb\xbf".to_vec();
    utf8_bom.extend_from_slice(text.as_bytes());
    let variants = [
        ("utf-8", text.as_bytes().to_vec(), "utf-8"),
        ("utf-8-bom", utf8_bom, "utf-8"),
        ("utf-16le-bom", utf16(text, true, true), "utf-16le"),
        ("utf-16be-bom", utf16(text, false, true), "utf-16be"),
        ("utf-16le", utf16(text, true, false), "utf-16le"),
        ("utf-16be", utf16(text, false, false), "utf-16be"),
        (
            "windows-1250",
            encoding_rs::WINDOWS_1250.encode(text).0.into_owned(),
            "windows-1250",
        ),
        ("ibm852", cp852, "ibm852"),
        (
            "iso-8859-2",
            encoding_rs::ISO_8859_2.encode(text).0.into_owned(),
            "iso-8859-2",
        ),
    ];
    variants
        .into_iter()
        .map(|(name, bytes, expected)| Sample {
            name: format!("polish[{index}] {name}"),
            bytes,
            expected: Some(expected),
        })
        .collect()
}

/// Deterministic pseudo-random bytes from a linear congruential generator.
fn pseudo_random(seed: u32, len: usize) -> Vec<u8> {
    let mut state = seed;
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            state.to_be_bytes()[0]
        })
        .collect()
}

fn binary_samples() -> Vec<Sample> {
    let mut png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR".to_vec();
    png.extend(pseudo_random(1, 600));

    let symbol_table: Vec<u8> = ["libc.so.6", "printf", "malloc", "free", "strlen", "memcpy"]
        .iter()
        .cycle()
        .take(60)
        .flat_map(|name| name.bytes().chain(std::iter::once(0)))
        .collect();

    let without_nul: Vec<u8> = pseudo_random(7, 800)
        .into_iter()
        .map(|byte| byte.max(1))
        .collect();

    let printable_with_controls: Vec<u8> = b"record header field value "
        .iter()
        .cycle()
        .take(500)
        .enumerate()
        .map(|(index, &byte)| if index % 9 == 0 { 0x02 } else { byte })
        .collect();

    [
        ("png", png),
        ("symbol table with NUL separators", symbol_table),
        ("random bytes without NUL", without_nul),
        (
            "printable bytes with STX separators",
            printable_with_controls,
        ),
    ]
    .into_iter()
    .map(|(name, bytes)| Sample {
        name: format!("binary {name}"),
        bytes,
        expected: None,
    })
    .collect()
}

fn corpus() -> Vec<Sample> {
    let mut samples: Vec<Sample> = POLISH
        .iter()
        .enumerate()
        .flat_map(|(index, text)| polish_variants(index, text))
        .collect();
    for (index, text) in WESTERN.iter().enumerate() {
        samples.push(Sample {
            name: format!("western[{index}] windows-1252"),
            bytes: encoding_rs::WINDOWS_1252.encode(text).0.into_owned(),
            expected: Some("windows-1252"),
        });
        samples.push(Sample {
            name: format!("western[{index}] utf-8"),
            bytes: text.as_bytes().to_vec(),
            expected: Some("utf-8"),
        });
    }
    samples.extend(binary_samples());
    samples
}

fn detected_label(bytes: &[u8]) -> Option<String> {
    detect(bytes, &DetectOptions::default())
        .into_encoding()
        .map(|encoding| encoding.label)
}

#[test]
fn long_samples_meet_required_accuracy() {
    let long: Vec<Sample> = corpus()
        .into_iter()
        .filter(|sample| sample.bytes.len() >= LONG_SAMPLE)
        .collect();
    let failures: Vec<String> = long
        .iter()
        .filter_map(|sample| {
            let actual = detected_label(&sample.bytes);
            (actual.as_deref() != sample.expected).then(|| {
                format!(
                    "{}: expected {:?}, detected {:?}",
                    sample.name, sample.expected, actual
                )
            })
        })
        .collect();
    let correct = long.len() - failures.len();
    assert!(
        correct * 100 >= long.len() * REQUIRED_ACCURACY_PERCENT,
        "{correct} of {} samples correct:\n{}",
        long.len(),
        failures.join("\n")
    );
}

#[test]
fn every_text_sample_is_long() {
    assert!(
        corpus()
            .iter()
            .filter(|sample| sample.expected.is_some())
            .all(|sample| sample.bytes.len() >= LONG_SAMPLE)
    );
}

#[test]
fn short_polish_samples_are_text() {
    // Single Polish words and short phrases of 3 to 9 characters.
    let words = [
        "\u{142}\u{105}ka",
        "G\u{119}\u{15b}",
        "\u{17a}r\u{f3}d\u{142}o",
        "\u{17b}\u{f3}\u{142}w",
        "\u{107}ma i o\u{15b}m",
    ];
    for word in words {
        let encodings = [
            encoding_rs::WINDOWS_1250.encode(word).0.into_owned(),
            encoding_rs::ISO_8859_2.encode(word).0.into_owned(),
            oem_cp::encode_string_lossy(word, &ENCODING_TABLE_CP852),
        ];
        for bytes in encodings {
            assert!(detected_label(&bytes).is_some(), "{word}: {bytes:?}");
        }
    }
}

#[test]
fn short_cp852_and_cp1250_samples_are_separated() {
    // Polish pangram of 18 characters with all nine Polish lowercase letters.
    let sentence = "Za\u{17c}\u{f3}\u{142}\u{107} g\u{119}\u{15b}l\u{105} ja\u{17a}\u{144}.";
    let cp852 = oem_cp::encode_string_lossy(sentence, &ENCODING_TABLE_CP852);
    let cp1250 = encoding_rs::WINDOWS_1250.encode(sentence).0.into_owned();
    assert_eq!(detected_label(&cp852).as_deref(), Some("ibm852"));
    assert_eq!(detected_label(&cp1250).as_deref(), Some("windows-1250"));
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
