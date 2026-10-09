# the world's best spreadsheet

A spreadsheet for modeling, built to [SPEC.md](SPEC.md): a stack-based array
language in cells, a real dependency graph, first-class units, and direct
manipulation (scrubbing, click-to-reference, draggable chart points).

```bash
cargo run -p wbs --release -- my-model.wbs.json
```

The first launch (when the file doesn't exist) opens a demo model; ⌘S saves.

## Layout

| crate | what |
|---|---|
| `crates/core` (`wbs-core`) | document model, lexer/compiler, interpreter, units, dependency graph, fill/copy/sort ops. No UI. |
| `crates/app` (`wbs`) | the egui desktop app: grid, formula bar, inspector, charts. |

```bash
cargo test            # core unit + engine tests, and headless UI tests
```

The UI tests drive the real app with synthetic input through `egui_kittest` and
write rendered snapshots to `target/ui-shots/`.

## Cells, in one sentence each

- `=` starts a program. It runs on an empty stack; the cell's value is the one value left.
- `:` starts a word definition: `: sq dup * ;`, or with locals `: npv { rate cfs } … ;`.
- `dim widgets`, `base [widget] widgets`, `[mi] = 1609.344 [m]` declare units (any cell, any sheet).
- A number, optionally with a unit (`5 [m/s]`), or an ISO date is a literal.
- `'` forces text; anything else is text.

## Language

Postfix, whitespace-separated, rank-polymorphic. `A1:A10 /+` sums (a range is
one array), `B1:B9 C1:C9 *` multiplies elementwise, scalars broadcast. `/op`
reduces and `\op` scans along the leading axis. Rank-1 and rank-2 results
spill. A reference to a spill's source is the whole array; a reference into a
spill is that element. The full word list is in the app's inspector under
"Language" (and `crates/core/src/parse.rs`).

Units: `5 [km] 300 [m] +` → `5.3 km`; `to[mph]` changes only the display unit;
`[N]`, `[J]`… are never applied automatically. Dimensions are rational vectors,
so `4 [m^2] sqrt` is `2 m`. `°C`/`°F`/dates are absolute: you can add a `Δ°C` or
`[day]` to one, subtract two, but not add two. The SI/imperial/currency library
is an ordinary, editable `units` sheet; exchange rates are inputs you can scrub.

## Design notes / decisions

- **References are stored as ids, not offsets.** Each reference stores the target
  row/column *ids* plus `$` flags; relative-ness is applied when copying or
  filling (the positional delta is re-resolved to ids at the destination). This
  is the only way inserts, deletes, moves and sorts can never rewrite or break
  a reference.
- **Rows/columns are tombstoned lists** (like a list CRDT): deleting the end row of
  a range shrinks the range instead of breaking it, and undo revives the ids.
- **Sorting** moves rows by id, so single references follow their cells; ranges
  over the sorted block keep covering the same block.
- **Dimensions are checked during evaluation**, not by a separate static pass. Since
  no dimension depends on a magnitude (`^` requires a fixed fractional exponent),
  the error lands on the same cell; downstream cells show `#upstream`.
- **Dimensionless display units (`%`) are absorbed** by a quantity with a
  dimension: `4 [%] 100 [USD] *` shows `4 USD`, not `400 USD*%`. Identical-factor
  cancellation is otherwise the only simplification.
- **Empty cells are not zero**: referencing one is an error naming the cell.
- **Fill**: one cell copies; two or more numbers in an arithmetic progression
  continue the series; otherwise the pattern repeats. Formulas move relative refs.
- **Charts**: `xs ys line|scatter`, `cats vals bar`, `layer`, `title`,
  `xlabel`, `ylabel`, `cols rows size`. Points read directly from literal cells
  are draggable and write the cell; derived points show which inputs they'd
  solve for (goal-seek is future work).

## Help system

Press **F1** (or the toolbar's Help) for help about the selected cell — or, while
editing, about the word at the cursor. ⌘/ searches.

- **Guides** (`crates/core/help/*.md`): a small markdown dialect with
  `[[word]]` / `[[#topic]]` links, tables, and ```` ```example ```` blocks whose
  lines are `program ⇒ documented result`.
- **Reference** (`crates/core/src/help.rs`, `WORDS`): stack effect, unit rule,
  details, examples and see-also for every builtin and syntax form.
- **Live pages**: units and user words are listed from the open workbook. A
  comment after a word's name documents it: `: sq ( x -- x² ) dup * ;`.
- **Context help**: a hint strip under the formula bar shows the doc for the token
  at the cursor, the stack at that point, and completions. Hovering a token in the
  formula bar shows its doc or value. The inspector explains errors in plain words
  and can step through any program token by token.
- **Playground**: run programs against the sample sheet or your workbook without
  changing anything.

Every example in the guides and reference is evaluated against a sample sheet
by `cargo test`. A wrong documented result, a broken link, an undocumented
builtin, or an error message without an explanation fails the build.

## Keys

Arrows/⇧arrows, Enter/Tab, F2 or double-click to edit, Delete clears, ⌘C/⌘X/⌘V,
⌘Z/⇧⌘Z, ⌘D fill down, ⌘E accept "extend formula", ⌘S save. Alt-drag a number
(in a cell or the formula bar) to scrub it, ⇧ for ×10. Right-click for
insert/delete rows/columns and sort.

## License

[Peer Production License](LICENSE): a copyfarleft license (John Magyar and
Dmytri Kleiner, derived from CC BY-NC-SA). You may use, share and adapt this
work non-commercially. Commercial use is allowed only for worker-owned
businesses or collectives that distribute all gains among their worker-owners.
