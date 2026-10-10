//! The help system's content and machinery: a reference entry for every
//! builtin word and syntax form, guide topics (mini-markdown with live
//! examples), search, and plain-language explanations of errors.
//!
//! Every example is evaluated against a small sample sheet, and the tests
//! check each example's documented result, so the docs can't drift from the
//! language.

use crate::engine::Engine;
use crate::ids::SheetId;
use crate::lex::Tok;
use crate::model::Sheet;
use crate::parse::{builtin_name, Builtin};

// ---- word & syntax reference -------------------------------------------------

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Category {
    Syntax,
    Arithmetic,
    Math,
    Compare,
    Stack,
    Arrays,
    Reduce,
    Units,
    Charts,
    Definitions,
}

impl Category {
    pub const ALL: [Category; 10] = [
        Category::Syntax,
        Category::Arithmetic,
        Category::Math,
        Category::Compare,
        Category::Stack,
        Category::Arrays,
        Category::Reduce,
        Category::Units,
        Category::Charts,
        Category::Definitions,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Category::Syntax => "Values & references",
            Category::Arithmetic => "Arithmetic",
            Category::Math => "Math functions",
            Category::Compare => "Comparison & choice",
            Category::Stack => "Stack shuffling",
            Category::Arrays => "Arrays",
            Category::Reduce => "Reduce & scan",
            Category::Units => "Units",
            Category::Charts => "Charts",
            Category::Definitions => "Definitions",
        }
    }
}

pub struct WordDoc {
    /// How the entry is written: `+`, `sum`, `[unit]`, `/op`.
    pub name: &'static str,
    /// Stack effect: inputs — outputs (top of stack on the right).
    pub effect: &'static str,
    pub category: Category,
    pub summary: &'static str,
    /// Dimension rule, if the word cares about units.
    pub units: &'static str,
    /// More detail (mini-markdown).
    pub details: &'static str,
    /// (program, documented result). A result starting with `!` means the
    /// program fails with an error containing the rest.
    pub examples: &'static [(&'static str, &'static str)],
    pub see: &'static [&'static str],
}

macro_rules! w {
    ($name:expr, $effect:expr, $cat:ident, $summary:expr, units: $units:expr, details: $details:expr, ex: [$(($p:expr, $r:expr)),* $(,)?], see: [$($s:expr),* $(,)?]) => {
        WordDoc {
            name: $name,
            effect: $effect,
            category: Category::$cat,
            summary: $summary,
            units: $units,
            details: $details,
            examples: &[$(($p, $r)),*],
            see: &[$($s),*],
        }
    };
}

