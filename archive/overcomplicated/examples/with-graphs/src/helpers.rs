//! Utility helpers used by the crate root and by `compute`.

use crate::compute;

pub fn clamp(x: i32) -> i32 {
    if x < 0 {
        0
    } else if x > 100 {
        100
    } else {
        x
    }
}

pub fn boosted(x: i32) -> i32 {
    let c = clamp(x);
    compute::scale(c) + 1
}
