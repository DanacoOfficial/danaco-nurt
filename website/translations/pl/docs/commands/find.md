---
command: find
group: core
summary: Wyszukuje wpisy w katalogu według wzorca nazwy i atrybutów.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# find

## Składnia {#synopsis}

```text
find <path> [<glob>] [name: value]...
```

## Opis {#description}

`find` przechodzi katalog `path` i tworzy jeden rekord `entry` dla każdego
wpisu pasującego do `glob`, `type`, `file_type` i `fuzzy`, do głębokości
`depth` poziomów. W odróżnieniu od `list` sam katalog główny jest kandydatem na
głębokości 0: `find "/a" depth: 0 type: any` daje dokładnie jeden rekord, gdy
`/a` istnieje, a w przeciwnym razie kończy się błędem `E_NOT_FOUND`. Katalog
główny będący plikiem jest kandydatem w ten sam sposób.

Wzorce `glob` są dopasowywane do ścieżki względnej wobec `path`, z separatorami
`/` w każdym systemie. `*`, `?` i `[...]` nie pasują do `/`; `**` pasuje do
dowolnej liczby katalogów, więc `*.rs` pasuje tylko na najwyższym poziomie, a
`**/*.rs` na dowolnej głębokości. Katalog główny jest dopasowywany po własnej
nazwie. Wpis pasujący do dowolnego wzorca z listy jest wybierany. Wzorce
uwzględniają wielkość liter, chyba że `case_sensitive: false`.

`glob` i `exclude` porównują wzorce i ścieżki w postaci normalizacji Unicode C
(NFC). Nazwa zapisana w postaci rozłożonej, jak na woluminach macOS, pasuje do
wzorca zapisanego w postaci złożonej, a wzorzec w postaci rozłożonej pasuje do
nazwy złożonej.

Wpisy są wykluczane przed dopasowaniem, a wykluczony katalog nie jest
otwierany, więc `!pattern` nie może przywrócić wpisu poniżej wykluczonego
katalogu. Reguły obowiązują w tej kolejności: `hidden: false`,
`exclude_preset`, `exclude` oraz, z `gitignore: true`, pliki ignorowania.

| `exclude_preset` | Wykluczone katalogi |
|---|---|
| `vcs` | `.git`, `.hg`, `.svn`; także plik `.git` |
| `deps` | `node_modules`, `vendor`, `.venv`, `venv` |
| `build` | `target`, `dist`, `build`, `out` |
| `cache` | `.cache`, `__pycache__`, `.pytest_cache` |

`exclude` używa składni gitignore: wzorzec bez `/` pasuje do nazwy na dowolnej
głębokości, początkowy `/` zakotwicza go w `path`, końcowy `/` pasuje tylko do
katalogów, a decyduje ostatni pasujący wzorzec, przy czym `!pattern` przywraca.
Z `gitignore: true` pliki `.ignore` obowiązują w każdym katalogu, a
`.gitignore`, `.git/info/exclude` i globalny plik wykluczeń obowiązują w
repozytorium git; obowiązują także pliki przodków `path`, a głębszy plik ma
pierwszeństwo.

`file_type` wybiera pliki, których nazwa pasuje do wzorca jednego z typów.
Typy są takie jak w `search` i noszą nazwy z `rg --type-list`. Wzorzec pasuje
do całej nazwy, `*` oznacza dowolny ciąg znaków, a wielkość liter ma znaczenie:
`*.c` nie pasuje do `main.C`. Katalogi i dowiązania, które nie są śledzone, nie
są nigdy wybierane przez `file_type`.

