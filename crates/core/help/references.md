# References, names and inputs

## Writing references

| form | meaning |
|---|---|
| `A1` | a cell |
| `A1:B5` | a range: one array |
| `$A$1`, `$A1`, `A$1` | absolute column and/or row when copying or filling |
| `rates!B2`, `'my sheet'!B2` | a cell on another sheet |
| `growth` | a named cell |

The easiest way to write a reference is not to: while editing a program, click a cell to insert it, or drag across cells to insert a range. References are colour-coded in the editor and on the grid.

## References follow their cells

Inserting, deleting, moving or sorting rows and columns never breaks or rewrites a reference. Internally a reference points at the cell itself, not at "two rows up". If you insert a row inside a range, the range grows. If you delete the last row of a range, the range shrinks. Only a reference to a deleted cell becomes `#ref!` — and undo brings it back.

"Relative" matters only when you copy or fill: then a relative reference moves with the formula, and an `$`-absolute one stays put.

## Names

Name any cell in the inspector. Names may contain letters, digits, `_` and `.`, like `rates.eur`. Use them anywhere a reference goes:

```example
growth           ⇒ 0.05
price 2 *        ⇒ 2,000 USD
1 growth + A1:A3 ^    ⇒ [1.05, 1.1025, 1.157625]
```

## Inputs

Tick **input** next to a name and the cell is tinted yellow and listed in the Inputs panel. There you can drag each value directly. Inputs are the knobs of your model: rates, sizes, assumptions. See [[#modeling]].

## Empty cells

A reference to an empty cell is an error, not zero. If you mean zero, type 0. This makes a missing input visible instead of silently wrong.

## Cycles

If cells depend on each other in a loop, every cell in the loop shows `#cycle`, the loop is listed in the message, and the cells are highlighted red.
