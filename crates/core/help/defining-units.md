# Defining units and dimensions

Units are ordinary cells. The built-in SI, imperial and currency library is on the **units** sheet — read it, edit it, or add declarations to any cell in any sheet.

## A new unit of an existing dimension

```
[furlong] = 201.168 [m]
[fortnight] = 14 [day]
```

The right-hand side is a program. Its value — a number with a unit — defines the new unit's size and dimension.

## A new dimension

Things you count — widgets, people, seats — are their own dimension, so they can't be added to money by accident:

```
dim widgets
base [widget] widgets
```

Then units compose as you'd expect:

```example
12 [USD/widget] 100 [widget] *   ⇒ 1,200 USD
1 [widget] 1 [USD] +             ⇒ ! needs matching units
```

## Conversion factors are inputs

A definition can reference cells, so an exchange rate is just an input. Scrub it and every price in euros updates.

```
[EUR] = rates!B2 [USD]
```

The dimension of a unit never depends on the value — only its size does — so unit errors are still found where they happen.

## Absolute units

End a definition with `offset n` (in base units) to make an absolute unit, like the ones in [[#absolute]]:

```
[Δ°C] = 1 [K]
[°C] = 1 [Δ°C] offset 273.15
```

## Prefixes

There's no automatic `k`, `m` or `µ` prefixing — `km`, `ms` and friends are listed explicitly on the units sheet, so `ms` always means milliseconds and never metre-seconds. Add the ones you need.

## Duplicates

If two cells define the same unit, the first (in sheet order) wins and the other shows an error.
