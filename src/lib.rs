//! guixvis — interactive package explorer and dependency visualizer for GNU Guix.
//!
//! Indexes every GNU Guix package through an embedded Guile script executed by
//! `guix repl`, caches the result as gzipped JSON, and exposes it through a
//! keyboard-first terminal UI: fuzzy search, package details, dependency and
//! reverse-dependency trees, and a force-directed dependency graph.

pub mod app;
pub mod blob;
pub mod cache;
pub mod error;
pub mod graph;
pub mod graph_worker;
pub mod guix_env;
pub mod index;
pub mod indexer;
pub mod model;
mod process;
pub mod relations;
pub mod search;
pub mod theme;
pub mod ui;
#[cfg(feature = "web")]
pub mod web;

/// Canonical name and version, used for `--version` output and UI.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
