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
        let mut visit = |dep: u32, kind: DepKind| -> Result<(), WalkError> {
            work += 1;
            if work.is_multiple_of(256) && cancelled() {
                return Err(WalkError::Cancelled);
            }
            kinds[dep as usize] |= 1 << kind as u8;
            if depths[dep as usize] == usize::MAX {
                depths[dep as usize] = next_depth;
                queue.push_back(dep);
            }
            Ok(())
        };
        match direction {
            Direction::Dependencies => {
                for (dep, kind) in index.packages[id as usize].typed_deps() {
                    visit(dep, kind)?;
                }
            }
            Direction::Dependents => {
                for &dep in index.dependents[id as usize].iter() {
                    for kind in index.packages[dep as usize].dep_kinds(id) {
                        visit(dep, kind)?;
                    }
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
                    let mut visible = Vec::new();
                    for (i, hit) in set.hits.iter().enumerate() {
                        if i.is_multiple_of(256) && cancelled() {
                            break;
                        }
                        let p = &index.packages[hit.id as usize];
                        if matches_relation(&p.name, &p.version, &req.query) {
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
    let hay = format!("{} {}", name.to_lowercase(), version.to_lowercase());
    query
        .split_whitespace()
        .all(|term| hay.contains(&term.to_lowercase()))
}
