# Temperatures and dates

Some units measure a point on a scale rather than an amount: 20 °C, 68 °F, 2026-10-08. You can't add two of them (what is 20 °C + 20 °C?), but you can subtract them, add a difference, compare them and average them.

## Temperature

`°C` and `°F` are absolute. Their differences use `Δ°C` and `Δ°F`. Arithmetic happens in kelvin.

```example
20 [°C] 5 [Δ°C] +        ⇒ 25 °C
30 [°C] 20 [°C] -        ⇒ 10 Δ°C
212 [°F] to[°C]          ⇒ 100 °C
20 [°C] 10 [°C] +        ⇒ ! can't add two absolute values
```

Use Δ units in compound quantities like heat capacity: `[J/(kg*Δ°C)]`.

## Dates

An ISO date like `2026-10-08` is an absolute point in time. Add or subtract durations; subtract two dates to get days.

```example
2026-10-08 30 [day] +        ⇒ 2026-11-07
2026-12-25 2026-10-08 -      ⇒ 78 day
2026-10-08 2 [week] +        ⇒ 2026-10-22
```

A date cell fills as a series: type two dates a week apart, select both and drag the fill handle.

## The rule

| operation | allowed? |
|---|---|
| absolute + difference | yes → absolute |
| absolute − absolute | yes → difference |
| absolute + absolute | no |
| `mean` of absolutes | yes |
| `sum` of absolutes | no |

When a value has to be multiplied (for example a temperature in a gas-law formula), it's used through its linear unit: kelvin for temperatures, days for dates.
