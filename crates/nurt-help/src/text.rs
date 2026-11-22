// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Fixed texts of the help and of the description layers, in English and
//! Polish. Command lines, parameters, codes and record fields are generated
//! from the catalog and never translated.

use nurt_contracts::envelope::Language;

/// Canonical sentence that describes the product.
pub const TAGLINE: &str = "Command shell and language where every change can be undone";

/// First line of the `help` output: the product name, the package version
/// and the canonical sentence.
#[must_use]
pub fn first_line() -> String {
    let mut chars = TAGLINE.chars();
    let lowered: String = chars
        .next()
        .map(|first| first.to_lowercase().chain(chars).collect())
        .unwrap_or_default();
    format!("Danaco Nurt {} - {lowered}", env!("CARGO_PKG_VERSION"))
}

/// Line of a description layer with the priority that decides whether the
/// line survives shortening: lines of priority 0 are always kept.
pub(crate) struct Line {
    pub(crate) priority: u32,
    pub(crate) en: &'static str,
    pub(crate) pl: &'static str,
}

impl Line {
    const fn same(priority: u32, text: &'static str) -> Self {
        Self {
            priority,
            en: text,
            pl: text,
        }
    }

    pub(crate) fn text(&self, language: Language) -> &'static str {
        match language {
            Language::Pl => self.pl,
            _ => self.en,
        }
    }
}

/// Block of lines with an optional heading.
pub(crate) struct Block {
    pub(crate) header_en: Option<&'static str>,
    pub(crate) header_pl: Option<&'static str>,
    /// The lines are programs, shown in the syntax variant of the request.
    pub(crate) programs: bool,
    pub(crate) lines: &'static [Line],
}

impl Block {
    pub(crate) fn header(&self, language: Language) -> Option<&'static str> {
        match language {
            Language::Pl => self.header_pl,
            _ => self.header_en,
        }
    }
}

const INTRO: Block = Block {
    header_en: None,
    header_pl: None,
    programs: false,
    lines: &[Line {
        priority: 0,
        en: "Danaco Nurt (`nurt`) — console for files, text, programs and the system (replaces bash and PowerShell; identical on Linux, macOS and Windows). Call parameter: `program` (Nurt text).",
        pl: "Danaco Nurt (`nurt`) — konsola do plików, tekstu, programów i systemu (zastępuje basha i PowerShella; działa tak samo w systemach Linux, macOS i Windows). Parametr wywołania: `program` (tekst Nurt).",
    }],
};

const PROGRAM: Block = Block {
    header_en: Some("PROGRAM"),
    header_pl: Some("PROGRAM"),
    programs: false,
    lines: &[
        Line {
            priority: 0,
            en: "program = [let name = PIPELINE]… PIPELINE. PIPELINE = source |> stage |> stage… A line ends a command; a line starting with `|>` continues it. Comment: `#`.",
            pl: "program = [let nazwa = POTOK]… POTOK. POTOK = źródło |> etap |> etap… Koniec wiersza kończy polecenie; wiersz zaczynający się od `|>` je kontynuuje. Komentarz: `#`.",
        },
        Line {
            priority: 0,
            en: "Data between stages are typed records (entry, line, match…), not text. `@name` = result of let; `...@name.path` = list spread. There is no $, cd, ;, &&, ||, -flags, text substitution or current directory.",
            pl: "Dane między etapami to typowane rekordy (entry, line, match…), nie tekst. `@name` = wynik `let`; `...@name.path` = rozwinięcie listy. Nie ma $, cd, ;, &&, ||, flag z myślnikiem, podstawiania tekstu ani bieżącego katalogu.",
        },
        Line {
            priority: 0,
            en: r##"Literals: path = quoted text "/srv/app" or "C:/proj" (always quoted, absolute, `/` separator; relative only with call parameter `base`); text "…" (escapes \\ \" \n \t \u{..}) or raw r"…" / r#"…"#; regex /…/i (flags i m s x; `\/` for slash; RE2 syntax); glob "**/*.go"."##,
            pl: r##"Literały: ścieżka = tekst w cudzysłowie "/srv/app" lub "C:/proj" (zawsze w cudzysłowie, bezwzględna, separator `/`; względna tylko z parametrem wywołania `base`); tekst "…" (sekwencje \\ \" \n \t \u{..}) lub surowy r"…" / r#"…"#; wyrażenie regularne /…/i (flagi i m s x; `\/` dla ukośnika; składnia RE2); glob "**/*.go"."##,
        },
        Line {
            priority: 1,
            en: r#"Also: size 10mb 1.5gib; duration 30s 5m 1h30m 7d; time "2026-11-21T10:00:00Z", ago(7d), now(); range 5, 1..5, 40.., ..9; list [a, b]; map {k: "v"}; document key "a.b[0].c"."#,
            pl: r#"Ponadto: rozmiar 10mb 1.5gib; czas trwania 30s 5m 1h30m 7d; chwila "2026-11-21T10:00:00Z", ago(7d), now(); zakres 5, 1..5, 40.., ..9; lista [a, b]; mapa {k: "v"}; klucz dokumentu "a.b[0].c"."#,
        },
        Line {
            priority: 0,
            en: "Arguments: positional first, then `name: value`. Enum values are bare words (order: desc).",
            pl: "Argumenty: najpierw pozycyjne, potem `name: value`. Wartości wyliczeniowe to gołe słowa (order: desc).",
        },
        Line::same(
            1,
            "Expressions (filter, assert, select): record fields, == != < <= > >= ~ (regex) !~ in and or not + - * /, functions lower upper len trim contains(a,b) starts_with(a,b) ends_with(a,b) ago(d) now().",
        ),
    ],
};

