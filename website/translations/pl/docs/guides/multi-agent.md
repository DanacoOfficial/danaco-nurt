---
name: multi-agent
description: Kilku agentów lub kilka sesji zmieniających te same pliki bez wzajemnego nadpisywania.
commands: [read, info, write, replace, edit, set, history, undo]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/guides/multi-agent.md -->
<!-- Responsibility:   Documents multi-agent. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# wielu agentów {#multi-agent}

## Etag {#the-etag}

Każdy odczyt pliku zawiera `etag`, wartość identyfikującą jego zawartość. Zmiana pliku jest stosowana tylko wtedy, gdy plik nadal ma zawartość, która została odczytana. `read` i `info` zwracają `etag` w swoich rekordach, a polecenie zmieniające sprawdza go przed zapisem.

```nurt
info "/srv/app/config.toml"
```

## Zapisy warunkowe {#conditional-writes}

Parametr `if_match` czyni sprawdzenie jawnym: zmiana jest stosowana tylko wtedy, gdy plik ma podany `etag`. Przyjmują go `write`, `replace`, `insert`, `edit`, `set` i `patch`. Bez `if_match` sprawdzenie używa stanu, który odczytało to samo wywołanie.

```nurt
write "/srv/app/config.toml" "mode = 1\n" mode: overwrite if_match: "a41f9c"
```

## Nieaktualna zawartość {#stale-content}

Zmiana pliku, który inna sesja zmodyfikowała po jego odczytaniu, kończy się błędem `E_STALE` i pozostawia plik bez zmian. Błąd wskazuje ścieżkę i bieżący `etag`. Środkiem zaradczym jest ponowny odczyt pliku, ocena, czy zmiana nadal ma zastosowanie, i wysłanie zmiany z nowym `etag`.

## Ponawianie {#retrying}

Odczyt następujący po `E_STALE` zwraca bieżącą zawartość. `read` z parametrem `unchanged_since` nie zwraca niczego, gdy plik nie zmienił się od podanego `etag`, co czyni powtarzane sprawdzanie tanim. Program, który zmienia kilka plików, zatrzymuje się na pierwszym konflikcie, zanim jakikolwiek plik zostanie zapisany.

```nurt
read "/srv/app/config.toml" unchanged_since: "a41f9c"
```

## Sesje i dziennik {#sessions-and-the-journal}

Parametr wywołania `session` nazywa sesję wywołania. Dziennik rejestruje sesję każdej operacji, a `history` wyświetla operacje wszystkich sesji; `undo` cofa wyłącznie operacje bieżącej sesji, chyba że podano `all_sessions: true`. Cofnięcie operacji, której pliki zostały zmienione przez inną sesję, zgłasza konflikt zamiast odrzucać cudzą zmianę; zob. przewodnik `journal-undo`.

```nurt
history limit: 20
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
