//! Stage-9 graph fixture. Two modules + cross-module + intra-module
//! function calls give the call graph and module graph something to
//! observe.

pub mod helpers;
pub mod compute;

pub fn run(x: i32) -> i32 {
    let normalized = helpers::clamp(x);
    compute::scale(normalized)
}
