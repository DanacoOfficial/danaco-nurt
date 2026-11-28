---
name: journal-undo
description: Naprawianie pomyłki za pomocą dziennika: historia, cofnięcie, ponowienie, limity i miejsce.
commands: [history, undo, redo, journal.status, journal.space, journal.limits, journal.pin, journal.purge]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/guides/journal-undo.md -->
<!-- Responsibility:   Documents journal-undo. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# dziennik i cofnięcie {#journal-undo}

## Dziennik {#the-journal}

Każda zmiana plików, wartości dostawców lub stanu repozytorium jest rejestrowana wraz z poprzednim stanem. `history` wyświetla operacje, od najnowszej, wraz z ich stanem: `available`, `partial`, `expired` lub `undone`. `limit`, `since`, `before` i `path` zawężają listę.

```nurt
history limit: 10
history path: "/srv/app/main.go"
```

## Cofanie i ponawianie {#undoing-and-redoing}

`undo` cofa ostatnią operację albo operację wskazaną parametrem `id`. `to` i `since` cofają kilka operacji, od najnowszej, a `paths` przywraca tylko wskazane ścieżki operacji. `redo` stosuje cofniętą operację ponownie. Cofnięcie naprawia pomyłkę widoczną w wyniku; sprawdzenie przez `preview: true` lub `assert` przed zmianą nadal jest bezpieczniejszym postępowaniem.

```nurt
undo 18 paths: ["/srv/app/main.go"]
redo
```

## Konflikty {#conflicts}

Gdy plik zmienił się po operacji, jej cofnięcie odrzuciłoby późniejszą zmianę. `on_conflict` wybiera zachowanie: `report` zatrzymuje i wyświetla konflikty, `abort` pozostawia wszystko bez zmian, `force` przywraca stary stan, `include_later` cofa także późniejsze operacje, a `partial` przywraca ścieżki bez konfliktu. Błąd `E_JOURNAL_CONFLICT` wymienia ścieżki będące w konflikcie.

```nurt
undo on_conflict: partial
```

## Wpisy częściowe i nieodwracalne {#partial-and-irreversible-entries}

Wpis jest `partial`, gdy nie udało się zachować poprzedniego stanu niektórych celów. `history state: partial` wyświetla takie wpisy, a `undo on_conflict: partial` przywraca to, co zostało zachowane. Skutki poza systemem plików, takie jak uruchomienie usługi lub wypchnięcie do repozytorium zdalnego, są rejestrowane jako nieodwracalne i zgłaszane osobno. Błąd `E_JOURNAL_PARTIAL` oznacza cofnięcie, które przywróciło tylko część operacji.

## Limity i miejsce {#limits-and-space}

Wpisy wygasają, gdy limity dziennika zostaną przekroczone. `journal.status` pokazuje liczbę wpisów, zajęte miejsce, najstarszy wpis i obowiązujące limity. `journal.limits` pokazuje lub zmienia limity, `journal.pin` chroni wpisy przed wygaśnięciem, a `journal.space` zgłasza miejsce zajmowane przez dziennik na każdym woluminie. `journal.purge` zwalnia wpisy i zajmowane przez nie miejsce, i nie można go cofnąć.

```nurt
journal.status
journal.limits scope: user
journal.space
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