pub const WORDS: &[WordDoc] = &[
    // -- syntax ---------------------------------------------------------------
    w!("123", "— n", Syntax, "A number literal pushes itself.",
        units: "Dimensionless until you apply a unit.",
        details: "Write `-3`, `2.5`, `1e6` or `1_000_000`. A bare number in a cell (no `=`) is a literal number cell; those are the cells you can scrub.",
        ex: [("42", "42"), ("1_000_000", "1,000,000"), ("2.5e-3", "0.0025")], see: ["[unit]", "2026-10-08"]),
    w!("2026-10-08", "— date", Syntax, "An ISO date pushes an absolute point in time.",
        units: "Dates are absolute: add or subtract a duration; subtracting two dates gives a duration in days.",
        details: "Dates use the `[date]` unit from the units sheet. Like °C, you can't add two dates.",
        ex: [("2026-10-08", "2026-10-08"), ("2026-10-08 30 [day] +", "2026-11-07"), ("2026-12-25 2026-10-08 -", "78 day")], see: ["[unit]", "#absolute"]),
    w!("\"text\"", "— text", Syntax, "A string in double quotes pushes text.",
        units: "",
        details: "Text is used for chart titles and labels, and for `if` branches. Text cells (no `=`) can be referenced too; a range of text cells is a text array.",
        ex: [("\"hello\"", "hello"), ("C1:C3", "[apples, pears, plums]")], see: ["title", "bar"]),
    w!("A1", "— value", Syntax, "A cell reference pushes that cell's value.",
        units: "The value keeps its unit.",
        details: "`$A$1`, `$A1`, `A$1` mark an axis absolute for copying and filling. `Sheet!A1` reaches another sheet. A reference to a spilling cell pushes the whole array; a reference to a cell inside a spill pushes that element. While editing, click a cell to insert its reference.",
        ex: [("A1", "1"), ("$A$2 A3 +", "5"), ("B2", "20 m")], see: ["A1:B5", "name", "#references"]),
    w!("A1:B5", "— array", Syntax, "A range pushes one array: rank 1 for a single row or column, rank 2 otherwise.",
        units: "All numbers in a range must share a dimension; the first cell's unit is used for display.",
        details: "Empty cells in a range are an error (they're not zero); end the range with `?` (`A1:A100?`) to leave them out. Drag across cells while editing to insert a range.",
        ex: [("A1:A5", "[1, 2, 3, 4, 5]"), ("A1:B2", "! mixes units"), ("A4:A7", "! A6 is empty"), ("A1:A5 sum", "15"), ("B1:B3", "[10, 20, 30] m")], see: ["A1", "A1:B5?", "/op", "#arrays"]),
    w!("A1:B5?", "— list", Syntax, "A range ending in `?` may have empty cells: it pushes its non-empty cells as a list.",
        units: "The non-empty cells must still share a dimension, and can't mix text and numbers.",
        details: "An empty cell in a plain range is an error, because treating it as zero would be a guess. The `?` says gaps are expected: `A1:A100? sum` adds up whatever has been filled in. The result is always a list — a table's non-empty cells are read row by row — so its shape doesn't depend on which cells happen to be empty. Only empty cells are skipped; a cell with an error is still an error. `?` works on ranges only: `A1?` is an error.",
        ex: [("A4:A7?", "[4, 5]"), ("A4:A7? sum", "9"), ("A4:A7 sum", "! A6 is empty"), ("G1:H2?", "[1, 2, 3]"), ("C2:C5?", "[pears, plums]"), ("A5:B6?", "! mixes units"), ("A6:A9?", "[]"), ("A6:A9? sum", "0"), ("A1? sum", "! ? only applies to a range")],
        see: ["A1:B5", "sum", "#references"]),
    w!("name", "— value", Syntax, "A cell's name pushes that cell, like a reference.",
        units: "",
        details: "Name a cell in the inspector. Names may contain letters, digits, `_` and `.` (`rates.eur`) and can't shadow words. Tick “input” to list it in the Inputs panel.",
        ex: [("growth 1 +", "1.05"), ("price 2 *", "2,000 USD")], see: ["A1", "#references"]),
    w!("( … )", "—", Syntax, "A comment. A lone `(` starts it and the next `)` ends it.",
        units: "",
        details: "Comments are ignored. In a word definition, a comment right after the name is the word's documentation and shows up in help and tooltips: `: sq ( x -- x² ) dup * ;`.",
        ex: [("2 ( two ) 3 ( three ) +", "5")], see: [":"]),
    // -- arithmetic -------------------------------------------------------------
    w!("+", "a b — a+b", Arithmetic, "Adds, elementwise.",
        units: "Dimensions must match. The result displays in a's unit. Two absolute values (temperatures, dates) can't be added.",
        details: "Arrays add elementwise and scalars broadcast; arrays of different shapes must agree on their leading axes.",
        ex: [("2 3 +", "5"), ("5 [km] 300 [m] +", "5.3 km"), ("A1:A3 10 +", "[11, 12, 13]"), ("1 [m] 1 [s] +", "! needs matching units")], see: ["-", "sum", "#units"]),
    w!("-", "a b — a−b", Arithmetic, "Subtracts, elementwise.",
        units: "Dimensions must match. absolute − absolute gives a difference (Δ).",
        details: "",
        ex: [("10 4 -", "6"), ("30 [°C] 20 [°C] -", "10 Δ°C"), ("A1:A3 1 -", "[0, 1, 2]")], see: ["+", "neg"]),
    w!("*", "a b — a×b", Arithmetic, "Multiplies, elementwise.",
        units: "Dimensions multiply; display units compose (`m*m` shows `m^2`). A dimensionless unit like % is absorbed into a dimensioned one.",
        details: "",
        ex: [("6 7 *", "42"), ("3 [m] 4 [m] *", "12 m^2"), ("B1:B3 2 *", "[20, 40, 60] m"), ("4 [%] 200 [USD] *", "8 USD")], see: ["/", "^"]),
    w!("/", "a b — a÷b", Arithmetic, "Divides, elementwise.",
        units: "Dimensions divide.",
        details: "Written alone with spaces around it. Attached to a word (`/+`) it's a reduce.",
        ex: [("1 4 /", "0.25"), ("100 [km] 2 [h] /", "50 km/h"), ("B1:B3 A1:A3 /", "[10, 10, 10] m")], see: ["*", "/op"]),
    w!("^", "a n — aⁿ", Arithmetic, "Raises to a power, elementwise.",
        units: "The exponent must be dimensionless. If a has units, n must be a single simple fraction (`2`, `0.5`, `1/3`) so the result's dimension is known. Written in the program, it is known before evaluating; read from a cell, the result's dimension is known only once it's computed.",
        details: "",
        ex: [("2 10 ^", "1,024"), ("3 [m] 2 ^", "9 m^2"), ("1 growth + A1:A3 ^", "[1.05, 1.1025, 1.157625]")], see: ["sqrt", "exp"]),
    w!("neg", "a — −a", Arithmetic, "Negates.", units: "Keeps the unit. Not allowed on absolute values.", details: "",
        ex: [("5 neg", "-5"), ("B1 neg", "-10 m")], see: ["abs"]),
    w!("abs", "a — |a|", Arithmetic, "Absolute value.", units: "Keeps the unit.", details: "",
        ex: [("-3 [m] abs", "3 m")], see: ["neg"]),
    // -- math --------------------------------------------------------------------
    w!("sqrt", "a — √a", Math, "Square root.", units: "Exponents halve: √(m²) = m, and √m is m^1/2.", details: "",
        ex: [("16 sqrt", "4"), ("9 [m^2] sqrt", "3 m")], see: ["^"]),
    w!("exp", "a — eᵃ", Math, "e to the power a.", units: "Needs a dimensionless argument.", details: "",
        ex: [("0 exp", "1"), ("1 exp", "2.7182818")], see: ["log"]),
    w!("log", "a — ln a", Math, "Natural logarithm.", units: "Needs a dimensionless argument.", details: "",
        ex: [("1 log", "0"), ("1 exp log", "1"), ("1 [m] log", "! dimensionless")], see: ["log10", "log2", "exp"]),
    w!("log10", "a — log₁₀ a", Math, "Base-10 logarithm.", units: "Needs a dimensionless argument.", details: "",
        ex: [("1000 log10", "3")], see: ["log"]),
    w!("log2", "a — log₂ a", Math, "Base-2 logarithm.", units: "Needs a dimensionless argument.", details: "",
        ex: [("8 log2", "3")], see: ["log"]),
    w!("sin", "a — sin a", Math, "Sine (radians).", units: "Needs a dimensionless argument.", details: "",
        ex: [("pi 2 / sin", "1")], see: ["cos", "tan", "pi"]),
    w!("cos", "a — cos a", Math, "Cosine (radians).", units: "Needs a dimensionless argument.", details: "",
        ex: [("0 cos", "1")], see: ["sin", "tan"]),
    w!("tan", "a — tan a", Math, "Tangent (radians).", units: "Needs a dimensionless argument.", details: "",
        ex: [("0 tan", "0")], see: ["sin", "cos"]),
    w!("floor", "a — ⌊a⌋", Math, "Rounds down, in the value's display unit.", units: "Keeps the unit.", details: "",
        ex: [("2.7 floor", "2"), ("1234 [m] to[km] floor", "1 km")], see: ["ceil", "round"]),
    w!("ceil", "a — ⌈a⌉", Math, "Rounds up, in the value's display unit.", units: "Keeps the unit.", details: "",
        ex: [("2.2 ceil", "3")], see: ["floor", "round"]),
    w!("round", "a — a rounded", Math, "Rounds to the nearest whole number (halves away from zero), in the display unit.", units: "Keeps the unit.", details: "",
        ex: [("2.5 round", "3"), ("1.26 [km] to[m] round", "1,260 m")], see: ["floor", "ceil"]),
    w!("pi", "— π", Math, "Pushes π.", units: "", details: "",
        ex: [("pi", "3.1415927")], see: ["sin"]),
    w!("min", "a b — smaller", Math, "Elementwise minimum.", units: "Dimensions must match.", details: "Use `/min` to find the smallest element of an array.",
        ex: [("3 7 min", "3"), ("A1:A5 3 min", "[1, 2, 3, 3, 3]")], see: ["max", "/op"]),
    w!("max", "a b — larger", Math, "Elementwise maximum.", units: "Dimensions must match.", details: "Use `/max` to find the largest element of an array.",
        ex: [("3 7 max", "7"), ("A1:A5 3 max", "[3, 3, 3, 4, 5]")], see: ["min", "/op"]),
    // -- comparison --------------------------------------------------------------
    w!("<", "a b — a<b", Compare, "1 where a is less than b, else 0.", units: "Dimensions must match; the result is dimensionless.", details: "",
        ex: [("3 5 <", "1"), ("A1:A5 3 <", "[1, 1, 0, 0, 0]")], see: ["if", ">"]),
    w!(">", "a b — a>b", Compare, "1 where a is greater than b, else 0.", units: "Dimensions must match.", details: "",
        ex: [("1 [km] 999 [m] >", "1")], see: ["<", "if"]),
    w!("<=", "a b — a≤b", Compare, "1 where a ≤ b, else 0.", units: "Dimensions must match.", details: "",
        ex: [("A1:A5 3 <=", "[1, 1, 1, 0, 0]")], see: ["<"]),
    w!(">=", "a b — a≥b", Compare, "1 where a ≥ b, else 0.", units: "Dimensions must match.", details: "",
        ex: [("A1:A5 3 >=", "[0, 0, 1, 1, 1]")], see: [">"]),
    w!("=", "a b — a=b", Compare, "1 where a equals b, else 0.", units: "Compares canonical values, so units convert.", details: "Not to be confused with the `=` that starts a program.",
        ex: [("5 [km] 5000 [m] =", "1")], see: ["!="]),
    w!("!=", "a b — a≠b", Compare, "1 where a differs from b, else 0.", units: "Dimensions must match.", details: "",
        ex: [("1 2 !=", "1")], see: ["="]),
    w!("not", "a — ¬a", Compare, "1 where a is 0, else 0.", units: "Needs a dimensionless argument.", details: "",
        ex: [("0 not", "1"), ("A1:A3 2 > not", "[1, 1, 0]")], see: ["if"]),
    w!("if", "cond a b — a or b", Compare, "Picks a where cond is non-zero, b elsewhere.",
        units: "The condition is dimensionless. Numbers a and b must share a dimension, even when the condition picks one of them whole, so the result's dimension doesn't depend on the condition.",
        details: "With a single condition it picks a whole value (any type). With an array condition it picks elementwise, broadcasting a and b.",
        ex: [("1 \"yes\" \"no\" if", "yes"), ("A1:A5 3 > A1:A5 0 if", "[0, 0, 0, 4, 5]"), ("1 2 [m] 3 [s] if", "! needs matching units")], see: ["<", "not"]),
    // -- stack -------------------------------------------------------------------
    w!("dup", "a — a a", Stack, "Duplicates the top value.", units: "", details: "",
        ex: [("3 dup *", "9")], see: ["over", "drop"]),
    w!("drop", "a —", Stack, "Discards the top value.", units: "", details: "",
        ex: [("1 2 drop", "1")], see: ["dup"]),
    w!("swap", "a b — b a", Stack, "Swaps the top two values.", units: "", details: "",
        ex: [("10 2 swap /", "0.2")], see: ["over", "rot"]),
    w!("over", "a b — a b a", Stack, "Copies the second value to the top.", units: "", details: "",
        ex: [("2 3 over * +", "8")], see: ["dup", "swap"]),
    w!("rot", "a b c — b c a", Stack, "Rotates the third value to the top.", units: "", details: "",
        ex: [("10 2 3 rot - -", "9")], see: ["swap"]),
    // -- arrays ------------------------------------------------------------------
    w!("range", "n — [0 … n−1]", Arrays, "The first n whole numbers, starting at 0.", units: "n must be a dimensionless whole number.", details: "",
        ex: [("5 range", "[0, 1, 2, 3, 4]"), ("3 range 1 +", "[1, 2, 3]")], see: ["len"]),
    w!("len", "a — n", Arrays, "Length along the leading axis (1 for a single value).", units: "", details: "",
        ex: [("A1:A5 len", "5"), ("7 len", "1")], see: ["range"]),
    w!("rev", "a — a reversed", Arrays, "Reverses along the leading axis.", units: "", details: "",
        ex: [("3 range rev", "[2, 1, 0]")], see: ["first", "last"]),
    w!("join", "a b — a then b", Arrays, "Concatenates along the leading axis.", units: "Dimensions must match.", details: "Single values join as one-element rows, so `1 2 join` builds a list.",
        ex: [("1 2 join", "[1, 2]"), ("A1:A2 A4:A5 join", "[1, 2, 4, 5]")], see: ["couple"]),
    w!("couple", "a b — [a, b]", Arrays, "Stacks two same-shaped values into a new leading axis.", units: "Dimensions must match.", details: "",
        ex: [("1 2 join 3 4 join couple", "[[1, 2], [3, 4]]")], see: ["join", "transpose"]),
    w!("transpose", "a — aᵀ", Arrays, "Swaps rows and columns. A list becomes a single row.", units: "", details: "",
        ex: [("1 2 join 3 4 join couple transpose", "[[1, 3], [2, 4]]")], see: ["couple"]),
    w!("first", "a — first row", Arrays, "The first element along the leading axis.", units: "", details: "",
        ex: [("A1:A5 first", "1")], see: ["last", "pick"]),
    w!("last", "a — last row", Arrays, "The last element along the leading axis.", units: "", details: "",
        ex: [("A1:A5 last", "5")], see: ["first", "pick"]),
    w!("pick", "a i — row i", Arrays, "The element at index i (counting from 0).", units: "i must be a dimensionless whole number.", details: "",
        ex: [("A1:A5 2 pick", "3"), ("A1:A5 9 pick", "! out of range")], see: ["first", "last"]),
    // -- reduce ------------------------------------------------------------------
    w!("sum", "a — Σa", Reduce, "Adds up every element.", units: "Keeps the unit. Absolute values can't be summed.", details: "Unlike `/+`, `sum` collapses every axis to a single number.",
        ex: [("A1:A5 sum", "15"), ("B1:B5 sum", "150 m")], see: ["/op", "mean"]),
    w!("mean", "a — mean", Reduce, "The average of every element.", units: "Keeps the unit; works on absolute values too.", details: "",
        ex: [("A1:A5 mean", "3"), ("20 [°C] 30 [°C] join mean", "25 °C")], see: ["sum"]),
    w!("/op", "a — reduced", Reduce, "Reduce: folds a two-input word between the rows of an array.",
        units: "Whatever the word requires.",
        details: "`/+` sums, `/*` multiplies, `/max` finds the maximum. Any word that takes two values and leaves one works, including your own. On a 2-D array it combines rows, giving one result per column.",
        ex: [("A1:A5 /+", "15"), ("A1:A5 /*", "120"), ("A1:A5 /max", "5"), ("1 2 join 3 4 join couple /+", "[4, 6]")], see: ["\\op", "sum"]),
    w!("\\op", "a — running", Reduce, "Scan: like reduce, keeping every intermediate result.",
        units: "Whatever the word requires.",
        details: "`\\+` is a running total; `\\max` a running maximum.",
        ex: [("A1:A5 \\+", "[1, 3, 6, 10, 15]"), ("3 1 join 4 join 1 join 5 join \\max", "[3, 3, 4, 4, 5]")], see: ["/op"]),
    // -- units -------------------------------------------------------------------
    w!("[unit]", "a — a·unit", Units, "Multiplies the top value by a unit.",
        units: "Inside the brackets: `*`, `/`, `^n` (fractions allowed: `^1/2`) and parentheses.",
        details: "Units come from declarations anywhere in the workbook (see the `units` sheet). Absolute units (°C, °F, `date`) only apply to plain numbers.",
        ex: [("5 [km]", "5 km"), ("9.81 [m/s^2]", "9.81 m/s^2"), ("20 [°C]", "20 °C"), ("3 [furlong]", "! unknown unit")], see: ["to[unit]", "#units"]),
    w!("to[unit]", "a — a", Units, "Shows a value in another unit of the same dimension. Only the display changes.",
        units: "The dimensions must match.",
        details: "Derived units are never applied automatically: ask for them with `to[N]`, `to[kWh]`…",
        ex: [("5 [km] to[m]", "5,000 m"), ("100 [km/h] to[m/s]", "27.777778 m/s"), ("2 [kg] 3 [m/s^2] * to[N]", "6 N"), ("5 [km] to[s]", "! dimensions differ")], see: ["[unit]"]),
    // -- charts ------------------------------------------------------------------
    w!("line", "xs ys — chart", Charts, "A line chart. Spills into a block of cells (6×14 by default).",
        units: "Axes are labelled with the values' units.",
        details: "Points whose y comes straight from a literal number cell can be dragged on the chart to edit that cell.",
        ex: [("A1:A5 B1:B5 line", "chart · line (5 points) · 6×14 cells")], see: ["scatter", "bar", "layer", "#charts"]),
    w!("scatter", "xs ys — chart", Charts, "A scatter chart.", units: "Axes use the values' units.", details: "",
        ex: [("A1:A5 B1:B5 scatter", "chart · scatter (5 points) · 6×14 cells")], see: ["line"]),
    w!("bar", "cats vals — chart", Charts, "A bar chart; categories may be text or numbers.", units: "", details: "",
        ex: [("C1:C3 D1:D3 bar", "chart · bar (3 points) · 6×14 cells")], see: ["line"]),
    w!("layer", "chart chart — chart", Charts, "Draws two charts on the same axes.", units: "The y dimensions (and x dimensions) must match.", details: "",
        ex: [("A1:A5 A1:A5 line A1:A5 A1:A5 sq line layer", "chart · line (5 points) + line (5 points) · 6×14 cells")], see: ["line"]),
    w!("title", "chart \"t\" — chart", Charts, "Sets the chart's title.", units: "", details: "",
        ex: [("A1:A5 B1:B5 line \"distance\" title", "chart \"distance\" · line (5 points) · 6×14 cells")], see: ["xlabel", "ylabel"]),
    w!("xlabel", "chart \"t\" — chart", Charts, "Sets the x-axis label (default: the x unit).", units: "", details: "",
        ex: [("A1:A5 B1:B5 line \"step\" xlabel", "chart · line (5 points) · 6×14 cells")], see: ["ylabel"]),
    w!("ylabel", "chart \"t\" — chart", Charts, "Sets the y-axis label (default: the y unit).", units: "", details: "",
        ex: [("A1:A5 B1:B5 line \"height\" ylabel", "chart · line (5 points) · 6×14 cells")], see: ["xlabel"]),
    w!("size", "chart cols rows — chart", Charts, "Sets how many cells the chart spills over.", units: "", details: "",
        ex: [("A1:A5 B1:B5 line 4 8 size", "chart · line (5 points) · 4×8 cells")], see: ["line"]),
    // -- definitions -------------------------------------------------------------
    w!(":", ": name ( doc ) body ;", Definitions, "Defines a word in a cell. Cells that use it recalculate when you edit it.",
        units: "",
        details: "A cell starting with `:` is a definition. Optional `{ a b }` right after the name (or after the doc comment) pops named locals — the last name is the top of the stack. A comment right after the name documents the word.",
        ex: [("3 sq", "9"), ("3 4 hyp", "5")], see: ["( … )", "#words"]),
    w!("dim", "dim name", Definitions, "Declares a new base dimension (`dim widgets`).", units: "Values in different dimensions can't be added.",
        details: "A cell holding just `dim name` is a declaration. Give the dimension a unit with `base`.", ex: [], see: ["base", "#defining-units"]),
    w!("base", "base [unit] dim", Definitions, "Declares the canonical unit of a dimension (`base [widget] widgets`).", units: "",
        details: "", ex: [("12 [USD/widget] 100 [widget] *", "1,200 USD")], see: ["dim", "[u] = …"]),
    w!("[u] = …", "[u] = value", Definitions, "Defines a unit as a value with units: `[mi] = 1609.344 [m]`.",
        units: "The unit's dimension is the value's dimension.",
        details: "The value may reference cells, so a conversion factor like an exchange rate is an ordinary input you can scrub. End with `offset n` to define an absolute unit like °C (`[°C] = 1 [Δ°C] offset 273.15`).",
        ex: [], see: ["dim", "base", "#defining-units"]),
];

