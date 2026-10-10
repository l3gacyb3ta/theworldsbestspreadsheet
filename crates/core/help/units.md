# Units

Every number carries a unit, and the sheet checks them. Put a unit in square brackets after a number: it multiplies the number by that unit.

```example
5 [km]                   ⇒ 5 km
9.81 [m/s^2] 3 [s] *     ⇒ 29.43 m/s
100 [km] 2 [h] /         ⇒ 50 km/h
```

Inside the brackets: unit names, `*`, `/`, powers (`^2`, `^-1`, fractions like `^1/2`) and parentheses: `[J/(kg*K)]`.

## Adding needs matching dimensions

`+`, `-`, comparisons, `min` and `max` need both sides to measure the same thing. The result is shown in the left side's unit.

```example
5 [km] 300 [m] +         ⇒ 5.3 km
300 [m] 5 [km] +         ⇒ 5,300 m
1 [m] 1 [s] +            ⇒ ! needs matching units
```

The error appears on the cell where the mismatch happens. Cells that depend on it show `#upstream` and link back to it.

## Checked before anything is computed

A dimension never depends on a value, so the sheet works out every cell's dimension from its program and the dimensions of the cells it reads — without computing a single number. A mismatch shows up the moment you type it, even if an input is still empty or has its own error: `=A1 1 [m] 1 [s] + +` reports the `+` straight away, not that `A1` is empty. The inspector shows a cell's dimension even when it has no value yet, and while editing, the hint strip shows the dimensions on the stack where evaluation can't go.

What it can't know ahead is left to evaluation: an empty cell could hold any unit, and an exponent read from a cell (`A1 B1 ^`) is only known once computed — write it in the program (`A1 2 ^`) and it is.

## Multiplying composes units

`*` and `/` combine units, cancelling identical ones. Nothing is renamed behind your back: `kg*m/s^2` stays `kg*m/s^2` until you ask for newtons.

```example
2 [kg] 3 [m/s^2] *          ⇒ 6 kg*m/s^2
2 [kg] 3 [m/s^2] * to[N]    ⇒ 6 N
3 [m] 3 [m] *               ⇒ 9 m^2
```

One exception keeps percentages sensible: a dimensionless unit such as `%` is absorbed when it multiplies something with a dimension.

```example
4 [%] 200 [USD] *        ⇒ 8 USD
5 [%] 2 *                ⇒ 10 %
1 growth +               ⇒ 1.05
```

## Showing a value in another unit

[[to[unit]]] changes only how a value is displayed; it errors if the dimensions differ.

```example
5 [km] to[m]             ⇒ 5,000 m
100 [km/h] to[mph]       ⇒ 62.137119 mph
5 [km] to[s]             ⇒ ! dimensions differ
```

## Where units come from

Every unit is declared in a cell — the built-in library lives on the **units** sheet, where you can read and edit it. Add your own units and dimensions anywhere: see [[#defining-units]]. For temperatures and dates, which work a little differently, see [[#absolute]].

## Functions that need plain numbers

`exp`, `log`, trig functions and exponents need dimensionless values. Divide by a reference quantity first.

```example
1 [m] exp                ⇒ ! dimensionless
3 [m] 1 [m] / exp        ⇒ 20.085537
```
