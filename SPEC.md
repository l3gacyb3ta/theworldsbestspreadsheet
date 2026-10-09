# spreadsheet spec (v0)

A spreadsheet for modeling. Stack-based array language in cells, a real dependency graph, first-class units, and direct manipulation everywhere. No backwards compatibility with Excel/Lotus syntax.

Implementation language: Rust. Rendering stack, UI toolkit, and crate choices are up to the implementer.

## guiding principle

The sheet should be predictable, not clever. It never silently guesses, rewrites, or simplifies on the user's behalf. When it can't do something, it says so on the cell where the problem is. Every behavior should be explainable in one sentence.

## 1. data model

- Every cell has a stable internal id. Rows and columns are ordered lists of ids.
- A1 notation is only a display/input format. Formulas are stored with references resolved to ids and rendered back to A1 for display.
- References are stored as **relative offsets** from the formula's own cell (R1C1-relative) unless marked absolute (`$A$1` style or a named input). This is what makes fill-drag and formula extension work.
- Inserting, deleting, moving, or sorting rows/columns must never break or rewrite references.
- Named cells / named inputs are first class (`growth`, `rates.eur`), usable anywhere a reference is.
- Design the document model so it can later live in Automerge (row/column order as list CRDTs, cells as map entries keyed by id). Collaboration is not v0, but nothing in the model should assume positional addressing or a single writer.

## 2. language

Concatenative and array-oriented, closest in spirit to Uiua: Forth's stack + APL's rank polymorphism.

- A cell's contents are either a literal or a program. A program is evaluated on an empty stack.
- **The cell's value is the top of the stack.** If anything else is left on the stack, the cell shows an inline error ("2 values left on stack") rather than a value.
- Values are arrays (scalars are rank 0). Every value carries a unit (see section 4).
- A range reference pushes **one array**, not N scalars. `A1:A10 /+` sums. `B1:B10 C1:C10 *` multiplies elementwise.
- Arithmetic words are rank-polymorphic and broadcast scalars against arrays.
- Reductions/scans use a modifier: `/+` reduce, `\+` scan.
- Minimal word set for v0: `+ - * / ^ neg abs sqrt exp log min max`, `dup drop swap over rot`, `/` reduce, `\` scan, `len`, `range` (iota), `if`, comparisons, `sum mean` as convenience words.
- User-defined words: a word definition is a cell (`: npv  rate cashflows ... ;`) and is part of the dependency graph, so editing a word recalculates its users.
- Syntax errors and type/unit errors are shown inline on the cell, pointing at the offending token where possible.

### spill

If a cell's value is an array of rank 1 or 2, it spills down/right into neighboring cells. Spilled cells are read-only and visibly marked as spilled. If a spill would overwrite a non-empty cell, the source cell shows a `#spill blocked` error and highlights the blocking cell. A reference to the source cell refers to the whole spilled array.

## 3. evaluation

- Explicit dependency graph from day one, built from the resolved references in each program.
- Incremental recalculation: edits mark downstream cells dirty; only dirty cells recompute, in topological order.
- Cycle detection reports the actual cycle (list of cells) and highlights it in the grid.
- Recalc must be fast enough that scrubbing a number (section 6) updates dependents at interactive framerates on sheets of a few thousand cells.

## 4. units

### representation

A value is `(magnitudes, dimension, display_unit)`:

- **magnitudes**: array of f64 stored in canonical base units.
- **dimension**: sparse map from base dimension to **rational** exponent. `m/s^2` is `{length: 1, time: -2}`. Rational, because `sqrt` must work on variances etc.
- **display_unit**: the unit the value is rendered in. Purely presentational.

All arithmetic operates on canonical magnitudes. Conversion only happens at input (literal to canonical) and display (canonical to display unit). No conversion logic lives inside arithmetic.

Arrays are homogeneous: one dimension and display unit per array.

### syntax

- A bracketed unit literal is a word that multiplies TOS by that unit: `5 [m/s]`, `9.81 [m/s^2] 3 [s] *`.
- Inside brackets is a small infix unit grammar: `*`, `/`, `^n` (rational allowed, `^1/2`), parentheses.
- `to[km/h]` changes only the display unit. It errors if dimensions don't match.
- A bare number is dimensionless.

