//! Serde types mirroring the JSON emitted by `data/guix-index.scm`.

use serde::{Deserialize, Serialize};

/// Schema version of the on-disk index format.
pub const SCHEMA_VERSION: u32 = 4;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChannelPin {
    pub name: String,
    pub commit: String,
}

/// Origin deliberately excludes channel URLs and environment secrets.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuixOrigin {
    pub executable: String,
    pub system: String,
    pub channels: Vec<ChannelPin>,
    pub verified: bool,
    pub mutable_package_path: bool,
}

impl GuixOrigin {
    pub fn is_verified(&self) -> bool {
        self.verified
            && !self.mutable_package_path
            && !self.executable.is_empty()
            && !self.system.is_empty()
            && !self.channels.is_empty()
            && self
                .channels
                .iter()
                .all(|c| !c.name.is_empty() && !c.commit.is_empty())
    }
}

/// A recoverable extraction failure. An index with diagnostics is incomplete.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexDiagnostic {
    pub package_id: u32,
    pub kind: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Header {
    pub schema: u32,
    #[serde(default)]
    pub guix_commit: String,
    #[serde(default)]
    pub generated_ms: String,
    pub package_count: u64,
    #[serde(default)]
    pub origin: GuixOrigin,
}

/// One package as emitted by the Guile indexer. All fields are owned Strings
/// here; `index::Index::from_doc` converts them into interned `Arc<str>`s.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PkgJson {
    pub id: u32,
    pub name: String,
    /// True for an enumerated catalog entry, false for a closure-only variant.
    #[serde(default = "catalog_default")]
    pub catalog: bool,
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
    pub inputs: Vec<u32>,
    #[serde(default)]
    pub propagated_inputs: Vec<u32>,
    #[serde(default)]
    pub native_inputs: Vec<u32>,
}

fn catalog_default() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDoc {
    pub header: Header,
    pub packages: Vec<PkgJson>,
    #[serde(default)]
    pub diagnostics: Vec<IndexDiagnostic>,
}