/// The reference entry for a builtin.
pub fn builtin_doc(b: Builtin) -> &'static WordDoc {
    word_doc(builtin_name(b)).expect("every builtin is documented")
}

pub fn word_doc(name: &str) -> Option<&'static WordDoc> {
    WORDS.iter().find(|w| w.name == name)
}

/// The reference entry that explains a token, if any.
pub fn doc_for_token(tok: &Tok) -> Option<&'static WordDoc> {
    match tok {
        Tok::Num(_) => word_doc("123"),
        Tok::Date(_) => word_doc("2026-10-08"),
        Tok::Str(_) => word_doc("\"text\""),
        Tok::Unit(_) => word_doc("[unit]"),
        Tok::To(_) => word_doc("to[unit]"),
        Tok::Ref(_) => word_doc("A1"),
        Tok::Range(_, _, false) => word_doc("A1:B5"),
        Tok::Range(_, _, true) => word_doc("A1:B5?"),
        Tok::Reduce(_) => word_doc("/op"),
        Tok::Scan(_) => word_doc("\\op"),
        Tok::Comment(_) => word_doc("( … )"),
        Tok::Word(w) if w == ":" || w == ";" || w == "{" || w == "}" => word_doc(":"),
        Tok::Word(w) => word_doc(w),
        _ => None,
    }
}

