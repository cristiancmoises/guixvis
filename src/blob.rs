//! Compact binary snapshot of an [`Index`].
//!
//! The Guile indexer emits JSON (about 20 MB for 32,500 packages); parsing it
//! on every start costs roughly 85 ms — more than half of the whole startup.
//! This module stores the *resolved* index instead: dependency ids, no name
//! lookups, no JSON tokenizer, no gzip. Loading such a snapshot measures
//! around 25 ms.
//!
//! The format is little-endian and length-prefixed. Every count and length is
//! validated against the remaining input before it is used to allocate, so a
//! corrupt or hostile file can only produce an error, never an allocation
//! stampede.

use std::sync::Arc;

use crate::error::IndexError;
use crate::index::{Index, Package};

/// File magic, version included so a stale file is rejected cheaply.
const MAGIC: &[u8; 8] = b"GUIV5IDX";
/// Upper bounds; anything beyond these is treated as corruption.
const MAX_PACKAGES: u32 = 1_000_000;
const MAX_STRING: u32 = 8 * 1024 * 1024;
const MAX_DEPS: u32 = 100_000;

#[derive(Debug, thiserror::Error)]
pub enum BlobError {
    #[error("not a guixvis index snapshot (bad magic)")]
    Magic,
    #[error("unsupported snapshot schema {0} (expected {1})")]
    Schema(u32, u32),
    #[error("snapshot truncated: need {need} more bytes at offset {at}")]
    Truncated { at: usize, need: usize },
    #[error("snapshot rejected by index validation: {0}")]
    Invalid(#[from] IndexError),
    #[error("invalid snapshot field: {0}")]
    Field(&'static str),
}

/// Encode an index snapshot.
pub fn encode(index: &Index) -> Vec<u8> {
    encode_payload(index)
}

/// Canonical persisted content; excludes the derived token and load timestamp.
pub(crate) fn encode_payload(index: &Index) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 * 1024 * 1024);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&crate::index::INDEX_SCHEMA_VERSION.to_le_bytes());
    out.extend_from_slice(&(index.packages.len() as u32).to_le_bytes());
    put_str(&mut out, &index.guix_commit);
    out.extend_from_slice(&index.generated_ms.to_le_bytes());
    put_str(&mut out, &index.origin.executable);
    put_str(&mut out, &index.origin.system);
    out.push(u8::from(index.origin.verified));
    out.push(u8::from(index.origin.mutable_package_path));
    out.extend_from_slice(&(index.origin.channels.len() as u32).to_le_bytes());
    for channel in &index.origin.channels {
        put_str(&mut out, &channel.name);
        put_str(&mut out, &channel.commit);
    }
    for p in &index.packages {
        out.extend_from_slice(&p.id.to_le_bytes());
        out.push(u8::from(p.catalog));
        put_str(&mut out, &p.name);
        put_str(&mut out, &p.version);
        put_str(&mut out, &p.synopsis);
        put_str(&mut out, &p.description);
        put_str(&mut out, &p.homepage);
        put_str(&mut out, &p.file);
        out.extend_from_slice(&p.line.to_le_bytes());
        out.extend_from_slice(&(p.licenses.len() as u32).to_le_bytes());
        for l in p.licenses.iter() {
            put_str(&mut out, l);
        }
        put_ids(&mut out, &p.inputs);
        put_ids(&mut out, &p.propagated);
        put_ids(&mut out, &p.native);
    }
    out.extend_from_slice(&(index.diagnostics.len() as u32).to_le_bytes());
    for d in index.diagnostics.iter() {
        out.extend_from_slice(&d.package_id.to_le_bytes());
        put_str(&mut out, &d.kind);
        put_str(&mut out, &d.code);
        put_str(&mut out, &d.message);
    }
    out
}

