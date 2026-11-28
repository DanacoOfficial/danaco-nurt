---
title: Bezpieczeństwo
order: 12
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/site/security.md -->
<!-- Responsibility:   Documents Security. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Bezpieczeństwo {#security}

## Zgłaszanie podatności {#reporting-a-vulnerability}

Podejrzenia podatności zgłasza się e-mailem na adres `support@danaco-group.pl`, z tematem zaczynającym się od `[security] nurt`, a nie w publicznym zgłoszeniu. Politykę, zakres i czasy obsługi opisuje [SECURITY.md](https://github.com/DanacoOfficial/danaco-nurt/blob/main/SECURITY.md).

## Właściwości powłoki {#properties-of-the-shell}

| Właściwość | Opis |
|---|---|
| Sprawdzenie przed skutkiem | Nazwy, parametry i typy są walidowane względem katalogu, zanim nastąpi jakikolwiek skutek |
| Brak powłoki | Programy nie przechodzą przez powłokę; ścieżka jest jedną wartością i nie jest dzielona ani rozwijana |
| Jeden skutek na program | Program z więcej niż jednym poleceniem zmieniającym lub uruchamiającym jest odrzucany z `E_ONE_EFFECT` |
| Raport | Każde uruchomienie zgłasza swoje cele, nietypowe skutki i skutki nieodwracalne |
| Dziennik | Zmiany plików są zapisywane i można je cofnąć w granicach limitów retencji |
| Sekrety | Parametry oznaczone w katalogu jako `secret` są maskowane w dzienniku, raporcie i wypisywanych drzewach |
| Pliki projektu | Plik projektu nie może nadać sobie zaufania; klucze rozszerzające zaufanie są w `.nurt.toml` ignorowane |

## Podniesienie uprawnień {#privilege-elevation}

Wywołanie może zażądać uprawnień administratora parametrem wywołania `privilege` (`account`, `elevated`, `auto`) oraz innego konta przez `as_user`. Nurt nie stosuje własnej polityki: decyzja należy do silnika osadzającego, który ustawia `privilege.allowed = false`, aby odrzucić `elevated` i `auto` z `E_PRIVILEGE` i przyczyną `disabled_by_configuration`. Plik projektu nie może ustawiać `privilege.*`.

| Backend | Sposób działania |
|---|---|
| `self` | proces już jest administratorem; operacja działa w tym samym procesie |
| `sudo`, `doas`, `run0` | `nurt elevated-exec` jest uruchamiany z uprawnieniami administratora i odczytuje jedno żądanie w formacie żądania protokołu ze standardowego wejścia; nie jest budowany żaden tekst powłoki i nie jest pytane o hasło (`-n`, `nopass`, `--no-ask-password`) |
| `broker` | uprzywilejowana usługa `nurt-elevation`; do czasu wydania zgłaszana jako `not_installed` |

Kolejność i zestaw backendów pochodzą z `privilege.backends`. Kontrola dostępu jest tą z systemu operacyjnego: konto, które może uruchamiać `nurt` przez `sudo`, `doas` lub `run0`, jest równoważne administratorowi, tak jak w przypadku tych programów. Podniesiony proces odmawia działania, jeśli jego tożsamość nie jest administratorem (lub kontem wskazanym przez `as_user`), zapisuje swój wpis dziennika w dzienniku konta, które o to poprosiło, z uprawnieniami tego konta (w Linuksie przez tożsamość systemu plików wątku, w macOS przez deskryptory, które są sprawdzane i przekazywane kontu przez `fchown` przed zapisem czegokolwiek; w Windows jest to to samo konto), i zwraca raport, który zawiera `elevated: true` oraz `backend`.

Właściwości istotne dla polityki uprawnień programu hosta zwraca `nurt check`; zobacz [Integracja](integration.md).

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
