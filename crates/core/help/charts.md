# Charts

A chart is a value. A program builds it from words, and it spills over a block of cells like any array. It's part of the dependency graph, so it redraws when its data changes.

```example
A1:A5 B1:B5 line                    ⇒ chart · line (5 points) · 6×14 cells
A1:A5 B1:B5 scatter                 ⇒ chart · scatter (5 points) · 6×14 cells
A1:A5 B1:B5 path                    ⇒ chart · path (5 points) · 6×14 cells
C1:C3 D1:D3 bar                     ⇒ chart · bar (3 points) · 6×14 cells
```

`line` draws y as a function of x. `path` joins its points in the order given instead — a traced shape, a trajectory, a phase plot like position against velocity — and marks every point.

## Composing

| word | does |
|---|---|
| [[layer]] | draws two charts on the same axes |
| [[title]] | `chart "text" title` |
| [[xlabel]], [[ylabel]] | axis labels (default: the values' units) |
| [[size]] | `chart cols rows size` — how many cells it covers |

```example
A1:A5 A1:A5 line A1:A5 A1:A5 sq line layer "n and n²" title   ⇒ chart "n and n²" · line (5 points) + line (5 points) · 6×14 cells
```

Layered charts must agree on units: you can't draw metres and seconds against the same y axis.

## Units on axes

Axes are labelled with the values' display units automatically. Dates on the x axis are shown as dates.

## Dragging points

If a point's y value comes straight from a number cell (through a range, a reference, or words that only rearrange values — `join`, `couple`, `rev`, `first`, `last`, `pick`, `transpose`, `reshape`, `to[…]` — with nothing computed in between, in this cell or in any cells it reads), the point is drawn hollow and you can drag it up and down — that writes the cell, keeping its unit and its decimals: dragging `180 [widget]` writes whole numbers, `4.0` moves in steps of 0.1. Hold Shift for finer steps. Bars with a dark cap are draggable the same way.

**Scatter and path points move in 2D; line points move only up and down.** On a `scatter` or `path`, the pointer's x writes the x value's cell and its y writes the y value's cell, each the same way (own decimals and unit; dates move in whole days, °C stays °C). A diagonal drag that writes two cells is one undo step. The chart's axes stay put until you let go.

**Alt locks the drag to one axis**, the one you've moved along most since you pressed, as in a drawing program: the other cell stays exactly as it was (and isn't part of the undo step). Release Alt and the point moves freely again; press it with Alt already held and it's locked from the start (a press on a chart point drags it, it doesn't scrub the cell beneath). Alt and Shift together give fine steps along one axis. The tooltip says `locked to x — release Alt to move freely` while it's on.

- **Only one axis from a cell**: if only x (or only y) comes straight from a number cell, only that axis moves; the cursor and the tooltip say which (`drag sideways to edit A3 — y is computed in C3, so only x moves`).
- **A history's start**: the first value of an [[iterate]] or [[rk4]] history is the start state, unchanged, so it counts as coming straight from the start's cell. On a phase plot (`=xs vs path` from an rk4 spring), the first point drags in 2D and writes `x0` and `v0` directly; every later point is computed.
- **One axis computed**: if x comes from a cell and y is computed (or the other way round), only the cell's axis moves.
- **Both computed**: the point solves for two inputs at once (below).

### Goal-seek: dragging a computed point

A point computed in a cell (`B17`, part of the revenue column `B10` spills) can be dragged too: that **goal-seeks** — it changes one number cell upstream so the point lands where you let go. "What growth rate makes month 8 hit 200k?" is one drag.

- **Which input**: the hover tooltip says which cell a drag would solve for, and lists the others. It starts with the first named input upstream (see [[#modeling]]), then other number cells in sheet order. **Click** the point (without dragging) to switch to the next one; the choice sticks for every point of that cell.
- **While dragging**, a dashed line marks the value under the pointer, and the tooltip shows the solved input (`solved: growth (B4) = 8.1 [%]`). The chart keeps its y range until you let go so the point stays under the pointer.
- **What's written**: the solved number, rounded to the input's own decimals — `4.0 [%]` gets one decimal, `120000 [USD]` whole dollars, a date whole days — keeping its unit. Hold **Shift** for two more decimals. The point lands within that rounding of the target, not exactly on it. The whole drag is one undo step.
- **No answer, no change**: if no value of the input puts the point there — out of reach, the value jumps over it, or the sheet errors on the way — the input stays as it was and the reason is shown next to the point (and in the status bar), e.g. `out of reach: B10 stays at 120,000 USD for growth from -4,092 % to 4,100 %`. The sheet never settles for the closest value it found.
- **Live or on release**: each move solves at once if a solve is quick (under 50 ms); on a sheet where it isn't, the drag only shows the dashed line and solves when you let go — the tooltip says so.

### Solving for two inputs: dragging a computed point in 2D

A scatter or path point whose x and y are both computed (a phase plot's spiral) drags in 2D: it solves for **two** inputs at once so the point lands under the pointer. "Which stiffness and damping put the first bounce here?" is one drag.

- **Which inputs**: the hover tooltip names the pair, starting with the first two named inputs upstream of the point's x or y. **Click** the point to move on to the next pair.
- **Alt** locks to one axis, the one moved along most, and goal-seeks the pair's first input alone, as on a line; the second stays as it was.
- **How**: Newton's method on the 2×2 matrix of exact derivatives (see [[#sensitivities]]), one sensitivity pass per input per step, with steps shortened where they overshoot and kept inside both inputs' ranges. It starts from the inputs' current values, so when several pairs of values would put the point there, you get the one continuously connected to where you started.
- **No answer, no change**: two-input solves fail more often than one. When they do, both inputs stay as they were and the point says why: `can't reach: spring and mass don't move this point independently here` (the two inputs only move it along one line), `damping doesn't move this point here`, an input at the end of its range, or no convergence. It never settles for the closest point.
- Both inputs are written rounded to their own decimals (Shift: two more), in one undo step. Live or on release, as above.

How the 1D search works: starting from the input's current value it tries values further and further away on both sides — steps of 1/16 of the value, doubling, up to 1024× it (from zero: up to a million of its last decimal place; dates: ±65,536 days) — until two neighbours straddle the target, then narrows in on it (Brent's method; whole numbers for inputs without decimals and for dates). It changes only the one input. A point the chart's own program computes (`1 2 3 join …`, or `B1:B5 2 *`) has no cell to solve through and isn't draggable.

## Room to spill

A chart needs its whole block of cells to be empty. If something's in the way you'll see `#spill blocked` and the blocker is outlined.
