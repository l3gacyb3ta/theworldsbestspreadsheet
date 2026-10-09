# Spill

When a cell's value is a list or a table, it spills: the first element shows in the cell itself, and the rest fill the cells below (lists) or below and to the right (tables). Charts spill too, over a block of cells.

- Spilled cells are drawn in blue inside a dashed outline. They're read-only; edit the source cell.
- A reference to the **source** cell means the whole array. `=A10 2 *` doubles every month if A10 spills a list of months.
- A reference to a cell **inside** the spill means just that element, and a range over spilled cells reads them like any other cells.
- If anything is in the way — a value, or another spill — the source shows `#spill blocked` and the blocking cell is outlined in red. Nothing is overwritten.

Spills resize themselves. In the demo model, scrub `months` and the table grows and shrinks, as does everything that reads it.

```example
5 range              ⇒ [0, 1, 2, 3, 4]
A1:A5 B1:B5 couple transpose  ⇒ ! units
```

(That last one fails because a table must have a single unit: A is plain numbers and B is metres.)

## Tips

- Leave room below a spilling formula. The inspector shows a spill's size, and selecting a spilled cell links to its source.
- To refer to the whole spill from elsewhere, refer to its source cell — the reference keeps working as the spill grows.
