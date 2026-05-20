//! Toolchain stamp recorded by the nightly sub-binary.
//!
//! Mirrors the shape of `cstat_schema::Toolchain` but lives here because
//! the nightly extractor doesn't depend on `cstat-schema` — it only
//! produces `cstat-ir` dumps. The orchestrator copies the values into
//! the envelope's `toolchain` field when both stamps are present.

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolchainStamp {
    pub channel: String,
    pub version: String,
    pub host: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolchain_roundtrips_through_json() {
        let stamp = ToolchainStamp {
            channel: "nightly".into(),
            version: "1.97.0-nightly (abcdef0 2026-01-15)".into(),
            host: "x86_64-unknown-linux-gnu".into(),
        };
        let json = serde_json::to_string(&stamp).unwrap();
        let decoded: ToolchainStamp = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, stamp);
    }
}
