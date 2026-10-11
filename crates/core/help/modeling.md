# Modeling: inputs, scrubbing and tracing

A model is a few inputs and a lot of consequences. The sheet is built to make "what if?" instant.

## Scrubbing

Hold **Alt** and drag a number left or right — in a cell, or any number in the formula bar — and every dependent cell updates as you move. Hold **Shift** too for ten times faster. The step size is the number's last decimal place: `4.0` moves by 0.1, `4.00` by 0.01. The whole drag is one undo step.

Scrubbing an input that has a range stops at its ends, like a slider: the number stays at the end and the pointer says which (`min 0 [1/s]`).

## Named inputs

Name the cells that are assumptions and tick **input** in the inspector. Inputs are tinted yellow and listed in the Inputs panel, where you can drag each value. A model reads better with names: `=1 growth + months range ^ start *`.

## Input ranges

**An input's range limits scrubbing, chart dragging and goal-seek; typing a value outside it is an error on that cell.**

Set it in the inspector, next to **input**: a min, a max, or both, each a number in the input's unit — `0 [1/s]` for a damping rate, `0 [%]` for a growth rate. Another unit of the same dimension works too (`6 [1/min]`), and a plain number on a percentage is a fraction (`0.5` is 50 %). An end can also be a reference or a name (`B7`, `max_damping`), or any formula that gives one number: the range then follows that cell, and the input is checked again whenever it changes. If the cell is empty or has an error, the range can't be used, and the input says so. A bound in the wrong dimension isn't set: the inspector says why, and nothing changes. Leave a field empty for no bound on that side.

- Scrubbing, in a cell, the formula bar or the Inputs panel, stops at the ends.
- Dragging a chart point bound to the input stops there too. A scatter point's x and y each stop at their own input's range, and the pointer says which axis stopped (`x at max 2.2`).
- Goal-seek only tries values inside the range; when the answer is outside, "out of reach" names the end it ran into (`out of reach within damping ≥ 0 [1/s]: …`).
- A value typed outside the range is kept as typed, never clamped, and the cell shows the error (`damping must be ≥ 0 [1/s]`); cells that read it show `#upstream` until you fix it.
- An input with both ends shows as a slider in the Inputs panel; dragging it stops at the ends, and the number beside it still takes any typed value.

A range belongs to the input: renaming keeps it, and unticking **input** removes it (undo brings both back). Changing it is one undo step.

## Trace

With **Trace** on (the toolbar toggle), selecting a cell highlights what it reads (blue) and what reads it (orange). The inspector lists both; click one to jump there.

## Step through a cell

The inspector's **Step through** shows a cell's program token by token with the stack after each one. It's the quickest way to understand someone else's formula, or to find where a unit goes wrong.

## Charts as instruments

A chart whose points come straight from input cells is a control: drag a point and the input changes. Drag a point that's computed and the sheet goal-seeks: it solves for the input value that puts the point where you let go — the first named input upstream, or click the point to switch to the next. If nothing reaches it, nothing changes and the point says why. Both stay inside the input's range (see Input ranges above). See [[#charts]].

## Exchange rates and other factors

Unit definitions can reference cells, so a conversion factor is an input like any other. See [[#defining-units]].

## The playground

The **Playground** in help runs any program against your workbook without changing it — handy for checking a calculation before you commit it to a cell.