const RULES: Block = Block {
    header_en: Some("RULES"),
    header_pl: Some("ZASADY"),
    programs: false,
    lines: &[
        Line {
            priority: 0,
            en: "1. One action per step: at most one changing stage (write, replace, remove, move, copy, run…), placed last. It may be followed by verify (…), and after the effect by assert, filter, select, first, last, skip, count and sort on the result records. A failed check after the effect is an error without rollback; only verify rolls back. Several actions = several calls.",
            pl: "1. Jedna akcja na krok: najwyżej jeden etap zmieniający (write, replace, remove, move, copy, run…), na końcu. Po nim może być verify (…), a po skutku assert, filter, select, first, last, skip, count i sort na rekordach wyniku. Niespełnione sprawdzenie po skutku to błąd bez wycofania; wycofuje wyłącznie verify. Kilka akcji = kilka wywołań.",
        },
        Line {
            priority: 0,
            en: "2. Patterns are allowed; the console lists the targets it computed. Optional check: assert count: 12 | paths: 12 | exit_code: 0.",
            pl: "2. Wzorce są dozwolone; konsola wypisuje obliczone cele. Opcjonalne sprawdzenie: assert count: 12 | paths: 12 | exit_code: 0.",
        },
        Line {
            priority: 1,
            en: "3. The result shows the effect: diff, files, counts, sizes, notable (outside project, mass delete…) and irreversible. A changing stage on an empty list does nothing (0).",
            pl: "3. Wynik pokazuje skutek: diff, pliki, liczby, rozmiary, notable (poza projektem, masowe usuwanie…) i irreversible. Etap zmieniający na pustej liście nic nie robi (0).",
        },
        Line {
            priority: 0,
            en: "4. An error before the effect stops the whole program and nothing is changed. Files are written atomically with an etag check (E_STALE); encoding and line endings are preserved; output is UTF-8.",
            pl: "4. Błąd przed skutkiem zatrzymuje cały program i nic się nie zmienia. Pliki są zapisywane atomowo ze sprawdzeniem etag (E_STALE); kodowanie i końce wierszy są zachowane; wyjście jest w UTF-8.",
        },
        Line {
            priority: 1,
            en: "5. A write that adds a NEW syntax error to a known format (JSON, TOML, YAML, XML, code) is rolled back (validate: reject).",
            pl: "5. Zapis, który wprowadza NOWY błąd składni do znanego formatu (JSON, TOML, YAML, XML, kod), jest wycofywany (validate: reject).",
        },
        Line {
            priority: 1,
            en: "6. The form is corrected when safe (typos, units, quotes, bash and PowerShell habits); the result has `corrected` and `ran` (what was executed). If a correction would touch the target of a change, the answer is E_AMBIGUOUS with ready programs to resend.",
            pl: "6. Forma jest poprawiana, gdy to bezpieczne (literówki, jednostki, cudzysłowy, nawyki basha i PowerShella); wynik zawiera `corrected` i `ran` (co wykonano). Gdy korekta dotknęłaby celu zmiany, odpowiedzią jest E_AMBIGUOUS z gotowymi programami do ponownego wysłania.",
        },
        Line {
            priority: 1,
            en: "7. A mistake visible in the result can be repaired with undo (history lists the operations; history state: partial and undo on_conflict: partial concern entries that can be restored only in part; undo paths: restores selected paths). It repairs errors and does not replace checking: use preview: true or assert when unsure.",
            pl: "7. Pomyłkę widoczną w wyniku można naprawić poleceniem undo (history wypisuje operacje; history state: partial i undo on_conflict: partial dotyczą wpisów przywracalnych tylko częściowo; undo paths: przywraca wybrane ścieżki). Naprawia błędy, ale nie zastępuje sprawdzania: gdy nie ma pewności, użyj preview: true lub assert.",
        },
        Line {
            priority: 1,
            en: "8. Limits: the console does not prevent logic errors of the programs you run; changes made by run are detected only under `watch` (default: the working directory of the program) and may be marked irreversible.",
            pl: "8. Ograniczenia: konsola nie zapobiega błędom logicznym uruchamianych programów; zmiany wykonane przez run są wykrywane tylko w `watch` (domyślnie katalog roboczy programu) i mogą być oznaczone jako nieodwracalne.",
        },
    ],
};

