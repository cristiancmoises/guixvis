//! In-memory package index: interned strings, reverse dependency edges,
//! and bounded BFS traversals used by the tree and graph views.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use crate::error::IndexError;
use crate::model::{IndexDoc, SCHEMA_VERSION};

/// How many synopsis characters go into the fuzzy-search haystack.
pub const SYN_LIMIT: usize = 200;

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
        let mut seen: Vec<u32> = Vec::with_capacity(self.dep_count());
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
            .filter(move |(i, _)| {
                if seen.contains(i) {
                    false
                } else {
                    seen.push(*i);
                    true
                }
            })
    }

    pub fn dep_count(&self) -> usize {
        self.inputs.len() + self.propagated.len() + self.native.len()
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

        // Pass 1: validate ids and names, intern names, resolve name -> id.
        let mut seen = vec![false; len];
        let mut names: HashMap<Arc<str>, u32> = HashMap::with_capacity(len);
        for pj in &doc.packages {
            let id = pj.id as usize;
            if id >= len {
                return Err(IndexError::IdOutOfRange(pj.id, len));
            }
            if seen[id] {
                return Err(IndexError::DuplicateId(pj.id));
            }
            seen[id] = true;
            if pj.name.is_empty() {
                return Err(IndexError::EmptyName(pj.id));
            }
            let name: Arc<str> = Arc::from(pj.name.as_str());
            names.entry(name).or_insert(pj.id);
        }

        // Pass 2: build packages in id order; invert dependency edges and
        // group packages by defining module in the same pass.
        let mut dependents: Vec<Vec<u32>> = vec![Vec::new(); len];
        let mut by_module: HashMap<Arc<str>, Vec<u32>> = HashMap::new();
        let intern = |s: &str| -> Arc<str> { Arc::from(s) };

        let mut packages: Vec<Package> = Vec::with_capacity(len);
        for pj in doc.packages {
            let file: Arc<str> = intern(&pj.file.0);
            if !file.is_empty() {
                by_module.entry(Arc::clone(&file)).or_default().push(pj.id);
            }

            // Deduplicate across input kinds; first occurrence wins.
            let mut edges: Vec<(u32, DepKind)> = Vec::new();
            let mut add = |list: &[String], kind: DepKind| {
                for n in list {
                    if let Some(dep) = names.get(n.as_str()) {
                        let dep = *dep;
                        if !edges.iter().any(|(d, _)| *d == dep) {
                            edges.push((dep, kind));
                            dependents[dep as usize].push(pj.id);
                        }
                    }
                    // Unknown names (objects that are not packages) are
                    // dropped: they are not reachable through this index.
                }
            };
            add(&pj.inputs, DepKind::Input);
            add(&pj.propagated_inputs, DepKind::Propagated);
            add(&pj.native_inputs, DepKind::Native);

            let ids_of = |kind: DepKind| -> Vec<u32> {
                edges
                    .iter()
                    .filter(|(_, k)| *k == kind)
                    .map(|(d, _)| *d)
                    .collect()
            };

            packages.push(Package {
                id: pj.id,
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
                inputs: ids_of(DepKind::Input).into(),
                propagated: ids_of(DepKind::Propagated).into(),
                native: ids_of(DepKind::Native).into(),
            });
        }

        // Freeze dependent lists (sorted for deterministic display order).
        let dependents: Vec<Arc<[u32]>> = dependents
            .into_iter()
            .map(|mut v| {
                v.sort_unstable();
                Arc::from(v.as_slice())
            })
            .collect();

        let generated_ms: u64 = header.generated_ms.parse().unwrap_or(0);
        Ok(Index {
            packages,
            names,
            dependents,
            by_module,
            guix_commit: header.guix_commit.clone(),
            generated_ms,
            built_ms,
        })
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
