---
name: notebooks
description: Odczyt i zmiana notatników Jupyter komórka po komórce.
commands: [notebook.cells, notebook.insert, notebook.replace, notebook.move, notebook.remove, notebook.clear_outputs, notebook.export, notebook.normalize, notebook.ops]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/guides/notebooks.md -->
<!-- Responsibility:   Documents notebooks. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# notatniki {#notebooks}

## Komórki {#cells}

Notatnik jest listą komórek z typem, źródłem i wynikami. `notebook.cells` je wyświetla. `view: outline` zwraca jeden wiersz na komórkę, `outputs` wybiera, czy wyniki są pomijane, podsumowywane, zwracane jako tekst, czy zwracane w całości, a `find` zwraca tylko komórki zawierające dany tekst.

```nurt
notebook.cells "/srv/a.ipynb" view: outline
notebook.cells "/srv/a.ipynb" find: "import pandas" scope: source
```

## Indeks i identyfikator {#index-and-identifier}

Komórkę wskazuje się przez `index`, czyli jej pozycję, lub przez `id`, czyli jej identyfikator. Pozycja zmienia się przy wstawianiu lub usuwaniu komórek; identyfikator nie. Notatniki zapisane przez starsze narzędzia mogą nie mieć identyfikatorów: `notebook.normalize` dodaje je oraz brakujące pola formatu.

```nurt
notebook.normalize "/srv/a.ipynb"
```

## Zmiana komórek {#changing-cells}

`notebook.replace` zastępuje źródło lub typ komórki albo tekst wewnątrz źródeł komórek przez `find` i `with`. `notebook.insert` dodaje komórkę na pozycji lub po komórce wskazanej przez `after_id`. `notebook.move`, `notebook.copy` i `notebook.remove` zmieniają kolejność i zbiór komórek.

```nurt
notebook.replace "/srv/a.ipynb" index: 3 source: "print(1)"
notebook.insert "/srv/a.ipynb" after_id: "c3" cell_type: markdown source: "## Results"
```

## Wyniki i kilka zmian {#outputs-and-several-changes}

`notebook.clear_outputs` usuwa wyniki komórek, a `reset_outputs` polecenia `notebook.replace` robi to samo dla zmienianych przez nie komórek. `notebook.ops` stosuje listę operacji na komórkach w jednym zapisie, więc notatnik nigdy nie zostaje zmieniony częściowo.

```nurt
notebook.clear_outputs "/srv/a.ipynb"
```

## Eksport {#export}

`notebook.export` zapisuje notatnik do nowego pliku jako skrypt, Markdown lub tekst, zależnie od `format`. Sam notatnik nie jest zmieniany. Wyniki nie wchodzą w skład wyeksportowanego skryptu.

```nurt
notebook.export "/srv/a.ipynb" to: "/srv/a.py" format: script
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