| Typ | Wzorce nazw |
|---|---|
| `asm` | `*.asm`, `*.s`, `*.S` |
| `c` | `*.c`, `*.h`, `*.H`, `*.c.in`, `*.h.in`, `*.cats` |
| `clojure` | `*.clj`, `*.cljc`, `*.cljs`, `*.cljx`, `*.edn` |
| `cmake` | `*.cmake`, `CMakeLists.txt` |
| `config` | `*.cfg`, `*.conf`, `*.config`, `*.ini` |
| `cpp` | `*.C`, `*.H`, `*.cc`, `*.cpp`, `*.cxx`, `*.c++`, `*.h`, `*.hh`, `*.hpp`, `*.hxx`, `*.h++`, `*.inl` |
| `cs` | `*.cs`, `*.csx` |
| `css` | `*.css`, `*.scss` |
| `csv` | `*.csv` |
| `dart` | `*.dart` |
| `docker` | `*Dockerfile*`, `*dockerfile*` |
| `elixir` | `*.ex`, `*.eex`, `*.exs`, `*.heex`, `*.leex`, `*.livemd` |
| `erlang` | `*.erl`, `*.hrl` |
| `go` | `*.go` |
| `gradle` | `*.gradle`, `*.gradle.kts` |
| `groovy` | `*.groovy`, `*.gradle` |
| `haskell` | `*.hs`, `*.lhs`, `*.cpphs`, `*.c2hs`, `*.hsc` |
| `html` | `*.htm`, `*.html`, `*.ejs` |
| `java` | `*.java`, `*.jsp`, `*.jspx`, `*.properties` |
| `js` | `*.js`, `*.jsx`, `*.vue`, `*.cjs`, `*.mjs` |
| `json` | `*.json`, `*.jsonc`, `composer.lock`, `*.sarif` |
| `jsonl` | `*.jsonl`, `*.ndjson` |
| `julia` | `*.jl` |
| `kotlin` | `*.kt`, `*.kts` |
| `less` | `*.less` |
| `lua` | `*.lua` |
| `make` | `GNUmakefile`, `Gnumakefile`, `gnumakefile`, `Makefile`, `makefile`, `Makefile.am`, `makefile.am`, `Makefile.in`, `makefile.in`, `*.mk`, `*.mak` |
| `markdown`, `md` | `*.markdown`, `*.md`, `*.mdown`, `*.mdwn`, `*.mkd`, `*.mkdn`, `*.mdx` |
| `nix` | `*.nix` |
| `ocaml` | `*.ml`, `*.mli`, `*.mll`, `*.mly` |
| `perl` | `*.perl`, `*.pl`, `*.PL`, `*.plh`, `*.plx`, `*.pm`, `*.t` |
| `php` | `*.php`, `*.php3`, `*.php4`, `*.php5`, `*.php7`, `*.php8`, `*.pht`, `*.phtml` |
| `protobuf` | `*.proto` |
| `ps` | `*.cdxml`, `*.ps1`, `*.ps1xml`, `*.psd1`, `*.psm1` |
| `py` | `*.py`, `*.pyi`, `*.pyw` |
| `r` | `*.R`, `*.r`, `*.Rmd`, `*.Rnw` |
| `rst` | `*.rst` |
| `ruby` | `Gemfile`, `*.gemspec`, `.irbrc`, `Rakefile`, `*.rake`, `*.rb`, `config.ru` |
| `rust` | `*.rs` |
| `sass` | `*.sass`, `*.scss` |
| `scala` | `*.scala`, `*.sbt` |
| `sh` | `*.bash`, `.bashrc`, `.bash_profile`, `.bash_login`, `.bash_logout`, `*.sh`, `.profile`, `*.zsh`, `.zshrc`, `.zshenv`, `.zprofile`, `*.ksh`, `*.csh`, `*.tcsh` |
| `sql` | `*.sql`, `*.psql` |
| `svelte` | `*.svelte` |
| `svg` | `*.svg` |
| `swift` | `*.swift` |
| `tex` | `*.tex`, `*.ltx`, `*.cls`, `*.sty`, `*.bib`, `*.dtx`, `*.ins` |
| `tf` | `*.tf`, `*.tfvars`, `*.tf.json` |
| `toml` | `*.toml`, `Cargo.lock` |
| `ts` | `*.ts`, `*.tsx`, `*.cts`, `*.mts` |
| `tsv` | `*.tsv` |
| `txt` | `*.txt` |
| `vue` | `*.vue` |
| `xml` | `*.xml`, `*.xml.dist`, `*.dtd`, `*.xsl`, `*.xslt`, `*.xsd`, `*.xjb`, `*.rng`, `*.sch`, `*.xhtml` |
| `yaml` | `*.yaml`, `*.yml` |
| `zig` | `*.zig` |

