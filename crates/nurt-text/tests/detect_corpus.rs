// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
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
    "Wczoraj wieczorem pojechaliśmy nad jezioro, żeby obejrzeć zachód słońca. \
     Woda była spokojna, a nad brzegiem kołysały się trzciny. Babcia opowiadała \
     o czasach, gdy w tej okolicy stał młyn i każdy gospodarz przywoził zboże \
     do mielenia. Wróciliśmy późno, zmęczeni, ale szczęśliwi.",
    "Zgodnie z regulaminem każdy uczestnik szkolenia otrzymuje zaświadczenie \
     o ukończeniu kursu. Warunkiem jest obecność na co najmniej dziewięćdziesięciu \
     procentach zajęć oraz zdanie egzaminu końcowego. Odwołania należy składać \
     w sekretariacie w ciągu czternastu dni od ogłoszenia wyników.",
    "Źródłem problemu okazała się źle skonfigurowana usługa sieciowa. Po \
     zmianie ustawień serwer odpowiadał szybciej, a liczba błędów spadła \
     prawie do zera. Następnym krokiem będzie przeniesienie bazy danych na \
     nowy dysk i sprawdzenie, czy kopie zapasowe dają się odtworzyć.",
    "Ślimak powoli wspinał się po łodydze, zostawiając za sobą lśniący ślad. \
     Dzieci przyglądały mu się z ciekawością i liczyły, ile czasu zajmie mu \
     dotarcie do liścia. Gdy wreszcie dotarł na szczyt, wszyscy klaskali, \
     a najmłodszy chłopiec ogłosił, że to najszybszy ślimak w ogrodzie.",
    "/* Obliczanie średniej wartości pomiarów z czujników temperatury. */\n\
     double srednia(const double *pomiary, size_t liczba) {\n\
     \x20   /* Pusta tablica oznacza brak pomiarów, więc zwracamy zero. */\n\
     \x20   if (liczba == 0) return 0.0;\n\
     \x20   double suma = 0.0; /* Suma wszystkich wartości. */\n\
     \x20   for (size_t i = 0; i < liczba; ++i) suma += pomiary[i];\n\
     \x20   return suma / liczba; /* Wynik w stopniach Celsjusza. */\n\
     }\n",
];

/// German, French and Spanish prose for `windows-1252`.
const WESTERN: [&str; 3] = [
    "Die Größe des Gartens überraschte die Gäste. Hinter dem Haus wuchsen \
     Äpfel, Birnen und Pflaumen, und am Zaun blühten Rosen in allen Farben. \
     Der Großvater erzählte, dass er die Bäume vor fünfzig Jahren gepflanzt \
     hatte, als das Grundstück noch eine Wiese war.",
    "L'été dernier, nous avons visité un petit village près de la côte. Les \
     maisons étaient peintes en blanc et les fenêtres décorées de fleurs. Le \
     soir, nous mangions des crêpes au café de la place, où le propriétaire \
     racontait des histoires de pêcheurs.",
    "El señor García abrió la puerta de la habitación y miró por la ventana. \
     En el jardín, los niños jugaban junto a la fuente mientras su madre leía \
     una canción antigua. Después de la cena, todos salieron a caminar por la \
     orilla del río hasta que se ocultó el sol.",
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
    let words = ["łąka", "Gęś", "źródło", "Żółw", "ćma i ośm"];
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
    let sentence = "Zażółć gęślą jaźń.";
    let cp852 = oem_cp::encode_string_lossy(sentence, &ENCODING_TABLE_CP852);
    let cp1250 = encoding_rs::WINDOWS_1250.encode(sentence).0.into_owned();
    assert_eq!(detected_label(&cp852).as_deref(), Some("ibm852"));
    assert_eq!(detected_label(&cp1250).as_deref(), Some("windows-1250"));
}
