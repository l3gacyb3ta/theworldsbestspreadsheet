//! Paste CSV: comma (or `;`, or tab) separated text, parsed with RFC 4180 quoting, each field typed
//! into a cell by the same rules as typing it, except that nothing pastes as a program or declaration.

use crate::engine::Edit;
use crate::ids::SheetId;
use crate::lex::{self, Tok};
use crate::model::{classify, Cell, Kind};
use crate::Engine;

/// The field separators Paste CSV offers. Many European exports use `;`, since `,` is their decimal mark.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Sep {
    #[default]
    Comma,
    Semicolon,
    Tab,
}

impl Sep {
    pub const ALL: [Sep; 3] = [Sep::Comma, Sep::Semicolon, Sep::Tab];

    pub fn char(self) -> char {
        match self {
            Sep::Comma => ',',
            Sep::Semicolon => ';',
            Sep::Tab => '\t',
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Sep::Comma => "comma",
            Sep::Semicolon => "semicolon",
            Sep::Tab => "tab",
        }
    }
}

/// Splits CSV text into rows of fields. A field in double quotes can hold the separator, newlines and
/// `""` (one quote); spaces around an unquoted field, and around the quotes of a quoted one, are dropped.
/// Lines end with `\n` or `\r\n`; a leading byte order mark and a final line break are ignored. Rows
/// keep the number of fields they have (ragged rows stay ragged).
pub fn parse(text: &str, sep: Sep) -> Vec<Vec<String>> {
    let sep = sep.char();
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    // the field was quoted (its content is kept exactly), and we're inside the quotes
    let (mut quoted, mut inside) = (false, false);
    let mut chars = text.chars().peekable();
    let end_field = |field: &mut String, quoted: &mut bool, row: &mut Vec<String>| {
        let f = std::mem::take(field);
        row.push(if *quoted { f } else { f.trim_matches(' ').to_string() });
        *quoted = false;
    };
    while let Some(ch) = chars.next() {
        if inside {
            match ch {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => inside = false,
                '\r' if chars.peek() == Some(&'\n') => {}
                _ => field.push(ch),
            }
            continue;
        }
        match ch {
            c if c == sep => end_field(&mut field, &mut quoted, &mut row),
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                end_field(&mut field, &mut quoted, &mut row);
                rows.push(std::mem::take(&mut row));
            }
            // a quote opens a quoted field only at its start (after any spaces)
            '"' if !quoted && field.trim_matches(' ').is_empty() => {
                field.clear();
                quoted = true;
                inside = true;
            }
            // spaces after the closing quote
            ' ' if quoted => {}
            _ => field.push(ch),
        }
    }
    if quoted || !field.is_empty() || !row.is_empty() {
        end_field(&mut field, &mut quoted, &mut row);
        rows.push(row);
    }
    rows
}

/// What typing a field into a cell gives: `None` for an empty field, the field itself where it reads as a
/// number, date or text, and `'`-quoted text where it would read as a program or declaration (`=A1`,
/// `dim x`) or starts with `'` or a space. In the header row (`header`) numbers are quoted too. So a paste never runs
/// anything, and text stays the text it was. With a column `unit`, a plain number gets it (`42` → `42 [USD]`);
/// a number with its own unit, a date and text are left as they are.
pub fn cell_text(field: &str, header: bool, unit: Option<&str>) -> Option<String> {
    match classify(field) {
        Kind::Empty => None,
        Kind::Number if !header => Some(match unit {
            Some(u) if is_plain_number(field) => format!("{} [{u}]", field.trim()),
            _ => field.trim().to_string(),
        }),
        Kind::Text if !field.starts_with(['\'', ' ', '\t']) => Some(field.to_string()),
        _ => Some(format!("'{field}")),
    }
}

/// A number without a unit: not `5 [m]`, not a date.
fn is_plain_number(field: &str) -> bool {
    matches!(lex::lex_code(field.trim()).as_slice(), [t] if matches!(t.tok, Tok::Num(_)))
}

/// A unit as typed for a column: `USD` or `[USD]`; `None` when blank.
pub fn column_unit(typed: &str) -> Option<&str> {
    let u = typed.trim();
    let u = u.strip_prefix('[').and_then(|u| u.strip_suffix(']')).unwrap_or(u).trim();
    (!u.is_empty()).then_some(u)
}

/// How many fields of column `col` (header row aside) a unit would go on, and how many it wouldn't
/// because they're text, dates or numbers with a unit already.
pub fn unit_coverage(rows: &[Vec<String>], header: bool, col: usize) -> (usize, usize) {
    let (mut gets, mut not) = (0, 0);
    for f in rows.iter().skip(header as usize).filter_map(|r| r.get(col)) {
        match classify(f) {
            Kind::Empty => {}
            Kind::Number if is_plain_number(f) => gets += 1,
            _ => not += 1,
        }
    }
    (gets, not)
}

