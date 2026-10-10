# Editing the grid

## Typing

Select a cell and start typing to replace it, or press F2 / double-click to edit what's there. Enter confirms and moves down, Tab moves right, Escape cancels. The formula bar edits the same text — click it to edit there.

While you edit a program:

- **Click a cell** to insert its reference at the cursor. **Drag** across cells to insert a range. Click another sheet's tab first to reference across sheets.
- Each reference gets its own colour, in the text and on the grid.
- The **hint strip** under the formula bar explains the word at the cursor, shows the stack at that point, and offers completions for words, names and units. Click a completion to insert it, or use the keyboard.
- **↓ / ↑** highlight a completion, and the strip shows what it does. Nothing is highlighted until you press ↓; ↑ from the first one goes back to what you typed.
- **Tab** or **Enter** inserts the highlighted completion and keeps editing. With nothing highlighted they confirm the cell as usual.
- **Escape** closes the completion list (until you type again); a second Escape cancels the edit.

## Fill

Drag the small square at the bottom-right of the selection.

- One cell is copied.
- Two or more numbers (or dates) that step evenly continue the series: 1, 2 → 3, 4, 5…
- Anything else repeats the pattern.
- Programs are copied with relative references moved, like copy and paste.

⌘D fills the selection down from its top row.

## Extending a formula

Type a program next to a filled column and the sheet offers to extend it down to the end of that column. Click the offer or press ⌘E.

## Copy and paste

⌘C / ⌘X / ⌘V. Programs pasted inside the sheet move their relative references. Text pasted from elsewhere is split on tabs and newlines and typed into the cells.

## Rows and columns

Drag a header border to resize; double-click it to fit the contents. Right-click for insert, delete, sort and fill. Sorting moves whole rows, and references follow the cells they point at.

## Sheets

The tabs at the bottom are the sheets; **+** adds one. Double-click a tab to rename it, right-click it to duplicate, move or delete it, or drag it to a new place. See [[#references]] for what happens to references.

## Undo

⌘Z undoes, ⇧⌘Z redoes. A whole scrub or chart drag is one step.
