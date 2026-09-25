//! In-memory package index: interned strings, reverse dependency edges,
//! and bounded BFS traversals used by the tree and graph views.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use crate::error::IndexError;
use crate::model::{GuixOrigin, IndexDiagnostic, IndexDoc, SCHEMA_VERSION};
use sha2::{Digest, Sha256};

pub use crate::model::SCHEMA_VERSION as INDEX_SCHEMA_VERSION;

/// How many synopsis characters go into the fuzzy-search haystack.
pub const SYN_LIMIT: usize = 200;

/// Terminal-safe metadata; multiline descriptions may retain line breaks/tabs.
fn clean_text(text: &str, multiline: bool) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() && !(multiline && matches!(c, '\n' | '\t')) {
                ' '
            } else {
                c
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DepKind {
    Input,
    Propagated,
    Native,
}

impl DepKind {
    pub fn label(self) -> &'static str {
        match self {
            DepKind::Input => "",
            DepKind::Propagated => "P",
            DepKind::Native => "N",
        }
    }
}

/// One package with interned fields. Dependency lists hold package *ids*.
#[derive(Debug)]
pub struct Package {
    pub id: u32,
    pub catalog: bool,
    pub name: Arc<str>,
    pub version: Arc<str>,
    pub synopsis: Arc<str>,
    pub description: Arc<str>,
    pub homepage: Arc<str>,
    pub licenses: Arc<[Arc<str>]>,
    pub file: Arc<str>,
    pub line: u64,
    pub inputs: Arc<[u32]>,
    pub propagated: Arc<[u32]>,
    pub native: Arc<[u32]>,
}

impl Package {
    /// All direct dependencies with their edge kind, deduplicated across
    /// input kinds (a package listed in both `inputs` and `native-inputs`
    /// appears once, as its first occurrence).
    pub fn deps(&self) -> impl Iterator<Item = (u32, DepKind)> + '_ {
        let mut seen = HashSet::with_capacity(self.relation_count());
        self.typed_deps().filter(move |(id, _)| seen.insert(*id))
    }

    /// All typed relations; the same object can appear under several kinds.
    pub fn typed_deps(&self) -> impl Iterator<Item = (u32, DepKind)> + '_ {
        self.inputs
            .iter()
            .copied()
            .map(|i| (i, DepKind::Input))
            .chain(
                self.propagated
                    .iter()
                    .copied()
                    .map(|i| (i, DepKind::Propagated)),
            )
            .chain(self.native.iter().copied().map(|i| (i, DepKind::Native)))
    }

    pub fn dep_count(&self) -> usize {
        self.deps().count()
    }

    pub fn relation_count(&self) -> usize {
        self.inputs.len() + self.propagated.len() + self.native.len()
    }

    pub fn dep_kinds(&self, id: u32) -> Vec<DepKind> {
        self.typed_deps()
            .filter_map(|(dep, kind)| (dep == id).then_some(kind))
            .collect()
    }
}

/// A node reached by a BFS walk, tagged with depth and the edge kind of the
/// link from its parent (for reverse walks the kind is always `None`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BfsNode {
    pub id: u32,
    pub depth: u8,
    pub kind: Option<DepKind>,
}

/// The full index. Immutable after construction; shared behind `Arc`.
#[derive(Debug)]
pub struct Index {
    pub packages: Vec<Package>,
    /// First id for a package name (names may repeat across versions).
    pub names: HashMap<Arc<str>, u32>,
    /// Reverse edges: all direct dependents of each package id.
    pub dependents: Vec<Arc<[u32]>>,
    /// Packages grouped by defining module file.
    pub by_module: HashMap<Arc<str>, Vec<u32>>,
    /// Channel commit this index was generated against ("" if unknown).
    pub guix_commit: String,
    /// Epoch milliseconds at index generation (from the Guile script).
    pub generated_ms: u64,
    /// Wall-clock time (ms) when this index was produced.
    pub built_ms: u64,
    pub diagnostics: Arc<[IndexDiagnostic]>,
    pub origin: GuixOrigin,
    snapshot: String,
}