`fuzzy` zachowuje wpisy, których nazwa zawiera znaki tekstu w tej samej
kolejności, i ustawia `rank`; wyższa ranga oznacza lepsze dopasowanie.
Dopasowania na początku nazwy, po `_`, `-`, `.`, `/` lub spacji, przy zmianie z
małej litery na wielką oraz w kolejnych znakach mają wyższą rangę. Tekst jest
dopasowywany bez względu na wielkość liter, chyba że zawiera wielką literę.
`sort: rank` umieszcza najlepsze dopasowania na początku i wymaga `fuzzy`.

Z `follow_links: true` dowiązania do katalogów są otwierane, a `type` klasyfikuje
dowiązanie według jego celu. Dowiązanie prowadzące z powrotem do katalogu,
który jest właśnie przechodzony, jest rekordem, ale nie jest otwierane; wpis
uwagi `link_loop_skipped` podaje liczbę takich dowiązań.

Rekordy zawierają `depth` (0 dla katalogu głównego) i `relative` (`.` dla
katalogu głównego). Ścieżka, która nie jest poprawnym Unicode, jest pokazywana
z U+FFFD w `path`, `parent` i `relative` oraz zawiera natywne bajty w
`raw_path`, `raw_parent` i `raw_relative`, jak w `list`; kolejny etap odczytuje
wpis z `raw_path`. Widok tekstowy listy zawierającej tylko część wpisów drzewa
rysuje katalogi powyżej nich, wyprowadzone z ich ścieżek.
Porządki `view` są takie jak w `list`: `flat` sortuje wszystkie rekordy według
`sort`, a remisy według ścieżki, natomiast `tree` i `sizes` przechodzą w głąb,
a wpisy jednego katalogu są sortowane według `sort`. `sort: path` porównuje
ścieżki składnik po składniku, więc katalog występuje tuż przed swoją zawartością.
Z `view: sizes` każdy rekord katalogu zawiera `total_size` i `file_count` dla
wszystkich plików poniżej niego, które przechodzą wykluczenia, niezależnie od
tego, czy pasują do `glob`, `type` lub `depth`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Katalog do przeszukania. |
| `glob` | `glob \| list<glob>` | `"**"` | Wzorce nazw względem katalogu. |
| `type` | `enum(file\|directory\|link\|any)` | `file` | Rodzaj wpisów do wyszukania. |
| `depth` | `int` | bez limitu | Maksymalna liczba poziomów katalogów, w które się schodzi. |
| `hidden` | `bool` | `true` | Uwzględnia ukryte wpisy. |
| `follow_links` | `bool` | `false` | Schodzi do katalogów wskazywanych przez dowiązania. |
| `gitignore` | `bool` | `false` | Stosuje reguły plików .gitignore. |
| `exclude` | `list<glob>` | `[]` | Wzorce do wykluczenia, w składni gitignore; początkowy ! przywraca dopasowanie. |
| `exclude_preset` | `list<enum(vcs\|deps\|build\|cache)>` | `[]` | Zdefiniowane wykluczenia: kontrola wersji, zależności, wyniki budowania, pamięci podręczne. |
| `file_type` | `list<text>` | `[]` | Typy plików według rozszerzenia, takie jak go lub ts. |
| `fuzzy` | `text` | brak | Szereguje wpisy według podobieństwa ich nazwy do tego tekstu. |
| `case_sensitive` | `bool` | `true` | Dopasowuje wzorce nazw z uwzględnieniem wielkości liter. |
| `sort` | `enum(path\|modified\|size\|rank)` | `path` | Klucz sortowania; rank porządkuje według podobieństwa rozmytego. |
| `order` | `enum(asc\|desc)` | `asc` | Kolejność sortowania. |
| `view` | `enum(flat\|tree\|sizes)` | `flat` | Lista płaska, drzewo albo drzewo z sumami rozmiarów katalogów. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `entry`, `registry_value` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

