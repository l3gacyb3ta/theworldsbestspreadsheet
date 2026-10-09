//! Tokenizer for cell programs. Tokens are whitespace separated, except that
//! `[unit]`, `to[unit]` and `"strings"` delimit themselves, so `5[m]` works.
//! Every token carries its byte span in the source so errors and editor
//! highlighting can point at it.

use crate::a1::{self, A1Ref};
use std::ops::Range;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Num(f64),
    /// ISO date literal, days since 1970-01-01.
    Date(i64),
    Str(String),
    /// `[m/s]` — contents without brackets.
    Unit(String),
    /// `to[km/h]`
    To(String),
    Ref(A1Ref),
    Range(A1Ref, A1Ref),
    /// A reference whose row/column was deleted.
    DeadRef,
    /// `/+`, `/max`, `/myword`
    Reduce(String),
    /// `\+`
    Scan(String),
    /// Any other chunk: builtin words, operators, user words, names, `:`, `;`, `{`, `}`.
    Word(String),
    /// `( stack comment )` — a lone `(` up to the next `)`. Ignored by the compiler.
    Comment(String),
    /// Unterminated string/bracket etc.
    Bad(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub span: Range<usize>,
}

pub fn parse_number(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    if b.is_empty() {
        return None;
    }
    let first = if b[0] == b'-' || b[0] == b'+' { b.get(1) } else { b.first() };
    match first {
        Some(c) if c.is_ascii_digit() || *c == b'.' => {}
        _ => return None,
    }
    if s.contains("_") {
        return s.replace('_', "").parse().ok();
    }
    let v: f64 = s.parse().ok()?;
    if v.is_finite() {
        Some(v)
    } else {
        None
    }
}

/// `YYYY-MM-DD` → days since epoch.
pub fn parse_date(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let y: i64 = s[0..4].parse().ok()?;
    let m: i64 = s[5..7].parse().ok()?;
    let d: i64 = s[8..10].parse().ok()?;
    if !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

pub fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
    }
}

// Howard Hinnant's civil date algorithms.
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn classify(chunk: &str) -> Tok {
    if let Some(v) = parse_number(chunk) {
        return Tok::Num(v);
    }
    if let Some(d) = parse_date(chunk) {
        return Tok::Date(d);
    }
    if chunk == "#ref!" {
        return Tok::DeadRef;
    }
    if chunk.len() > 1 {
        if let Some(rest) = chunk.strip_prefix('/') {
            return Tok::Reduce(rest.to_string());
        }
        if let Some(rest) = chunk.strip_prefix('\\') {
            return Tok::Scan(rest.to_string());
        }
    }
    if let Some((a, b)) = a1::parse_ref_or_range(chunk) {
        return match b {
            Some(b) => Tok::Range(a, b),
            None => Tok::Ref(a),
        };
    }
    Tok::Word(chunk.to_string())
}

/// Tokens without comments: what the compiler and cell classification see.
pub fn lex_code(src: &str) -> Vec<Token> {
    let mut v = lex(src);
    v.retain(|t| !matches!(t.tok, Tok::Comment(_)));
    v
}

pub fn lex(src: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if out.is_empty() && (c == b'=' || c == b':') {
            // program / word-definition marker is always its own token
            out.push(Token { tok: Tok::Word((c as char).to_string()), span: i..i + 1 });
            i += 1;
            continue;
        }
        if c == b'(' && b.get(i + 1).is_none_or(|n| n.is_ascii_whitespace()) {
            match src[i + 1..].find(')') {
                Some(j) => {
                    let end = i + 1 + j + 1;
                    out.push(Token { tok: Tok::Comment(src[i + 1..end - 1].trim().to_string()), span: start..end });
                    i = end;
                }
                None => {
                    out.push(Token { tok: Tok::Bad("missing ) to end the comment".into()), span: start..b.len() });
                    i = b.len();
                }
            }
            continue;
        }
        if c == b'"' {
            match src[i + 1..].find('"') {
                Some(j) => {
                    let end = i + 1 + j + 1;
                    out.push(Token { tok: Tok::Str(src[i + 1..end - 1].to_string()), span: start..end });
                    i = end;
                }
                None => {
                    out.push(Token { tok: Tok::Bad("unterminated string".into()), span: start..b.len() });
                    i = b.len();
                }
            }
            continue;
        }
        let is_to = src[i..].starts_with("to[");
        if c == b'[' || is_to {
            let open = if is_to { i + 2 } else { i };
            match src[open..].find(']') {
                Some(j) => {
                    let end = open + j + 1;
                    let inner = src[open + 1..end - 1].to_string();
                    let tok = if is_to { Tok::To(inner) } else { Tok::Unit(inner) };
                    out.push(Token { tok, span: start..end });
                    i = end;
                }
                None => {
                    out.push(Token { tok: Tok::Bad("missing ]".into()), span: start..b.len() });
                    i = b.len();
                }
            }
            continue;
        }
        // plain chunk; a leading quoted sheet name may contain spaces
        if c == b'\'' {
            i += 1;
            while i < b.len() {
                if b[i] == b'\'' {
                    if b.get(i + 1) == Some(&b'\'') {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
        }
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'[' && b[i] != b'"' {
            i += 1;
        }
        out.push(Token { tok: classify(&src[start..i]), span: start..i });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn toks(s: &str) -> Vec<Tok> {
        lex(s).into_iter().map(|t| t.tok).collect()
    }
    #[test]
    fn basics() {
        assert_eq!(
            toks("A1:A10 /+ 5[m/s] to[km/h] -3 - \"hi there\""),
            vec![
                Tok::Range(a1::parse_ref("A1").unwrap(), a1::parse_ref("A10").unwrap()),
                Tok::Reduce("+".into()),
                Tok::Num(5.0),
                Tok::Unit("m/s".into()),
                Tok::To("km/h".into()),
                Tok::Num(-3.0),
                Tok::Word("-".into()),
                Tok::Str("hi there".into()),
            ]
        );
        assert_eq!(toks("/ \\+"), vec![Tok::Word("/".into()), Tok::Scan("+".into())]);
        assert_eq!(toks("'my sheet'!B2"), vec![Tok::Ref(a1::parse_ref("'my sheet'!B2").unwrap())]);
        assert_eq!(
            toks(": sq ( x -- x² ) dup * ;")[2..4],
            [Tok::Comment("x -- x²".into()), Tok::Word("dup".into())]
        );
        assert_eq!(toks("(m"), vec![Tok::Word("(m".into())]);
    }
    #[test]
    fn dates() {
        assert_eq!(parse_date("1970-01-01"), Some(0));
        assert_eq!(parse_date("2000-03-01"), Some(11017));
        assert_eq!(civil_from_days(11017), (2000, 3, 1));
        assert_eq!(parse_date("2023-02-29"), None);
    }
}
