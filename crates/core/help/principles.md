# How it works: design rules

The sheet is meant to be predictable, not clever. It never silently guesses, rewrites or simplifies, and when it can't do something it says so on the cell where the problem is. Each behaviour can be explained in one sentence:

- **A cell's kind** is decided by its first characters ([[#cells]]).
- **A program's value** is the one value left on the stack ([[#stack]]).
- **A range** is one array; **empty cells** are errors, not zeros ([[#arrays]]).
- **A reference** points at a cell, not a position, so structure edits never break it ([[#references]]).
- **Relative references** move only when you copy or fill.
- **Units** are checked from the programs alone, before anything is computed, and again on every operation; mismatches are reported where they occur ([[#units]]).
- **Display units** compose and cancel identical factors, and are never renamed for you. The one exception: a dimensionless unit like % is absorbed into a dimensioned quantity.
- **Spills** never overwrite anything ([[#spill]]).
- **Fill** copies one value, continues an evenly-stepped series, and otherwise repeats the pattern ([[#editing]]).
- **Definitions** (words, units, dimensions) can live anywhere; the first one wins ([[#words]]).

## Under the hood

- Each row, column and cell has a stable id. A1 notation is only how references are shown and typed.
- Every cell's dependencies are known exactly, so an edit recomputes only what depends on it, in order. A sheet of a few thousand cells recalculates in a few milliseconds, which is what makes scrubbing feel live.
- Dimensions travel through the same dependency graph statically: each cell's dimension follows from its program and its inputs' dimensions. Changing a number never changes a dimension, so scrubbing skips that pass.
- The document is lists of row and column ids plus a map of cells, designed so it can later be shared and edited by several people at once.
