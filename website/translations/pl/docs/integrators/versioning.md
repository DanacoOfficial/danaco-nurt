<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/integrators/versioning.md -->
<!-- Responsibility:   Documents Versions and compatibility. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Wersje i zgodność {#versions-and-compatibility}

| Element | Postać wersji | Reguła |
|---|---|---|
| żądanie, odpowiedź, raport, drzewa składni | `nurt: "MAJOR.MINOR"` w każdym dokumencie | wersja podrzędna dodaje pola opcjonalne i nowe wartości wyliczeń oznaczonych jako rozszerzalne; czytnik ignoruje nieznane pola opcjonalne |
| protokół `nurt-rpc/1` | `initialize` zwraca protokół i funkcje | host sprawdza wersję główną i odczytuje listę funkcji zamiast porównywać wersje programów |
| ABI C | `nurt_abi_version()` | wersja główna musi się zgadzać; wersja podrzędna musi być co najmniej taka, względem jakiej zbudowano hosta |
| definicja narzędzia | podąża za `nurt-request/1` | nowa opcja staje się nowym argumentem opcjonalnym; nazwy narzędzi i istniejące argumenty nie zmieniają się w obrębie wersji głównej |
| katalog poleceń | `catalog_version` | nowe polecenia i parametry są dodawane w obrębie wersji głównej; opisy mogą się zmieniać między wersjami podrzędnymi |

Host, który wysyła dokument w nowszej wersji podrzędnej, niż konsola obsługuje, otrzymuje błąd języka wskazujący nieobsługiwane pole; nowsza wersja główna jest odrzucana jako nieobsługiwana. Hosty przypinają wersję główną używanych formatów i testują za pomocą zestawu zgodności `nurt-conformance`.

Implementacje i integracje, które przechodzą obowiązkowe testy `nurt-conformance`, mogą używać nazwy „Danaco Nurt Compatible”.

Pierwszym żądaniem sesji jest `initialize`; wynik wymienia protokół, wersje formatów, funkcje i limity:

<!-- schema: rpc -->
```json
{"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocol": "nurt-rpc/1", "client": {"name": "example-host", "version": "1.0.0"}, "capabilities": {"events": true}}}
```

<!-- schema: rpc#/$defs/initialize_result -->
```json
{
  "protocol": "nurt-rpc/1",
  "version": "0.1.2",
  "abi": 65536,
  "language": null,
  "ast": "1.1",
  "report": "1.0",
  "catalog_rev": null,
  "features": ["events", "preview"],
  "groups": ["core"],
  "limits": {"max_inflight": 16, "max_message_bytes": 16777216}
}
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
