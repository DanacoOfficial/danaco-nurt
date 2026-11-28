<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           spec/keypath.md -->
<!-- Responsibility:   Documents Key paths. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Ścieżki kluczy {#key-paths}

Ścieżka kluczy adresuje wartości wewnątrz dokumentu strukturalnego: JSON, JSONC,
YAML, TOML, XML, INI, CSV, TSV, plików właściwości Javy, plików środowiska oraz
list właściwości. Jest wartością typu języka `key`. Literał tekstowy w parametrze
typu `key` jest konwertowany przy wiązaniu programu, więc nieprawidłowo
zbudowany klucz jest zgłaszany przed wykonaniem jakiegokolwiek polecenia, z kodem
błędu `E_KEY_SYNTAX`.

Typem w Rust jest `nurt_contracts::keypath::KeyPath`. Jego parser i kanoniczny
drukarz zależą wyłącznie od biblioteki standardowej.

```
read "package.json" key: "scripts.test"
set "deploy.yaml" key: "#1:spec.replicas" value: 3
set "package.json" key: "keywords[+]" value: "cli"
read "Cargo.toml" key: "dependencies.*.version"
read "users.json" key: "users[?age >= 18 and active].name"
```

## Gramatyka {#grammar}

```
keypath      = [ doc_selector ] ( json_path | "$" | native ) ;
doc_selector = "#" ( digit { digit } | "*" ) ":" ;

native       = first { step } ;
first        = name | "*" | "**" | bracket ;
step         = "." ( name | "*" | "**" ) | bracket ;
name         = name_char { name_char } ;
name_char    = any character except "." "[" "]" '"' "'" ;
bracket      = "[" ws ( index | "*" | "**" | "+" | quoted | "?" filter ) ws "]" ;
index        = [ "-" ] digit { digit } ;

json_path    = "$" { "." ( name | "*" ) | ".." ( name | "*" | bracket ) | bracket } ;

quoted       = '"' { char | escape } '"' | "'" { char | escape } "'" ;
escape       = "\" ( '"' | "'" | "\" | "/" | "n" | "r" | "t" | "b" | "f"
             | "u" hex hex hex hex | "u{" hex { hex } "}" ) ;

filter       = or_expr ;
or_expr      = and_expr { ( "or" | "||" ) and_expr } ;
and_expr     = not_expr { ( "and" | "&&" ) not_expr } ;
not_expr     = ( "not" | "!" ) not_expr | comparison ;
comparison   = operand [ ( "==" | "!=" | "<" | "<=" | ">" | ">=" ) operand ] ;
operand      = "(" or_expr ")" | literal | field ;
field        = "@" { field_step } | field_name { field_step } ;
field_step   = "." field_name | "[" ws quoted ws "]" ;
field_name   = ( letter | "_" ) { letter | digit | "_" | "-" | "$" } ;
literal      = quoted | number | "true" | "false" | "null" ;
number       = [ "-" ] digit { digit } [ "." digit { digit } ] [ ( "e" | "E" ) [ "+" | "-" ] digit { digit } ] ;
```

`ws` to opcjonalny biały znak. Ścieżka zaczynająca się od `$`, po którym następuje
`.`, `[` lub koniec tekstu, jest JSONPath; każdy inny `$` rozpoczyna zwykłą nazwę,
więc `$schema` i `definitions.$ref` są nazwami.

## Segmenty {#segments}

| Zapis | Segment | Znaczenie |
|---|---|---|
| `name`, `["name"]`, `['name']` | klucz | element mapy |
| `[0]`, `[-1]` | indeks | element listy; wartości ujemne liczą od końca |
| `[*]`, `.*`, `*` | wieloznacznik | każdy element lub składnik jednego poziomu |
| `.**`, `**`, `[**]`, `..` (JSONPath) | rekurencyjny | zero lub więcej poziomów: `a.**` wybiera `a` i wszystkich jego potomków, `**.version` każdy element `version` na dowolnej głębokości |
| `[+]` | dołączenie | pozycja za ostatnim elementem listy; tylko jako ostatni segment |
| `[?expression]` | filtr | elementy listy (lub składniki mapy), dla których wyrażenie jest prawdziwe |
| `#1:`, `#*:` | selektor dokumentu | dokument strumienia wielodokumentowego (YAML); brak selektora oznacza pierwszy dokument |
| `$` samodzielnie | korzeń | cały dokument |

Reguły:

1. Nazwa zachowuje każdy znak tak, jak został zapisany, w tym wewnętrzne spacje;
   nic nie jest przycinane.
2. Nazwa złożona z cyfr jest kluczem: `a.0` to składnik `"0"` mapy albo pierwszy
   element, gdy węzeł jest listą; `a[0]` jest zawsze indeksem. Gdy polecenie
   tworzy węzły, `a.0` tworzy mapę, a `a[0]` listę.
3. W dokumentach XML nazwa zaczynająca się od `@` adresuje atrybut
   (`project.@version`), a `#text` zawartość tekstową; element z przestrzenią
   nazw jest zapisywany jako `prefix:name` z prefiksami dokumentu lub
   `["{uri}name"]`. We wszystkich innych formatach są to zwykłe nazwy, na
   przykład składnik JSON-LD `@context`.
4. Pola filtru są odczytywane względem testowanego elementu; samo `@` oznacza
   sam element (`tags[?@ == "beta"]`). Filtr, którego nie można obliczyć dla
   elementu (brakujące pole, wartości różnych typów przy `<`), jest dla tego
   elementu fałszywy.
5. Wycinki JSONPath (`[1:3]`), sumy (`[0,1]`), funkcje (`length(@)`) i
   odwołania bezwzględne wewnątrz filtrów (`$.x`) nie są obsługiwane i są
   zgłaszane jako błędy.

## Postać kanoniczna {#canonical-form}

Każda ścieżka kluczy ma jeden zapis kanoniczny, najkrótszy jednoznaczny. Używa go
pole `key` rekordów `item` oraz powiązane drzewo składni. Dla każdej ścieżki
kluczy `k` sparsowanie tekstu kanonicznego `k` zwraca `k`.

| Element | Zapis kanoniczny |
|---|---|
| klucz | goła nazwa po `.` (bez kropki dla pierwszego segmentu), gdy nazwa jest niepusta, nie zawiera `.`, `[`, `]`, `"`, `'` ani znaku sterującego, nie zaczyna się ani nie kończy białym znakiem i nie jest `*` ani `**`; w przeciwnym razie `["…"]` |
| pierwszy klucz | jak wyżej, a także w cudzysłowie, gdy jest to `$` lub wygląda jak selektor dokumentu (`#1:x`) |
| indeks | `[n]` bez zer wiodących |
| wieloznacznik | `[*]` |
| rekurencyjny | `**` jako pierwszy, `.**` w pozostałych miejscach |
| dołączenie | `[+]` |
| selektor dokumentu | `#n:` lub `#*:`; `#0:` jest pomijany |
| korzeń | `$` |
| tekst w cudzysłowie | cudzysłowy podwójne; `\"`, `\\`, `\n`, `\r`, `\t` oraz `\u{…}` dla pozostałych znaków sterujących |
| filtr | operatory `==`, `!=`, `<`, `<=`, `>`, `>=`, `and`, `or`, `not`, rozdzielone pojedynczymi spacjami; nawiasy tylko tam, gdzie wymaga ich priorytet (i zawsze wokół porównania pod `not`) |
| pole filtru | gołe nazwy połączone `.`; nazwa, która nie jest prawidłowym `field_name` lub jest jedną z `true`, `false`, `null`, `and`, `or`, `not`, w nawiasach kwadratowych (`@["a b"]`); sam element to `@` |
| liczba filtru | liczby całkowite tak, jak zapisano; pozostałe liczby w najkrótszej postaci, która odczytuje się jako ta sama wartość, zawsze z częścią ułamkową lub wykładnikiem (`1000.0`, `1e300`) |

Przykłady:

| Zapis | Postać kanoniczna |
|---|---|
| `scripts.test` | `scripts.test` |
| `$.a.b` | `a.b` |
| `$['a b']` | `a b` |
| `a.*` | `a[*]` |
| `$..name` | `**.name` |
| `a[ 007 ]` | `a[7]` |
| `#0:a` | `a` |
| `["a.b"].c` | `["a.b"].c` |
| `a[?@.name=='x'&&size>2]` | `a[?name == "x" and size > 2]` |
| `a[?!(x == 1)]` | `a[?not (x == 1)]` |

## Błędy {#errors}

Nieprawidłowo zbudowana ścieżka kluczy jest błędem wiązania `E_KEY_SYNTAX`. Błąd
zawiera przesunięcie w bajtach i kolumnę liczoną od jedności (w znakach) wewnątrz
tekstu klucza, rodzaj oraz poprawiony klucz, gdy jedna poprawka jest oczywista.

| Rodzaj | Przykład | Poprawka |
|---|---|---|
| `empty` | `""` | — |
| `leading_dot` | `.a` | `a` |
| `trailing_dot` | `a.` | `a` |
| `empty_name` | `a..b` | `a.**.b` |
| `dot_before_bracket` | `a.[0]` | `a[0]` |
| `empty_bracket` | `a[]` | `a[*]` |
| `unquoted_name` | `a[b]` | `a["b"]` |
| `character_requires_quoting` | `it's` | `["it's"]` |
| `unexpected_character` | `a[0]b` | `a[0].b` |
| `unexpected_end` | `a[` | — |
| `unterminated_string` | `a["x` | — |
| `invalid_escape` | `a["\q"]` | — |
| `index_out_of_range` | `a[99999999999999999999]` | — |
| `invalid_number` | `a[?x == 1e400]` | — |
| `empty_filter` | `a[?]` | — |
| `invalid_filter` | `a[?x = 1]` | `a[?x == 1]` |
| `misplaced_append` | `a[+].b` | — |
| `missing_path` | `#1:` | `#1:$` |
| `unsupported` | `a[1:3]` | — |

Filtry zagnieżdżają co najwyżej 64 poziomy nawiasów i negacji.

## Związek z drzewami składni {#relation-to-the-syntax-trees}

W powierzchniowym drzewie składni klucz jest węzłem `text` albo strukturalnym
węzłem `key`, którego segmentami są nazwy, indeksy, wieloznaczniki (`*`, `**`,
`+`) i filtry zapisane jako wyrażenia. `KeyPath::from_key_segs` konwertuje postać
strukturalną; filtry mogą używać pól, identyfikatorów, literałów, porównań oraz
`and`, `or`, `not`. W powiązanym drzewie składni klucz jest węzłem `key`
zawierającym tekst kanoniczny.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
