# Arrays, ranges and broadcasting

Every value is an array. A single number is an array with no axes (rank 0), a column of numbers has one axis (rank 1), a table has two.

## A range is one value

`A1:A5` pushes one array, not five numbers. That's why `A1:A5 sum` works without parentheses or commas.

```example
A1:A5            ⇒ [1, 2, 3, 4, 5]
A1:A5 sum        ⇒ 15
A1:A5 len        ⇒ 5
```

A range that's one row or one column is a list; anything wider is a table. Empty cells inside a range are an error — they're not quietly treated as zero.

## Elementwise words and broadcasting

Arithmetic, comparisons and math functions work element by element. A single value is combined with every element.

```example
A1:A5 10 *           ⇒ [10, 20, 30, 40, 50]
A1:A3 B1:B3 *        ⇒ [10, 40, 90] m
A1:A5 3 >            ⇒ [0, 0, 0, 1, 1]
```

Two arrays must have the same length. A list combines with each row of a table (its leading axis), so a column of rates can scale every row.

## Reduce and scan

Put `/` in front of a word to fold it across an array, and `\` for the running version. Any word that takes two values works, including your own.

```example
A1:A5 /+             ⇒ 15
A1:A5 /max           ⇒ 5
A1:A5 \+             ⇒ [1, 3, 6, 10, 15]
```

On a table, `/+` combines the rows, giving one total per column. [[sum]] and [[mean]] always collapse everything to one number.

## Building arrays

```example
5 range              ⇒ [0, 1, 2, 3, 4]
3 range 1 +          ⇒ [1, 2, 3]
1 2 join 3 join      ⇒ [1, 2, 3]
1 2 join 3 4 join couple    ⇒ [[1, 2], [3, 4]]
```

## Picking things out

```example
A1:A5 first          ⇒ 1
A1:A5 2 pick         ⇒ 3
A1:A5 3 > A1:A5 0 if ⇒ [0, 0, 0, 4, 5]
```

`if` with an array condition chooses element by element. Combined with `/min`, that finds the first index where something becomes true — see the "first profitable month" cell in the demo model.

An array result fills the cells below (and to the right) — see [[#spill]].
