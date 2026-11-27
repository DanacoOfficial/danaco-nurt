---
name: notebooks
description: Reading and changing Jupyter notebooks cell by cell.
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

# notebooks

## Cells

A notebook is a list of cells with a type, a source and outputs. `notebook.cells` lists them. `view: outline` returns one line per cell, `outputs` selects whether outputs are omitted, summarized, returned as text or returned in full, and `find` returns only cells that contain a text.

```nurt
notebook.cells "/srv/a.ipynb" view: outline
notebook.cells "/srv/a.ipynb" find: "import pandas" scope: source
```

## Index and identifier

A cell is addressed by `index`, its position, or by `id`, its identifier. A position changes when cells are inserted or removed; an identifier does not. Notebooks written by older tools can lack identifiers: `notebook.normalize` adds them and the missing fields of the format.

```nurt
notebook.normalize "/srv/a.ipynb"
```

## Changing cells

`notebook.replace` replaces the source or the type of a cell, or text inside cell sources with `find` and `with`. `notebook.insert` adds a cell at a position or after the cell named by `after_id`. `notebook.move`, `notebook.copy` and `notebook.remove` change the order and the set of cells.

```nurt
notebook.replace "/srv/a.ipynb" index: 3 source: "print(1)"
notebook.insert "/srv/a.ipynb" after_id: "c3" cell_type: markdown source: "## Results"
```

## Outputs and several changes

`notebook.clear_outputs` removes the outputs of cells, and `reset_outputs` of `notebook.replace` does the same for the cells it changes. `notebook.ops` applies a list of cell operations in one write, so the notebook is never left half changed.

```nurt
notebook.clear_outputs "/srv/a.ipynb"
```

## Export

`notebook.export` writes a notebook to a new file as a script, Markdown or text with `format`. The notebook itself is not changed. Outputs are not part of an exported script.

```nurt
notebook.export "/srv/a.ipynb" to: "/srv/a.py" format: script
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
