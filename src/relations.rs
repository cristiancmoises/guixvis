//! Complete dependency closures, independent of presentation limits.
use crate::index::{DepKind, Index};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Dependencies,
    Dependents,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RelationKey {
    pub snapshot: String,
    pub root: u32,
    pub direction: Direction,
}
#[derive(Clone, Debug)]
pub struct RelationHit {
    pub id: u32,
    pub depth: usize,
    pub kinds: Vec<DepKind>,
}
#[derive(Clone, Debug)]
pub struct RelationSet {
    pub key: RelationKey,
    pub hits: Vec<RelationHit>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum WalkError {
    InvalidRoot,
    Cancelled,
}

pub fn collect_relations(
    index: &Index,
    root: u32,
    direction: Direction,
    cancelled: &dyn Fn() -> bool,
) -> Result<RelationSet, WalkError> {
    if root as usize >= index.len() {
        return Err(WalkError::InvalidRoot);
    }
    if cancelled() {
        return Err(WalkError::Cancelled);
    }
    let mut depths = vec![usize::MAX; index.len()];
    let mut kinds = vec![0u8; index.len()];
    depths[root as usize] = 0;
    let mut queue = VecDeque::from([root]);
    let mut work = 0usize;
    while let Some(id) = queue.pop_front() {
        if cancelled() {
            return Err(WalkError::Cancelled);
        }
        let next_depth = depths[id as usize] + 1;
        let mut visit = |dep: u32, mask: u8| -> Result<(), WalkError> {
            // Keep cancellation checks measured in typed relations, including
            // reverse edges that now arrive as a merged category mask.
            for _ in 0..mask.count_ones() {
                work += 1;
                if work.is_multiple_of(256) && cancelled() {
                    return Err(WalkError::Cancelled);
                }
            }
            kinds[dep as usize] |= mask;
            if depths[dep as usize] == usize::MAX {
                depths[dep as usize] = next_depth;
                queue.push_back(dep);
            }
            Ok(())
        };
        match direction {
            Direction::Dependencies => {
                for (dep, kind) in index.packages[id as usize].typed_deps() {
                    visit(dep, 1 << kind as u8)?;
                }
            }
            Direction::Dependents => {
                for (dep, mask) in index.typed_dependents(id) {
                    visit(dep, mask)?;
                }
            }
        }
    }
    let mut hits: Vec<_> = depths
        .into_iter()
        .enumerate()
        .filter(|(id, depth)| *id != root as usize && *depth != usize::MAX)
        .map(|(id, depth)| RelationHit {
            id: id as u32,
            depth,
            kinds: [DepKind::Input, DepKind::Propagated, DepKind::Native]
                .into_iter()
                .filter(|kind| kinds[id] & (1 << *kind as u8) != 0)
                .collect(),
        })
        .collect();
    hits.sort_unstable_by(|a, b| {
        let pa = &index.packages[a.id as usize];
        let pb = &index.packages[b.id as usize];
        pa.name
            .cmp(&pb.name)
            .then_with(|| pa.version.cmp(&pb.version))
            .then(a.id.cmp(&b.id))
    });
    if cancelled() {
        return Err(WalkError::Cancelled);
    }
    Ok(RelationSet {
        key: RelationKey {
            snapshot: index.snapshot_id().into(),
            root,
            direction,
        },
        hits,
    })
}

pub struct RelationReply {
    pub ticket: u64,
    pub key: RelationKey,
    pub query: String,
    pub set: Arc<RelationSet>,
    pub visible: Vec<usize>,
}

struct Request {
    ticket: u64,
    root: u32,
    direction: Direction,
    query: String,
}

pub struct RelationWorker {
    requests: Option<mpsc::Sender<Request>>,
    epoch: Arc<AtomicU64>,
    reply: Arc<Mutex<Option<RelationReply>>>,
    thread: Option<JoinHandle<()>>,
}

impl RelationWorker {
    pub fn spawn(index: Arc<Index>) -> Self {
        let (tx, rx) = mpsc::channel::<Request>();
        let epoch = Arc::new(AtomicU64::new(0));
        let reply = Arc::new(Mutex::new(None));
        let worker_epoch = Arc::clone(&epoch);
        let worker_reply = Arc::clone(&reply);
        let thread = std::thread::Builder::new()
            .name("guixvis-relations".into())
            .spawn(move || {
                // At most two closures, one per direction for the current root.
                let mut cache: HashMap<RelationKey, Arc<RelationSet>> = HashMap::new();
                // Fold only packages actually filtered, then reuse that text
                // across queries and closure directions for this index.
                let mut lowered: Vec<Option<String>> = vec![None; index.len()];
                while let Ok(mut req) = rx.recv() {
                    while let Ok(newer) = rx.try_recv() {
                        req = newer;
                    }
                    let cancelled = || worker_epoch.load(Ordering::Relaxed) != req.ticket;
                    if cancelled() {
                        continue;
                    }
                    let key = RelationKey {
                        snapshot: index.snapshot_id().into(),
                        root: req.root,
                        direction: req.direction,
                    };
                    cache.retain(|key, _| key.root == req.root);
                    let set = if let Some(set) = cache.get(&key) {
                        Arc::clone(set)
                    } else {
                        let Ok(set) =
                            collect_relations(&index, req.root, req.direction, &cancelled)
                        else {
                            continue;
                        };
                        let set = Arc::new(set);
                        cache.insert(key.clone(), Arc::clone(&set));
                        set
                    };
                    let filter = RelationFilter::new(&req.query);
                    let mut visible = Vec::new();
                    for (i, hit) in set.hits.iter().enumerate() {
                        if i.is_multiple_of(256) && cancelled() {
                            break;
                        }
                        if filter.is_empty() {
                            visible.push(i);
                            continue;
                        }
                        let text = lowered[hit.id as usize].get_or_insert_with(|| {
                            let p = &index.packages[hit.id as usize];
                            relation_text(&p.name, &p.version)
                        });
                        if filter.matches_lowered(text) {
                            visible.push(i);
                        }
                    }
                    let mut out = worker_reply.lock().expect("relation reply mutex poisoned");
                    if !cancelled() {
                        *out = Some(RelationReply {
                            ticket: req.ticket,
                            key,
                            query: req.query,
                            set,
                            visible,
                        });
                    }
                }
            })
            .expect("failed to spawn relation worker");
        Self {
            requests: Some(tx),
            epoch,
            reply,
            thread: Some(thread),
        }
    }

