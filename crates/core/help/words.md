# Defining words

When you write the same steps twice, name them. A cell starting with `:` defines a word; it ends with `;`.

```
: sq ( x -- x² ) dup * ;
```

Now `sq` works in any cell, on any sheet, and on arrays too. Edit the definition and every cell that uses it recalculates.

```example
3 sq             ⇒ 9
A1:A3 sq         ⇒ [1, 4, 9]
```

## Documenting a word

A comment right after the name — `( … )` — is the word's documentation. By convention it shows the stack effect: inputs `--` outputs. Help, tooltips and the editor's hint strip show it.

## Locals

Juggling the stack gets hard with more than two inputs. Name them instead: `{ a b }` pops values into locals, with the last name taking the top of the stack.

```
: hyp ( a b -- hypotenuse ) { a b } a sq b sq + sqrt ;
```

```example
3 4 hyp          ⇒ 5
```

A finance example, from the demo model — net present value of a list of cash flows:

```
: npv ( rate cfs -- pv ) { rate cfs } cfs 1 rate + cfs len range ^ / sum ;
```

## Using your word with reduce and scan

Any word that turns two values into one can be used with `/` and `\`: `/myword`, `\myword`.

## Rules

- Names start with a letter and can't be a builtin word.
- Words can't call themselves; use arrays (`range`, `/op`, `\op`) instead of loops.
- Your words are listed under **Your words** in the help sidebar.
