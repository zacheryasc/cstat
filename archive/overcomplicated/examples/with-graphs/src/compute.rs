//! Numeric helpers.

use crate::helpers;

pub fn scale(x: i32) -> i32 {
    let c = helpers::clamp(x);
    c * 2
}

pub fn quadratic(x: i32) -> i32 {
    scale(x) + scale(x)
}