// ---- guide topics ---------------------------------------------------------------

pub struct Topic {
    pub id: &'static str,
    pub title: &'static str,
    pub body: &'static str,
}

macro_rules! topic {
    ($id:literal) => {
        ($id, include_str!(concat!("../help/", $id, ".md")))
    };
}

const TOPIC_SOURCES: &[(&str, &str)] = &[
    topic!("welcome"),
    topic!("cells"),
    topic!("stack"),
    topic!("arrays"),
    topic!("spill"),
    topic!("references"),
    topic!("units"),
    topic!("absolute"),
    topic!("defining-units"),
    topic!("words"),
    topic!("editing"),
    topic!("modeling"),
    topic!("charts"),
    topic!("errors"),
    topic!("keys"),
    topic!("settings"),
    topic!("principles"),
];

pub fn topics() -> Vec<Topic> {
    TOPIC_SOURCES
        .iter()
        .map(|(id, src)| {
            let title = src.lines().next().and_then(|l| l.strip_prefix("# ")).unwrap_or(id);
            let body = src.split_once('\n').map(|x| x.1).unwrap_or("");
            Topic { id, title, body }
        })
        .collect()
}

pub fn topic(id: &str) -> Option<Topic> {
    topics().into_iter().find(|t| t.id == id)
}

