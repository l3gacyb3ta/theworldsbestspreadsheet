# Errors and how to fix them

When something can't be computed, the cell where it happens says so. The grid shows a short code, and the status bar, inspector and formula bar show the full message, with the offending token underlined in red.

| code | meaning |
|---|---|
| `#err` | the problem is in this cell |
| `#upstream` | a cell this one reads has an error — the inspector links to it |
| `#cycle` | this cell is part of a circular reference |
| `#spill blocked` | the result needs cells that aren't empty |

Select an error cell and the inspector explains it in plain words, with a link to the relevant help. **Step through** shows exactly which token failed and what was on the stack.

## Every error

{{errors}}
