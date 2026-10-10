# Cells: the five kinds

A cell's first characters decide what it is. There's no guessing beyond this table, and the formula bar shows the kind next to the address.

| starts with | kind |
|---|---|
| `=` | a **program**: postfix code; the cell shows its result |
| `:` | a **word definition**: `: sq dup * ;` |
| `dim`, `base`, `[unit] =` | a **unit declaration** |
| a number or ISO date | a **number**, optionally with a unit: `5`, `2.5 [m/s]`, `2026-10-08` |
| `'` | forced **text** (the quote isn't shown) |
| anything else | **text** |

## Numbers

A number cell is a literal: `120000 [USD]`, `4.0 [%]`, `24`. These are the cells you scrub with Alt-drag, list as inputs, and edit by dragging chart points. Keep the decimals you want to step by: scrubbing `4.0` moves in steps of 0.1, `4.00` in steps of 0.01.

A number wider than its column runs into empty cells on its left. If there's no room it's rounded to fit: fewer decimals first, then scientific notation (`1.23e8`), but never to fewer than 3 significant digits, so a shortened number stays close to the real one. If even that doesn't fit it shows `###`, never a cut-off number. Hover it, or select it and look in the inspector, to see the whole value, and double-click the column's header border to widen it.

## Programs

A program runs on an empty stack, and the cell's value is the one value left at the end. See [[#stack]].

```example
=2 3 +           ⇒ 5
=A1:A5 sum       ⇒ 15
=B1 2 *          ⇒ 20 m
```

## Text

Anything that isn't one of the other kinds is text, so labels just work. If you want text that looks like a number or a declaration — say a part number `00123` or the words `dim sum` — start it with `'`.

## Definitions

Word definitions ([[:]]) and unit declarations ([[dim]], [[base]], [[[u] = …]]) can live in any cell on any sheet. They show their own text, and every cell that uses them updates when you edit them.