// ---- mini markdown --------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading(String),
    Para(String),
    Bullet(String),
    /// Plain code lines.
    Code(Vec<String>),
    /// ```example lines: `program ⇒ result`.
    Example(Vec<(String, String)>),
    /// Two-column table rows from `| a | b |` lines.
    Table(Vec<(String, String)>),
    /// `{{name}}`: generated content (e.g. the error table); one of `DIRECTIVES`.
    Generated(String),
}

/// The `{{name}}` directives the help window knows how to fill in.
pub const DIRECTIVES: &[&str] = &["errors", "sample", "settings"];

/// Paragraphs are separated by blank lines; `## ` headings, `- ` bullets,
/// fenced code, `| a | b |` tables, `{{name}}` directives.
pub fn parse_markdown(src: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut para = String::new();
    let mut lines = src.lines().peekable();
    let flush = |para: &mut String, out: &mut Vec<Block>| {
        if !para.trim().is_empty() {
            out.push(Block::Para(para.trim().to_string()));
        }
        para.clear();
    };
    while let Some(line) = lines.next() {
        let t = line.trim_end();
        if let Some(fence) = t.strip_prefix("```") {
            flush(&mut para, &mut out);
            let mut body = Vec::new();
            for l in lines.by_ref() {
                if l.trim_end() == "```" {
                    break;
                }
                body.push(l.to_string());
            }
            if fence.trim() == "example" {
                out.push(Block::Example(
                    body.iter()
                        .filter(|l| !l.trim().is_empty())
                        .map(|l| match l.split_once('⇒') {
                            Some((p, r)) => (p.trim().to_string(), r.trim().to_string()),
                            None => (l.trim().to_string(), String::new()),
                        })
                        .collect(),
                ));
            } else {
                out.push(Block::Code(body));
            }
        } else if let Some(h) = t.strip_prefix("## ") {
            flush(&mut para, &mut out);
            out.push(Block::Heading(h.to_string()));
        } else if let Some(b) = t.strip_prefix("- ") {
            flush(&mut para, &mut out);
            let mut item = b.to_string();
            while let Some(next) = lines.peek() {
                if next.starts_with("  ") && !next.trim().is_empty() {
                    item.push(' ');
                    item.push_str(next.trim());
                    lines.next();
                } else {
                    break;
                }
            }
            out.push(Block::Bullet(item));
        } else if t.starts_with('|') {
            flush(&mut para, &mut out);
            let mut rows = Vec::new();
            let mut row_line = Some(t.to_string());
            while let Some(l) = row_line.take() {
                let cells: Vec<String> = l.trim().trim_matches('|').split('|').map(|c| c.trim().to_string()).collect();
                if cells.len() >= 2 && !cells[0].chars().all(|c| c == '-') {
                    rows.push((cells[0].clone(), cells[1..].join(" | ")));
                }
                if lines.peek().is_some_and(|n| n.trim_start().starts_with('|')) {
                    row_line = lines.next().map(|s| s.to_string());
                }
            }
            out.push(Block::Table(rows));
        } else if let Some(d) = t.strip_prefix("{{").and_then(|d| d.strip_suffix("}}")) {
            flush(&mut para, &mut out);
            out.push(Block::Generated(d.trim().to_string()));
        } else if t.trim().is_empty() {
            flush(&mut para, &mut out);
        } else {
            if !para.is_empty() {
                para.push(' ');
            }
            para.push_str(t.trim());
        }
    }
    flush(&mut para, &mut out);
    out
}

#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text(String),
    Code(String),
    Bold(String),
    /// `[[target]]` or `[[target|label]]`. Targets: `#topic-id` or a word.
    Link { target: String, label: String },
}

pub fn inlines(s: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut rest = s;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("[[") {
            if let Some(mut end) = r.find("]]") {
                // `[[to[unit]]]`: the link closes at the last `]]` of a run
                while r[end + 2..].starts_with(']') {
                    end += 1;
                }
                if !text.is_empty() {
                    out.push(Inline::Text(std::mem::take(&mut text)));
                }
                let inner = &r[..end];
                let (target, label) = match inner.split_once('|') {
                    Some((t, l)) => (t.to_string(), l.to_string()),
                    None => (inner.to_string(), link_label(inner)),
                };
                out.push(Inline::Link { target, label });
                rest = &r[end + 2..];
                continue;
            }
        }
        if let Some(r) = rest.strip_prefix('`') {
            if let Some(end) = r.find('`') {
                if !text.is_empty() {
                    out.push(Inline::Text(std::mem::take(&mut text)));
                }
                out.push(Inline::Code(r[..end].to_string()));
                rest = &r[end + 1..];
                continue;
            }
        }
        if let Some(r) = rest.strip_prefix("**") {
            if let Some(end) = r.find("**") {
                if !text.is_empty() {
                    out.push(Inline::Text(std::mem::take(&mut text)));
                }
                out.push(Inline::Bold(r[..end].to_string()));
                rest = &r[end + 2..];
                continue;
            }
        }
        let ch = rest.chars().next().unwrap();
        text.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    if !text.is_empty() {
        out.push(Inline::Text(text));
    }
    out
}

