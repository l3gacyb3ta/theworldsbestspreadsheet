//! Charts are values. A program builds one by composing words
//! (`xs ys line`, `layer`, `title`, ...) and it spills into a block of cells.

use crate::value::{Num, Text};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mark {
    Line,
    Scatter,
    Bar,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Xs {
    Num(Num),
    Text(Text),
}

impl Xs {
    pub fn len(&self) -> usize {
        match self {
            Xs::Num(n) => n.len(),
            Xs::Text(t) => t.data.len(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub mark: Mark,
    pub xs: Xs,
    pub ys: Num,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chart {
    pub layers: Vec<Layer>,
    pub title: Option<String>,
    pub xlabel: Option<String>,
    pub ylabel: Option<String>,
    /// Spill region size in cells.
    pub rows: usize,
    pub cols: usize,
}

pub const DEFAULT_ROWS: usize = 14;
pub const DEFAULT_COLS: usize = 6;

impl Chart {
    pub fn single(layer: Layer) -> Chart {
        Chart { layers: vec![layer], title: None, xlabel: None, ylabel: None, rows: DEFAULT_ROWS, cols: DEFAULT_COLS }
    }
    pub fn x_label(&self) -> String {
        if let Some(l) = &self.xlabel {
            return l.clone();
        }
        match self.layers.first().map(|l| &l.xs) {
            Some(Xs::Num(n)) if !n.q.disp.is_none() => format!("[{}]", n.q.disp),
            _ => String::new(),
        }
    }
    pub fn y_label(&self) -> String {
        if let Some(l) = &self.ylabel {
            return l.clone();
        }
        match self.layers.first() {
            Some(l) if !l.ys.q.disp.is_none() => format!("[{}]", l.ys.q.disp),
            _ => String::new(),
        }
    }
}
