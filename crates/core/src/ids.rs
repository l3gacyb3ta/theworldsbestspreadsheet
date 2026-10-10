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
    static RNG: Cell<u64> = Cell::new(seed());
}

/// OS entropy, so two processes started at the same instant (two peers) never share a sequence.
/// The clock is only a fallback for a system without an entropy source.
fn seed() -> u64 {
    getrandom::u64().unwrap_or_else(|_| {
        let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
        t ^ 0x9E37_79B9_7F4A_7C15
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    /// Two fresh workbooks created at the same instant (each thread is a fresh generator, like a
    /// fresh process) must not share ids: the seed comes from the OS, not the clock.
    #[test]
    fn ids_come_from_os_entropy() {
        let go = Arc::new(Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let go = go.clone();
                std::thread::spawn(move || {
                    go.wait();
                    let wb = crate::stdlib::default_workbook();
                    (wb.sheets[0].id, wb.sheets[0].rows.ids()[0], wb.sheets[0].cols.ids()[0])
                })
            })
            .collect();
        let firsts: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        for (i, a) in firsts.iter().enumerate() {
            for b in &firsts[i + 1..] {
                assert_ne!(a.0, b.0);
                assert_ne!(a.1, b.1);
                assert_ne!(a.2, b.2);
            }
        }
        assert_ne!(seed(), seed());
    }
}
