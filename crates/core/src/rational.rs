//! Small exact rationals for dimension exponents (`m^1/2` must survive `sqrt`).

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Rational {
    pub num: i64,
    pub den: i64,
}

fn gcd(a: i64, b: i64) -> i64 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a.max(1)
}

impl Rational {
    pub const ZERO: Rational = Rational { num: 0, den: 1 };
    pub const ONE: Rational = Rational { num: 1, den: 1 };

    pub fn new(num: i64, den: i64) -> Rational {
        assert!(den != 0, "zero denominator");
        let s = if den < 0 { -1 } else { 1 };
        let g = gcd(num, den);
        Rational { num: s * num / g, den: s * den / g }
    }
    pub fn int(n: i64) -> Rational {
        Rational { num: n, den: 1 }
    }
    pub fn is_zero(self) -> bool {
        self.num == 0
    }
    pub fn add(self, o: Rational) -> Rational {
        Rational::new(self.num * o.den + o.num * self.den, self.den * o.den)
    }
    pub fn neg(self) -> Rational {
        Rational { num: -self.num, den: self.den }
    }
    pub fn mul(self, o: Rational) -> Rational {
        Rational::new(self.num * o.num, self.den * o.den)
    }
    pub fn to_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }
    /// Exact rational for a float if it has a small denominator (used by `^`).
    pub fn from_f64(x: f64) -> Option<Rational> {
        if !x.is_finite() {
            return None;
        }
        for den in 1..=12i64 {
            let n = x * den as f64;
            if (n - n.round()).abs() < 1e-9 && n.abs() < 1e12 {
                return Some(Rational::new(n.round() as i64, den));
            }
        }
        None
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}
