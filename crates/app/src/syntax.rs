//! Formula text analysis for the editor: reference colors and token styling.

use eframe::egui::{text::LayoutJob, Color32, FontId, Stroke, TextFormat};
use std::ops::Range;
use wbs_core::ids::SheetId;
use wbs_core::lex::{self, Tok};
use wbs_core::model::{classify, Kind, Workbook};
use wbs_core::parse::builtin;

pub const REF_COLORS: [Color32; 8] = [
    Color32::from_rgb(0x25, 0x63, 0xeb),
    Color32::from_rgb(0xdc, 0x26, 0x26),
    Color32::from_rgb(0x16, 0xa3, 0x4a),
    Color32::from_rgb(0x93, 0x33, 0xea),
    Color32::from_rgb(0xea, 0x58, 0x0c),
    Color32::from_rgb(0x08, 0x91, 0xb2),
    Color32::from_rgb(0xdb, 0x27, 0x77),
    Color32::from_rgb(0x65, 0xa3, 0x0d),
];

pub const C_NUM: Color32 = Color32::from_rgb(0x1d, 0x4e, 0xd8);
pub const C_UNIT: Color32 = Color32::from_rgb(0x0f, 0x76, 0x6e);
pub const C_STR: Color32 = Color32::from_rgb(0x15, 0x80, 0x3d);
pub const C_WORD: Color32 = Color32::from_rgb(0x7e, 0x22, 0xce);
pub const C_DIM: Color32 = Color32::from_rgb(0x6b, 0x72, 0x80);
pub const C_ERR: Color32 = Color32::from_rgb(0xdc, 0x26, 0x26);

/// A reference in the text and the rectangle of positions it points at.
#[derive(Clone, Debug)]
pub struct RefHi {
    pub span: Range<usize>,
    pub color: Color32,
    pub sheet: SheetId,
    pub r0: usize,
    pub c0: usize,
    pub r1: usize,
    pub c1: usize,
}

pub fn analyze(text: &str, wb: &Workbook, home: SheetId) -> Vec<RefHi> {
    if !classify(text).has_refs() {
        return vec![];
    }
    let mut out: Vec<RefHi> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for t in lex::lex(text) {
        let src = &text[t.span.clone()];
        let target = match &t.tok {
            Tok::Ref(r) => {
                let sid = match &r.sheet {
                    Some(n) => wb.sheet_by_name(n).map(|s| s.id),
                    None => Some(home),
                };
                sid.map(|s| (s, r.row, r.col, r.row, r.col))
            }
            Tok::Range(a, b) => {
                let sid = match &a.sheet {
                    Some(n) => wb.sheet_by_name(n).map(|s| s.id),
                    None => Some(home),
                };
                sid.map(|s| (s, a.row.min(b.row), a.col.min(b.col), a.row.max(b.row), a.col.max(b.col)))
            }
            Tok::Word(w) if builtin(w).is_none() => wb.names.get(w.as_str()).and_then(|n| {
                let (r, c) = wb.pos(n.cell)?;
                Some((n.cell.sheet, r, c, r, c))
            }),
            _ => None,
        };
        let Some((sheet, r0, c0, r1, c1)) = target else { continue };
        let key = src.trim_start_matches('$').replace('$', "");
        let idx = match seen.iter().position(|s| *s == key) {
            Some(i) => i,
            None => {
                seen.push(key);
                seen.len() - 1
            }
        };
        out.push(RefHi { span: t.span, color: REF_COLORS[idx % REF_COLORS.len()], sheet, r0, c0, r1, c1 });
    }
    out
}

/// The token under (or ending right at) byte offset `at`.
pub fn token_at(text: &str, at: usize) -> Option<lex::Token> {
    lex::lex(text).into_iter().find(|t| t.span.start <= at && at <= t.span.end && !t.span.is_empty())
}

pub fn char_to_byte(text: &str, ci: usize) -> usize {
    text.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(text.len())
}

pub fn layout(text: &str, refs: &[RefHi], err: Option<&Range<usize>>, font: FontId, base: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let kind = classify(text);
    let mut styles: Vec<(Range<usize>, Color32)> = Vec::new();
    if kind.has_refs() || kind == Kind::Number {
        for t in lex::lex(text) {
            let c = match &t.tok {
                Tok::Num(_) | Tok::Date(_) => C_NUM,
                Tok::Unit(_) | Tok::To(_) => C_UNIT,
                Tok::Str(_) => C_STR,
                Tok::Reduce(_) | Tok::Scan(_) => C_WORD,
                Tok::Word(w) if w == "=" || w == ":" || w == ";" || w == "{" || w == "}" => C_DIM,
                Tok::Word(w) if builtin(w).is_some() => C_WORD,
                Tok::Bad(_) | Tok::DeadRef => C_ERR,
                Tok::Comment(_) => C_DIM,
                _ => base,
            };
            styles.push((t.span, c));
        }
        for r in refs {
            if let Some(s) = styles.iter_mut().find(|s| s.0 == r.span) {
                s.1 = r.color;
            }
        }
    }
    let mut pos = 0;
    let push = |job: &mut LayoutJob, range: Range<usize>, color: Color32| {
        if range.is_empty() {
            return;
        }
        let mut fmt = TextFormat { font_id: font.clone(), color, ..Default::default() };
        if let Some(e) = err {
            if range.start >= e.start && range.end <= e.end {
                fmt.background = Color32::from_rgba_unmultiplied(0xdc, 0x26, 0x26, 40);
                fmt.underline = Stroke::new(1.5, C_ERR);
            }
        }
        job.append(&text[range], 0.0, fmt);
    };
    for (span, color) in styles {
        if span.start < pos {
            continue;
        }
        push(&mut job, pos..span.start, base);
        push(&mut job, span.clone(), color);
        pos = span.end;
    }
    push(&mut job, pos..text.len(), base);
    job
}
