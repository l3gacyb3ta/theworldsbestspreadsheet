//! Stable identifiers. Rows, columns and sheets get random 64-bit ids so that
//! independent writers (a future Automerge doc) never collide; positions are
//! only ever derived from the ordered id lists in a `Sheet`.

use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct RowId(pub u64);
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct ColId(pub u64);
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct SheetId(pub u64);

/// A cell is addressed by (sheet, row id, column id). It survives inserts,
/// deletes, moves and sorts of other rows and columns.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct CellKey {
    pub sheet: SheetId,
    pub row: RowId,
    pub col: ColId,
}

thread_local! {
    static RNG: Cell<u64> = Cell::new({
        let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
        t ^ 0x9E37_79B9_7F4A_7C15
    });
}

pub fn fresh_id() -> u64 {
    RNG.with(|r| {
        // splitmix64
        let mut z = r.get().wrapping_add(0x9E37_79B9_7F4A_7C15);
        r.set(z);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    })
}