pub(crate) const HEAD: &[Block] = &[INTRO, PROGRAM, RULES];

/// Heading of the command section of the core layer.
pub(crate) const COMMANDS_LEGEND: Line = Line {
    priority: 0,
    en: "CORE COMMANDS (UPPER = positional, * = required, … = list; name:value = default; name:a|b = enumeration, default first; name: = no default)",
    pl: "POLECENIA RDZENIA (WERSALIKI = pozycyjny, * = wymagany, … = lista; name:value = wartość domyślna; name:a|b = wyliczenie, domyślna pierwsza; name: = bez wartości domyślnej)",
};

/// Headings of the subsections of the command section.
pub(crate) const SOURCES_HEADING: (&str, &str) = ("sources:", "źródła:");
pub(crate) const STAGES_HEADING: (&str, &str) = ("stages:", "etapy:");
pub(crate) const CHANGES_HEADING: (&str, &str) = ("changes:", "zmiany:");

const RECORDS: Block = Block {
    header_en: None,
    header_pl: None,
    programs: false,
    lines: &[Line::same(
        2,
        "RECORDS (fields): entry(path name stem extension parent type size modified created mode owner link_target encoding line_ending binary hash etag) · content(path text lines etag encoding line_ending) · line(path no text) · match(path no column text line groups) · exec(command cwd exit_code signal timed_out stdout stderr changes) · job(id state exit_code elapsed stdout stderr) · operation(id time kind summary state files irreversible)",
    )],
};

const CALLS: Block = Block {
    header_en: None,
    header_pl: None,
    programs: false,
    lines: &[Line {
        priority: 2,
        en: "CALL PARAMETERS: preview (only targets and effect), base (directory for relative paths), limit=50, cursor, fields, format=text|json|markdown|compact, verbosity=quiet|normal|detailed, diff=none|stat|unified, max_chars, save_to, timeout, session, language.",
        pl: "PARAMETRY WYWOŁANIA: preview (tylko cele i skutek), base (katalog dla ścieżek względnych), limit=50, cursor, fields, format=text|json|markdown|compact, verbosity=quiet|normal|detailed, diff=none|stat|unified, max_chars, save_to, timeout, session, language.",
    }],
};

