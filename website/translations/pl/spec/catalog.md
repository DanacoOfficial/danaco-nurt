<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           spec/catalog.md -->
<!-- Responsibility:   Documents Command catalog `nurt-catalog/1`. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Katalog poleceń `nurt-catalog/1` {#command-catalog-nurt-catalog1}

Katalog poleceń jest jedynym źródłem sygnatur poleceń. Czytają go: wiązanie,
wbudowana pomoc, opis dla modelu, dokumentacja referencyjna, definicje
narzędzi i zestaw testów zgodności. Składa się z:

| Plik | Zawartość |
|---|---|
| `catalog/_index.toml` | indeks `nurt-catalog-index/1`: pliki katalogu i oczekiwana liczba poleceń |
| `catalog/core.toml` | polecenia podstawowe, zawsze opisywane modelowi |
| `catalog/<group>.toml` | jeden plik na każdą grupę opcjonalną, opisywaną na żądanie |
| `spec/schemas/nurt-catalog-1.schema.json` | schemat JSON pliku katalogu, generowany z typów Rust |

Typy Rust znajdują się w module `nurt_contracts::spec`: `CatalogFile`,
`CatalogIndex`, `StageSpec`, `ParamSpec`, `ParamType`, `DefaultValue`,
`IoSpec`, `FormSpec`, `ActionEffect`, `ExternalUse`.

## 1. Plik katalogu {#1-catalog-file}

```toml
format = "nurt-catalog/1"
group = "documents"
summary = "Structured documents and SQLite."

[[command]]
name = "set"
category = "change"
reversibility = "reversible"
effect = "local"
implementation = "builtin"
input = ["none", "entry"]
output = ["effect"]
summary = "Set values at key paths, preserving layout."

[[command.param]]
name = "path"
type = "path | list<path>"
position = 1
required = true
summary = "Document to change."
```

### 1.1 Klucze pliku {#11-file-keys}

| Klucz | Typ | Wymagany | Znaczenie |
|---|---|---|---|
| `format` | tekst | tak | zawsze `nurt-catalog/1` |
| `group` | identyfikator | tak | `core` lub nazwa grupy; równa nazwie pliku bez `.toml` |
| `summary` | tekst | tak | jedno zdanie zakończone kropką |
| `command` | tablica tabel | nie | polecenia grupy |

### 1.2 Klucze polecenia (`[[command]]`) {#12-command-keys-command}

| Klucz | Typ | Wymagany | Znaczenie |
|---|---|---|---|
| `name` | tekst | tak | pełna nazwa polecenia: identyfikator albo dwa identyfikatory połączone kropką (`git.status`); unikalna w całym katalogu |
| `group` | identyfikator | nie | zwykle pomijany; gdy obecny, równy grupie pliku |
| `category` | wyliczenie | tak | `source`, `transform`, `check`, `change`, `run`, `system`, `journal`, `help` |
| `reversibility` | wyliczenie | tak | `read`, `reversible`, `reversible_if_preserved`, `irreversible` |
| `effect` | wyliczenie | tak | skutek domyślny: `read`, `local`, `external`, `process` |
| `implementation` | wyliczenie | tak | `builtin`, `user_program`, `language_server`, `package_manager`, `system_program` |
| `input` | lista rodzajów | tak | akceptowane rodzaje wejścia (sekcja 4) |
| `output` | lista rodzajów | tak | produkowane rodzaje wyjścia (sekcja 4) |
| `summary` | tekst | tak | jedno zdanie zakończone kropką |
| `reversibility_note` | tekst | nie | warunki odwracalności; pomijany, gdy ich nie ma |
| `effects` | lista tabel | nie | akcje, których skutek różni się od domyślnego (sekcja 7) |
| `waits` | bool | nie | polecenie zawsze blokuje wywołanie przez oczekiwanie (`sleep`); domyślnie `false` |
| `os` | lista wyliczeń | nie | systemy operacyjne, do których polecenie jest ograniczone: `linux`, `macos`, `windows`; pusta oznacza wszystkie; w innych systemach wiązanie kończy się błędem `E_OS_ONLY` |
| `external` | tabela | nie | użycie zewnętrznego programu lub składnika systemu poza `implementation` (sekcja 8) |
| `fallback` | wyliczenie | nie | implementacja używana, gdy podstawowa jest niedostępna, taka jak analiza składniowa `builtin` dla polecenia `language_server` bez zainstalowanego serwera; wynik podaje, która się wykonała |
| `required_groups` | lista identyfikatorów | nie | grupy wyłączne, z których musi zostać podany dokładnie jeden parametr (sekcja 6) |
| `param` | tablica tabel | nie | parametry (sekcja 2) |
| `form` | tablica tabel | nie | formy zależne od wejścia polecenia polimorficznego (sekcja 5) |

### 1.3 Klucze parametru (`[[command.param]]`) {#13-parameter-keys-commandparam}

| Klucz | Typ | Wymagany | Znaczenie |
|---|---|---|---|
| `name` | identyfikator | tak | nazwa parametru, unikalna w poleceniu |
| `type` | wyrażenie typu | tak | sekcja 3 |
| `position` | liczba całkowita ≥ 1 | nie | pozycja wśród argumentów pozycyjnych formy podstawowej; pozycje polecenia tworzą ciąg `1..n` |
| `required` | bool | nie | wymagany w formie podstawowej; domyślnie `false`; wymagany parametr nie ma `default`; z `default_from` brakuje go tylko wtedy, gdy jego źródło też nie ma wartości |
| `repeated` | bool | nie | parametr pozycyjny zbierający kilka argumentów w listę; jego typ akceptuje listę; co najwyżej jeden na polecenie |
| `per_record` | bool | nie | wartość może być wyrażeniem obliczanym na każdym rekordzie wejściowym (`to: "/dst/" + name`) |
| `default` | tekst | nie | wartość domyślna jako literał języka w wariancie A: `"1"`, `"true"`, `'"**"'`, `"asc"`, `"120s"`, `"10mb"`, `"[]"`; nieobecna oznacza brak wartości (`null`) |
| `unbounded_default` | bool | nie | wartością domyślną jest „bez limitu”; wyklucza `default` |
| `default_from` | identyfikator | nie | wartością domyślną jest wartość innego parametru polecenia lub parametru wywołania `base`, przekonwertowana na typ tego parametru (ścieżka staje się listą jednoelementową): `run cwd` domyślnie przyjmuje `base`, `run watch` przyjmuje `cwd`; wyklucza `default` i `unbounded_default` |
| `default_note` | tekst | nie | opis obliczanej wartości domyślnej („ostatnia operacja”) |
| `values` | lista identyfikatorów | warunkowo | wartości wyliczenia typu, powtórzone dla czytelności; wymagane i równe wartościom typu, w kolejności, gdy typ zawiera `enum(…)` |
| `summary` | tekst | tak | krótki opis |
| `exclusive_group` | identyfikator | nie | parametry o tej samej nazwie grupy wzajemnie się wykluczają (sekcja 6) |
| `waits` | bool | nie | wartość tego parametru większa od domyślnej albo wartość `forever` oznacza etap `waits: true` w drzewie powiązanym (`run wait:` powyżej `3s`, `jobs wait:` powyżej `0s`) |
| `secret` | bool | nie | wartość jest poufna (poświadczenia w adresie lub nagłówku); jest maskowana w dzienniku, w raporcie i w drukowanym drzewie powiązanym |

Domyślna wartość w postaci gołego słowa (`asc`, `auto`) jest jedną z `values`;
słowo `all` jest też akceptowane dla parametrów, których typ zawiera `range`
(cały zakres).

## 2. Parametry wywołania nie są parametrami poleceń {#2-call-parameters-are-not-command-parameters}

Poniższe parametry należą do koperty wywołania (`nurt-request/1`, `options`) i
do opcji `--name` wiersza poleceń, a nie do żadnego polecenia. Dotyczą każdego
polecenia i nigdy nie są wymieniane w plikach katalogu.

| Parametr | Typ i wartość domyślna | Znaczenie |
|---|---|---|
| `preview` | `bool = false` | tylko obliczenie celów i skutku, bez zmieniania czegokolwiek |
| `base` | `path` | katalog, względem którego rozwiązywane są ścieżki względne; nie jest to katalog bieżący |
| `limit` | `int = 50` (najwyżej 5000) | rekordy lub zmiany na stronę wyniku |
| `cursor` | `text` | kursor następnej strony; ponowny odczyt strony nie uruchamia ponownie programu zmieniającego |
| `fields` | `list<text>` | pola wyniku, jak końcowe `select` |
| `format` | `enum(text\|json\|markdown\|compact) = text` | postać wyniku |
| `verbosity` | `enum(quiet\|normal\|detailed) = normal` | poziom szczegółowości raportu skutków |
| `diff` | `enum(none\|stat\|unified) = unified` | postać różnic w raporcie zmiany |
| `diff_context` | `int = 3` | wiersze kontekstu różnic |
| `max_chars` | `int = 0` (bez limitu) | limit pojedynczego pola tekstowego wyniku; skrócone pola niosą `truncated: true` |
| `save_to` | `path` | zapisanie pełnego wyniku do pliku (UTF-8) zamiast jego zwrócenia |
| `timeout` | `duration = 120s` | limit czasu całego programu (`run timeout:` ogranicza jeden program) |
| `session` | `text` | identyfikator sesji zapisywany w dzienniku |
| `language` | `enum(en\|pl) = en` | język pomocy i podpowiedzi; kody, pola i słowa kluczowe są zawsze angielskie |
| `script` | `bool = false` | tryb skryptu: kilka potoków, każdy jako osobna operacja |
| `privilege` | `enum(account\|elevated\|auto) = account` | uprawnienia operacji: konto uruchamiające Nurt; administrator (`root` w systemach Linux i macOS, `LocalSystem` w systemie Windows); albo `auto`, które działa z uprawnieniami konta i powtarza cały etap raz z `elevated`, gdy cel kończy się błędem `E_PERMISSION` lub `E_PRIVILEGE` przed zachowaniem jakiejkolwiek zmiany. Wartość domyślna pochodzi z klucza konfiguracji `privilege.default`. Obowiązuje dla każdego polecenia zmieniającego, dla `run` i dla poleceń odczytujących. Żadne polecenie nie jest oznaczone jako „nigdy z podwyższonymi uprawnieniami” |
| `as_user` | `text` | konto wykonujące operację (odpowiednik `sudo -u NAME`; Linux i macOS); implikuje podwyższenie dla przełączenia; w systemie Windows kończy się błędem `E_UNSUPPORTED` |

Polecenie może mieć parametr o nazwie parametru wywołania, gdy jego znaczenie
jest węższe i podane w katalogu: `run timeout:` ogranicza jeden program, a
parametr wywołania `timeout` ogranicza cały program; `write format:` jest
serializacją zapisywanych rekordów, a parametr wywołania `format` jest postacią
wyniku. Obie nigdy nie kolidują, ponieważ parametry wywołania nie są częścią
tekstu programu.

Dwa parametry wywołania, `privilege` i `as_user`, są wyjątkiem: mogą też być
zapisane na etapie, jako argumenty nazwane, w każdym poleceniu, które nie
deklaruje parametru o tej samej nazwie (`write "/etc/hosts" "..." privilege:
elevated`, `package.install ["jq"] privilege: elevated`, `run [...] cwd: "/"
as_user: "deploy"`). Wiązanie wyjmuje je z sygnatury, sprawdza wartość
(`privilege` to `account`, `elevated` lub `auto`, zapisane jako słowo lub
tekst; `as_user` to niepuste słowo lub tekst; każdy najwyżej raz na etap) i
dołącza je do argumentów etapu (`privilege` jako wartość `enum`, `as_user` jako
`text`) po parametrach sygnatury, z `default: false`. Wartość zapisana na
etapie zastępuje opcję żądania i klucz konfiguracji `privilege.default` dla
całego wywołania.

Reguła `privilege: auto`: wywołanie najpierw działa z uprawnieniami konta. Gdy
kończy się błędem `E_PERMISSION` lub `E_PRIVILEGE` i nie pozostawiło żadnej
operacji (brak `op`: nic nie zachowano), całe wywołanie jest powtarzane raz z
`elevated`; raport podaje `elevated: true` i `elevation_reason:
permission_denied`. Wywołanie, które już coś zmieniło, nigdy nie jest
powtarzane, a druga odmowa jest zwracana bez zmian. Etap odczytuje wartość
efektywną z opcji swojego kontekstu: `ctx.options().privilege()` i
`ctx.options().effective_privilege()`; wykonawca już uruchomił wywołanie z
uprawnieniami, które ta wartość wskazuje, więc etap nigdy nie musi sam
podwyższać uprawnień.
### 2.0 `preview` na etapie i skrót dla wartości logicznej {#20-preview-on-a-stage-and-the-boolean-shorthand}

Etap, który nie ma parametru o nazwie `preview`, przyjmuje `preview` jako swój
ostatni argument: `remove preview`, `remove preview: true` i `remove preview:
@dry` obliczają skutek tego etapu bez jego zastosowania. Wiązanie zachowuje
wartość w powiązanym etapie jako argument `preview`; wykonawca planuje etap i
raportuje dla jego części status `planned`. Parametr wywołania `preview` koperty
pokazuje podgląd całego programu.

Parametr typu `bool` zapisany bez wartości jest `true`: słowo, które nazywa
parametr `bool` polecenia, nie jest zapisane w inny sposób i nie jest wartością
parametru wyliczeniowego polecenia, oznacza `name: true`. `false` jest zawsze
zapisywane jawnie (`hidden: false`). Forma długa pozostaje poprawna. Reguła
dotyczy poleceń, wywołania funkcji i parametrów skryptu (`nurt run file.nurt
dry`). Parametr funkcji lub skryptu, który nie jest `bool`, zapisany bez
wartości, to `E_MISSING_PARAM` z oczekiwanym typem.

### 2.1 Wspólne parametry poleceń {#21-shared-command-parameters}

Parametry o tej samej nazwie mają to samo znaczenie i ten sam typ w każdym
poleceniu, które je ma. Wartość domyślna może różnić się w zależności od
polecenia i jest zawsze zapisana w katalogu.

| Parametr | Typ i zwykła wartość domyślna |
|---|---|
| `case_sensitive` | `bool = true` |
| `whole_word` | `bool = false` |
| `whitespace` | `enum(exact\|trailing\|indent\|normalize)`; `exact` dla wyszukiwania, `normalize` dla `replace` i `edit` |
| `encoding` | `text = "auto"` |
| `line_ending` | `enum(lf\|crlf\|cr\|keep) = keep` |
| `hidden` | `bool = true` |
| `gitignore` | `bool = false` |
| `on_missing` | `enum(skip\|error) = skip` |
| `on_conflict` | `enum(overwrite\|skip\|error)`, wartość domyślna zależna od polecenia |
| `expect` | `range` |
| `if_match` | `text` (etag) albo `text \| map<text>` (ścieżka do etagu) dla poleceń z wieloma plikami docelowymi |
| `validate` | `enum(off\|warn\|reject) = reject` |
| `depth` | `int` |
| `type` | `enum(file\|directory\|link\|any)` dla rodzajów wpisów |
| `order` | `enum(asc\|desc) = asc` |
| `file_type` | `list<text> = []`, typy plików według rozszerzenia (`go`, `ts`); obowiązuje w każdym trybie wyjścia |
| `hash` | `enum(none\|sha256\|sha512\|sha1\|md5\|blake3\|crc32)`; lista algorytmów jest taka sama wszędzie, gdzie wybiera się algorytm skrótu |

## 3. Wyrażenia typów {#3-type-expressions}

Klucz `type` zawiera wyrażenie typu. Parser jest częścią kontraktu
(`ParamType::parse`); nieprawidłowe wyrażenie czyni plik katalogu
nieprawidłowym.

```text
type        = alternative { "|" alternative } ;
alternative = scalar
            | "list" [ "<" type ">" ]
            | "map"  [ "<" type ">" ]
            | "stream" "<" identifier ">"
            | "enum" "(" identifier { "|" identifier } ")" ;
scalar      = "text" | "int" | "number" | "bool" | "path" | "glob" | "regex"
            | "size" | "duration" | "percent" | "instant" | "key" | "bytes" | "range"
            | "expression" | "pipeline" | "field" | "named_expression" | "any" ;
identifier  = lowercase letter { lowercase letter | digit | "_" } ;
```

Spacje i tabulatory między tokenami są ignorowane. Wewnątrz `list<…>` i
`map<…>` obowiązuje pełna gramatyka, więc `list<text | regex>` jest listą,
której elementami są teksty lub wyrażenia regularne.

| Typ | Znaczenie |
|---|---|
| `text`, `int`, `number`, `bool` | wartości podstawowe |
| `path`, `glob`, `regex`, `key`, `instant`, `size`, `duration` | wartości z literałami języka; tekst w cudzysłowie jest niejawnie konwertowany na `path`, `glob`, `instant`, `key`, `size` i `duration` w miejscu tego typu |
| `percent` | udział pojemności, zapisany jako tekst w cudzysłowie: `"10%"`; polecenie może też akceptować `"10%free"`, udział wolnego miejsca, i mówi o tym w opisie parametru; nie ma osobnego literału, tekst jest konwertowany w miejscu tego typu jak `size` i `duration` |
| `bytes` | surowe bajty (base64 w JSON) |
| `range` | `5`, `1..5`, `40..`, `..9`, `all`; wiersze liczone od 1, granice włącznie |
| `expression` | wyrażenie obliczane na każdym rekordzie wejściowym (`filter`, `assert all:`) |
| `pipeline` | zagnieżdżony potok w nawiasach (`verify (…)`, `write from: (…)`) |
| `field` | nazwa pola rekordu (`sort size`) |
| `named_expression` | para `name: expression` obliczająca nowe pole (`select total: size * 2`) |
| `any` | dowolna wartość |
| `enum(a\|b)` | jedno z gołych słów |
| `list<T>`, `map<T>` | lista; mapa z kluczami tekstowymi; gołe `list` i `map` oznaczają `list<any>` i `map<any>` |
| `stream<R>` | wynik nazwanego potoku `@name` z rekordami rodzaju `R` (`any` dla dowolnego rodzaju) |
| `A \| B` | dowolna z alternatyw |

Kanoniczny tekst typu (używany w serializacji, pomocy i JSON) zapisuje
alternatywy oddzielone `" | "`, wartości wyliczenia oddzielone `|` bez spacji,
a gołe `list` i `map` jako `list<any>` i `map<any>`. Parsowanie tekstu
kanonicznego daje ten sam typ.

Błędy są typowane (`TypeParseError`): `empty`, `unexpected_end`,
`unexpected_char`, `unknown_type`, `invalid_enum_value`,
`duplicate_enum_value`, `duplicate_alternative`, `too_deep` (zagnieżdżenie
głębsze niż 16 poziomów), każdy z przesunięciem bajtowym problemu.

## 4. Rodzaje wejścia i wyjścia {#4-input-and-output-kinds}

`input` i `output` wymieniają rodzaje (`IoSpec`):

| Rodzaj | Znaczenie |
|---|---|
| `none` | tylko wejście: polecenie rozpoczyna potok (forma źródłowa) |
| `any` | rekordy dowolnego rodzaju |
| `same` | tylko wyjście: rekordy wejścia, bez zmiany rodzaju (`filter`, `sort`) |
| słowo kluczowe skalarne (`int`, `text`) | pojedyncza wartość (`count` produkuje `int`, `help` produkuje `text`) |
| rodzaj rekordu (`entry`, `match`, `effect`, …) | rekordy tego rodzaju |

Rodzaj `effect` jest raportem skutków etapu zmieniającego lub uruchamiającego,
widzianym jako rekord; etapy po skutku (`assert`, `filter`, `select`, `first`,
`last`, `skip`, `count`, `sort`, `verify`) go przetwarzają.

## 5. Formy poleceń polimorficznych {#5-forms-of-polymorphic-commands}

Niektóre polecenia akceptują różne parametry w zależności od swojego wejścia:
forma źródłowa `replace` przyjmuje `pattern`, `with` i `in`, a forma etapu po
`search` przyjmuje tylko `with`. Takie polecenia wymieniają formy:

```toml
[[command.form]]
name = "source"
input = ["none"]
positional = ["pattern", "with"]
required = ["pattern", "with", "in"]
summary = "Replace a pattern in the given files."

[[command.form]]
name = "matches"
input = ["match", "node", "item"]
positional = ["with"]
required = ["with"]
params = ["with", "occurrence", "template", "expect", "if_match", "validate"]
summary = "Replace the text of the input matches."
```

| Klucz | Wymagany | Znaczenie |
|---|---|---|
| `name` | tak | nazwa formy, unikalna w poleceniu |
| `input` | tak | rodzaje wejścia formy; każdy rodzaj wejścia polecenia należy do dokładnie jednej formy |
| `output` | nie | rodzaje wyjścia formy; pusta oznacza wyjście polecenia |
| `positional` | nie | parametry pozycyjne formy, w kolejności |
| `required` | nie | parametry wymagane formy; nazwa grupy wyłącznej oznacza „dokładnie jeden z grupy” |
| `params` | nie | parametry akceptowane przez formę; pusta oznacza wszystkie |
| `expand` | nie | w formie źródłowej: parametr, który nazywa wpisy, na których pracuje polecenie; wiązanie rozwija formę (sekcja 5.2) |
| `summary` | tak | jedno zdanie |

Wiązanie wybiera formę, której `input` zawiera rodzaj rekordów wejściowych
(`none` dla źródła). Pierwsza forma jest formą podstawową: klucze `position` i
`required` parametrów oraz klucz polecenia `required_groups` opisują ją, a
kontrola katalogu sprawdza, czy są zgodne. Polecenie bez form ma jedną formę
opisaną przez swoje parametry.

### 5.1 Powtarzane parametry pozycyjne {#51-repeated-positional-parameters}

Parametr powtarzany (`repeated = true`) zbiera argumenty pozycyjne w listę. Gdy
po nim następują parametry pozycyjne (`copy FROM… TO`), późniejsze pozycje
przyjmują ostatnie argumenty, a parametr powtarzany przyjmuje resztę, jak w
`copy "/a" "/b" "/dst"` (`from: ["/a", "/b"]`, `to: "/dst"`). Przy jednym
argumencie pozycyjnym przyjmuje go parametr powtarzany.

### 5.2 Rozwinięte formy źródłowe {#52-expanded-source-forms}

Forma źródłowa z kluczem `expand` nie jest wiązana tak, jak została zapisana.
Wiązanie rozwija ją do głowy `info`, której `path` jest wartością nazwanego
parametru, a następnie polecenia powiązanego w swojej formie dla wejścia
`entry`:

```toml
[[command.form]]
name = "source"
input = ["none"]
positional = ["path"]
required = ["path"]
expand = "path"
summary = "Remove the given files and directories."
```

`remove "/a" "/b"` wiąże się z `info ["/a", "/b"] |> remove`, więc oba
programy mają to samo drzewo powiązane. `info` akceptuje listę ścieżek, nie
schodzi do katalogów i raportuje brakującą ścieżkę jako wpis z `exists: false`,
który polecenie obsługuje zgodnie ze swoim parametrem `on_missing`. Katalog
podstawowy rozwija formy źródłowe `copy` i `move` (`from`) oraz `remove`,
`rename` i `attributes` (`path`). Nazwany parametr akceptuje ścieżkę, forma nie
przyjmuje wejścia, a forma dla wejścia `entry` nie akceptuje tego parametru.
Formy źródłowe bez `expand` (`replace`, `insert`) są wiązane tak, jak zostały
zapisane.

## 6. Grupy wyłączne i wymagane {#6-exclusive-and-required-groups}

Parametry mające wspólne `exclusive_group` wzajemnie się wykluczają: co
najwyżej jeden z nich może być podany. Grupa wymieniona w kluczu polecenia
`required_groups` (lub na liście `required` formy) wymaga dokładnie jednego.
Grupa ma co najmniej dwóch członków, a jej nazwa nie jest nazwą parametru.

```toml
required_groups = ["condition"]
[[command.param]]
name = "count"
type = "range"
exclusive_group = "condition"
summary = "Expected number of records."
```

## 7. Skutki akcji {#7-effects-of-actions}

`effects` wymienia akcje, których skutek lub odwracalność różni się od wartości
domyślnej polecenia:

```toml
effects = [
  { action = "push", effect = "external", reversibility = "irreversible" },
]
```

`action` jest identyfikatorem nazywającym jedno z poniższych:

| Znaczenie | Przykład |
|---|---|
| wartość parametru `action` polecenia | `git.stash` `action: drop` |
| wartość innego parametru wyliczeniowego | `http.request` `method: post` (`action = "post"`) |
| nazwa parametru, którego użycie zmienia skutek | `http.request` `save:` (`action = "save"`) |
| rodzaj rekordu wejściowego, dla poleceń, których skutek od niego zależy | `remove` na `job` (`action = "job"`) |

`note` podaje warunek, gdy nie wynika on oczywiście z identyfikatora.

## 8. Użycie zewnętrzne {#8-external-use}

`external` oznacza użycie programu lub składnika systemu spoza pliku
wykonywalnego, jak wymaga tego każda taka operacja:

```toml
external = { kind = "system_program", programs = ["launchctl"], note = "Services on macOS." }
```

| Klucz | Wymagany | Znaczenie |
|---|---|---|
| `kind` | tak | `user_program`, `language_server`, `package_manager` lub `system_program` |
| `programs` | nie | nazwy programów |
| `when` | nie | parametr, którego użycie wyzwala użycie zewnętrzne; nieobecny, gdy bezwarunkowe |
| `note` | nie | uwaga pokazywana w opisie i w raporcie |

## 9. Indeks {#9-index}

```toml
format = "nurt-catalog-index/1"
total = 148

[[file]]
name = "core"
file = "core.toml"
summary = "Core commands for files, text, records, programs, the journal and help."
commands = 33
```

Pierwszy wpis to `core`; każdy wpis nazywa `<name>.toml`; suma `commands` jest
równa `total`.

### 9.1 Rewizja katalogu {#91-catalog-revision}

Rewizja katalogu identyfikuje dokładny katalog użyty do wiązania i jest
zapisywana jako `catalog_rev` w każdym powiązanym drzewie:

```
catalog_rev = "sha256:" + lowercase_hex(SHA-256(UTF-8(JCS(files))))
```

`files` jest tablicą JSON plików katalogu w kolejności indeksu, każdy
zserializowany z `CatalogFile` (postać walidowana przez
`nurt-catalog-1.schema.json`), a JCS to kanoniczny JSON z RFC 8785. Każda
zmiana polecenia, parametru lub opisu zmienia więc rewizję. Rejestr poleceń
silnika raportuje tę samą wartość jako `catalog_rev` w wyniku zapytania o
katalog i w `nurt_info`. Grupy zarejestrowanych rozszerzeń następują po plikach
indeksu jako pliki tej samej postaci, więc rejestr z rozszerzeniami raportuje
inną rewizję niż ta zapisana w powiązanym drzewie, która nazywa wbudowany
katalog użyty do wiązania. Rewizja tabel poprawek, `corr_rev`, ma tę samą
postać.

### 9.2 Polecenie `next` {#92-the-command-next}

`next` jest poleceniem podstawowym kategorii `source`, które odczytuje
następną część skróconego wyniku (`spec/report.md`, sekcja 2.1). Jego
parametry: `id` (pozycja 1, tekst: `c3`, liczba albo słowa `all` i `full`),
`count` (pozycja 2, int), `all` (bool) i `full_lines` (bool). Formy krótkie to
`next`, `next c3`, `next 200`, `next c3 200`, `next all` i `next full`; każda
forma krótka ma ten sam wynik co jej forma długa, a `next |> filter …`
kontynuuje część jako potok. Polecenie niczego nie zmienia, nie zapisuje wpisu
dziennika i nie kosztuje skutku w `E_ONE_EFFECT`.

### 9.3 Grupa `administration` {#93-the-group-administration}

Plik `administration.toml` zawiera polecenia zmieniające system poza systemem
plików, polecenia usług i wywołania zarządzania systemu Windows. Każde
polecenie zmieniające akceptuje `privilege` (sekcja 2.1) i zapisuje stan przed i
po w swoim raporcie, z którego `undo` wyprowadza odwrotność; polecenie
oznaczone `irreversible` nie ma odwrotności, a jego podgląd wymienia to, co
zostanie utracone. Opis narzędzia ładuje grupę na żądanie pod nazwą
`administration`.

| Polecenie | Kategoria | Skutek | Odwracalność |
|---|---|---|---|
| `services` | source | read | read |
| `start` | run | external | reversible_if_preserved |
| `restart` | run | external | irreversible |
| `enable` | change | external | reversible_if_preserved |
| `disable` | change | external | reversible_if_preserved |
| `windows.cim` | change | read | read |
| `firewall.rules` | source | read | read |
| `firewall.allow` | change | external | reversible |
| `firewall.deny` | change | external | reversible |
| `firewall.remove` | change | external | reversible |
| `network.route_add` | change | external | reversible |
| `network.route_remove` | change | external | reversible |
| `network.dns_set` | change | external | reversible |
| `clock.timezone_set` | change | external | reversible |
| `clock.set` | change | external | irreversible |
| `clock.ntp` | change | external | reversible |
| `accounts.create` | change | external | reversible_if_preserved |
| `accounts.modify` | change | external | reversible_if_preserved |
| `accounts.remove` | change | external | reversible_if_preserved |
| `accounts.password` | change | external | irreversible |
| `groups.create` | change | external | reversible_if_preserved |
| `groups.remove` | change | external | reversible_if_preserved |
| `groups.add_member` | change | external | reversible_if_preserved |
| `groups.remove_member` | change | external | reversible_if_preserved |
| `mount` | change | external | reversible_if_preserved |
| `unmount` | change | external | reversible_if_preserved |
| `sysctl.set` | change | external | reversible_if_preserved |
| `modules.load` | change | external | reversible_if_preserved |
| `modules.unload` | change | external | reversible_if_preserved |
| `power.restart` | change | external | reversible_if_preserved |
| `power.shutdown` | change | external | reversible_if_preserved |
| `power.cancel` | change | external | irreversible |
| `disks.partition` | change | external | irreversible |
| `disks.format` | change | external | irreversible |

## 10. Kontrole {#10-checks}

`CatalogFile::check`, `CatalogIndex::check` i `check_catalog` zwracają listę
problemów; zestaw testów wymaga pustej listy dla każdego pliku w `catalog/`:

1. każdy plik parsuje się do `CatalogFile` (nieznane klucze są odrzucane), ma
   format `nurt-catalog/1` i grupę równą nazwie pliku;
2. nazwy poleceń są poprawne i unikalne w całym katalogu;
3. każde wyrażenie typu daje się sparsować; każdy `default` jest niepusty;
   parametr wymagany nie ma wartości domyślnej; `values` powtarzają wartości
   wyliczenia typu; wartość domyślna w postaci gołego słowa jest jedną z wartości; `default_from`
   nazywa inny parametr polecenia lub `base` i wyklucza `default`;
4. pozycje każdego polecenia tworzą ciąg `1..n`; parametr powtarzany jest
   pozycyjny i akceptuje listę;
5. grupy wyłączne mają co najmniej dwóch członków; grupy wymagane istnieją;
6. formy są spójne z parametrami i obejmują każdy rodzaj wejścia raz;
   `expand` występuje tylko w formie źródłowej, nazywa parametr akceptujący
   ścieżkę, a ten parametr nie jest akceptowany przez formę dla wejścia
   `entry`;
7. indeks wymienia każdy plik wraz z liczbą jego poleceń, a gdy każdy
   wymieniony plik jest obecny, liczba poleceń jest równa `total`.

## 11. Zgodność wsteczna {#11-compatibility}

W obrębie `nurt-catalog/1` katalog tylko rośnie: nowe polecenia, nowe
parametry opcjonalne, których wartość domyślna zachowuje poprzednie
zachowanie, nowe wartości wyliczeń i nowe klucze opcjonalne. Usunięcie lub
zmiana nazwy polecenia albo parametru, niezgodna zmiana typu lub zmiana wartości
domyślnej jest zmianą wersji głównej.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