fn link_label(target: &str) -> String {
    match target.strip_prefix('#') {
        Some(id) => topic(id).map(|t| t.title.to_string()).unwrap_or_else(|| id.to_string()),
        None => target.to_string(),
    }
}

// ---- errors ---------------------------------------------------------------------

pub struct ErrorHelp {
    /// Substrings of the error message this entry explains (any matches).
    pub patterns: &'static [&'static str],
    pub title: &'static str,
    pub why: &'static str,
    pub fix: &'static str,
    pub topic: &'static str,
}

pub const ERRORS: &[ErrorHelp] = &[
    ErrorHelp {
        patterns: &["must be ≥", "must be ≤", "'s range:"],
        title: "Outside the input's range",
        why: "This input has a range (min and max in the inspector), and the value typed here is outside it — or the range itself doesn't fit the input. The value is kept as typed, never clamped; cells that read it show #upstream.",
        fix: "Type a value inside the range, or change the range in the inspector. Scrubbing, chart dragging and goal-seek stop at the range's ends by themselves.",
        topic: "modeling",
    },
    ErrorHelp {
        patterns: &["values left on stack"],
        title: "Values left on the stack",
        why: "A program must leave exactly one value — the cell's value. Extra values usually mean a missing operator.",
        fix: "Add the word that combines them (`+`, `*`, `join`…) or `drop` the ones you don't need. Step through the cell to see the stack.",
        topic: "stack",
    },
    ErrorHelp {
        patterns: &["nothing left on the stack"],
        title: "Nothing left on the stack",
        why: "The program consumed every value (for example it ends with `drop`).",
        fix: "Make sure the last word leaves a result.",
        topic: "stack",
    },
    ErrorHelp {
        patterns: &["needs more values", "needs two values", "needs three values", "needs a value", "takes "],
        title: "Not enough values on the stack",
        why: "A word needed more inputs than were on the stack. In postfix the inputs come first: `2 3 +`, not `2 + 3`.",
        fix: "Put the inputs before the word. Step through the cell to see what's on the stack at each point.",
        topic: "stack",
    },
    ErrorHelp {
        patterns: &["needs matching units", "units differ", "mixes units"],
        title: "Units don't match",
        why: "Adding, subtracting, comparing or combining values needs them to be the same kind of quantity — you can't add metres to seconds.",
        fix: "Check which value has the unexpected unit (the inspector shows each value's dimension). Convert with a rate or fix the input's unit.",
        topic: "units",
    },
    ErrorHelp {
        patterns: &["can't show"],
        title: "Can't convert to that unit",
        why: "`to[unit]` only changes how a value is displayed, so the target unit must measure the same dimension.",
        fix: "Pick a unit of the same dimension, or compute the quantity you want first (e.g. divide by a time to get a speed).",
        topic: "units",
    },
    ErrorHelp {
        patterns: &["dimensionless"],
        title: "Needs a plain number",
        why: "Functions like `exp`, `log` and `sin` only make sense on dimensionless numbers, and exponents must be dimensionless.",
        fix: "Divide by a reference quantity first (e.g. `x 1 [m] /`).",
        topic: "units",
    },
    ErrorHelp {
        patterns: &["absolute"],
        title: "Absolute values (temperatures, dates)",
        why: "20 °C and 2026-10-08 are points, not amounts. You can subtract two of them or add a difference, but adding two (or summing them) is meaningless.",
        fix: "Subtract to get a difference (`Δ°C`, days), or add a duration such as `5 [Δ°C]` or `30 [day]`.",
        topic: "absolute",
    },
    ErrorHelp {
        patterns: &["unknown unit"],
        title: "Unknown unit",
        why: "Units come from declarations in the workbook; this one isn't declared anywhere.",
        fix: "Check the spelling against the units sheet, or declare it: `[furlong] = 201.168 [m]`.",
        topic: "defining-units",
    },
    ErrorHelp {
        patterns: &["unknown word or name", "unknown word"],
        title: "Unknown word or name",
        why: "The token isn't a builtin word, one of your words, or a cell name. References need capital letters (`A1`, not `a1`).",
        fix: "Check the spelling, define the word in a cell (`: name … ;`), or name a cell in the inspector.",
        topic: "words",
    },
    ErrorHelp {
        patterns: &["is empty"],
        title: "Reference to an empty cell",
        why: "Empty cells are not zero — that would be a silent guess.",
        fix: "Put a value in the cell (0 if you mean zero) or change the reference. If a range is meant to have gaps, end it with `?` (`A1:A100? sum`) to leave its empty cells out.",
        topic: "references",
    },
    ErrorHelp {
        patterns: &["has an error"],
        title: "An input has an error",
        why: "This cell reads a cell that has its own error, so it can't compute.",
        fix: "Fix the source cell — the inspector links to it.",
        topic: "errors",
    },
    ErrorHelp {
        patterns: &["cycle:"],
        title: "Circular reference",
        why: "These cells depend on each other in a loop, so none of them can be computed first. The cycle is highlighted in red.",
        fix: "Break the loop by replacing one of the references with an input.",
        topic: "references",
    },
    ErrorHelp {
        patterns: &["#spill blocked"],
        title: "Spill blocked",
        why: "The cell's result is an array or chart that needs the cells below/right, but one of them isn't empty, or another formula's spill needs some of the same cells. When two spills overlap, both are blocked — neither wins. The blocking cell is outlined in red.",
        fix: "Clear or move the blocking cell, or move one of the formulas so the spills don't overlap.",
        topic: "spill",
    },
    ErrorHelp {
        patterns: &["shapes", "don't match"],
        title: "Array shapes don't match",
        why: "Elementwise words need arrays of the same length (or a single value, which broadcasts).",
        fix: "Check that both ranges cover the same number of cells.",
        topic: "arrays",
    },
    ErrorHelp {
        patterns: &["expects a number", "expects text", "expects a chart", "range mixes text", "range contains"],
        title: "Wrong kind of value",
        why: "The word needs a different kind of value — for example a number where text was given.",
        fix: "Check the order of inputs; step through the cell to see what each value is.",
        topic: "stack",
    },
    ErrorHelp {
        patterns: &["deleted sheet", "sheet was deleted"],
        title: "Reference to a deleted sheet",
        why: "The sheet this reference (or name) pointed into was deleted.",
        fix: "Undo the deletion, or point the reference at a cell on another sheet.",
        topic: "references",
    },
    ErrorHelp {
        patterns: &["deleted cell", "deleted cells", "#ref"],
        title: "Reference to a deleted cell",
        why: "The row or column this reference pointed to was deleted, or moved cells were dropped on the cell.",
        fix: "Undo the deletion or move, or point the reference at another cell.",
        topic: "references",
    },
    ErrorHelp {
        patterns: &["already defined"],
        title: "Defined twice",
        why: "Two cells define the same word, unit or dimension. The first one (in sheet order) wins; the other shows this error.",
        fix: "Rename or delete one of the definitions.",
        topic: "words",
    },
    ErrorHelp {
        patterns: &["nested more than"],
        title: "Words nested too deeply",
        why: "A word calls itself (directly or through others) without stopping.",
        fix: "Words can't recurse; use arrays (`range`, `/op`, `\\op`) instead of loops.",
        topic: "words",
    },
    ErrorHelp {
        patterns: &["end with ;", "missing", "unterminated", "expected", "only appears", "only applies to a range", "is a builtin", "reserved", "only 1 may appear", "a base unit", "a unit"],
        title: "Syntax",
        why: "The text doesn't follow the language's syntax at the highlighted token.",
        fix: "Compare with the reference entry for the word or form you're using.",
        topic: "cells",
    },
    ErrorHelp {
        patterns: &["out of range", "whole number"],
        title: "Index out of range",
        why: "An index or count was outside what the array holds, or not a whole number.",
        fix: "Indexes count from 0 up to `len` − 1.",
        topic: "arrays",
    },
];

