//! Typed errors for index data, the indexer subprocess, and the disk cache.

use thiserror::Error;

/// Validation failures while turning an `IndexDoc` into an in-memory `Index`.
#[derive(Debug, Error)]
pub enum IndexError {
    #[error("index exceeds the safety limit for {0}")]
    Limit(&'static str),
    #[error("cache schema {0} unsupported (expected {1})")]
    Schema(u32, u32),
    #[error("channel commit mismatch: cache={0} live={1}")]
    Commit(String, String),
    #[error("package count mismatch: header={0} packages={1}")]
    Count(u64, usize),
    #[error("package id {0} out of range (len {1})")]
    IdOutOfRange(u32, usize),
    #[error("duplicate package id {0}")]
    DuplicateId(u32),
    #[error("package #{0} has an empty name")]
    EmptyName(u32),
    #[error("package `{1}` references unknown dependency `{0}`")]
    UnknownDep(String, String),
}

/// Failures of the `guix repl` indexer subprocess.
#[derive(Debug, Error)]
pub enum IndexerError {
    #[error("`guix` binary not found; looked at: {0}")]
    NotFound(String),
    #[error("indexer timed out after {0}s")]
    Timeout(u64),
    #[error("indexer exited with: {0}")]
    Exited(String),
    #[error("indexer produced no output")]
    NoOutput,
}

/// Failures while reading, validating, or writing the on-disk cache.
#[derive(Debug, Error)]
pub enum CacheError {
    #[error("cache read failed: {0}")]
    Read(String),
    #[error("cache parse failed: {0}")]
    Parse(String),
    #[error("cache write failed: {0}")]
    Write(String),
    #[error("cache index invalid: {0}")]
    Invalid(#[from] IndexError),
}
