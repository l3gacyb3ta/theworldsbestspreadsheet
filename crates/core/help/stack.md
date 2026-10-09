# Thinking in stacks

Programs are postfix: values first, then the word that uses them. Read left to right, keeping a stack of values in your head — or watch the stack in the strip under the formula bar while you type.

```example
2 3 +            ⇒ 5
2 3 4 * +        ⇒ 14
2 3 + 4 *        ⇒ 20
```

`2 3 4 * +` pushes 2, 3 and 4, multiplies the top two (3 × 4 = 12), then adds (2 + 12). No parentheses and no precedence rules — the order you write is the order things happen.

## The one-value rule

When the program ends, exactly one value must be left; it becomes the cell's value. Leftovers are an error, which catches a missing operator immediately.

```example
1 2 3            ⇒ ! 3 values left on stack
1 2 3 + +        ⇒ 6
```

## Tokens are separated by spaces

`2 3+` is not `2 3 +` — `3+` is one token, which isn't a word. Clicking cells while editing inserts references with spaces around them, so click order is stack order: click A1, click B1, type `*`.

## Shuffling the stack

Sometimes values arrive in the wrong order or you need one twice:

| word | effect |
|---|---|
| [[dup]] | a — a a |
| [[drop]] | a — |
| [[swap]] | a b — b a |
| [[over]] | a b — a b a |
| [[rot]] | a b c — b c a |

```example
3 dup *          ⇒ 9
10 2 swap /      ⇒ 0.2
```

Stack effects in this help read `inputs — outputs`, with the top of the stack on the right.

## When a word needs more

If a word runs out of inputs you'll see "needs more values on the stack". Use **Step through** in the inspector to see the stack after every token, and the error highlighted at the token where it happened.