pub fn explain_error(msg: &str) -> Option<&'static ErrorHelp> {
    ERRORS.iter().find(|e| e.patterns.iter().any(|p| msg.contains(p)))
}

// ---- search -----------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    Topic(&'static str),
    Word(&'static str),
}

/// Ranked matches across topics and the reference. Exact word names first,
/// then prefixes, titles, summaries and finally body text.
pub fn search(query: &str) -> Vec<(Hit, String)> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return vec![];
    }
    let mut scored: Vec<(u32, Hit, String)> = Vec::new();
    for w in WORDS {
        let name = w.name.to_lowercase();
        let score = if name == q {
            100
        } else if name.starts_with(&q) {
            80
        } else if w.summary.to_lowercase().contains(&q) {
            50
        } else if w.details.to_lowercase().contains(&q) || w.units.to_lowercase().contains(&q) {
            30
        } else {
            0
        };
        if score > 0 {
            scored.push((score, Hit::Word(w.name), format!("{}  {}", w.name, w.summary)));
        }
    }
    for t in topics() {
        let title = t.title.to_lowercase();
        let body = t.body.to_lowercase();
        let score = if title.contains(&q) {
            70
        } else if body.contains(&q) {
            20 + (body.matches(&q).count().min(9) as u32)
        } else {
            0
        };
        if score > 0 {
            let snippet = snippet(t.body, &q);
            scored.push((score, Hit::Topic(t.id), format!("{} — {}", t.title, snippet)));
        }
    }
    // settings are listed on a generated page, so search their declarations
    for s in crate::settings::SETTINGS {
        let score = if s.label.to_lowercase().contains(&q) || s.key.contains(&q) {
            60
        } else if s.help.to_lowercase().contains(&q) {
            25
        } else {
            0
        };
        if score > 0 {
            scored.push((score, Hit::Topic("settings"), format!("Settings — {} ({})", s.label, s.key)));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.into_iter().map(|(_, h, s)| (h, s)).collect()
}

fn snippet(body: &str, q: &str) -> String {
    let plain: String = body.lines().filter(|l| !l.starts_with("```")).collect::<Vec<_>>().join(" ");
    let lower = plain.to_lowercase();
    let Some(i) = lower.find(q) else {
        return plain.chars().take(80).collect();
    };
    let start = plain[..i].char_indices().rev().nth(30).map(|(j, _)| j).unwrap_or(0);
    let s: String = plain[start..].chars().take(90).collect();
    if start > 0 {
        format!("…{s}…")
    } else {
        format!("{s}…")
    }
}

// ---- the sample sheet examples run against ------------------------------------

pub const SAMPLE: &[(&str, &str)] = &[
    ("A1", "1"),
    ("A2", "2"),
    ("A3", "3"),
    ("A4", "4"),
    ("A5", "5"),
    ("B1", "10 [m]"),
    ("B2", "20 [m]"),
    ("B3", "30 [m]"),
    ("B4", "40 [m]"),
    ("B5", "50 [m]"),
    ("C1", "apples"),
    ("C2", "pears"),
    ("C3", "plums"),
    ("D1", "3"),
    ("D2", "1"),
    ("D3", "2"),
    ("E1", "0.05"),
    ("E2", "1000 [USD]"),
    ("G1", "1"),
    ("H1", "2"),
    ("G2", "3"),
    ("F1", "dim widgets"),
    ("F2", "base [widget] widgets"),
    ("F3", ": sq ( x -- x² ) dup * ;"),
    ("F4", ": hyp ( a b -- hypotenuse ) { a b } a sq b sq + sqrt ;"),
];

pub const SAMPLE_NAMES: &[(&str, &str)] = &[("growth", "E1"), ("price", "E2")];

/// A workbook holding the sample data, plus the id of its sample sheet.
pub fn sample_engine() -> (Engine, SheetId) {
    let mut wb = crate::stdlib::default_workbook();
    wb.sheets[0] = Sheet::new("sample");
    let sid = wb.sheets[0].id;
    let mut e = Engine::new(wb);
    for (at, text) in SAMPLE {
        let r = crate::a1::parse_ref(at).unwrap();
        let k = e.wb.sheets[0].key(r.row, r.col).unwrap();
        e.set_text(k, text);
    }
    for (name, at) in SAMPLE_NAMES {
        let r = crate::a1::parse_ref(at).unwrap();
        let k = e.wb.sheets[0].key(r.row, r.col).unwrap();
        e.set_name(name, Some(k), true).unwrap();
    }
    (e, sid)
}

/// Runs an example the way the help shows it: `Ok(summary)` or `Err(message)`.
pub fn run_example(e: &Engine, sheet: SheetId, program: &str) -> Result<String, String> {
    let text = if program.starts_with('=') { program.to_string() } else { format!("={program}") };
    match e.eval_scratch(&text, sheet).result {
        Ok(v) => Ok(v.summary(8)),
        Err(err) => Err(err.msg),
    }
}

/// Does an example's actual outcome match its documented result?
pub fn example_matches(expected: &str, actual: &Result<String, String>) -> bool {
    match (expected.strip_prefix('!'), actual) {
        (Some(want), Err(msg)) => msg.contains(want.trim()),
        (None, Ok(v)) => v == expected,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::BUILTINS;

    #[test]
    fn every_builtin_is_documented() {
        for (name, b, _) in BUILTINS {
            let d = word_doc(name).unwrap_or_else(|| panic!("no help entry for builtin {name}"));
            assert!(!d.summary.is_empty());
            assert_eq!(builtin_doc(*b).name, *name);
        }
    }

    #[test]
    fn see_also_links_resolve() {
        for w in WORDS {
            for s in w.see {
                match s.strip_prefix('#') {
                    Some(id) => assert!(topic(id).is_some(), "{}: no topic {id}", w.name),
                    None => assert!(word_doc(s).is_some(), "{}: no entry {s}", w.name),
                }
            }
        }
    }

    #[test]
    fn reference_examples_are_true() {
        let (e, sid) = sample_engine();
        // the checker itself must be able to fail
        assert!(!example_matches("5", &run_example(&e, sid, "2 2 +")));
        assert!(!example_matches("! nope", &run_example(&e, sid, "2 2 +")));
        assert!(example_matches("! needs matching units", &run_example(&e, sid, "1 [m] 1 [s] +")));
        let n: usize = WORDS.iter().map(|w| w.examples.len()).sum();
        assert!(n > 100, "only {n} examples");
        let mut bad = Vec::new();
        for w in WORDS {
            for (p, want) in w.examples {
                let got = run_example(&e, sid, p);
                if !example_matches(want, &got) {
                    bad.push(format!("{}: `{p}` documented as `{want}`, got {got:?}", w.name));
                }
            }
        }
        assert!(bad.is_empty(), "\n{}", bad.join("\n"));
    }

    #[test]
    fn topic_examples_are_true_and_links_resolve() {
        let (e, sid) = sample_engine();
        let mut bad = Vec::new();
        for t in topics() {
            for b in parse_markdown(t.body) {
                let text = match &b {
                    Block::Para(s) | Block::Bullet(s) | Block::Heading(s) => s.clone(),
                    Block::Table(rows) => rows.iter().map(|(a, b)| format!("{a} {b}")).collect::<Vec<_>>().join(" "),
                    Block::Example(ex) => {
                        for (p, want) in ex {
                            let got = run_example(&e, sid, p);
                            if want.is_empty() || !example_matches(want, &got) {
                                bad.push(format!("{}: `{p}` documented as `{want}`, got {got:?}", t.id));
                            }
                        }
                        String::new()
                    }
                    Block::Generated(g) => {
                        if !DIRECTIVES.contains(&g.as_str()) {
                            bad.push(format!("{}: unknown directive {g}", t.id));
                        }
                        String::new()
                    }
                    Block::Code(_) => String::new(),
                };
                for i in inlines(&text) {
                    if let Inline::Link { target, .. } = i {
                        let ok = match target.strip_prefix('#') {
                            Some(id) => topic(id).is_some(),
                            None => word_doc(&target).is_some(),
                        };
                        if !ok {
                            bad.push(format!("{}: broken link [[{target}]]", t.id));
                        }
                    }
                }
            }
        }
        assert!(bad.is_empty(), "\n{}", bad.join("\n"));
    }

    #[test]
    fn settings_page_lists_every_setting() {
        let body = topic("settings").expect("a settings topic").body;
        assert!(parse_markdown(body).contains(&Block::Generated("settings".into())));
        for s in crate::settings::SETTINGS {
            for i in inlines(s.help) {
                if let Inline::Link { target, .. } = i {
                    assert!(topic(target.trim_start_matches('#')).is_some() || word_doc(&target).is_some(), "{}: broken link {target}", s.key);
                }
            }
        }
        assert!(search("autosave").iter().any(|(h, s)| *h == Hit::Topic("settings") && s.contains("autosave.enabled")));
    }

    #[test]
    fn errors_have_explanations() {
        let (mut e, sid) = sample_engine();
        let k = e.wb.sheet(sid).unwrap().key(10, 0).unwrap();
        for prog in [
            "=1 2 3",
            "=1 drop",
            "=+",
            "=1 [m] 1 [s] +",
            "=1 [m] to[s]",
            "=1 [m] exp",
            "=20 [°C] 20 [°C] +",
            "=1 [furlong]",
            "=frobnicate",
            "=A20",
            "=A1:A3 A1:A2 +",
            "=\"x\" 1 +",
            "=A1:A5 9 pick",
            ": sq dup * ;",
            "=1 \"oops",
            "=A1? sum",
            "=A6:A9 sum",
        ] {
            e.set_text(k, prog);
            let msg = match e.shown(k) {
                crate::engine::Shown::Error(err) => err.msg.clone(),
                _ => panic!("{prog} should fail"),
            };
            assert!(explain_error(&msg).is_some(), "no explanation for `{prog}` → {msg}");
        }
        // an input outside its range, and a range that no longer fits its input
        e.set_text(k, "1");
        e.set_name("damping", Some(k), true).unwrap();
        e.set_range("damping", "0", "").unwrap();
        for text in ["-1", "1 [m]"] {
            e.set_text(k, text);
            let crate::engine::Shown::Error(err) = e.shown(k) else { panic!("{text} should fail") };
            assert!(explain_error(&err.msg).is_some_and(|h| h.title.contains("range")), "{text} → {}", err.msg);
        }
    }

    #[test]
    fn markdown() {
        let b = parse_markdown("## Hi\n\nsome `code` and [[sum]]\nmore\n\n- one\n  cont\n```example\n1 2 + ⇒ 3\n```\n| a | b |\n|---|---|\n| c | d |\n{{errors}}\n");
        assert_eq!(b[0], Block::Heading("Hi".into()));
        assert_eq!(b[1], Block::Para("some `code` and [[sum]] more".into()));
        assert_eq!(b[2], Block::Bullet("one cont".into()));
        assert_eq!(b[3], Block::Example(vec![("1 2 +".into(), "3".into())]));
        assert_eq!(b[4], Block::Table(vec![("a".into(), "b".into()), ("c".into(), "d".into())]));
        assert_eq!(b[5], Block::Generated("errors".into()));
        assert_eq!(
            inlines("use `dup` or [[#stack|the stack]]"),
            vec![
                Inline::Text("use ".into()),
                Inline::Code("dup".into()),
                Inline::Text(" or ".into()),
                Inline::Link { target: "#stack".into(), label: "the stack".into() }
            ]
        );
    }

    #[test]
    fn search_ranks_exact_words_first() {
        let r = search("sum");
        assert_eq!(r[0].0, Hit::Word("sum"));
        let r = search("temperature");
        assert!(r.iter().any(|(h, _)| *h == Hit::Topic("absolute")));
    }
}