impl Index {
    /// Validate and convert a raw document. Every failure mode is typed and
    /// carries context; nothing here panics on malformed input.
    pub fn from_doc(doc: IndexDoc, built_ms: u64) -> Result<Self, IndexError> {
        let header = &doc.header;
        if header.schema != SCHEMA_VERSION {
            return Err(IndexError::Schema(header.schema, SCHEMA_VERSION));
        }
        let len = doc.packages.len();
        if header.package_count as usize != len {
            return Err(IndexError::Count(header.package_count, len));
        }

        if len > 1_000_000 {
            return Err(IndexError::Limit("packages"));
        }
        // IDs come from Guile object identity, never from a name lookup.
        let intern = |s: &str| -> Arc<str> { Arc::from(s) };

        let mut packages: Vec<Package> = Vec::with_capacity(len);
        for pj in &doc.packages {
            let file: Arc<str> = intern(&pj.file.0);

            let unique = |list: &[u32]| -> Result<Arc<[u32]>, IndexError> {
                if list.len() > 100_000 {
                    return Err(IndexError::Limit("dependencies"));
                }
                let mut seen = std::collections::HashSet::new();
                Ok(list
                    .iter()
                    .copied()
                    .filter(|id| seen.insert(*id))
                    .collect::<Vec<_>>()
                    .into())
            };

            packages.push(Package {
                id: pj.id,
                catalog: pj.catalog,
                name: intern(&pj.name),
                version: intern(&pj.version),
                synopsis: intern(&pj.synopsis),
                description: intern(&pj.description),
                homepage: intern(&pj.homepage),
                licenses: pj
                    .licenses
                    .iter()
                    .map(|l| intern(l))
                    .collect::<Vec<_>>()
                    .into(),
                file,
                line: pj.file.1,
                inputs: unique(&pj.inputs)?,
                propagated: unique(&pj.propagated_inputs)?,
                native: unique(&pj.native_inputs)?,
            });
        }

        let generated_ms: u64 = header.generated_ms.parse().unwrap_or(0);
        Index::from_parts(
            packages,
            header.guix_commit.clone(),
            generated_ms,
            built_ms,
            doc.diagnostics,
            header.origin.clone(),
        )
    }

    /// Assemble an index from packages whose dependency lists already hold
    /// ids: validate them, derive the name lookup, the reverse edges and the
    /// module grouping.
    ///
    /// Both cache paths land here, so the derived structures are built by one
    /// piece of code with one set of checks.
    pub fn from_packages(
        packages: Vec<Package>,
        guix_commit: String,
        generated_ms: u64,
        built_ms: u64,
    ) -> Result<Self, IndexError> {
        Self::from_parts(
            packages,
            guix_commit,
            generated_ms,
            built_ms,
            Vec::new(),
            GuixOrigin::default(),
        )
    }

    pub(crate) fn from_parts(
        mut packages: Vec<Package>,
        guix_commit: String,
        generated_ms: u64,
        built_ms: u64,
        mut diagnostics: Vec<IndexDiagnostic>,
        mut origin: GuixOrigin,
    ) -> Result<Self, IndexError> {
        let len = packages.len();
        if len > 1_000_000 || diagnostics.len() > 1_000_000 {
            return Err(IndexError::Limit("index entries"));
        }
        packages.sort_unstable_by_key(|p| p.id);
        let mut seen = vec![false; len];
        let mut names: HashMap<Arc<str>, u32> = HashMap::with_capacity(len);
        let mut dependents: Vec<Vec<u32>> = vec![Vec::new(); len];
        let mut by_module: HashMap<Arc<str>, Vec<u32>> = HashMap::new();

        for p in &mut packages {
            for (text, multiline) in [
                (&mut p.name, false),
                (&mut p.version, false),
                (&mut p.synopsis, false),
                (&mut p.description, true),
                (&mut p.homepage, false),
                (&mut p.file, false),
            ] {
                let cleaned = clean_text(text, multiline);
                if cleaned.as_str() != text.as_ref() {
                    *text = cleaned.into();
                }
            }
            p.licenses = p
                .licenses
                .iter()
                .map(|s| Arc::from(clean_text(s, false)))
                .collect::<Vec<_>>()
                .into();
            for edges in [&mut p.inputs, &mut p.propagated, &mut p.native] {
                if edges.len() > 100_000 {
                    return Err(IndexError::Limit("dependencies"));
                }
                let mut unique = HashSet::with_capacity(edges.len());
                *edges = edges
                    .iter()
                    .copied()
                    .filter(|id| unique.insert(*id))
                    .collect::<Vec<_>>()
                    .into();
            }
            let id = p.id as usize;
            if id >= len {
                return Err(IndexError::IdOutOfRange(p.id, len));
            }
            if seen[id] {
                return Err(IndexError::DuplicateId(p.id));
            }
            seen[id] = true;
            if p.name.is_empty() {
                return Err(IndexError::EmptyName(p.id));
            }
            if p.catalog {
                names.entry(Arc::clone(&p.name)).or_insert(p.id);
            }
            if !p.file.is_empty() {
                by_module.entry(Arc::clone(&p.file)).or_default().push(p.id);
            }
            for dep in p
                .inputs
                .iter()
                .chain(p.propagated.iter())
                .chain(p.native.iter())
            {
                if *dep as usize >= len {
                    return Err(IndexError::IdOutOfRange(*dep, len));
                }
                dependents[*dep as usize].push(p.id);
            }
        }

        for p in &packages {
            names.entry(Arc::clone(&p.name)).or_insert(p.id);
        }
        for diagnostic in &mut diagnostics {
            if diagnostic.package_id as usize >= len {
                return Err(IndexError::IdOutOfRange(diagnostic.package_id, len));
            }
            diagnostic.kind = clean_text(&diagnostic.kind, false);
            diagnostic.code = clean_text(&diagnostic.code, false);
            diagnostic.message = clean_text(&diagnostic.message, false);
        }

        let dependents: Vec<Arc<[u32]>> = dependents
            .into_iter()
            .map(|mut v| {
                v.sort_unstable();
                v.dedup();
                Arc::from(v.as_slice())
            })
            .collect();

        if origin.channels.len() > 4096 {
            return Err(IndexError::Limit("channels"));
        }
        for text in [&mut origin.executable, &mut origin.system] {
            let clean = clean_text(text, false);
            if clean != *text {
                origin.verified = false;
                *text = clean;
            }
        }
        for channel in &mut origin.channels {
            for text in [&mut channel.name, &mut channel.commit] {
                let clean = clean_text(text, false);
                if clean != *text {
                    origin.verified = false;
                    *text = clean;
                }
            }
        }
        origin.channels.sort();
        origin.channels.dedup();
        origin.verified = origin.is_verified();
        let mut index = Index {
            packages,
            names,
            dependents,
            by_module,
            guix_commit: clean_text(&guix_commit, false),
            generated_ms,
            built_ms,
            diagnostics: diagnostics.into(),
            origin,
            snapshot: String::new(),
        };
        let digest = Sha256::digest(crate::blob::encode_payload(&index));
        index.snapshot = format!("{digest:x}");
        Ok(index)
    }

