//! Core of the spreadsheet: document model, stack language, units, and the
//! dependency graph. No UI code lives here.

pub mod a1;
pub mod bounds;
pub mod chart;
pub mod dims;
pub mod engine;
pub mod eval;
pub mod help;
pub mod ids;
pub mod lex;
pub mod model;
pub mod ops;
pub mod parse;
pub mod poskey;
pub mod rational;
pub mod settings;
pub mod solve;
pub mod stdlib;
pub mod units;
pub mod value;

pub use engine::{CellError, Edit, Engine, ErrKind, Shown};
pub use ids::CellKey;
