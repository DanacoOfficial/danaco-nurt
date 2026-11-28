---
title: Integracja
order: 9
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/site/integration.md -->
<!-- Responsibility:   Documents Integration. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Integracja {#integration}

Program hosta może wywoływać Danaco Nurt na cztery sposoby. Narzędzie widziane przez model nazywa się `nurt` w każdej integracji.

| Interfejs | Zastosowanie |
|---|---|
| Serwer MCP | `nurt mcp --stdio` lub `nurt mcp --http <address>` udostępnia narzędzie `nurt` |
| JSON-RPC | `nurt serve --stdio` obsługuje `nurt-rpc/1`; opcja `--socket <path>` jest zarezerwowana i kończy się kodem wyjścia 3 |
| Biblioteka C | `libnurt` i `nurt.h`; korzystają z nich powiązania dla innych języków |
| Biblioteka Rust | pakiet `nurt-core` |

## Narzędzie {#the-tool}

Narzędzie przyjmuje program w postaci tekstu lub drzewa składni i zwraca raport skutków. Schemat narzędzia to `nurt-tool/1`; wypisuje go `nurt schema tool`. Definicję narzędzia dla platformy, wraz z opisem dla modelu, tworzy konsola z katalogu poleceniem `nurt schema tool --format <format>`, dzięki czemu zawsze zgadza się z zainstalowaną wersją.

```text
nurt schema tool
nurt schema tool --format anthropic
```

## Polityki silnika {#policies-of-the-engine}

Polecenie `nurt check` i funkcja biblioteczna `nurt_check` zwracają skutki, ścieżki, cele, programy zewnętrzne, hosty sieciowe i grupy programu bez jego uruchamiania. Silnik stosuje do tego wyniku własną politykę uprawnień, zanim wywoła narzędzie.

## Dokumentacja dla integratorów {#documentation-for-integrators}

Przewodnik dla integratorów, z definicjami narzędzi dla dostawców modeli i przykładami polityk, to katalog [docs/integrators](https://github.com/DanacoOfficial/danaco-nurt/tree/main/docs/integrators) repozytorium.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
