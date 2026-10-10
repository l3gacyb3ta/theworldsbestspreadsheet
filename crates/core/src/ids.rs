//! Stable identifiers. Rows, columns and sheets get random 64-bit ids so that
//! independent writers (a future Automerge doc) never collide; positions are
//! only ever derived from the position keys in a `Sheet`'s axes. Rows past the
//! stored ones have deterministic ids (`mix`), so every peer agrees on them.

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
        let z = r.get().wrapping_add(0x9E37_79B9_7F4A_7C15);
        r.set(z);
        mix(z)
    })
}

const M1: u64 = 0xBF58_476D_1CE4_E5B9;
const M2: u64 = 0x94D0_49BB_1331_11EB;

/// splitmix64's finaliser. It is a bijection, so `unmix` gives the input back: a virtual row's id
/// (`mix(base + k)`) tells which row `k` it is without anything stored.
pub fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(M1);
    z = (z ^ (z >> 27)).wrapping_mul(M2);
    z ^ (z >> 31)
}

pub fn unmix(mut z: u64) -> u64 {
    z = unshift(z, 31).wrapping_mul(inverse(M2));
    z = unshift(z, 27).wrapping_mul(inverse(M1));
    unshift(z, 30)
}

/// Undoes `x ^ (x >> s)`.
fn unshift(y: u64, s: u32) -> u64 {
    let (mut x, mut t) = (y, y >> s);
    while t != 0 {
        x ^= t;
        t >>= s;
    }
    x
}

/// The inverse of an odd number mod 2^64 (Newton's iteration doubles the correct bits each step).
const fn inverse(a: u64) -> u64 {
    let mut x = a;
    let mut i = 0;
    while i < 6 {
        x = x.wrapping_mul(2u64.wrapping_sub(a.wrapping_mul(x)));
        i += 1;
    }
    x
}

/// Row and column ids, for `model::Axis`. `TAG` keeps a sheet's virtual rows and columns apart.
pub trait AxisId: Copy + Eq + std::hash::Hash + Ord + std::fmt::Debug {
    const TAG: u64;
    fn raw(self) -> u64;
    fn from_raw(x: u64) -> Self;
}

impl AxisId for RowId {
    const TAG: u64 = 0x0072_6f77; // "row"
    fn raw(self) -> u64 {
        self.0
    }
    fn from_raw(x: u64) -> Self {
        RowId(x)
    }
}

impl AxisId for ColId {
    const TAG: u64 = 0x0063_6f6c; // "col"
    fn raw(self) -> u64 {
        self.0
    }
    fn from_raw(x: u64) -> Self {
        ColId(x)
    }
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
                    (wb.sheets[0].id, wb.sheets[0].rows.get(0).unwrap(), wb.sheets[0].cols.get(0).unwrap())
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

    #[test]
    fn unmix_inverts_mix() {
        for x in [0, 1, 2, 0xdead_beef, u64::MAX, fresh_id(), fresh_id()] {
            assert_eq!(unmix(mix(x)), x);
            assert_eq!(mix(unmix(x)), x);
        }
    }
}
