// Deliberately malformed for the unparseable-file contract test.
// `syn::parse_file` should reject this; `lib.rs` next door must still parse.

pub fn oops(  -> {
    let unterminated = "
}
