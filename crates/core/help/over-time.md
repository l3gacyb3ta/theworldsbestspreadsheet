# Equations over time

A lot of models are "take the state, apply a rule, repeat": a balance growing with interest, a population, a spring bouncing. You could fill a column with the same formula a thousand times, but one cell can hold the whole run: [[iterate]] for a step rule, [[rk4]] for a rate of change.

## Words as values

Both take a word to run. A quote in front of a word pushes the word itself instead of running it:

```example
'sq              ⇒ 'sq
'sq 1 +          ⇒ ! expects a number, got word
```

## iterate: a rule applied n times

`n s1 … sk 'step k iterate` runs `step` n times on a state of k values and leaves k arrays, each holding that value's history: the start, then each step's result, n+1 values in all.

```
: grow ( x -- x' ) 1.1 * ;
```

```example
3 1 'grow 1 iterate            ⇒ [1, 1.1, 1.21, 1.331]
4 100 [USD] 'grow 1 iterate    ⇒ [100, 110, 121, 133.1, 146.41] USD
```

The state can be several values, each with its own unit: a position in m and a speed in m/s step side by side. The step word takes them all and leaves the next state, in the same order. The arrays come back in that order too, the first state's at the bottom, so keep the one you want with `drop`, `swap drop` or `pick`:

```example
4 0 1 'swap 2 iterate drop     ⇒ [0, 1, 0, 1, 0]
4 0 1 'swap 2 iterate swap drop     ⇒ [1, 0, 1, 0, 1]
```

The step has to give each state back in its own dimension, or the cell says which one it changed:

```example
3 1 [m] 'sq 1 iterate          ⇒ ! 'sq turns state 1 from length into length^2
```

## rk4: equations of motion

When you know how fast things change rather than the next value, you have a differential equation. `t0 h n s1 … sk 'deriv k rk4` solves it with fourth-order Runge–Kutta: from time t0, n steps of size h. The derivative word takes the time and the state, `( t s1 … sk -- d1 … dk )`, and leaves each state's rate of change.

A dropped ball: height changes at the speed, speed changes at −g.

```
: fall ( t y v -- dy dv ) { t y v } v -9.81 [m/s^2] ;
```

```example
0 [s] 0.5 [s] 2 10 [m] 0 [m/s] 'fall 2 rk4 drop        ⇒ [10, 8.77375, 5.095] m
0 [s] 0.5 [s] 2 10 [m] 0 [m/s] 'fall 2 rk4 swap drop   ⇒ [0, -4.905, -9.81] m/s
```

Units check the physics. Each stage of the method computes state + step × derivative, which only makes sense when the derivative times a time has the state's dimension. Swap the state's order and the derivative of a speed comes out as a length:

```example
0 [s] 0.5 [s] 2 0 [m/s] 10 [m] 'fall 2 rk4    ⇒ ! state 1 is length/time, but its derivative × the step is length*time
```

## A spring

A mass on a spring with damping: the restoring force is −k·x, the damping −c·v. With the inputs named `stiffness`, `mass` and `friction`:

```
: spring ( t x v -- dx dv ) { t x v } v   stiffness mass / x * neg   friction mass / v * - ;
```

```example
0 [s] 0.25 [s] 4 1 [m] 0 [m/s] 'spring 2 rk4 drop    ⇒ [1, 0.87760417, 0.54058838, 0.071425562, -0.41510799] m
```

A time column to chart against is `n 1 + range h * t0 +`:

```example
5 range 0.25 [s] * 0 [s] +    ⇒ [0, 0.25, 0.5, 0.75, 1] s
```

Put the positions in one cell and the times in another, and `line` draws the motion. The result is an ordinary computed array, so its points on a chart drag by goal-seek: pull the first bounce lower and the sheet solves for the friction that puts it there. An input range on `friction` (see [[#modeling]]) keeps that answer physical.

## Limits

- Up to 1,000,000 steps, and 10,000,000 values in all.
- The step size is fixed. Smaller steps are more accurate; halve the step and compare to see if the answer has settled.
- The word runs at least once, even for 0 steps, so its unit errors always show.
