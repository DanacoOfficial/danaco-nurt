<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/commands/README.md -->
<!-- Responsibility:   Documents Command documentation. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Dokumentacja poleceń {#command-documentation}

Każde polecenie katalogu ma jedną stronę: opis referencyjny dla ludzi oraz
źródło przykładów i uwag, które pokazują wbudowana pomoc i opis dla modelu.
Sygnatur nie pisze się ręcznie: nazwy, typy, pozycje, wartości domyślne
i wartości wyliczeń pochodzą z katalogu (`catalog/*.toml`, format
`nurt-catalog/1`, zob. `spec/catalog.md`), a budowa sprawdza, czy każda strona
jest z nim zgodna.

## 1. Pliki {#1-files}

| Ścieżka | Zawartość |
|---|---|
| `docs/commands/<command>.md` | strona jednego polecenia; nazwa pliku jest pełną nazwą polecenia (`replace.md`, `git.status.md`) |
| `docs/commands/_template.md` | szablon nowej strony |
| `docs/commands/README.md` | te konwencje |

Każde polecenie katalogu ma dokładnie jedną stronę, a każda strona należy do
polecenia katalogu.

## 2. Blok nagłówkowy {#2-front-matter}

Strona zaczyna się blokiem nagłówkowym YAML:

```yaml
---
command: replace
group: core
summary: Replace text in files or in the found text.
since: 0.1.0
---
```

| Pole | Reguła |
|---|---|
| `command` | pełna nazwa polecenia; równa nazwie pliku bez `.md` |
| `group` | grupa polecenia w katalogu |
| `summary` | równe polu `summary` polecenia w katalogu |
| `since` | wersja Danaco Nurt, która wprowadziła polecenie |

## 3. Sekcje {#3-sections}

Sekcje występują w tej kolejności i pod tymi nagłówkami. Sekcje oznaczone
jako „generowana” zapisuje generator na podstawie katalogu; są sprawdzane,
a nie edytowane.

| Nagłówek | Zawartość | Źródło |
|---|---|---|
| `# <command>` | tytuł: nazwa polecenia | blok nagłówkowy |
| `## Synopsis` | postacie polecenia w wariancie A, po jednej w wierszu | generowana |
| `## Description` | co robi polecenie, w kilku akapitach; zachowanie przy pustym wejściu; sposób wyznaczania celów | pisana |
| `## Parameters` | tabela: nazwa, typ, wartość domyślna, opis; parametry wymagane na początku | generowana, opisy można rozszerzać |
| `## Input and output` | przyjmowane i wytwarzane rodzaje rekordów, dla każdej postaci | generowana |
| `## Effect` | kategoria, skutek, odwracalność i jej warunki, programy zewnętrzne, zachowanie dziennika | generowana, uwagi pisane |
| `## Examples` | co najmniej trzy przykłady wykonywane przez budowę (sekcja 4) | pisana |
| `## Errors` | kody błędów zgłaszane przez polecenie z typowym sposobem naprawy | pisana |
| `## Related` | powiązane polecenia i poradniki | pisana |

Opis parametru na stronie może być dłuższy niż pole `summary` w katalogu, ale
nie może być z nim sprzeczny. Strona może dodać sekcję `## Notes` przed
`## Examples`, opisującą różnice między platformami.

## 4. Przykłady {#4-examples}

Każda strona ma co najmniej trzy przykłady. Każdy przykład to blok kodu
z oznaczeniem `nurt example`, zapisany w wariancie A składni, po którym może
następować blok z oznaczeniem `fs-before` i blok z oznaczeniem
`text expected`:

````markdown
Replace a call in one file:

```nurt example
replace "oldName(" "newName(" in: ["/home/user/app/a.go"] occurrence: 1
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/a.go", "type": "file", "content_utf8": "x := oldName(1)\n" }
] }
```

```text expected
1 file changed, 1 replacement
```
````

| Blok | Reguła |
|---|---|
| `nurt example` | jeden program; musi przejść analizę składni, powiązanie z katalogiem i uruchomienie |
| `fs-before` | opcjonalna migawka wirtualnego systemu plików sprzed przykładu, w formacie `nurt-fs-snapshot/1`; bez niej przykład działa na pustym systemie plików |
| `text expected` | opcjonalny fragment, który musi wystąpić w wyniku tekstowym przykładu |

Zdanie przed przykładem mówi, co przykład robi. Przykłady używają ścieżek
`/home/user/…` i `C:/Users/user/…` oraz hosta `example.com`; nigdy nie zależą
od maszyny, która je buduje. Pierwsze trzy przykłady strony to przykłady
pokazywane przez `help <command>`; `help <command> examples: N` pokazuje ich
więcej.

## 5. Warstwy dokumentacji generowane ze stron {#5-documentation-layers-generated-from-the-pages}

| Warstwa | Treść pobierana z katalogu | Treść pobierana ze strony |
|---|---|---|
| 1, opis rdzenia dla modelu | nazwa, parametry pozycyjne, wartości domyślne, wartości wyliczeń, podsumowanie | pierwszy przykład wybranych poleceń rdzenia |
| 2, opis grupy | to samo dla poleceń grupy | pierwszy przykład wybranych poleceń |
| 3, poradniki | tylko nazwy poleceń | brak; poradniki są osobnymi dokumentami |
| 4, wbudowana pomoc (`help <command>`) | składnia, parametry z wartościami domyślnymi, wejście i wyjście, skutek | pierwsze trzy przykłady, `## Errors`, `## Related` |
| 5, opis referencyjny (`nurt help --format man\|html\|markdown`) | wszystko | cała strona |

Czytanie każdej warstwy jest opcjonalne; żadne polecenie nie wymaga
przeczytania dokumentacji przed uruchomieniem.

## 6. Kontrole {#6-checks}

Budowa kończy się błędem, gdy:

1. polecenie katalogu nie ma strony albo strona nie ma polecenia;
2. blok nagłówkowy różni się od katalogu (`command`, `group`, `summary`);
3. sekcja generowana różni się od tekstu wygenerowanego z katalogu;
4. strona ma mniej niż trzy bloki `nurt example`;
5. przykład nie przechodzi analizy składni, nie daje się powiązać z katalogiem,
   nie uruchamia się na wirtualnym systemie plików albo jego wynik nie zawiera
   fragmentu `text expected`;
6. nazwa parametru lub polecenia podana w odwrotnych apostrofach nie istnieje
   w katalogu.

## 7. Styl {#7-style}

Strony są pisane po angielsku, w czasie teraźniejszym i w bezosobowym stylu
dokumentacji referencyjnej („`insert` adds text before or after …”), bez
narracji samouczka. Polecenia, parametry, wartości i pola rekordów zapisuje się
w odwrotnych apostrofach. Wiersze są łamane w okolicy 80 znaków, z wyjątkiem
tabel i kodu.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
