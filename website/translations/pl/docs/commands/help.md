---
command: help
group: core
summary: Opisuje polecenia, grupy, błędy, przewodniki, składnię i poprawki.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# help

## Składnia {#synopsis}

```text
help [<topic>] [name: value]...
```

## Opis {#description}

`help` zwraca dokumentację jako tekst. Bez argumentów wypisuje wprowadzenie:
linię produktów, krótki opis programów i dziennika oraz listy poleceń
podstawowych, grup, przewodników i tematów. Pierwszy wiersz to
`Danaco Nurt <version> - command shell and language where every change
can be undone`.

Temat jest rozstrzygany w tej kolejności, gdy nie podano `kind`: polecenie, kod
błędu, `syntax`, `corrections`, `commands`, `pipelines`, `let`, `paths`, `records`, `lists`, grupa, przewodnik. Ta sama nazwa może oznaczać
polecenie i grupę (`verify`): `help verify` odpowiada dla polecenia,
`help verify kind: group` i `help group: verify` odpowiadają dla grupy.

Gdy podano `kind`, temat jest wyszukiwany tylko wśród tego rodzaju.
`kind: syntax` akceptuje temat `syntax` i tematy jego części (`pipelines`, `let`,
`paths`, `records`, `lists`); `kind: corrections` akceptuje temat
`corrections`. Każda inna nazwa kończy się błędem `E_UNKNOWN_NAME` i nazwami,
które akceptuje dany rodzaj.

| Temat | Odpowiedź |
|---|---|
| polecenie | sygnatura, parametry z wartościami domyślnymi, rodzaje wejścia i wyjścia, skutek, pierwsze `examples` przykłady ze strony polecenia, typowe błędy, polecenia powiązane |
| grupa | opis grupy: jej polecenia z parametrami i przykładami |
| kod błędu | znaczenie, etapy, w których występuje, typowa poprawka |
| przewodnik | tekst przewodnika |
| `syntax` | gramatyka, literały i wyrażenia oraz reguła dotycząca etapów po skutku |
| `corrections` | klasy i polityka poprawek, rodziny reguł i pola wyniku |
| `commands` | każde polecenie katalogu pod swoją grupą, wraz z jego streszczeniem |
| `pipelines`, `let`, `paths`, `records`, `lists` | część składni, którą wskazuje nazwa, czyli wiersze `syntax`, które mają zastosowanie; wskazówki błędów odsyłają do tych tematów |
| `search` | polecenia, kody błędów i przewodniki pasujące do słów, najlepsze pierwsze |

Przy `kind: syntax` temat jest pusty, `syntax` albo jednym z tematów
`pipelines`, `let`, `paths`, `records`, `lists`; przy `kind: corrections` jest
pusty albo `corrections`. Każda inna nazwa kończy się błędem `E_UNKNOWN_NAME` i
nie jest ignorowana.

Tematy `group`, `skill` i `search` wzajemnie się wykluczają i wykluczają temat
pozycyjny. Nieznany temat kończy się błędem `E_UNKNOWN_NAME`; błąd wymienia w
`expected` do ośmiu najbliższych nazw.

Odpowiedź jest tekstem. Z parametrem wywołania `format: json` odpowiedź jest
dokumentem JSON o tej samej zawartości. `help "commands"` wyświetla polecenia
wszystkich grup; `help group: "<group>"` opisuje jedną grupę.

Parametr wywołania `language: pl`, albo `output.language = "pl"`, gdy wywołanie
go nie podaje, wybiera język polski dla stałych tekstów `help` i `describe`:
wprowadzenia, nagłówków i etykiet. Parametr wywołania ma pierwszeństwo przed
konfiguracją. Nazwy poleceń, parametry, pola rekordów, kody błędów, streszczenia
poleceń i przewodniki pozostają po angielsku.

`language: pl` ustawia także komunikaty wyniku na język polski: `message` błędu
to polskie znaczenie jego kodu, po którym następują `found`, `expected` i
`problem`; angielski komunikat jest zachowany w szczególe błędu `message_en`.
`detail` każdej uwagi i każdego nieodwracalnego skutku zaczyna się od polskiego
tekstu jego kodu, po którym następuje angielski szczegół, jeśli istnieje. Kody,
programy w `fix` i `next_steps` oraz temat pomocy nigdy nie są tłumaczone. Kod
bez polskiego tekstu zachowuje swój komunikat angielski.

`help` nigdy nie zmienia stanu i żadne polecenie nie wymaga jego przeczytania
przed uruchomieniem.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `topic` | `text` | lista poleceń, grup, przewodników i tematów | Polecenie, grupa, kod błędu, składnia lub poprawki. |
| `kind` | `enum(command\|group\|error\|skill\|syntax\|corrections)` | `command` | Rodzaj tematu. |
| `group` | `text` | brak | Grupa do opisania, tak samo jak kind: group. |
| `skill` | `text` | brak | Przewodnik do pokazania, tak samo jak kind: skill. |
| `search` | `text` | brak | Słowa do wyszukania w poleceniach, błędach i przewodnikach. |
| `examples` | `int` | `3` | Liczba pokazywanych przykładów. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `text` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `help` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Pokaż wprowadzenie.

```nurt example
help
```

```text expected
command shell and language where every change can be undone
```

Pokaż sygnaturę polecenia z jego pierwszym przykładem.

```nurt example
help "replace" examples: 1
```

```text expected
replace
```

Pokaż opis grupy.

```nurt example
help group: "documents"
```

```text expected
GROUP documents
```

Pokaż znaczenie i poprawkę kodu błędu.

```nurt example
help "E_ASSERT"
```

```text expected
E_ASSERT
```

Wyszukaj tematy, które wspominają słowo.

```nurt example
help search: "rename symbol"
```

```text expected
code.rename
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_UNKNOWN_NAME` | temat nie jest poleceniem, grupą, kodem błędu, przewodnikiem ani tematem | wybierz nazwę z `expected` albo uruchom `help`, aby zobaczyć listę |
| `E_PARAM_CONFLICT` | podano więcej niż jeden z `topic`, `group`, `skill` i `search` | zachowaj jeden z nich |

## Powiązane {#related}

`describe`, `evaluate`, `help skill: "bash-migration"`.
