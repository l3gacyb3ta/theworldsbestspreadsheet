//! A1 notation: purely an input/display format over positions.

#[derive(Clone, Debug, PartialEq)]
pub struct A1Ref {
    pub sheet: Option<String>,
    pub col: usize,
    pub row: usize,
    pub col_abs: bool,
    pub row_abs: bool,
}

pub fn col_name(mut i: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (i % 26) as u8);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}

pub fn cell_name(row: usize, col: usize) -> String {
    format!("{}{}", col_name(col), row + 1)
}

pub fn sheet_prefix(name: &str) -> String {
    let plain = !name.is_empty()
        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !name.chars().next().unwrap().is_ascii_digit();
    if plain {
        format!("{name}!")
    } else {
        format!("'{}'!", name.replace('\'', "''"))
    }
}

pub fn format_ref(r: &A1Ref) -> String {
    let mut s = String::new();
    if let Some(sh) = &r.sheet {
        s.push_str(&sheet_prefix(sh));
    }
    if r.col_abs {
        s.push('$');
    }
    s.push_str(&col_name(r.col));
    if r.row_abs {
        s.push('$');
    }
    s.push_str(&(r.row + 1).to_string());
    s
}

/// Parse `$A$1`-style cell part (no sheet).
fn parse_cell_part(s: &str) -> Option<(usize, usize, bool, bool)> {
    let b = s.as_bytes();
    let mut i = 0;
    let col_abs = b.first() == Some(&b'$');
    if col_abs {
        i += 1;
    }
    let cs = i;
    while i < b.len() && b[i].is_ascii_uppercase() {
        i += 1;
    }
    let letters = &s[cs..i];
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    let row_abs = b.get(i) == Some(&b'$');
    if row_abs {
        i += 1;
    }
    let ds = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i != b.len() || ds == i || b[ds] == b'0' {
        return None;
    }
    let row: usize = s[ds..].parse().ok()?;
    let mut col = 0usize;
    for c in letters.bytes() {
        col = col * 26 + (c - b'A' + 1) as usize;
    }
    Some((row - 1, col - 1, col_abs, row_abs))
}

/// Split an optional `Sheet!` / `'Sheet name'!` prefix. Returns (sheet, rest).
fn split_sheet(s: &str) -> Option<(Option<String>, &str)> {
    if let Some(stripped) = s.strip_prefix('\'') {
        // quoted sheet name, '' escapes a quote
        let mut name = String::new();
        let mut chars = stripped.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == '\'' {
                if let Some((_, '\'')) = chars.peek() {
                    chars.next();
                    name.push('\'');
                    continue;
                }
                let rest = &stripped[i + 1..];
                return rest.strip_prefix('!').map(|r| (Some(name), r));
            }
            name.push(c);
        }
        return None;
    }
    match s.find('!') {
        Some(i) => {
            let name = &s[..i];
            if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return None;
            }
            Some((Some(name.to_string()), &s[i + 1..]))
        }
        None => Some((None, s)),
    }
}

pub fn parse_ref(s: &str) -> Option<A1Ref> {
    let (sheet, rest) = split_sheet(s)?;
    let (row, col, col_abs, row_abs) = parse_cell_part(rest)?;
    Some(A1Ref { sheet, col, row, col_abs, row_abs })
}

/// `A1` or `A1:B5` (sheet prefix only on the first corner).
pub fn parse_ref_or_range(s: &str) -> Option<(A1Ref, Option<A1Ref>)> {
    let (sheet, rest) = split_sheet(s)?;
    match rest.split_once(':') {
        Some((a, b)) => {
            let (r1, c1, ca1, ra1) = parse_cell_part(a)?;
            let (r2, c2, ca2, ra2) = parse_cell_part(b)?;
            Some((
                A1Ref { sheet: sheet.clone(), col: c1, row: r1, col_abs: ca1, row_abs: ra1 },
                Some(A1Ref { sheet, col: c2, row: r2, col_abs: ca2, row_abs: ra2 }),
            ))
        }
        None => {
            let (row, col, col_abs, row_abs) = parse_cell_part(rest)?;
            Some((A1Ref { sheet, col, row, col_abs, row_abs }, None))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names() {
        assert_eq!(col_name(0), "A");
        assert_eq!(col_name(25), "Z");
        assert_eq!(col_name(26), "AA");
        assert_eq!(col_name(27), "AB");
        assert_eq!(col_name(701), "ZZ");
        assert_eq!(col_name(702), "AAA");
    }
    #[test]
    fn parse() {
        let r = parse_ref("$B$12").unwrap();
        assert_eq!((r.row, r.col, r.col_abs, r.row_abs), (11, 1, true, true));
        let r = parse_ref("rates!AA3").unwrap();
        assert_eq!(r.sheet.as_deref(), Some("rates"));
        assert_eq!((r.row, r.col), (2, 26));
        let r = parse_ref("'my sheet'!C1").unwrap();
        assert_eq!(r.sheet.as_deref(), Some("my sheet"));
        assert!(parse_ref("growth").is_none());
        assert!(parse_ref("A0").is_none());
        assert!(parse_ref("a1").is_none());
        let (a, b) = parse_ref_or_range("A1:B$3").unwrap();
        assert_eq!(a.row, 0);
        assert_eq!(b.unwrap().row_abs, true);
    }
}