/// The cells of `rows` with their top-left at `at`, as one edit; `header` pastes the first row as text,
/// and `units[j]` (a unit as `column_unit` reads it, or blank) goes on column j's plain numbers.
/// Empty fields clear their cell.
pub fn paste(e: &Engine, rows: &[Vec<String>], header: bool, units: &[String], sheet: SheetId, at: (usize, usize)) -> Edit {
    let s = e.wb.sheet(sheet).unwrap();
    let mut out = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        for (j, field) in row.iter().enumerate() {
            let Some(k) = s.key(at.0 + i, at.1 + j) else { continue };
            let unit = units.get(j).and_then(|u| column_unit(u));
            out.push((k, cell_text(field, header && i == 0, unit).map(|t| Cell::new(e.wb.parse_text(&t, sheet)))));
        }
    }
    Edit::Cells(out)
}

/// (rows, columns) the pasted block covers.
pub fn size(rows: &[Vec<String>]) -> (usize, usize) {
    (rows.len(), rows.iter().map(|r| r.len()).max().unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(text: &str) -> Vec<Vec<String>> {
        parse(text, Sep::Comma)
    }
    fn rows(r: &[&[&str]]) -> Vec<Vec<String>> {
        r.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect()
    }

    #[test]
    fn plain_fields_and_line_endings() {
        assert_eq!(p("a,b,c\n1,2,3\n"), rows(&[&["a", "b", "c"], &["1", "2", "3"]]));
        assert_eq!(p("a,b\r\n1,2\r\n"), rows(&[&["a", "b"], &["1", "2"]]));
        // no final line break
        assert_eq!(p("a,b\n1,2"), rows(&[&["a", "b"], &["1", "2"]]));
        // spaces around unquoted fields go
        assert_eq!(p("Smith, John, 42"), rows(&[&["Smith", "John", "42"]]));
        // empty fields and trailing separators stay
        assert_eq!(p("a,,c,\n"), rows(&[&["a", "", "c", ""]]));
        // a blank line is a row with one empty field
        assert_eq!(p("a\n\nb"), rows(&[&["a"], &[""], &["b"]]));
        assert_eq!(p(""), Vec::<Vec<String>>::new());
    }

    #[test]
    fn quoting() {
        assert_eq!(p(r#""a, b",c"#), rows(&[&["a, b", "c"]]));
        assert_eq!(p(r#""say ""hi""",x"#), rows(&[&[r#"say "hi""#, "x"]]));
        assert_eq!(p("\"two\nlines\",x\ny,z"), rows(&[&["two\nlines", "x"], &["y", "z"]]));
        assert_eq!(p("\"two\r\nlines\",x\r\n"), rows(&[&["two\nlines", "x"]]));
        // a quoted field keeps its spaces; spaces outside the quotes go
        assert_eq!(p(r#"a, " b " ,c"#), rows(&[&["a", " b ", "c"]]));
        assert_eq!(p(r#""",x"#), rows(&[&["", "x"]]));
        assert_eq!(p(r#""""#), rows(&[&[""]]));
        // a quote inside an unquoted field is just a character
        assert_eq!(p(r#"5'10",x"#), rows(&[&[r#"5'10""#, "x"]]));
    }

    #[test]
    fn ragged_rows_bom_and_separators() {
        assert_eq!(p("a,b,c\n1\n1,2"), rows(&[&["a", "b", "c"], &["1"], &["1", "2"]]));
        assert_eq!(size(&p("a,b,c\n1\n1,2")), (3, 3));
        assert_eq!(p("\u{feff}a,b"), rows(&[&["a", "b"]]));
        assert_eq!(parse("1,5;2,5\n\"a;b\";c", Sep::Semicolon), rows(&[&["1,5", "2,5"], &["a;b", "c"]]));
        assert_eq!(parse("a b\tc,d\n", Sep::Tab), rows(&[&["a b", "c,d"]]));
    }

    #[test]
    fn fields_become_cells_by_the_typing_rules() {
        let t = |f: &str| cell_text(f, false, None);
        let s = |x: &str| Some(x.to_string());
        // numbers and ISO dates
        assert_eq!(t("42"), s("42"));
        assert_eq!(t("-3.5"), s("-3.5"));
        assert_eq!(t("1e6"), s("1e6"));
        assert_eq!(t("2026-10-11"), s("2026-10-11"));
        assert_eq!(t(" 7 "), s("7"));
        assert_eq!(t("5 [m]"), s("5 [m]"));
        assert_eq!(classify(&t("2026-10-11").unwrap()), Kind::Number);
        // nothing else is guessed: these are text
        for f in ["1,234.5", "$12", "12%", "10/11/2026", "TRUE", "Smith", "two\nlines"] {
            assert_eq!(t(f), s(f), "{f}");
            assert_eq!(classify(f), Kind::Text, "{f}");
        }
        // what would run or declare something pastes as text
        for f in ["=A1", "= 1 2 +", ": sq dup * ;", "dim widgets", "base [w] widgets", "[mi] = 1609.344 [m]"] {
            let c = t(f).unwrap();
            assert_eq!(c, format!("'{f}"));
            assert_eq!(classify(&c), Kind::Text);
        }
        // text that starts with a quote or a space keeps it
        assert_eq!(t("'quoted"), s("''quoted"));
        assert_eq!(t(" b "), s("' b "));
        assert_eq!(t(""), None);
        assert_eq!(t("  "), None);
    }

    #[test]
    fn a_header_row_is_text() {
        assert_eq!(cell_text("2026", true, None), Some("'2026".into()));
        assert_eq!(cell_text("Name", true, None), Some("Name".into()));
        assert_eq!(cell_text("=x", true, None), Some("'=x".into()));
        assert_eq!(cell_text("", true, None), None);
    }

    #[test]
    fn paste_is_one_undoable_edit_at_the_top_left() {
        use crate::engine::Shown;
        use crate::stdlib::default_workbook;
        let mut e = Engine::new(default_workbook());
        let s = e.wb.sheets[0].id;
        let k = |e: &Engine, r, c| e.wb.sheet(s).unwrap().key(r, c).unwrap();
        e.set_text(k(&e, 2, 1), "old");
        e.set_text(k(&e, 0, 0), "10");
        let text = "when,n,note\r\n2026-10-11,42,=A1\r\n\"2026-10-12\",\"1,234.5\",\r\n";
        let ed = paste(&e, &parse(text, Sep::Comma), true, &[], s, (1, 1));
        let inv = e.apply(ed);
        let text_at = |e: &Engine, r, c| e.wb.cell_text(k(e, r, c));
        assert_eq!((text_at(&e, 1, 1), text_at(&e, 1, 2), text_at(&e, 1, 3)), ("when".into(), "n".into(), "note".into()));
        assert_eq!(text_at(&e, 2, 1), "2026-10-11");
        assert_eq!(text_at(&e, 2, 3), "'=A1");
        assert_eq!(text_at(&e, 3, 2), "1,234.5");
        // the empty last field clears what was there
        assert_eq!(text_at(&e, 3, 3), "");
        assert_eq!(e.kind(k(&e, 2, 2)), Kind::Number);
        assert_eq!(e.kind(k(&e, 2, 3)), Kind::Text);
        match e.shown(k(&e, 2, 3)) {
            Shown::Value { value, .. } => assert_eq!(value.display_at(0, 0), "=A1"),
            _ => panic!("no value"),
        }
        e.apply(inv);
        assert_eq!(text_at(&e, 2, 1), "old");
        assert_eq!(text_at(&e, 1, 1), "");
        assert_eq!(text_at(&e, 0, 0), "10");
    }

    #[test]
    fn a_column_unit_goes_on_plain_numbers_only() {
        let t = |f: &str| cell_text(f, false, Some("USD"));
        assert_eq!(t("42"), Some("42 [USD]".into()));
        assert_eq!(t(" -3.5 "), Some("-3.5 [USD]".into()));
        assert_eq!(t("5 [m]"), Some("5 [m]".into()));
        assert_eq!(t("2026-10-11"), Some("2026-10-11".into()));
        assert_eq!(t("$12"), Some("$12".into()));
        assert_eq!(t("=A1"), Some("'=A1".into()));
        assert_eq!(cell_text("42", true, Some("USD")), Some("'42".into()));
        assert_eq!((column_unit(" [USD] "), column_unit("m/s"), column_unit(" "), column_unit("[]")), (Some("USD"), Some("m/s"), None, None));
        let rows = parse("price\n1\n2\nn/a\n\n2026-01-01\n3 [EUR]", Sep::Comma);
        assert_eq!(unit_coverage(&rows, true, 0), (2, 3));
        assert_eq!(unit_coverage(&rows, false, 0), (2, 4));
        assert_eq!(unit_coverage(&rows, false, 5), (0, 0));
    }

    #[test]
    fn pasting_with_a_column_unit_gives_quantities() {
        use crate::engine::Shown;
        use crate::stdlib::default_workbook;
        let mut e = Engine::new(default_workbook());
        let s = e.wb.sheets[0].id;
        let k = |e: &Engine, r, c| e.wb.sheet(s).unwrap().key(r, c).unwrap();
        assert!(e.check_unit("km").is_ok());
        assert!(e.check_unit("m/s^2").is_ok());
        assert!(e.check_unit("furlong").is_err());
        assert!(e.check_unit("m*").is_err());
        let rows = parse("d,t\n5,2\n7,x", Sep::Comma);
        let ed = paste(&e, &rows, true, &["[km]".into(), "".into()], s, (0, 0));
        e.apply(ed);
        assert_eq!(e.wb.cell_text(k(&e, 1, 0)), "5 [km]");
        assert_eq!(e.wb.cell_text(k(&e, 1, 1)), "2");
        assert_eq!(e.wb.cell_text(k(&e, 0, 0)), "d");
        match e.shown(k(&e, 2, 0)) {
            Shown::Value { value, .. } => assert_eq!(value.display_at(0, 0), "7 km"),
            _ => panic!("no value"),
        }
    }
}
