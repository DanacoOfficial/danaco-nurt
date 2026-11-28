---
title: Język
order: 4
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/site/language.md -->
<!-- Responsibility:   Documents Language. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Język {#language}

Język definiuje specyfikacja: [drzewo składni](../../spec/syntax-tree.md), [katalog](../../spec/catalog.md), [rekordy](../../spec/records.md) i [etapy](../../spec/stages.md). Ta strona podsumowuje reguły, których używa dokumentacja.

## Programy i potoki {#programs-and-pipelines}

Program to ciąg instrukcji rozdzielonych podziałami wiersza. Instrukcja jest potokiem lub powiązaniem `let`. Potok to polecenie, po którym następują polecenia połączone przez `|>`. Podział wiersza przed `|>` kontynuuje potok.

```nurt example
let recent = find "/home/user/app/src" glob: "**/*.ts" |> filter modified > ago(1h)
@recent |> count
```

`let name = pipeline` zapisuje wynik pod nazwą, a `@name` go używa. W tekście nie ma zmiennych ani podstawiania `$` w wartościach.

## Wartości {#values}

| Rodzaj | Zapis | Przykład |
|---|---|---|
| Tekst | podwójne cudzysłowy, sekwencje `\n \t \\ \"` | `"/home/user/app"` |
| Tekst surowy | `r"…"` lub `r#"…"#` | `r#"import "fmt""#` |
| Wyrażenie regularne | między ukośnikami, flagi `i m s x` | `/oldName\(/i` |
| Liczba całkowita, liczba | cyfry, dozwolone `_` | `1_000`, `1.5` |
| Rozmiar | liczba i jednostka `b kb mb gb tb kib mib gib tib` | `10mb` |
| Czas trwania | liczby z jednostkami `ms s m h d w` | `1h30m` |
| Zakres | `5`, `1..5`, `40..`, `..9`, `all` | `10..20` |
| Lista, mapa | nawiasy kwadratowe, nawiasy klamrowe | `["a", "b"]`, `{CI: "1"}` |
| Wartość logiczna, null | `true`, `false`, `null` | `true` |

Rozmiar zapisany w jednostkach dziesiętnych (`mb`) liczy `1mb` jako 1 000 000 bajtów; jednostka binarna (`mib`) liczy 1 048 576 bajtów.

## Argumenty {#arguments}

Argumenty pozycyjne występują w kolejności pozycji w katalogu. Argumenty nazwane zapisuje się `name: value` i mogą występować w dowolnym miejscu po pozycyjnych. Parametry mają typy; wymienia je [gramatyka typów](../../spec/catalog.md).

## Wyrażenia {#expressions}

`filter`, `sort`, `select`, `assert` i `evaluate` przyjmują wyrażenia obliczane dla każdego rekordu. Operatory to `== != < <= > >= ~ !~ in and or not + - * /`. `~` dopasowuje wyrażenie regularne. Funkcje obejmują `ago(duration)`.

## Warunki {#conditions}

`when` uruchamia jedną gałąź. Warunek jest wyrażeniem `bool`: porównania, `and`, `or`, `not`, `exists <path>`, potok odczytujący w nawiasach (`(processes name: "nginx" |> count) > 0`), `exit_code` oraz pola rekordu przechowywanego przez `let`. `otherwise when` się powtarza, a `otherwise` jest opcjonalne. Warunek jest obliczany raz, przed swoją gałęzią; raport podaje jego wartość i numer gałęzi. Warunek, który nie jest `bool`, daje `E_TYPE` przed jakimkolwiek skutkiem, gdy jego typ jest znany przy wiązaniu: porównanie, goły potok, `exit_code` oraz pole rekordu pętli, którego typ deklaruje rekord wyjściowy polecenia źródłowego. To samo dotyczy każdego użycia takiego pola, którego nie może spełnić żadna wartość jego typu: `f.size ~ "x"`, `f.name < 3`, `f.name == 3`, `not f.name`, `f.path - 1`, `1 in f.size`. Jedynym wyjątkiem jest pole typu `any` lub `json`, którego typ zależy od danych: jest ono sprawdzane przy obliczaniu warunku, a niepowodzenie przywraca wtedy konstrukcję. Potok w warunku tylko odczytuje: etap, który może zmienić świat i bierze swoją akcję z wartości niebędącej literałem (`http.request … method: @m`, `git.branch delete: @a`), daje `E_TYPE` przy wiązaniu.

```nurt example
when (read "/etc/app.toml" key: "mode") == "prod" {
    replace /debug = true/ "debug = false" in: "/etc/app.toml" expect: 1
} otherwise when exists "/srv/app/.staging" {
    write "/srv/app/.ready" "staging\n" mode: overwrite
} otherwise {
    write "/srv/app/.dev" "1\n" mode: overwrite
}
```

## Pętle {#loops}

`each <records> as name { … }` uruchamia blok raz dla każdego rekordu, po kolei; `name` jest rekordem, a `name.path` polem. `repeat until: <bool> every: <duration> limit: <duration or number> { … }` uruchamia blok, a następnie sprawdza warunek; `while <bool> limit: <duration or number> { … }` sprawdza warunek przed każdym przebiegiem. Bez `limit` pętla kończy się przy `loop.max_iterations` (10 000) lub `loop.max_time` (10 minut): `repeat` kończy się wtedy `E_LIMIT`, `while` kończy się normalnie, a raport mówi, że zakończył ją limit. Z `limit` pętla ignoruje skonfigurowane wartości domyślne: liczba iteracji nie jest wtedy ograniczona czasem, a czas trwania nie jest ograniczony liczbą iteracji. `preview` planuje jedną iterację `repeat` lub `while`, ponieważ liczba iteracji jest znana dopiero po uruchomieniu; `each` planuje każdą iterację. `and` i `or` obliczają oba operandy, więc potok w prawym operandzie również się uruchamia, gdy rozstrzyga lewy. `break` i `continue` działają wewnątrz pętli, zwykle po `when`. `each` przyjmuje `parallel: N`; każda pętla przyjmuje `atomic: false`, które zachowuje iteracje ukończone przed niepowodzeniem i wskazuje tę, która się nie powiodła.

```nurt example
each (find "/srv/app" glob: "**/*.log") as f {
    lines f.path |> filter text ~ /ERROR/ |> write "/srv/reports/" + f.name + ".err" mode: overwrite
}
```

## Kilka zmian w jednym wywołaniu {#several-changes-in-one-call}

`steps { … }` zawiera potoki, z których każdy coś zmienia; `let` przekazuje wyniki do dalszych kroków.

```nurt example
steps {
    find "/srv/app/old" glob: "*.conf" |> move to: "/srv/app/conf"
    replace /old\/path/ "conf/path" in: "/srv/app/app.toml" expect: 3
}
```

## Funkcje {#functions}

`define name(parameter: type = default, …) { … }` definiuje funkcję; wywołanie zapisuje się `name(value, parameter: value)`. Ciało widzi swoje parametry (`@dir`) i własne `let`, nic z wywołującego. Wartość ostatniej instrukcji jest wynikiem. Rekurencja kończy się `E_LIMIT` przy `function.max_depth` (64).

```nurt example
define rotate_logs(dir: path, keep: int = 7) {
    find @dir glob: "*.log.*" |> sort modified order: desc |> skip @keep |> remove
}
rotate_logs("/var/log/app1")
rotate_logs("/var/log/app2", keep: 14)
```

## Parametry skryptu {#script-parameters}

Plik zaczyna się od `params { … }`; każdy wiersz to nazwa, typ i opcjonalna wartość domyślna. `nurt run cleanup.nurt dir: "/srv/app" days: 7 dry` przekazuje wartości, `nurt run cleanup.nurt --help` wypisuje deklarację, a silnik pobiera wartości z elementu `params` żądania. Brakująca wartość, nieznana nazwa (z najbliższą nazwą) lub wartość niewłaściwego typu są zgłaszane, zanim cokolwiek się uruchomi.

```nurt example
params {
    dir: path
    days: int = 30
    dry: bool = false
}
find @dir glob: "**/*.log" |> filter modified < ago(@days * 1d) |> remove preview: @dry
```

## Reguły każdej konstrukcji {#rules-of-every-construct}

Konstrukcja jest jednym wpisem dziennika: jedno `undo` odwraca ją w całości. Działa na zasadzie wszystko albo nic: gdy część się nie powiedzie, dotychczas wprowadzone zmiany są wycofywane, a części, których nie można wycofać (uruchomienie programu, restart), są wymieniane przed zmianą i wskazywane w błędzie. `preview` oblicza wybraną gałąź, iteracje i każdy skutek bez zmiany. Typy, nazwy i liczba argumentów są sprawdzane przed jakimkolwiek skutkiem, a konstrukcje są częścią drzewa składni: żaden tekst nie jest obliczany.

## Skrót dla wartości logicznych {#boolean-shorthand}

Parametr typu `bool` zapisany bez wartości ma wartość `true`: `remove "/tmp/x" preview`, `nurt run cleanup.nurt dry`. `false` zawsze zapisuje się jawnie (`preview: false`). Parametr, który nie jest `bool`, zapisany bez wartości daje `E_MISSING_PARAM` z oczekiwanym typem.

## Korekty {#corrections}

Nurt akceptuje nawyki z innych powłok i formy z drobnymi błędami. Korekta zmienia formę programu i nigdy jego znaczenie. Korekty dotykające celu zmiany są zgłaszane z uszeregowanymi kandydatami zamiast być stosowane. Nawyki bash i PowerShell oraz postać każdego z nich w Nurt wymieniają ściągawki dla [bash](../cheatsheets/nurt-for-bash-users.md) i [PowerShell](../cheatsheets/nurt-for-powershell-users.md).

## Błędy {#errors}

Błąd ma stały kod, taki jak `E_UNKNOWN_PARAM`, komunikat oraz, gdy są znane, etap, pozycję i program zastępczy. [Kody błędów](../../spec/errors.md) wymieniają kody.

## Wybór konstrukcji {#choosing-a-construct}

| Zadanie | Zapis | Dziennik i niepowodzenie |
|---|---|---|
| jedna zmiana | potok z jednym etapem zmieniającym na końcu | jeden wpis |
| kilka zmian, które stoją lub upadają razem | `steps { … }` | jeden wpis, wszystko albo nic |
| zmiana zależna od znalezionego stanu | `when … otherwise …` | jeden wpis z wybraną gałęzią |
| ta sama zmiana dla wielu rekordów | `each … as name { … }` | jeden wpis; `atomic: false` zachowuje ukończone iteracje |
| oczekiwanie na stan lub powtarzanie do limitu | `repeat until:` lub `while`, z `limit` | jeden wpis; limit kończy się `E_LIMIT` w `repeat` |
| zmiana używana kilka razy w pliku | `define name(…) { … }` | część wpisu wywołującego |
| plik przyjmujący wartości | `params { … }`, następnie `nurt run file.nurt name: value` | sprawdzane przed jakimkolwiek skutkiem |

Pojedynczy potok z więcej niż jednym etapem zmieniającym daje `E_ONE_EFFECT`; korekta jako pierwszego kandydata proponuje te same potoki wewnątrz `steps`. Tryb `script: true` pozostaje dla skryptów, które chcą osobnego wpisu dziennika dla każdego potoku.

## Gdzie opisano pozostałe funkcje {#where-the-other-features-are-described}

- `sql` zmienia bazę danych bez `write: true`, akceptuje `BEGIN`, `COMMIT`, `ROLLBACK`, `ATTACH`, `VACUUM` i `VACUUM INTO` oraz zapisuje w dzienniku każdy plik bazy danych, który zmienia: zobacz [polecenie](../commands/sql.md).
- `create` tworzy potoki nazwane (`type: fifo`) i węzły urządzeń (`type: device`): zobacz [przewodnik dla Linuksa i macOS](../guides/unix.md).
- Konta, grupy, zamontowania, parametry jądra, zapora sieciowa, sieć, zegar, zasilanie i dyski oraz praca z podwyższonymi uprawnieniami przez `privilege` i `as_user`: zobacz [przewodnik administracji](../guides/administration.md).
- Nawyki bash i PowerShell oraz ich postać w Nurt: zobacz przewodniki dla [bash](../guides/bash-migration.md) i [PowerShell](../guides/powershell-migration.md).

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
