# Sensitivities

How much does the payback period move per dollar of panel price? The sheet can answer exactly, not by scrubbing and eyeballing: every value can carry its derivative with respect to one input along with it, through every word, user word, `iterate` and `rk4` step.

## Derivatives have units

A sensitivity is the output's unit per the input's: years of payback per USD/kW of panel price comes out in `yr*kW/USD`, metres of position per N·s/m of damping in `m*s/kg`. It displays like any other quantity, and a percentage input gives a change per percentage point:

```
=100 [USD] growth *        with growth = 4 %: 1 USD/%
```

## Where you see them

- **Dragging a computed chart point.** While the sheet goal-seeks the input, the pointer shows the local sensitivity in the input's own unit: `breakeven: -0.8 yr per +100 USD/kW`.
- **The inspector.** Select a cell and open **Sensitivities** to list how it moves with each named input upstream of it, inputs read inside words included.
- **[[sensitivity]].** `B10 sensitivity` is a tornado chart: one bar per named input, the change in B10 across that input's range (or across ±10% of its value when it has none), largest first.

## Faster goal-seek

Goal-seek steps straight towards the answer with the exact slope (Newton's method), usually in a few steps. When a step can't be taken (the value is flat there, the step would leave the input's range, or it isn't converging), it falls back to the bracketing search it always used, so the answer, or the reason there is none, is the same. Whole-number inputs and dates only bracket.

## Sensitivities are local

They're the slope where the model is now. Through `min`, `max`, `if` and comparisons they follow the branch currently chosen, so they can jump where the choice flips: the battery's sensitivity to its capacity changes suddenly when it starts filling up. `floor`, `ceil`, `round` and comparisons count as flat. A tornado bar is that slope times the input's swing, so a model with kinks can swing more or less than the bar says.

## Limits

- One pass per input: each costs about as much as recalculating the cells between that input and the output.
- An input that reaches the output through a unit definition (an exchange rate defining `[EUR]`) has no sensitivity yet; the inspector and the chart say so.
- A date or temperature input without a range is left out of the tornado chart: ±10% of a date means nothing.
