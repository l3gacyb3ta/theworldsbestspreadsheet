//! Position keys ("fractional indexing", design note §2.2): strings over `0-9a-z`, compared
//! bytewise. An axis orders its ids by `(key, id)`, and there is always a key between two others,
//! so an insert or a sort writes keys only for the rows it places, never for their neighbours.
//!
//! There are two kinds of keys:
//! - fixed: a letter and six base-36 digits. Virtual row k's key is `t` + k (`fixed('t', k)`);
//!   the i-th row of a file saved before keys existed gets `m` + i, so those come first.
//! - made by `between`: they never end in `0` and are at least 8 characters, so none is a prefix
//!   of a fixed key. That leaves room between any two different keys (the only pairs without
//!   any are `x` and `x0`, `x00`, …).

const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
const MIN_LEN: usize = 8;
/// `i`, the middle digit.
const MID: u8 = 18;

fn base36(mut k: usize, width: usize) -> String {
    let mut d = vec![b'0'; width];
    for c in d.iter_mut().rev() {
        *c = DIGITS[k % 36];
        k /= 36;
    }
    String::from_utf8(d).unwrap()
}

/// `prefix` and `k` in six base-36 digits (k < 36⁶).
pub fn fixed(prefix: char, k: usize) -> String {
    format!("{prefix}{}", base36(k, 6))
}

fn val(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'z' => c - b'a' + 10,
        _ => 0,
    }
}

/// A key strictly between `lo` (`""`: no lower bound) and `hi` (`None`: no upper bound). If the
/// bounds are out of order (only possible after a merge) it is just above `lo`.
pub fn between(lo: &str, hi: Option<&str>) -> String {
    let a: Vec<u8> = lo.bytes().map(val).collect();
    let mut d = match hi {
        Some(h) if lo < h => mid(&a, &h.bytes().map(val).collect::<Vec<_>>()),
        _ => above(&a),
    };
    while d.len() < MIN_LEN {
        d.push(MID);
    }
    d.into_iter().map(|v| DIGITS[v as usize] as char).collect()
}

/// `n` keys in order between `lo` and `hi`, under one random prefix, so rows inserted (or sorted)
/// at the same place by two people at once each stay together (§2.2).
pub fn run(lo: &str, hi: Option<&str>, n: usize) -> Vec<String> {
    let mut p = between(lo, hi);
    let r = crate::ids::fresh_id();
    p.push_str(&base36((r % (36 * 36 * 36)) as usize, 3));
    let mut width = 1;
    while 36usize.pow(width as u32) < n {
        width += 1;
    }
    (0..n).map(|i| format!("{p}{}i", base36(i, width))).collect()
}

/// Digits of a key above `a`, with no upper bound.
fn above(a: &[u8]) -> Vec<u8> {
    match a.first() {
        None => vec![MID],
        Some(&35) => {
            let mut v = vec![35];
            v.extend(above(&a[1..]));
            v
        }
        Some(&d) => vec![(d + 36) / 2],
    }
}

/// Digits of a key between `a` and `b` (a < b). The result is never a prefix of `b`, so anything
/// appended to it stays below `b`.
fn mid(a: &[u8], b: &[u8]) -> Vec<u8> {
    let n = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let mut out = b[..n].to_vec();
    match (a.get(n), b.get(n)) {
        (_, None) => return above(a),
        (None, Some(&db)) => {
            if db >= 2 {
                out.push(db / 2);
            } else if db == 1 {
                out.extend([0, MID]);
            } else {
                out.push(0);
                out.extend(mid(&[], &b[n + 1..]));
            }
        }
        (Some(&da), Some(&db)) => {
            if db - da >= 2 {
                out.push((da + db) / 2);
            } else {
                out.push(da);
                out.extend(above(&a[n + 1..]));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::fresh_id;

    fn check(lo: &str, hi: Option<&str>) -> String {
        let k = between(lo, hi);
        assert!(lo < k.as_str() && hi.is_none_or(|h| k.as_str() < h), "{lo} < {k} < {hi:?}");
        assert!(!k.ends_with('0') && k.len() >= MIN_LEN, "{k}");
        k
    }

    #[test]
    fn fixed_keys_sort_by_index() {
        assert_eq!(fixed('t', 0), "t000000");
        assert_eq!(fixed('t', 37), "t000011");
        assert!(fixed('m', 99_999) < fixed('t', 0));
        for k in [0, 1, 35, 36, 1295, 1296, 99_999] {
            assert!(fixed('t', k) < fixed('t', k + 1));
            check(&fixed('t', k), Some(&fixed('t', k + 1)));
        }
    }

    #[test]
    fn always_room_in_between() {
        // keep inserting at the front, at the back and in the middle of a growing list
        let mut keys = vec![fixed('m', 0), fixed('m', 1), fixed('t', 0), fixed('t', 1)];
        for i in 0..3000usize {
            let r = fresh_id() as usize;
            let at = match i % 3 {
                0 => 0,
                1 => keys.len(),
                _ => r % (keys.len() + 1),
            };
            let lo = if at == 0 { "" } else { keys[at - 1].as_str() };
            let k = check(lo, keys.get(at).map(|s| s.as_str()));
            keys.insert(at, k);
        }
        // the same gap, again and again
        let (mut lo, hi) = (fixed('t', 4), fixed('t', 5));
        for _ in 0..500 {
            lo = check(&lo, Some(&hi));
        }
        let mut hi = fixed('t', 5);
        for _ in 0..500 {
            hi = check(&fixed('t', 4), Some(&hi));
        }
    }

    #[test]
    fn runs_stay_together_and_in_order() {
        let (lo, hi) = (fixed('t', 3), fixed('t', 4));
        let a = run(&lo, Some(&hi), 40);
        let b = run(&lo, Some(&hi), 3);
        for w in a.windows(2) {
            assert!(w[0] < w[1]);
        }
        assert!(a.iter().chain(&b).all(|k| lo < *k && *k < hi));
        // b's keys are all on one side of a's (unless both drew the same random prefix)
        if a[0][..11] != b[0][..11] {
            assert!(b.iter().all(|k| k < &a[0]) || b.iter().all(|k| k > &a[39]));
        }
    }
}