    pub fn request(&self, root: u32, direction: Direction, query: String) -> u64 {
        let ticket = self.epoch.fetch_add(1, Ordering::Relaxed) + 1;
        *self.reply.lock().expect("relation reply mutex poisoned") = None;
        if let Some(tx) = &self.requests {
            let _ = tx.send(Request {
                ticket,
                root,
                direction,
                query,
            });
        }
        ticket
    }

    pub fn take_reply(&self) -> Option<RelationReply> {
        self.reply
            .lock()
            .ok()?
            .take()
            .filter(|r| r.ticket == self.epoch.load(Ordering::Relaxed))
    }

    pub fn cancel(&self) {
        self.epoch.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut out) = self.reply.lock() {
            *out = None;
        }
    }
}

impl Drop for RelationWorker {
    fn drop(&mut self) {
        self.cancel();
        self.requests = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn matches_relation(name: &str, version: &str, query: &str) -> bool {
    RelationFilter::new(query).matches(name, version)
}

/// Literal AND terms, Unicode-lowercased once for a filtering pass.
pub(crate) struct RelationFilter {
    terms: Vec<String>,
}

impl RelationFilter {
    pub(crate) fn new(query: &str) -> Self {
        Self {
            terms: query.split_whitespace().map(str::to_lowercase).collect(),
        }
    }

    fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    pub(crate) fn matches(&self, name: &str, version: &str) -> bool {
        self.is_empty() || self.matches_lowered(&relation_text(name, version))
    }

    fn matches_lowered(&self, text: &str) -> bool {
        self.terms.iter().all(|term| text.contains(term))
    }
}

fn relation_text(name: &str, version: &str) -> String {
    // Fold fields independently to retain Unicode context-sensitive casing.
    format!("{} {}", name.to_lowercase(), version.to_lowercase())
}