const EXAMPLES: Block = Block {
    header_en: Some("EXAMPLES"),
    header_pl: Some("PRZYKŁADY"),
    programs: true,
    lines: &[
        Line::same(
            2,
            r#"find "/srv/app/src" glob: "**/*.go" |> filter size > 1mb |> sort size order: desc |> first 20"#,
        ),
        Line::same(
            2,
            r#"search /oldName\(/ "/srv/app/src" glob: "**/*.go" |> assert paths: 12 |> replace "newName(""#,
        ),
        Line::same(3, r#"find "/srv/app/build" glob: "**/*.tmp" |> remove"#),
        Line::same(3, r#"write "/srv/app/.gitignore" "dist/\n" mode: append"#),
        Line::same(4, r#"read "/srv/app/main.go" lines: 10..40"#),
        Line::same(
            2,
            r#"run ["go","test","./..."] cwd: "/srv/app" |> assert exit_code: 0"#,
        ),
        Line::same(
            4,
            "let ts = find \"/srv/app\" glob: \"**/*.ts\" |> filter modified > ago(1h)\n@ts |> count",
        ),
    ],
};

const HELP_POINTER: Block = Block {
    header_en: None,
    header_pl: None,
    programs: false,
    lines: &[Line {
        priority: 1,
        en: r#"Help: help, help search: "words", help group: "documents", help "E_ASSERT", help skill: "bash-migration". Help is optional; it is never required before running a program."#,
        pl: r#"Pomoc: help, help search: "słowa", help group: "documents", help "E_ASSERT", help skill: "bash-migration". Pomoc jest opcjonalna; nigdy nie jest wymagana przed uruchomieniem programu."#,
    }],
};

pub(crate) const MIDDLE: &[Block] = &[RECORDS, CALLS, EXAMPLES];

pub(crate) const TAIL: &[Block] = &[HELP_POINTER];

/// Introduction to the list of groups in the core layer.
pub(crate) const GROUPS_LEAD: Line = Line {
    priority: 1,
    en: "Groups (described on request with help group: NAME):",
    pl: "Grupy (opisy na żądanie poleceniem help group: NAZWA):",
};

/// Labels of the help output.
pub(crate) struct Labels {
    pub(crate) intro: &'static [&'static str],
    pub(crate) closing: &'static [&'static str],
    pub(crate) examples: &'static str,
    pub(crate) errors: &'static str,
    pub(crate) related: &'static str,
    pub(crate) params: &'static str,
    pub(crate) input: &'static str,
    pub(crate) output: &'static str,
    pub(crate) effect: &'static str,
    pub(crate) stage: &'static str,
    pub(crate) fix: &'static str,
    pub(crate) commands: &'static str,
    pub(crate) groups: &'static str,
    pub(crate) guides: &'static str,
    pub(crate) topics: &'static str,
    pub(crate) topic_names: &'static str,
    pub(crate) no_matches: &'static str,
    pub(crate) search_for: &'static str,
}

const LABELS_EN: Labels = Labels {
    intro: &[
        "A program is a pipeline: a source, then stages joined by |>.",
        r#"  find "/home/user/app" glob: "**/*.log" |> remove"#,
        "Changes to files are recorded in the journal: history lists the operations and undo reverts one.",
        "Add preview: true to see the targets and the effect before anything changes.",
    ],
    closing: &[
        r#"help <command> shows the signature, the parameters and examples; help search: "words" finds a topic."#,
        "Help is optional: no command requires it.",
    ],
    examples: "examples:",
    errors: "errors:",
    related: "related:",
    params: "params:",
    input: "input:",
    output: "output:",
    effect: "effect:",
    stage: "stage:",
    fix: "fix:",
    commands: "commands:",
    groups: "groups:",
    guides: "guides:",
    topics: "topics:",
    topic_names: "syntax, corrections, error codes such as E_ASSERT",
    no_matches: "no matches; run help for the list of topics",
    search_for: "search:",
};

const LABELS_PL: Labels = Labels {
    intro: &[
        "Program jest potokiem: źródło, a po nim etapy połączone znakiem |>.",
        r#"  find "/home/user/app" glob: "**/*.log" |> remove"#,
        "Zmiany plików są zapisywane w dzienniku: history wypisuje operacje, a undo cofa jedną z nich.",
        "Parametr preview: true pokazuje cele i skutek, zanim cokolwiek się zmieni.",
    ],
    closing: &[
        r#"help <command> pokazuje sygnaturę, parametry i przykłady; help search: "słowa" znajduje temat."#,
        "Pomoc jest opcjonalna: żadne polecenie jej nie wymaga.",
    ],
    examples: "przykłady:",
    errors: "błędy:",
    related: "powiązane:",
    params: "parametry:",
    input: "wejście:",
    output: "wyjście:",
    effect: "skutek:",
    stage: "etap:",
    fix: "naprawa:",
    commands: "polecenia:",
    groups: "grupy:",
    guides: "przewodniki:",
    topics: "tematy:",
    topic_names: "syntax, corrections, kody błędów, na przykład E_ASSERT",
    no_matches: "brak wyników; help wypisuje listę tematów",
    search_for: "szukanie:",
};

pub(crate) const fn labels(language: Language) -> &'static Labels {
    match language {
        Language::Pl => &LABELS_PL,
        _ => &LABELS_EN,
    }
}

