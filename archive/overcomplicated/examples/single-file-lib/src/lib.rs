pub fn add(a: i32, b: i32) -> i32 { a + b }

pub fn classify(x: i32) -> &'static str {
    if x < 0 {
        "neg"
    } else if x == 0 {
        "zero"
    } else {
        "pos"
    }
}
