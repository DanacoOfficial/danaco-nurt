<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           spec/syntax-tree.md -->
<!-- Responsibility:   Documents Syntax trees: `nurt-ast/1` and `nurt-bound/1`. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Drzewa składni: `nurt-ast/1` i `nurt-bound/1` {#syntax-trees-nurt-ast1-and-nurt-bound1}

Nurt wykonuje i zapisuje drzewo składni, nigdy tekst programu. Tekst programu w
dowolnym wariancie składni jest poprawiany i parsowany do **drzewa
powierzchniowego** (`nurt-ast/1`). Powiązanie drzewa powierzchniowego z katalogiem
poleceń tworzy **drzewo powiązane** (`nurt-bound/1`). Instancja na urządzeniu
zdalnym otrzymuje drzewo powierzchniowe zakodowane jako JSON lub CBOR i wiąże je
własnym katalogiem, więc program nigdy nie przechodzi przez inny interpreter i
nie występują warstwy cytowania. Dziennik i raport skutku przechowują drzewo
powiązane.

Definicjami normatywnymi są schematy JSON
[`schemas/nurt-ast-1.schema.json`](schemas/nurt-ast-1.schema.json) i
[`schemas/nurt-bound-1.schema.json`](schemas/nurt-bound-1.schema.json). Typy Rust
modułów `nurt_contracts::ast` i `nurt_contracts::bound` serializują się dokładnie
do tych dokumentów.

## 1. Identyfikatory i wersje {#1-identifiers-and-versions}

| Element | Drzewo powierzchniowe | Drzewo powiązane |
|---|---|---|
| Nazwa formatu | `nurt-ast/1` | `nurt-bound/1` |
| Składnik wersji | `"nurt": "1.1"` | `"nurt": "1.1"` i `"bound": "1.1"` |
| Typ nośnika | `application/vnd.nurt.ast+json; v=1`, `application/vnd.nurt.ast+cbor; v=1` | `application/vnd.nurt.bound+json; v=1`, `application/vnd.nurt.bound+cbor; v=1` |
| Rozszerzenie pliku | `.nurt.json` (drzewo), `.nurt` (tekst programu, UTF-8) | — |
| Schemat | `nurt-ast-1.schema.json` | `nurt-bound-1.schema.json` |
| Kodowanie kanoniczne | JSON według RFC 8785 (JCS); deterministyczny CBOR (RFC 8949, sekcja 4.2) | tak samo |

Wersje są zapisywane jako `MAJOR.MINOR`. W obrębie jednej wersji głównej dodawane
są tylko składniki opcjonalne i nowe rodzaje węzłów; nowe polecenia i parametry
są danymi katalogu i nigdy nie zmieniają schematu. Czytnik akceptuje dokument
własnej wersji głównej, którego wersja podrzędna nie jest większa od jego własnej;
nowsza wersja podrzędna jest odrzucana z błędem `E_AST_VERSION` i listą
nieznanych rodzajów węzłów. Rodzaje węzłów znane czytnikowi wymieniają
`nurt_contracts::ast::SURFACE_KINDS` i `nurt_contracts::bound::BOUND_KINDS`. Po
zmianie wersji głównej czytniki akceptują poprzednią wersję główną przez co
najmniej 24 miesiące.

Oba dokumenty są odrębne: schemat powierzchniowy odrzuca składniki korzenia
`bound`, `catalog_rev` i `corr_rev`, składnik etapu `effect` oraz składnik
argumentu `default`; schemat powiązany je wymaga.

## 2. Konwencje wspólne {#2-common-conventions}

* Każdy obiekt węzła zawiera swój rodzaj w składniku `"t"`. Obiekty argumentów i
  wpisy map nie są węzłami i nie mają `"t"`.
* Składniki opcjonalne są pomijane, gdy ich nie ma. Pusta ścieżka pól odwołania
  jest pomijana. Tablice będące częścią węzła (`args`, `stages`, `items`,
  `entries`) są zawsze obecne, być może puste.
* Dane użytkownika nigdy nie pojawiają się jako nazwa składnika: wpisy map są
  obiektami `{"k": key, "v": value}`.
* Czytnik odrzuca nieznany składnik dowolnego obiektu z błędem `E_AST_SCHEMA`.
* Identyfikatory pasują do `[a-z][a-z0-9_]*` i mają co najwyżej 64 bajty; nazwy
  poleceń mają co najwyżej 129 bajtów. Słowa zastrzeżone `let`, `true`, `false`,
  `null`, `and`, `or`, `not` i `in` nie są dopuszczalne jako nazwa wyniku `let`,
  jako nazwa odwołania ani jako węzeł `ident`; pozostają dopuszczalne jako
  nazwy parametrów i klucze map.
* `meta` jest obiektem w korzeniu z opcjonalnymi składnikami `source` (tekst
  programu), `variant` (`a`, `b` lub `c`) i `comments` (obiekty `{span, text}`,
  tekst bez wiodącego `#`); inne składniki są zachowywane. Komentarze nie są
  częścią programu; formater przywraca je z `meta.comments`.
* Dokument ma co najwyżej 128 poziomów głębokości: głębokością zagnieżdżenia jest
  liczba wartości JSON na ścieżce od korzenia do najgłębszej wartości, przy czym
  korzeń leży na głębokości 1. Parser mierzy tę samą głębokość na budowanym
  drzewie i odrzuca głębszy program z błędem `E_SYNTAX` (`nesting_too_deep`), więc
  program akceptowany przez parser zawsze koduje się do dokumentu powierzchniowego
  akceptowanego przez dekodery.
* `span` to `[line, column, end_line, end_column]`. Wiersze i kolumny zaczynają
  się od 1, kolumny liczą skalarne wartości Unicode, a pozycja końcowa jest
  wyłączna (pozycja bezpośrednio za ostatnim znakiem). Pozycje nie przekraczają
  2^32 - 1. Zakresy mają charakter informacyjny.

### Zakresy drzewa powierzchniowego {#spans-of-the-surface-tree}

| Węzeł | Obejmuje |
|---|---|
| `let` | od `let` do końca potoku |
| `pipeline` | od początku głowy do końca ostatniego etapu |
| `stage` | od nazwy polecenia do jego ostatniego tokenu: końca ostatniego argumentu albo, gdy argumenty są zapisane w nawiasach (notacja B), nawiasu zamykającego |
| argument | argument nazwany od jego nazwy do końca wartości; argument pozycyjny jest równy swojej wartości |
| literał, identyfikator | token, wraz z cudzysłowami, ogranicznikami `r#` tekstu surowego i flagami wyrażenia regularnego |
| `ref` | od `@` do końca ostatniego pola |
| `list`, `map`, `call`, `sub` | od nawiasu otwierającego lub nazwy do nawiasu zamykającego |
| wpis mapy | od klucza do końca wartości |
| `bin` | od początku lewego operandu do końca prawego operandu |
| `un`, `spread` | od operatora do końca operandu |
| `field` | od początku wyrażenia rekordu do końca nazwy pola |
| `range` | od pierwszej do ostatniej zapisanej granicy |
| wyrażenie w nawiasach | węzeł wyrażenia wewnętrznego; jego zakres obejmuje nawiasy |

## 3. Drzewo powierzchniowe {#3-surface-tree}

Składniki korzenia: `nurt` (wymagany), `program` (wymagany), `meta`.
`program.statements` zawiera co najmniej jedną instrukcję.

| `t` | Składniki | Postać tekstowa (notacja słowna) |
|---|---|---|
| `let` | `name`, `value: pipeline` | `let recent = find …` |
| `pipeline` | `head: stage \| ref`, `stages: [stage]` | `find … \|> filter … \|> first 20` |
| `stage` | `name`, `args: [argument]` | `filter size > 1mb` |
| argument | `name?`, `value` | `glob: "**/*.go"`, `20` |
| `text` | `v` | `"abc"`, `r"abc"`, `r#"…"#` |
| `ident` | `v` | `desc`, `size` |
| `regex` | `v`, `flags` (`imsx`, w tej kolejności) | `/oldName\(/i` |
| `int`, `num` | `v` (nieujemna) | `40`, `3.5` |
| `size` | `v` (bajty) | `10mb` = 10 000 000, `1kib` = 1 024 |
| `dur` | `v` (milisekundy) | `30s`, `1h30m` |
| `bool`, `null` | `v` / — | `true`, `null` |
| `list` | `items` | `["a", "b"]` |
| `map` | `entries: [{k, v, span?}]` | `{replace: "a", with: "b"}` |
| `ref` | `name`, `fields?` | `@recent`, `@recent.path` |
| `spread` | `x` | `...@recent.path` (tylko elementy listy) |
| `field` | `name`, `of?` | `file.size` |
| `call` | `fn`, `args` | `ago(1h)` |
| `un` | `op` (`not`, `neg`), `x` | `not hidden`, `-1` |
| `bin` | `op`, `l`, `r` | `size > 1mb` |
| `range` | `lo?`, `hi?` | `10..12`, `40..`, `..9` |
| `sub` | `p: pipeline` | `(run […] \|> assert exit_code: 0)` |
| `key` | `segs` | klucz strukturalny: nazwy, indeksy, `{"wild": "*" \| "**" \| "+"}`, `{"filter": expr}` |

### Konstrukcje sterujące (wersja 1.1) {#control-constructs-version-11}

Wersja 1.1 dodaje poniższe instrukcje. Blok jest tablicą co najmniej jednej
instrukcji; czytnik wersji 1.0 odrzuca dokument, który ją zawiera, z błędem
`E_AST_VERSION`, nazywając rodzaje węzłów, których nie zna. Dokument wersji 1.0
jest odczytywany bez zmian. Słowa rozpoczynające konstrukcję są zastrzeżone
wyłącznie na początku instrukcji, więc `steps`, `params` i pozostałe słowa
pozostają dopuszczalne jako nazwy parametrów i klucze map.

| `t` | Składniki | Postać tekstowa |
|---|---|---|
| `when` | `branches: [{cond, body, span?}]`, `otherwise?: block` | `when COND { … } otherwise when COND { … } otherwise { … }` |
| `each` | `source`, `name`, `options?: [{name, value}]`, `body` | `each (find "/d") as f parallel: 4 { … }` |
| `repeat` | `options: [{name, value}]` (`until` wymagane), `body` | `repeat until: COND every: 5s limit: 2m { … }` |
| `while` | `cond`, `options?`, `body` | `while COND limit: 20 { … }` |
| `break`, `continue` | — | `break` |
| `steps` | `body` | `steps { … }` |
| `define` | `name`, `params?: [{name, type, default?}]`, `body` | `define rotate(dir: path, keep: int = 7) { … }` |
| `params` | `params: [{name, type, default?}]` | `params { dir: path  days: int = 30 }` |

Reguły konstrukcji:

1. Wywołanie funkcji jest `stage`, którego nazwą jest funkcja; parser odczytuje
   `name(…)` jako notację wywołania dla każdej nazwy zdefiniowanej wcześniej w
   programie, we wszystkich trzech wariantach.
2. `type` jest zapisywany tak jak w katalogu: `path`, `int`, `list<text>`,
   `enum(a, b)`. Nazwa parametru jest deklarowana jednokrotnie.
3. Opcje są nazwane; sama nazwa opcji oznacza `true` (`atomic`). Opcjami `each`
   są `parallel` i `atomic`, `repeat` `until`, `every`, `limit` i `atomic`,
   `while` `limit` i `atomic`.
4. W warunku `exists <operand>` jest `call` do `exists`.
5. Zakresy konstrukcji obejmują ją od pierwszego słowa do klamry zamykającej.
   Formater zachowuje komentarze konstrukcji przed nią.

Reguły drzewa powierzchniowego:

1. Drzewo nie zależy od katalogu. Argumenty pozycyjne zachowują swoją kolejność
   i nie mają nazwy; nie jest dodawana żadna wartość domyślna.
2. Parser tworzy `text` dla każdego ciągu w cudzysłowie i `ident` dla każdego
   gołego identyfikatora. Ścieżki, wzorce glob, chwile i klucze są węzłami
   `text`; mechanizm wiązania nadaje im typy według ich miejsca w sygnaturze.
   Strukturalny węzeł `key` jest alternatywą dla klucza zapisanego jako tekst, dla
   producentów budujących drzewa bezpośrednio. Drukarka zapisuje węzeł `key` jako
   literał tekstowy w kanonicznej notacji klucza, więc sparsowanie wydrukowanego
   programu daje węzeł `text`; oba wiążą się z tą samą wartością `key` drzewa
   powiązanego.
3. Wartości tekstowe są przechowywane po przetworzeniu sekwencji ucieczki. W
   wyrażeniu regularnym `\/` jest przechowywane jako `/`; każda inna sekwencja
   ucieczki jest przechowywana bez zmian. Wzorzec nie jest pusty. Flagi są
   wybierane z `imsx` i zapisywane w tej kolejności.
4. Literały liczbowe są nieujemne: `-1` to `un` z `op: "neg"` zastosowanym do
   `int` 1. Każda liczba całkowita drzewa mieści się w ±(2^53 - 1), zakresie,
   który kanoniczny JSON przedstawia dokładnie; większy literał jest błędem
   leksykalnym.
5. `spread` pojawia się wyłącznie jako element listy.
6. Identyfikator bezpośrednio po którym następuje `:` zawsze rozpoczyna argument
   nazwany, nawet gdy jest słowem operatora (`replace "a" "b" in: ["/srv/a.txt"]`).
7. Operatory binarne, w kolejności rosnącej siły wiązania: `or`; `and`; `not`
   (jednoargumentowy); `== != < <= > >= ~ !~ in` (niełączne); `+ -`; `* /`;
   jednoargumentowy `-`; dostęp do pola `.` i wywołania.
8. Wariant składni B tworzy to samo drzewo powierzchniowe co wariant A. Wariant C
   zapisuje każdy argument z nazwą, a parser zachowuje nazwy, ponieważ nie czyta
   katalogu; drzewo powierzchniowe programu w wariancie C różni się zatem od
   wariantu A, a drzewa powiązane wszystkich trzech wariantów są równe.

Przykład: `ports state: listen |> filter port == 8080`

```json
{"nurt": "1.1", "program": {"statements": [{"t": "pipeline",
  "head": {"t": "stage", "name": "ports", "args": [
    {"name": "state", "value": {"t": "ident", "v": "listen", "span": [1, 14, 1, 20]}, "span": [1, 7, 1, 20]}],
    "span": [1, 1, 1, 20]},
  "stages": [{"t": "stage", "name": "filter", "args": [
    {"value": {"t": "bin", "op": "==",
      "l": {"t": "ident", "v": "port", "span": [1, 31, 1, 35]},
      "r": {"t": "int", "v": 8080, "span": [1, 39, 1, 43]}, "span": [1, 31, 1, 43]},
     "span": [1, 31, 1, 43]}], "span": [1, 24, 1, 43]}],
  "span": [1, 1, 1, 43]}]}}
```

## 4. Drzewo powiązane {#4-bound-tree}

Składniki korzenia: `nurt`, `bound`, `catalog_rev`, `corr_rev`, `program` (wszystkie
wymagane) i `meta`. `catalog_rev` identyfikuje katalog poleceń, a `corr_rev`
tabele poprawek użyte do wytworzenia drzewa. Oba są ciągami rewizji pasującymi do
`[A-Za-z0-9][A-Za-z0-9:._-]*`, o długości co najwyżej 128 bajtów. Mechanizm
wiązania zapisuje `catalog_rev` jako rewizję katalogu zdefiniowaną w
[`catalog.md`](catalog.md) (`sha256:` a po nim skrót szesnastkowy małymi literami),
a `corr_rev` w tej samej postaci, obliczoną na tabelach poprawek.

Instrukcje, `let` i `pipeline` mają taki sam kształt jak w drzewie
powierzchniowym. Etap ma składniki:

| Składnik | Znaczenie |
|---|---|
| `name` | kanoniczna nazwa polecenia z katalogu: jeden identyfikator albo grupa i identyfikator połączone `.` |
| `effect` | rodzaj skutku z katalogu: `read`, `local`, `external`, `process` |
| `phase` | faza wykonania (sekcja 5); wymagana |
| `waits` | `true`, gdy etap blokuje wywołanie przez oczekiwanie; pomijane, gdy `false`. Katalog oznacza polecenie jako `waits`, albo parametr oznaczony `waits` ma wartość większą niż jego wartość domyślna lub wartość `forever` |
| `args` | każdy parametr sygnatury, w kolejności sygnatury, a po nich parametry wywołania `privilege` i `as_user`, gdy zostały zapisane na etapie (`catalog.md`, sekcja 2) |
| `span` | zakres źródłowy etapu powierzchniowego |

Argument ma `name`, `value`, `default` (`true`, gdy wartość pochodzi z wartości
domyślnej katalogu; pomijane, gdy `false`) i `span` (nieobecny dla wartości
domyślnych).

Węzły wartości nie mają zakresu. Rodzaje: `null`; `bool`, `int`, `num`; `text`,
`path`, `glob`, `key`, `enum`, `instant` z tekstowym `v`; `size` i `dur` z
nieujemną liczbą całkowitą `v`; `regex` z `v` i `flags` z `imsx`; `list`; `map`;
`range`; `ref`; `sub`; oraz rodzaje wyrażeń `field`, `call`, `un`, `bin`,
`spread`. Liczby całkowite mieszczą się w ±(2^53 - 1); negacja stałej jest
składana, więc `-1` wiąże się z `int` -1.

| Rodzaj | Postać `v` |
|---|---|
| `path` | kanoniczna ścieżka bezwzględna: `/a/b`, `C:/a/b`, `//server/share/a` lub `provider:/a/b`; ukośniki w przód, bez pustego segmentu, segmentu `.` ani `..`, bez końcowego `/` poza korzeniem |
| `instant` | RFC 3339 w UTC z przyrostkiem `Z`; ułamki sekund bez końcowych zer (`2026-03-01T12:00:00Z`) |
| `glob`, `key` | niepusty tekst |
| `enum` | identyfikator |
| `regex` | niepusty wzorzec |

Reguły wiązania:

1. Każdy parametr sygnatury pojawia się raz, w kolejności sygnatury, niezależnie
   od tego, czy został zapisany pozycyjnie, z nazwą, czy wcale.
2. Parametr, który nie został zapisany, otrzymuje wartość domyślną katalogu z
   `default: true`. Parametr bez wartości domyślnej, parametr, którego wartość
   domyślna jest nieograniczona, parametr grupy wykluczającej, który nie został
   zapisany (jak w `assert`), oraz parametr postaci źródłowej, gdy polecenie jest
   wiązane jako etap, otrzymują `null` z `default: true`.
3. Literały są rzutowane na typ swojego miejsca: tekst na `path`, `glob`,
   `instant`, `key`; identyfikatory na `enum`; pojedyncza ścieżka na listę
   jednoelementową tam, gdzie miejscem jest lista ścieżek; liczba całkowita `n`
   na zakres `n..n` tam, gdzie miejscem jest zakres.
4. W wyrażeniach obliczanych na rekordach (`filter`, `select`, `sort`, parametry
   dla poszczególnych rekordów) goły identyfikator jest polem bieżącego rekordu
   i wiąże się z `field`.
5. Wartości wyprowadzone z innych parametrów są zapisywane jawnie: `mode`
   wzorca rozstrzyga się do `literal` lub `regex`, a parametr z `default_from` w
   katalogu otrzymuje wartość swojego źródła przekonwertowaną na jego typ
   (`run cwd` domyślnie przyjmuje parametr wywołania `base`, a domyślna lista
   `watch` polecenia `run` to lista zawierająca `cwd`).
6. Krótkie postaci źródłowe poleceń zmieniających są rozwijane do ich
   pojedynczego potoku. Postać źródłowa nazywa w `expand` parametr, który
   zawiera wpisy; jego wartość staje się `path` głowy `info`, po której następuje
   polecenie związane w postaci dla wejścia `entry`. `remove "/a"` wiąże się z
   `info ["/a"] |> remove`, więc oba programy mają to samo drzewo powiązane.
   `info` przyjmuje listę ścieżek, nie zagłębia się w katalogi i zgłasza brakującą
   ścieżkę jako wpis z `exists: false`, który polecenie obsługuje zgodnie ze
   swoim parametrem `on_missing`.
7. Potok sprawdzany przez `verify` jest wartością `sub` jego parametru `check`.
8. `effect` zawsze pochodzi z katalogu, nigdy z programu. Etapy faz `source`,
   `transform`, `check`, `after` i `verify` mają skutek `read`.

### Konstrukcje sterujące drzewa powiązanego (wersja 1.1) {#control-constructs-of-the-bound-tree-version-11}

Instrukcje `when`, `each`, `repeat`, `while`, `break`, `continue`, `steps`,
`define` i `params` mają te same nazwy i bloki co w drzewie powierzchniowym, z
następującymi różnicami: warunek i źródło są powiązanymi wyrażeniami typu `bool`
i listy; opcje pętli są składnikami (`parallel`, `atomic` (pomijane, gdy `true`),
`every_ms`, `limit`); limit to `{"kind": "iterations", "n": N}` lub
`{"kind": "time", "ms": N}`; parametr ma kanoniczny `type` i powiązaną wartość
`default`. Wywołanie funkcji jest głową potoku `{"t": "call", "name", "args"}`,
której `args` wymieniają każdy parametr z nazwą, z `default: true` przy
wartościach pochodzących z deklaracji. Potok odczytu wewnątrz wyrażenia jest
wartością `sub`, a `exists` jest `call`. Wiązanie sprawdza, przed jakimkolwiek
skutkiem: warunki nie są statycznie innego typu niż `bool`; opcje mają swoje
nazwy i typy; `break` i `continue` stoją wewnątrz pętli; `params` jest pierwszą
instrukcją; funkcje są definiowane przed swoim pierwszym wywołaniem, ciało funkcji
widzi tylko jej parametry, a wywołanie nazywa każdy wymagany parametr z wartością
jego typu.

## 5. Fazy {#5-phases}

| `phase` | Etap | Skutek w świecie |
|---|---|---|
| `source` | etap głowy wytwarzający rekordy | tylko odczyt |
| `transform` | transformacje rekordów przed skutkiem | brak |
| `check` | `assert` przed skutkiem; niepowodzenie niczego nie zmienia | brak |
| `mutate` | jedyny etap zmiany | zmiana, zapisana w dzienniku |
| `run` | jedyny etap uruchamiający program | skutki programu, śledzone |
| `after` | `assert`, `filter`, `select`, `first`, `last`, `skip`, `count`, `sort`, `group` na rekordach wytworzonych przez skutek | brak; nieudany `assert` zgłasza `E_ASSERT` bez wycofywania skutku |
| `verify` | `verify (…)`; niepowodzenie wycofuje skutek | wycofanie |

Potok ma co najwyżej jeden etap fazy `mutate` lub `run`. Etapy fazy `after`
stoją wyłącznie po nim, a `verify`, gdy jest obecny, jest ostatnim etapem. Każdy
inny etap po skutku jest odrzucany z błędem `E_ONE_EFFECT`. Zdekodowane drzewo
powiązane, które łamie te reguły, jest odrzucane z błędem `E_AST_SCHEMA`.

## 6. `ast_hash` {#6-asthash}

```
ast_hash = "sha256:" + lowercase_hex(SHA-256(UTF-8(JCS(hash_form(document)))))
```

Postacią skrótu dokumentu jest dokument bez składnika `meta` obiektu korzenia i
bez składnika `span` każdego obiektu (`nurt_contracts::ast::strip_for_hash`).
Jest ona brana z JSON wytworzonego przez serializację zdekodowanego, otypowanego
drzewa, więc pominięte składniki opcjonalne i składniki zapisane ze swoją wartością
domyślną (`"waits": false`, `"default": false`) nigdy nie zmieniają skrótu.
Programy różniące się wyłącznie układem lub komentarzami albo zapisane w
wariantach składni A i B mają ten sam powierzchniowy `ast_hash`; programy w każdym
wariancie składni mają to samo drzewo powiązane. Raport i dziennik używają
`ast_hash` drzewa powiązanego, które zawiera każdą wartość domyślną; późniejsza
zmiana wartości domyślnej nigdy nie zmienia skrótu ani znaczenia zapisanego
programu. Składnik `ast_hash` wyniku `check` jest skrótem drzewa powiązanego; może
być nieobecny wyłącznie wtedy, gdy drzewo nie jest znane, nigdy dla programu
podanego jako tekst (`docs/reference/serve-protocol.md`).

## 7. Pliki zgodności {#7-conformance-files}

| Ścieżka | Zawartość |
|---|---|
| `golden/ast/<name>.nurt` | tekst programu, notacja słowna |
| `golden/ast/<name>.json` | drzewo powierzchniowe programu, z zakresami; drzewa bez tekstu programu używają węzłów strukturalnych i metadanych |
| `golden/ast/<name>.hash` | `ast_hash` drzewa powierzchniowego |
| `golden/ast/<name>.b.nurt` | ten sam program w wariancie B; jego drzewem powierzchniowym jest `<name>.json` |
| `golden/ast/<name>.c.nurt`, `<name>.c.json`, `<name>.c.hash` | ten sam program w wariancie C, jego drzewo powierzchniowe i jego `ast_hash` |
| `golden/bound/<name>.json` | drzewo powiązane tego samego programu względem katalogu referencyjnego |
| `golden/bound/<name>.hash` | `ast_hash` drzewa powiązanego |
| `golden/<format>/invalid/<case>.json` | dokument, który czytnik odrzuca |
| `golden/<format>/invalid/expected.tsv` | kod błędu i warstwa każdego odrzuconego dokumentu: `schema` (odrzuca go opublikowany schemat), `types` (odrzucają go schemat i typy Rust) lub `rules` (odrzucają go reguły faz z sekcji 5) |
| `golden/<format>/schema.generated.json` | schemat wygenerowany z typów Rust |

Wzorcowe drzewa powiązane używają stałych rewizji `cat-1` i `corr-1`.

Wzorcowy program ma drzewo powiązane tylko wtedy, gdy katalog referencyjny `cat-1` wiąże go w każdym
systemie. `example-41-windows-registry` tego nie spełnia: jego ścieżki należą do rejestru systemu Windows
(`registry:/HKCU/...`), więc wiązanie w innych systemach kończy się błędem `E_PATH_FOREIGN`, a drzewo
wzorcowe nie zależy od systemu, który je wytworzył. Jego drzewo powierzchniowe jest sprawdzane, a drzewo
powiązane nie jest zapisywane. To samo dotyczy `example-48-preview-and-program-search` i `structured-key`,
których składnia klucza jest ćwiczona wyłącznie jako drzewo powierzchniowe.
Testy zgodności sprawdzają, że każde drzewo jest prawidłowe względem opublikowanego schematu
i wygenerowanego schematu, deserializuje się do typów Rust i serializuje z powrotem do tej samej wartości JSON,
przetrwa bez zmian cykl przez CBOR i ma zapisany `ast_hash`, oraz że każdy nieprawidłowy dokument jest odrzucany
w swojej warstwie. Ustawienie `NURT_BLESS=1` regeneruje wygenerowane schematy i zapisane skróty po
zrecenzowanej zmianie.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