Wpisy uwag: `permission_denied_skipped`, `source_unavailable`,
`nfd_name`, `portability` i `case_alias` jak w `list` oraz
`link_loop_skipped` dla pętli dowiązań. Każdy wpis podaje liczbę dotyczących go
wpisów i próbkę do pięciu ścieżek.

## Przykłady {#examples}

Wyszukanie źródeł Rust projektu na dowolnej głębokości:

```nurt example
find "/home/user/app" glob: "**/*.rs"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/Cargo.toml", "type": "file", "content_utf8": "\n" },
  { "path": "/home/user/app/src/main.rs", "type": "file", "content_utf8": "\n" },
  { "path": "/home/user/app/src/parser/lexer.rs", "type": "file", "content_utf8": "\n" },
  { "path": "/home/user/app/web/fooBar.ts", "type": "file", "content_utf8": "\n" },
  { "path": "/home/user/app/web/fabric.ts", "type": "file", "content_utf8": "\n" },
  { "path": "/home/user/app/web/node_modules/left-pad/index.ts", "type": "file", "content_utf8": "\n" }
] }
```

Wyszukanie plików TypeScript poza katalogami kontroli wersji i zależności:

```nurt example
find "/home/user/app" exclude_preset: [vcs, deps] file_type: ["ts"]
```

Uszeregowanie plików według podobieństwa ich nazw do `fb`:

```nurt example
find "/home/user/app" fuzzy: "fb" sort: rank
```

Utworzenie rekordu jednej ścieżki jako źródła polecenia zmieniającego:

```nurt example
find "/home/user/app/Cargo.toml" depth: 0 type: any
```

Wyszukanie pliku według jego nazwy w postaci złożonej na woluminie, który
przechowuje nazwę w postaci rozłożonej; rekord zawiera nazwę `name` w NFC oraz
zapisane bajty w `raw_name`:

```nurt example
find "/home/user/notes" glob: "**/café.md"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/notes/2026/cafe\u0301.md", "type": "file", "content_utf8": "\n" },
  { "path": "/home/user/notes/2026/cafe.md", "type": "file", "content_utf8": "\n" }
] }
```

Odpowiedź, która pomija rekordy, kończy się wskazówką „more: next” i zawiera
pole `continuation`; polecenie `next` zwraca następną część bez ponownego
uruchamiania polecenia.

```text
matched 1480  shown 50  more: next
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | `path` nie istnieje | Sprawdź ścieżkę. |
| `E_PERMISSION` | `path` nie można odczytać | Przeszukaj katalog, który konto może odczytać. |
| `E_PATH_RELATIVE` | `path` jest względna i nie ustawiono katalogu bazowego | Podaj ścieżkę bezwzględną albo ustaw opcję `base`. |
| `E_INVALID_VALUE` | wzorzec `glob` lub `exclude` jest nieprawidłowy albo `depth` jest ujemne | Popraw wzorzec pokazany w `found`. |
| `E_UNKNOWN_VALUE` | nieznany typ pliku, zestaw wykluczeń lub wartość wyliczenia | Użyj jednej z wartości wymienionych w `expected`. |
| `E_MISSING_PARAM` | `sort: rank` bez `fuzzy` | Podaj `fuzzy` albo posortuj według innego klucza. |
| `E_LIMIT` | wynik przekracza limit rekordów | Zawęź go za pomocą `glob`, `depth` lub `exclude`. |

## Powiązane {#related}

`list` pokazuje wpisy jednego katalogu; `search` wyszukuje tekst w plikach;
`info` opisuje jeden wpis.
