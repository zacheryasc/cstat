//! Shared utilities for filesystem / source-text levels.

/// Linear-interpolated percentile (matches numpy's default "linear" mode).
///
/// `sorted` must be sorted ascending and non-empty.
pub(crate) fn percentile_u64(sorted: &[u64], q: f64) -> f64 {
    debug_assert!(!sorted.is_empty(), "percentile_u64 called on empty slice");
    let n = sorted.len();
    if n == 1 {
        return sorted[0] as f64;
    }
    let rank = q.clamp(0.0, 1.0) * (n - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    let frac = rank - lo as f64;
    let lo_val = sorted[lo] as f64;
    let hi_val = sorted[hi] as f64;
    lo_val + (hi_val - lo_val) * frac
}