    pub fn is_complete(&self) -> bool {
        self.diagnostics.is_empty()
    }

    pub fn snapshot_id(&self) -> &str {
        &self.snapshot
    }

    pub fn len(&self) -> usize {
        self.packages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    /// Direct dependent count of a package.
    pub fn dependents_count(&self, id: u32) -> usize {
        self.dependents[id as usize].len()
    }

    /// Packages defined in the same module file as `id` (including itself).
    pub fn module_neighbors(&self, id: u32) -> &[u32] {
        self.packages
            .get(id as usize)
            .and_then(|p| self.by_module.get(&p.file))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Bounded BFS over forward (dependency) edges. Root excluded from the
    /// result. `total` reports every node discovered before the cap.
    pub fn deps_bfs(&self, root: u32, max_depth: u8, cap: usize) -> (Vec<BfsNode>, usize) {
        self.bfs(root, max_depth, cap, false)
    }

    /// Bounded BFS over reverse (dependent) edges.
    pub fn dependents_bfs(&self, root: u32, max_depth: u8, cap: usize) -> (Vec<BfsNode>, usize) {
        self.bfs(root, max_depth, cap, true)
    }

    fn bfs(&self, root: u32, max_depth: u8, cap: usize, reverse: bool) -> (Vec<BfsNode>, usize) {
        let mut out: Vec<BfsNode> = Vec::new();
        if (root as usize) >= self.packages.len() {
            return (out, 0);
        }
        let mut total = 0usize;
        let mut visited = vec![false; self.packages.len()];
        let mut queue: VecDeque<(u32, u8, Option<DepKind>)> = VecDeque::new();
        visited[root as usize] = true;
        queue.push_back((root, 0, None));

        while let Some((id, depth, kind)) = queue.pop_front() {
            if id != root {
                if out.len() >= cap {
                    break; // materialization cap; totals stay honest
                }
                out.push(BfsNode { id, depth, kind });
            }
            if depth >= max_depth {
                continue;
            }
            if reverse {
                for d in self.dependents[id as usize].iter() {
                    if !visited[*d as usize] {
                        visited[*d as usize] = true;
                        total += 1;
                        queue.push_back((*d, depth + 1, None));
                    }
                }
            } else {
                let neighbors: Vec<(u32, DepKind)> = self.packages[id as usize].deps().collect();
                for (d, k) in neighbors {
                    if !visited[d as usize] {
                        visited[d as usize] = true;
                        total += 1;
                        queue.push_back((d, depth + 1, Some(k)));
                    }
                }
            }
        }
        (out, total)
    }
}