const CORRECTIONS_EN: &[&str] = &[
    "Corrections: a program that differs from the language only in form is corrected before it runs.",
    "classes: C certain (notation only) · S structural (a syntax mark added or removed) · F fuzzy name (edit distance) · P probable (touches the bounds of a literal).",
    "read-only programs: C, S, F and P are applied and reported.",
    "programs with a changing stage or run: C always; S when the structure is unambiguous; F when the name is not that of a changing stage, or differs by one edit and the whole program binds; P is never applied and gives E_AMBIGUOUS with candidates.",
    "rules: L01-L24 spelling, case, naming style, synonyms, quotes, brackets, separators, paths; T01-T09 sizes, durations, booleans, numbers, ranges; V01-V04 enumeration values, fields, functions, names; the tables B and P translate bash and PowerShell habits.",
    "result: corrected lists each correction with its rule and class, ran shows the program that was executed.",
    "E_AMBIGUOUS lists up to 3 ready programs; send one of them, or send accept with its reference; a reference expires (E_EXPIRED_CANDIDATE).",
    "never corrected: the bytes of literals, numbers, the set of targets, the kind of effect, mode parameters and base.",
    r#"help skill: "bash-migration" and help skill: "powershell-migration" show the habits that are translated."#,
];

const CORRECTIONS_PL: &[&str] = &[
    "Korekta: program, który odbiega od języka wyłącznie formą, jest poprawiany przed wykonaniem.",
    "klasy: C pewna (sama notacja) · S strukturalna (dodany lub usunięty znak składni) · F nazwa rozmyta (odległość edycyjna) · P prawdopodobna (dotyka granic literału).",
    "programy odczytu: klasy C, S, F i P są stosowane i raportowane.",
    "programy z etapem zmieniającym lub run: C zawsze; S gdy struktura jest jednoznaczna; F gdy nazwa nie należy do etapu zmieniającego albo różni się o jedną edycję i cały program się wiąże; P nigdy nie jest stosowana i daje E_AMBIGUOUS z kandydatami.",
    "reguły: L01-L24 pisownia, wielkość liter, styl nazw, synonimy, cudzysłowy, nawiasy, separatory, ścieżki; T01-T09 rozmiary, czasy trwania, wartości logiczne, liczby, zakresy; V01-V04 wartości wyliczeniowe, pola, funkcje, nazwy; nawyki basha i PowerShella przekładają tabele B i P.",
    "wynik: corrected wymienia każdą korektę z regułą i klasą, ran pokazuje wykonany program.",
    "E_AMBIGUOUS wymienia do 3 gotowych programów; wyślij jeden z nich albo wyślij accept z jego odnośnikiem; odnośnik wygasa (E_EXPIRED_CANDIDATE).",
    "nigdy nie są korygowane: bajty literałów, liczby, zbiór celów, rodzaj skutku, parametry trybu i base.",
    r#"help skill: "bash-migration" i help skill: "powershell-migration" pokazują przekładane nawyki."#,
];

/// Lines of the `corrections` topic.
pub(crate) const fn corrections(language: Language) -> &'static [&'static str] {
    match language {
        Language::Pl => CORRECTIONS_PL,
        _ => CORRECTIONS_EN,
    }
}

/// Lines of the `syntax` topic: the grammar, the literals, the expressions
/// and the first rule, taken from the core layer so that both agree.
pub(crate) fn syntax(language: Language) -> Vec<&'static str> {
    let mut lines: Vec<&'static str> = PROGRAM
        .lines
        .iter()
        .map(|line| line.text(language))
        .collect();
    if let Some(rule) = RULES.lines.first() {
        lines.push(rule.text(language));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_line_names_the_version_and_the_tagline_in_ascii() {
        let line = first_line();
        assert!(line.starts_with(concat!("Danaco Nurt ", env!("CARGO_PKG_VERSION"), " - ")));
        assert!(line.ends_with("command shell and language where every change can be undone"));
        assert!(line.is_ascii());
    }

    #[test]
    fn tagline_is_the_canonical_sentence() {
        assert_eq!(
            TAGLINE,
            "Command shell and language where every change can be undone"
        );
    }

    #[test]
    fn priority_zero_lines_exist_in_every_head_block() {
        for block in HEAD {
            assert!(block.lines.iter().any(|line| line.priority == 0));
        }
    }

    #[test]
    fn syntax_topic_ends_with_the_rule_on_stages_after_the_effect() {
        let lines = syntax(Language::En);
        assert!(
            lines
                .last()
                .is_some_and(|line| line.contains("after the effect"))
        );
    }
}
