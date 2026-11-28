<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/integrators/policy.md -->
<!-- Responsibility:   Documents Policy. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Polityka {#policy}

Konsola uruchamia otrzymywane programy; nie decyduje, które programy agent może uruchamiać. Host egzekwuje swoją politykę przed `run`, na danych, które konsola oblicza bez żadnych skutków:

| Wywołanie | Skutek | Zwraca |
|---|---|---|
| `check` (`nurt serve`), `nurt_check` (ABI C) | brak; parsowanie, poprawki, wiązanie i plan | najsilniejszy skutek, najsłabszą odwracalność, ścieżki nazwane przez program, cele sieciowe, programy do uruchomienia, grupy poleceń, poprawki |
| `run` z `preview: true` | brak; przygotowanie do planu skutku | cele, które program by zmienił, z różnicami, skutki `notable` i `irreversible` znane z góry |

Odpowiedzią `check` jest dokument `nurt-check/1`:

| Pole | Zawartość |
|---|---|
| `format` | `nurt-check/1` |
| `effect` | najsilniejszy skutek planu: `read`, `local` (zmienia pliki lokalne), `external` (działa poza lokalnym systemem plików, na przykład `git.push` lub żądanie `POST`) albo `process` (uruchamia programy) |
| `reversibility` | najsłabsza odwracalność: `read`, `reversible`, `reversible_if_preserved` albo `irreversible` |
| `operations` | liczba operacji; więcej niż jedna dla skryptu |
| `waits` | `true`, gdy etap blokuje wywołanie przez oczekiwanie |
| `paths` | ścieżki nazwane przez literały programu, w kolejności występowania |
| `network` | jeden wpis na punkt końcowy: `command`, `param`, `source` (`literal`, `record`, `remote`, `catalog`, `implementation`), `target` (`scheme`, `host`, `port`, `url`; `null`, gdy nieznany przed uruchomieniem) oraz `remote` |
| `programs` | jeden wpis na program zewnętrzny: `command`, `program`, `implementation`, `launcher`, `source` |
| `groups` | grupy poleceń używane przez plan, w kolejności pierwszego użycia |
| `corrections` | poprawki zastosowane do tekstu programu przed wiązaniem: `id`, `rule`, `class`, `at`, `from`, `to`, `kind`, `alternatives` |
| `details` | fakty zgłoszone przez statyczną kontrolę poleceń programu: `command`, `index`, `details` |

Metoda `check` w `nurt serve` zwraca ten dokument ze składową `diagnostics` (diagnostyka parsowania i wiązania); obie części są wymagane w wyniku. `nurt_check` w ABI C zwraca dokument `nurt-check/1`.

Przykłady używają wiązania Pythona, w którym `CheckResult.raw` zawiera dokument.

Decyzje polityki są podejmowane na poprawionym i powiązanym programie oraz na rzeczywistych celach, nigdy na tekście programu.

Przykład dokumentu dla programu, który wypycha gałąź:

<!-- schema: check -->
```json
{
  "format": "nurt-check/1",
  "effect": "external",
  "reversibility": "irreversible",
  "operations": 1,
  "waits": false,
  "paths": ["/home/user/app"],
  "network": [
    {"command": "git.push", "param": null, "source": "remote", "target": null, "remote": "origin"}
  ],
  "programs": [],
  "groups": ["git"],
  "corrections": [],
  "details": []
}
```

Odpowiedź `nurt serve` na `check` dla powyższego programu:

<!-- schema: rpc -->
```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "format": "nurt-check/1",
    "effect": "external",
    "reversibility": "irreversible",
    "operations": 1,
    "waits": false,
    "paths": ["/home/user/app"],
    "network": [
      {"command": "git.push", "param": null, "source": "remote", "target": null, "remote": "origin"}
    ],
    "programs": [],
    "groups": ["git"],
    "corrections": [],
    "details": [],
    "diagnostics": []
  }
}
```

## Zabranianie polecenia: `git.push` {#forbidding-a-command-gitpush}

```python
check = nurt.check(program)
commands = {use["command"] for use in check.raw["network"]}
commands |= {facts["command"] for facts in check.raw["details"]}
if "git.push" in commands:
    return tool_error("git.push is not allowed in this session")
response = nurt.run(program)
```

`check` działa po poprawkach i wiązaniu oraz nazywa każde polecenie jego pełną nazwą, więc alias, inny zapis lub poprawiona pisownia polecenia, które tylko odczytuje, zostaną wychwycone. Błędnie napisana nazwa polecenia, które coś zmienia, nie jest poprawiana: `check` odpowiada `E_AMBIGUOUS`, a pierwszym kandydatem jest program z poprawnie zapisaną nazwą, więc zabronionego polecenia nie można osiągnąć także przez literówkę. Host, który nie dopuszcza żadnego skutku poza lokalnym systemem plików, porównuje zamiast tego `effect` z `external`.

## Dozwolone katalogi {#allowed-directories}

```python
ALLOWED = ("/home/user/app/",)

check = nurt.check(program, base="/home/user/app")
outside = [path for path in check.raw["paths"] if not path.startswith(ALLOWED)]
if outside:
    return tool_error(f"paths outside the workspace: {outside}")
preview = nurt.run(program, base="/home/user/app", preview=True)
targets = [change["path"] for change in preview.report.get("changes", [])]
if any(not path.startswith(ALLOWED) for path in targets):
    return tool_error("the program changes files outside the workspace")
```

`check` wymienia ścieżki zapisane w programie; `preview` wymienia pliki, które program by zmienił (`report.changes`), po rozwinięciu wzorców glob i filtrowaniu. Potrzebne są oba: wzorzec wewnątrz dozwolonego katalogu może nadal dotrzeć do dowiązania symbolicznego prowadzącego na zewnątrz.

## Cele sieciowe poleceń `http.*` {#network-targets-of-http}

Dla poleceń korzystających z sieci `check` zwraca każdy punkt końcowy, z którym program się łączy: polecenie, pochodzenie celu oraz, gdy jest znany przed uruchomieniem, schemat, host, port i URL. Host, który zezwala na dostęp do jednej usługi:

```python
check = nurt.check(program)
for use in check.raw["network"]:
    target = use["target"]
    if target is None or target["host"] != "api.example.com" or target["scheme"] != "https":
        return tool_error(f"network target not allowed: {use['command']}")
```

Cele obliczane w czasie wykonania (URL odczytany z pliku) mają `source: "record"` i `target: null`; ścisła polityka odrzuca takie programy lub najpierw uruchamia je z `preview: true`. Host, który musi dopuszczać wyłącznie odczyty, porównuje `effect` dokumentu z `read`.

## Programy uruchamiane przez `run` {#programs-started-by-run}

`programs` w dokumencie wymienia programy zewnętrzne, które plan by uruchomił, wraz z poleceniem uruchamiającym każdy z nich i pochodzeniem nazwy programu. Do tej listy stosuje się politykę hosta dla poleceń powłoki. Programy, o których wiadomo, że zmieniają stan zdalny (publikowanie, wdrażanie, powłoki zdalne), są zgłaszane w `irreversible` z przyczyną `external_effect`.

## Potwierdzenie przez użytkownika {#confirmation-by-the-user}

Host, który przed zmianą pyta użytkownika, uruchamia program z `preview: true`, pokazuje cele i różnice z raportu, a następnie uruchamia ten sam program bez `preview`. Dziennik rejestruje każdą zmianę; `undo` cofa operację według jej numeru `op`.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