### dimension rules

Every word declares a dimension rule alongside its stack effect:

- `+ -`, comparisons, `min max`: dimensions must be equal; result keeps the left operand's display unit.
- `* /`: exponent vectors add / subtract.
- `^ n`: exponents multiply by n (n must be a dimensionless literal or constant for this to be static).
- `sqrt`: exponents halve.
- `exp log sin cos` etc.: argument must be dimensionless.

Because dimensions don't depend on magnitudes, dimensions are propagated through the dependency graph statically. A unit mismatch is reported **on the cell where it occurs**, not as a wrong number downstream.

### user-extensible units

Unit definitions live in the sheet as declarations (a dedicated units sheet by convention, but any cell works):

```
dim currency
base [USD] currency
[EUR] = rates!B2 [USD]
[mi] = 1609.344 [m]
dim widgets
```

- `dim name` declares a new base dimension. This is how users get `widgets`, `people`, `currency`, etc. Adding widgets to USD is a unit error; `[USD/widget]` just works.
- `base [unit] dim` declares the canonical unit of a dimension.
- `[unit] = expr` defines a derived unit. The expression may reference cells, so conversion factors (exchange rates, etc.) are ordinary inputs in the dependency graph and can be scrubbed. Dimensions stay static; only magnitudes depend on values.
- Ship a built-in SI + common imperial library as a default units sheet the user can read and edit.

### affine and logarithmic units

- °C and °F are allowed for input and display only. Arithmetic on absolute temperatures goes through K. Adding two absolute temperatures is an error. Provide delta units (`[Δ°C]`) for differences.
- dB and other log units: out of scope for v0. Provide explicit functions later.

### display simplification

- Keep the composed unit produced by the operations (e.g. `kg*m/s^2`), cancelling identical factors only.
- Never auto-simplify to derived units (`N`, `J`). The user asks with `to[N]`.

## 5. grid UX (must-haves)

These are the things that make Excel good and are non-negotiable:

- Fill-drag: drag the corner handle to fill a value, continue a numeric/date series, or extend a formula with relative references adjusted.
- Formula extension: typing in a column adjacent to a filled formula column offers to extend.
- Resize columns and rows by dragging; double-click to autofit.
- Click-to-reference: while editing a formula, clicking a cell (or drag-selecting a range) inserts its reference token at the cursor. Click order equals stack order, which is a nice property of postfix.
- Referenced cells are color-highlighted while editing a formula.
- Keyboard navigation, copy/paste of values and formulas (formulas paste with relative refs adjusted), undo/redo.
- Formula bar shows the program; cell shows the value with its display unit.

## 6. modeling features

- **Scrubbing**: alt-drag (or similar) on any numeric literal, in a cell or in the formula bar, changes it continuously and all dependents update live. This is the core interaction for modeling and should feel immediate.
- **Named inputs**: any cell can be named and marked as an input; inputs are listed in a side panel.
- **Trace**: select a cell to see its precedents and dependents highlighted.

## 7. charts

- A chart is a **value**: a program returns a chart, and it spills into a rectangular region. It lives in the dependency graph like any other cell.
- Charts are built by composing words, not via a wizard: `xs ys line`, `xs ys scatter`, `cats vals bar`, plus composition words for layering, axes, labels. Axes use the values' units automatically.
- **Bidirectional editing, v0 scope**: dragging a point that is bound directly to a literal input cell writes that cell. Points backed by derived values are not draggable in v0 but show, on hover, which input they would solve for.
- **Later**: dragging a derived point picks one upstream input and goal-seeks it with a 1-D root finder ("what growth rate makes this hit 1M").

## 8. out of scope for v0

Collaboration/sync, structured tables (use Grist/Airtable), Excel import/export, log units, auto unit simplification, multi-variable solving.

## suggested build order

1. Document model with stable ids, relative refs, A1 rendering.
2. Language: parser, stack evaluator, arrays, errors.
3. Dependency graph + incremental recalc + cycle detection.
4. Grid UI with editing, click-to-reference, fill-drag, resize.
5. Units: representation, literals, dimension rules, static propagation, user definitions.
6. Spill.
7. Scrubbing.
8. Charts, then bidi for literal-bound points.
