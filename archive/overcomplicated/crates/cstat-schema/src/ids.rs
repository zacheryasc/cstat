//! Cross-level identity types per spec §6.
//!
//! Every per-entity record carries the set of IDs by which it is known.
//! Cross-level joins are explicit on these IDs.

use serde::{Deserialize, Serialize};

/// Project-relative path, posix-normalized (forward slashes).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FileId(pub String);

impl FileId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Crate-qualified module path, e.g. `cstat::diagnostics::scoring`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModuleId(pub String);

impl ModuleId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Identifies a syntactic function item: `(file, fq_path, item_kind, ast_node_hash)`.
///
/// `ast_node_hash` is a structural hash of the item; it survives reformatting
/// but not real edits, and distinguishes overloads / multiple `impl` blocks.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AstFuncId {
    pub file: FileId,
    pub fq_path: String,
    pub item_kind: String,
    pub ast_node_hash: String,
}

/// Mangled symbol name (post-rustc mangling).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SymbolId(pub String);

impl SymbolId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Demangled symbol with type substitutions, joinable to AstFuncId.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MonoId(pub String);

impl MonoId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// All IDs an entity may be known by (spec §6). Any subset may be present.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdSet {
    #[serde(rename = "FileId", skip_serializing_if = "Option::is_none", default)]
    pub file: Option<FileId>,
    #[serde(rename = "ModuleId", skip_serializing_if = "Option::is_none", default)]
    pub module: Option<ModuleId>,
    #[serde(rename = "AstFuncId", skip_serializing_if = "Option::is_none", default)]
    pub ast_func: Option<AstFuncId>,
    #[serde(rename = "SymbolId", skip_serializing_if = "Option::is_none", default)]
    pub symbol: Option<SymbolId>,
    #[serde(rename = "MonoId", skip_serializing_if = "Option::is_none", default)]
    pub mono: Option<MonoId>,
}
