<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/reference/extensions.md -->
<!-- Responsibility:   Documents Extensions. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Rozszerzenia {#extensions}

Rozszerzenia dodają do Danaco Nurt polecenia, formaty dokumentów i gramatyki bez
zmiany programu. Rozszerzenie to katalog z manifestem `nurt-ext.toml` i
komponentem WebAssembly albo, gdy konfiguracja na to pozwala, biblioteką
natywną.

## Katalog rozszerzeń {#extension-directory}

| Element | Wartość |
|---|---|
| Klucz konfiguracji | `ext.dir` (ciąg znaków, ścieżka) |
| Zmienna środowiskowa | `NURT_EXT_DIR` |
| Wartość domyślna | `ext` w katalogu konfiguracji |
| Wartość względna | rozwiązywana względem katalogu konfiguracji |
| Układ | jeden podkatalog na rozszerzenie, nazwany jak rozszerzenie |

Klucz `ext.native` (wartość logiczna, domyślnie `false`) zezwala na natywne
biblioteki rozszerzeń. Biblioteki natywne są dostępne wyłącznie w kompilacjach
z funkcją `native`.

## Instalacja, usuwanie i listowanie {#installation-removal-and-listing}

| Operacja | Zachowanie | Błędy |
|---|---|---|
| install | Odczytuje i sprawdza manifest katalogu źródłowego, sprawdza, czy istnieje każdy wymieniony w nim plik, kopiuje katalog do `<ext.dir>/<name>` przez katalog tymczasowy i przełącza go przez zmianę nazwy. Zainstalowana wersja jest zastępowana wyłącznie na żądanie. | `E_CATALOG` (odrzucony manifest), `E_NOT_FOUND` (brak manifestu lub wymienionego pliku), `E_EXISTS` (zainstalowane, nie zażądano zastąpienia), `E_UNSUPPORTED` (dowiązanie symboliczne lub plik specjalny w źródle, niedozwolone rozszerzenie natywne), `E_INVALID_VALUE` (katalog źródłowy zawiera `<ext.dir>`), `E_IO` |
| remove | Usuwa `<ext.dir>/<name>`, niezależnie od tego, czy nadaje się do użycia. | `E_INVALID_VALUE` (nieprawidłowa nazwa), `E_NOT_FOUND`, `E_IO` |
| list | Wymienia każdy podkatalog `ext.dir` w kolejności nazw wraz z nazwą, katalogiem, wersją, wersją API, rodzajem i pełnymi nazwami poleceń. Rozszerzenie, które nie nadaje się do użycia, jest wymieniane ze składnikiem `problem`. | `E_IO` |

Przerwana instalacja pozostawia poprzednią wersję na miejscu. Katalogi, których
nazwy zaczynają się od `.`, są obszarami tymczasowymi i nie są wymieniane;
każda instalacja używa własnego katalogu tymczasowego, więc instalacje jednego
rozszerzenia nie nadpisują nawzajem swoich kopii.

### Zaufanie {#trust}

Instalacja nie weryfikuje podpisu ani sumy kontrolnej źródła: konto, które
instaluje rozszerzenie, ufa jego katalogowi źródłowemu. Rozszerzenie
WebAssembly działa w piaskownicy i sięga do plików wyłącznie przez wywołania
hosta opisane poniżej, w granicach `permissions.read` i `permissions.write`;
uprawnienia są sprawdzane, gdy rozszerzenie prosi o ścieżkę, i ponownie, gdy
jego zapisy są stosowane. Rozszerzenie natywne działa w procesie silnika bez
takich ograniczeń, dlatego `ext.native` jest domyślnie wyłączone.

## Manifest {#manifest}

```toml
[ext]
name = "acme"
version = "1.2.0"
api = "nurt:ext@1.0"
kind = "wasm"
wasm = "acme.wasm"
description = "Checks of acme files"

[[stage]]
name = "lint"
summary = "Reports problems of acme files."
input = ["entry"]
output = ["finding"]

[[stage.param]]
name = "strict"
type = "bool"
summary = "Reports style problems too."

[[format]]
id = "acme-conf"
extensions = ["acmeconf"]

[[grammar]]
language = "acme"
extensions = ["acme"]
wasm = "grammar/acme.wasm"

[permissions]
read = ["${project}"]
write = []
network = []
```

Nieznane klucze są odrzucane.

### Sekcja `[ext]` {#section-ext}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `name` | ciąg znaków | wymagane | Nazwa rozszerzenia i prefiks poleceń: małe litery ASCII, cyfry i `_`, zaczynające się od litery, co najwyżej 64 bajty. Nie może być grupą, poleceniem ani prefiksem polecenia rdzenia. |
| `version` | ciąg znaków | wymagane | Wersja rozszerzenia, Semantic Versioning 2.0.0. |
| `api` | ciąg znaków | wymagane | Wymagana wersja interfejsu, `nurt:ext@MAJOR.MINOR`; numer poprawki jest akceptowany i ignorowany. |
| `kind` | `"wasm"` \| `"native"` | wymagane | Postać dystrybucji. |
| `wasm` | ciąg znaków | brak | Plik komponentu; wymagany dla `kind = "wasm"`. |
| `library` | ciąg znaków | brak | Plik biblioteki; wymagany dla `kind = "native"`. |
| `description` | ciąg znaków | brak | Opis w jednym wierszu; streszczenie grupy poleceń. |

Nazwy plików są ścieżkami względnymi wewnątrz katalogu rozszerzenia; `..`,
ścieżki bezwzględne i prefiksy korzenia są odrzucane.

### Wpisy `[[stage]]` {#entries-stage}

Każdy wpis deklaruje polecenie `<ext.name>.<name>`.

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `name` | ciąg znaków | wymagane | Nazwa polecenia bez prefiksu; te same reguły co `ext.name`. |
| `summary` | ciąg znaków | generowane | Opis w jednym wierszu. |
| `input` | tablica ciągów znaków | `[]` | Przyjmowane rodzaje rekordów; `any` przyjmuje każdy rodzaj. |
| `output` | tablica ciągów znaków | `[]` (dowolny rodzaj) | Wytwarzane rodzaje rekordów. |
| `source` | wartość logiczna | `false` | Polecenie może rozpocząć potok. Polecenie z pustym `input` musi być źródłem. |
| `writes` | wartość logiczna | `false` | Polecenie zmienia pliki; wymaga niepustego `permissions.write`. |
| `network` | wartość logiczna | `false` | Polecenie łączy się z siecią; wymaga niepustego `permissions.network`. |
| `param` | tablica tabel | `[]` | Parametry, z kluczami parametru katalogu poleceń. |

Polecenie bez `writes` jest etapem odczytu kategorii `transform` (lub `source`
bez wejścia). Polecenie z `writes` jest etapem zmiany z odwracalnym skutkiem
lokalnym.

### Wpisy `[[format]]` {#entries-format}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `id` | ciąg znaków | wymagane | Identyfikator formatu: małe litery ASCII, cyfry, `_` i `-`. |
| `extensions` | tablica ciągów znaków | `[]` | Rozszerzenia nazw plików bez początkowej kropki. |
| `editable` | wartość logiczna | `false` | Klucze można zmieniać z zachowaniem układu. |

### Wpisy `[[grammar]]` {#entries-grammar}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `language` | ciąg znaków | wymagane | Identyfikator języka; te same reguły co `format.id`. |
| `extensions` | tablica ciągów znaków | `[]` | Rozszerzenia nazw plików bez początkowej kropki. |
| `wasm` | ciąg znaków | wymagane | Plik komponentu gramatyki. |
| `tags` | ciąg znaków | brak | Plik zapytań o znaczniki. |
| `locals` | ciąg znaków | brak | Plik zapytań o zakresy lokalne. |

### Sekcja `[permissions]` {#section-permissions}

| Klucz | Typ | Wartość domyślna | Znaczenie |
|---|---|---|---|
| `read` | tablica ciągów znaków | `[]` | Katalogi, które rozszerzenie może odczytywać. |
| `write` | tablica ciągów znaków | `[]` | Katalogi, które rozszerzenie może zmieniać; są także odczytywalne. |
| `network` | tablica ciągów znaków | `[]` | Hosty, z którymi rozszerzenie może się łączyć. |

Katalog jest bezwzględny albo zaczyna się od symbolu zastępczego:

| Symbol zastępczy | Wartość podczas wykonania |
|---|---|
| `${project}` | Katalog bazowy żądania. |
| `${home}` | Katalog domowy konta. |
| `${temp}` | Katalog tymczasowy. |
| `${ext}` | Katalog rozszerzenia. |

Wpis, którego symbol zastępczy nie ma wartości, niczego nie przyznaje. Ścieżka
jest przyznana, gdy leży poniżej przyznanego katalogu zarówno w postaci
zapisanej, jak i po rozwiązaniu dowiązań symbolicznych. Ścieżki ze składnikami
`..` i ścieżki względne są odrzucane.

## Wersje interfejsu {#interface-versions}

Host implementuje `nurt:ext@1.0`. Manifest jest akceptowany, gdy jego wersja
główna jest równa wersji głównej hosta, a jego wersja podrzędna nie jest nowsza.
Każda inna wersja powoduje niepowodzenie instalacji z błędem `E_CATALOG`, a
komunikat wymienia obie wersje.

## Interfejs WebAssembly {#webassembly-interface}

Komponent implementuje świat `extension` pakietu `nurt:ext@1.0.0`:

```wit
package nurt:ext@1.0.0;

interface host {
    record failure {
        code: string,
        message: string,
    }

    enum log-level {
        error,
        warn,
        info,
        debug,
        trace,
    }

    read-file: func(path: string) -> result<list<u8>, failure>;
    list-dir: func(path: string) -> result<list<string>, failure>;
    write-file: func(path: string, content: list<u8>) -> result<_, failure>;
    log: func(level: log-level, message: string);
}

interface stage {
    run: func(command: string, args: string, input: list<string>) -> result<list<string>, string>;
}

world extension {
    import host;
    export stage;
}
```

### Funkcja `stage.run` {#function-stagerun}

| Parametr | Typ | Wartość |
|---|---|---|
| `command` | ciąg znaków | Pełna nazwa polecenia, `<extension>.<name>`. |
| `args` | ciąg znaków | Obiekt JSON z nazwy parametru na wartość; parametry bez wartości są pomijane. Wyrażenia dla poszczególnych rekordów są obliczane jednokrotnie, bez rekordu. |
| `input` | lista ciągów znaków | Rekordy wejściowe, po jednym dokumencie JSON każdy. |

Wynik: rekordy wyjściowe, po jednym dokumencie JSON każdy, albo komunikat
opisujący, dlaczego polecenie zakończyło się niepowodzeniem.

Rekord ma postać JSON `{"kind": "<record kind>", "fields": {...}}` z wartościami
pól w zwykłym JSON (teksty, liczby, wartości logiczne, listy i obiekty).
Składniki inne niż `kind` i `fields` są odrzucane.

### Wywołania hosta {#host-calls}

| Wywołanie | Dostęp | Wynik |
|---|---|---|
| `read-file` | poniżej `permissions.read` lub `permissions.write` | Cała zawartość pliku, co najwyżej `limits.max_file_bytes_text` bajtów. |
| `list-dir` | poniżej `permissions.read` lub `permissions.write` | Nazwy wpisów w kolejności bajtów. |
| `write-file` | poniżej `permissions.write`, polecenia z `writes = true` | Zapis jest rejestrowany, a nie wykonywany. Podgląd pokazuje go jako zmianę; krok zastosowania zapisuje go przez dziennik, więc zmianę można cofnąć. Zarejestrowane zapisy jednego wywołania mają razem co najwyżej `limits.max_file_bytes_text` bajtów; kolejny zapis jest odrzucany z błędem `E_LIMIT`. |
| `log` | zawsze | Dodaje zdarzenie `log` z poleceniem jako źródłem. |

Odrzucone wywołanie zwraca `failure` z kodem `E_PERMISSION`, `E_PATH_RELATIVE`,
`E_NOT_FOUND`, `E_LIMIT` lub `E_IO`. Gdy polecenie kończy się wtedy
niepowodzeniem, błędem polecenia jest odrzucone wywołanie.

### Limity {#limits}

| Limit | Wartość | Błąd |
|---|---|---|
| Paliwo wykonania na wywołanie | 10 000 000 000 jednostek, około jednej na instrukcję | `E_LIMIT` |
| Pamięć liniowa na instancję | 512 MiB | `E_STAGE_FAILED` |

Każde wywołanie działa w nowej instancji; żaden stan nie przetrwa między
wywołaniami.

## Biblioteki natywne {#native-libraries}

Rozszerzenie natywne to biblioteka współdzielona eksportująca trzy funkcje C:

```c
uint32_t nurt_ext_api(void);
int32_t nurt_ext_run(const uint8_t *request, size_t request_len,
                     uint8_t **response, size_t *response_len);
void nurt_ext_free(uint8_t *response, size_t response_len);
```

| Funkcja | Kontrakt |
|---|---|
| `nurt_ext_api` | Zwraca wersję interfejsu jako `MAJOR << 16 \| MINOR`. |
| `nurt_ext_run` | Otrzymuje obiekt JSON `{"command", "args", "input"}` i zapisuje odpowiedź zaalokowaną przez bibliotekę. Status 0 z `{"records": [...]}`; każdy inny status z `{"error": "<message>"}`. |
| `nurt_ext_free` | Zwalnia odpowiedź; wywoływana dokładnie raz na odpowiedź. |

Kod natywny działa z uprawnieniami procesu, więc host nie może ograniczyć
ścieżek, do których sięga. Rozszerzenia natywne są ładowane wyłącznie przy
`ext.native = true` i nie mogą deklarować poleceń z `writes = true`.

## Błędy {#errors}

| Kod | Warunek |
|---|---|
| `E_CATALOG` | Odrzucony manifest, niezgodność wersji interfejsu, nazwa rdzenia, komponent lub biblioteka nie do użycia, brakujący eksport `nurt:ext/stage@1.0.0`, nazwa zajęta w rejestrze. |
| `E_PERMISSION` | Wywołanie hosta dotarło do ścieżki poza uprawnieniami. |
| `E_STAGE_FAILED` | Polecenie zwróciło niepowodzenie, zakończyło się pułapką, zwróciło nieprawidłowy rekord albo zaplanowało zapisy bez `writes = true`. |
| `E_LIMIT` | Polecenie wyczerpało swoje paliwo wykonania. |
| `E_UNSUPPORTED` | Rozszerzenie natywne niedozwolone lub niewbudowane albo polecenie natywne deklaruje `writes = true`. |
| `E_NOT_FOUND`, `E_EXISTS`, `E_IO` | Instalacja, usuwanie i listowanie. |

## Przykłady {#examples}

Polecenie odczytu rozszerzenia w potoku:

```
find src -name '*.acme' |> acme.lint strict=true
```

Manifest polecenia, które zmienia pliki:

```toml
[ext]
name = "fmt"
version = "0.3.0"
api = "nurt:ext@1.0"
kind = "wasm"
wasm = "fmt.wasm"

[[stage]]
name = "apply"
input = ["entry"]
writes = true

[permissions]
write = ["${project}"]
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