/// Decode a snapshot written by [`encode`].
pub fn decode(bytes: &[u8], built_ms: u64) -> Result<Index, BlobError> {
    let mut r = Reader { buf: bytes, at: 0 };
    if r.take(8)? != MAGIC {
        return Err(BlobError::Magic);
    }
    let schema = r.u32()?;
    if schema != crate::index::INDEX_SCHEMA_VERSION {
        return Err(BlobError::Schema(
            schema,
            crate::index::INDEX_SCHEMA_VERSION,
        ));
    }
    let count = r.capped_u32(MAX_PACKAGES)?;
    // Even an empty package needs much more than one byte; reject implausible
    // counts before reserving memory, independently of the absolute limit.
    if count as usize > bytes.len() / 40 {
        return Err(BlobError::Field("package count"));
    }
    let guix_commit = r.string()?;
    let generated_ms = r.u64()?;
    let executable = r.string()?;
    let system = r.string()?;
    let verified = r.boolean()?;
    let mutable_package_path = r.boolean()?;
    let channel_count = r.capped_u32(4096)?;
    let mut channels = Vec::new();
    for _ in 0..channel_count {
        channels.push(crate::model::ChannelPin {
            name: r.string()?,
            commit: r.string()?,
        });
    }
    let origin = crate::model::GuixOrigin {
        executable,
        system,
        verified,
        mutable_package_path,
        channels,
    };

    let mut packages = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let id = r.u32()?;
        let catalog = match r.take(1)?[0] {
            0 => false,
            1 => true,
            _ => return Err(BlobError::Field("catalog")),
        };
        let name = r.string()?;
        let version = r.string()?;
        let synopsis = r.string()?;
        let description = r.string()?;
        let homepage = r.string()?;
        let file = r.string()?;
        let line = r.u64()?;
        let n_lic = r.capped_u32(4096)?;
        let mut licenses = Vec::with_capacity(n_lic as usize);
        for _ in 0..n_lic {
            licenses.push(Arc::<str>::from(r.string()?));
        }
        let inputs = r.ids()?;
        let propagated = r.ids()?;
        let native = r.ids()?;
        packages.push(Package {
            id,
            catalog,
            name: Arc::from(name),
            version: Arc::from(version),
            synopsis: Arc::from(synopsis),
            description: Arc::from(description),
            homepage: Arc::from(homepage),
            licenses: licenses.into(),
            file: Arc::from(file),
            line,
            inputs: inputs.into(),
            propagated: propagated.into(),
            native: native.into(),
        });
    }

    let n = r.capped_u32(MAX_PACKAGES)?;
    let mut diagnostics = Vec::new();
    for _ in 0..n {
        diagnostics.push(crate::model::IndexDiagnostic {
            package_id: r.u32()?,
            kind: r.string()?,
            code: r.string()?,
            message: r.string()?,
        });
    }
    if r.at != bytes.len() {
        return Err(BlobError::Field("trailing bytes"));
    }
    Ok(Index::from_parts(
        packages,
        guix_commit,
        generated_ms,
        built_ms,
        diagnostics,
        origin,
    )?)
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn put_ids(out: &mut Vec<u8>, ids: &[u32]) {
    out.extend_from_slice(&(ids.len() as u32).to_le_bytes());
    for id in ids {
        out.extend_from_slice(&id.to_le_bytes());
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn boolean(&mut self) -> Result<bool, BlobError> {
        match self.take(1)?[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(BlobError::Field("boolean")),
        }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], BlobError> {
        let end = self.at.checked_add(n).ok_or(BlobError::Truncated {
            at: self.at,
            need: n,
        })?;
        if end > self.buf.len() {
            return Err(BlobError::Truncated {
                at: self.at,
                need: n,
            });
        }
        let slice = &self.buf[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn u32(&mut self) -> Result<u32, BlobError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, BlobError> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }

    /// A count that is about to drive an allocation: bound it first.
    fn capped_u32(&mut self, max: u32) -> Result<u32, BlobError> {
        let v = self.u32()?;
        if v > max {
            return Err(BlobError::Truncated {
                at: self.at,
                need: v as usize,
            });
        }
        Ok(v)
    }

    fn string(&mut self) -> Result<String, BlobError> {
        let len = self.capped_u32(MAX_STRING)? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| BlobError::Truncated {
            at: self.at,
            need: len,
        })
    }

    fn ids(&mut self) -> Result<Vec<u32>, BlobError> {
        let n = self.capped_u32(MAX_DEPS)? as usize;
        if n > (self.buf.len() - self.at) / 4 {
            return Err(BlobError::Field("dependency count"));
        }
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(self.u32()?);
        }
        Ok(out)
    }
}
