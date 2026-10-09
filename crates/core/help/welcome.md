# Welcome

This is a spreadsheet for building models. It looks like a grid, but three things work differently from Excel:

- **Programs, not formulas.** A cell that starts with `=` holds a short postfix program: values go on a stack, words act on them. `=2 3 +` is 5. A whole range is one value, so `=A1:A100 sum` adds a column.
- **Units are real.** `=100 [km] 2 [h] /` is `50 km/h`. Adding metres to seconds is an error on the cell where it happens, not a wrong number three sheets later.
- **Everything is direct.** Alt-drag any number to scrub it and watch the model move. Click cells while typing to insert references. Drag chart points to change the inputs behind them.

## Your first minute

- Click an empty cell and type `=2 3 +`, then press Enter.
- Type `=` in another cell, click the cell you just made, type `10 *` and press Enter. You've written `=A1 10 *` without typing a reference.
- In a third cell type `5 [km]`. Hold Alt and drag on it left and right — the number changes, and so does anything that uses it.
- Press F1 any time. With a cell selected it opens help for what's in that cell.

## Where to go next

- [[#cells]] — the five kinds of cell, and how the sheet decides.
- [[#stack]] — how postfix programs work.
- [[#arrays]] and [[#spill]] — ranges, broadcasting, and results that fill several cells.
- [[#units]] — quantities, conversions and your own units.
- [[#modeling]] — named inputs, scrubbing and tracing.
- [[#errors]] — what every error means and how to fix it.

## How help works

Every example in help is live: it's evaluated against a small sample sheet, and the result is shown next to it. Click **Try** to open an example in the playground and change it. While you edit a cell, the strip under the formula bar explains the word at the cursor and shows the stack. Hover over any token in the formula bar for its documentation, and use **Step through** in the inspector to watch a cell compute token by token.

The sample sheet the examples use:

{{sample}}
