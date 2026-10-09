# Charts

A chart is a value. A program builds it from words, and it spills over a block of cells like any array. It's part of the dependency graph, so it redraws when its data changes.

```example
A1:A5 B1:B5 line                    ⇒ chart · line (5 points) · 6×14 cells
A1:A5 B1:B5 scatter                 ⇒ chart · scatter (5 points) · 6×14 cells
C1:C3 D1:D3 bar                     ⇒ chart · bar (3 points) · 6×14 cells
```

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

If a point's y value comes straight from a number cell (through a range or reference, with nothing computed in between), the point is drawn hollow and you can drag it up and down — that writes the cell, keeping its unit. Bars with a dark cap are draggable the same way.

Points computed from other cells aren't draggable yet. Hover one to see which inputs it depends on — those are the cells a future goal-seek would adjust.

## Room to spill

A chart needs its whole block of cells to be empty. If something's in the way you'll see `#spill blocked` and the blocker is outlined.
