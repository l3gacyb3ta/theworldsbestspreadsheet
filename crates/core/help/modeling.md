# Modeling: inputs, scrubbing and tracing

A model is a few inputs and a lot of consequences. The sheet is built to make "what if?" instant.

## Scrubbing

Hold **Alt** and drag a number left or right — in a cell, or any number in the formula bar — and every dependent cell updates as you move. Hold **Shift** too for ten times faster. The step size is the number's last decimal place: `4.0` moves by 0.1, `4.00` by 0.01. The whole drag is one undo step.

## Named inputs

Name the cells that are assumptions and tick **input** in the inspector. Inputs are tinted yellow and listed in the Inputs panel, where you can drag each value. A model reads better with names: `=1 growth + months range ^ start *`.

## Trace

With **Trace** on (the toolbar toggle), selecting a cell highlights what it reads (blue) and what reads it (orange). The inspector lists both; click one to jump there.

## Step through a cell

The inspector's **Step through** shows a cell's program token by token with the stack after each one. It's the quickest way to understand someone else's formula, or to find where a unit goes wrong.

## Charts as instruments

A chart whose points come straight from input cells is a control: drag a point and the input changes. Drag a point that's computed and the sheet goal-seeks: it solves for the input value that puts the point where you let go — the first named input upstream, or click the point to switch to the next. If nothing reaches it, nothing changes and the point says why. See [[#charts]].

## Exchange rates and other factors

Unit definitions can reference cells, so a conversion factor is an input like any other. See [[#defining-units]].

## The playground

The **Playground** in help runs any program against your workbook without changing it — handy for checking a calculation before you commit it to a cell.
