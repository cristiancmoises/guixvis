//! Serde types mirroring the JSON emitted by `data/guix-index.scm`.

use serde::{Deserialize, Serialize};

/// Schema version of the on-disk index format.
pub const SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Header {
    pub schema: u32,
    #[serde(default)]
    pub guix_commit: String,
    #[serde(default)]
    pub generated_ms: String,
    pub package_count: u64,
}

/// One package as emitted by the Guile indexer. All fields are owned Strings
/// here; `index::Index::from_doc` converts them into interned `Arc<str>`s.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PkgJson {
    pub id: u32,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub synopsis: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub licenses: Vec<String>,
    /// `[file, line]` as emitted by the script; `["", 0]` when unknown.
    #[serde(default)]
    pub file: (String, u64),
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub propagated_inputs: Vec<String>,
    #[serde(default)]
    pub native_inputs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDoc {
    pub header: Header,
    pub packages: Vec<PkgJson>,
}
