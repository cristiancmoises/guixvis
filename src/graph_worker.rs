//! Cancellable projection and layout, never executed during a terminal draw.
use crate::graph::{Dir, GraphView};
use crate::index::Index;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;

pub struct GraphReply {
    pub ticket: u64,
    pub snapshot: String,
    pub view: GraphView,
}
pub struct GraphWorker {
    requests: Option<mpsc::Sender<(u64, u32, u8)>>,
    epoch: Arc<AtomicU64>,
    reply: Arc<Mutex<Option<GraphReply>>>,
    thread: Option<JoinHandle<()>>,
}
impl GraphWorker {
    pub fn spawn(index: Arc<Index>) -> Self {
        let (tx, rx) = mpsc::channel::<(u64, u32, u8)>();
        let epoch = Arc::new(AtomicU64::new(0));
        let reply = Arc::new(Mutex::new(None));
        let worker_epoch = Arc::clone(&epoch);
        let worker_reply = Arc::clone(&reply);
        let thread = std::thread::Builder::new()
            .name("guixvis-graph".into())
            .spawn(move || {
                while let Ok(mut req) = rx.recv() {
                    while let Ok(next) = rx.try_recv() {
                        req = next;
                    }
                    let cancelled = || worker_epoch.load(Ordering::Relaxed) != req.0;
                    let Ok(view) =
                        GraphView::build_cancellable(&index, req.1, req.2, Dir::Deps, &cancelled)
                    else {
                        continue;
                    };
                    let mut out = worker_reply.lock().expect("graph reply mutex poisoned");
                    if !cancelled() {
                        *out = Some(GraphReply {
                            ticket: req.0,
                            snapshot: index.snapshot_id().into(),
                            view,
                        });
                    }
                }
            })
            .expect("failed to spawn graph worker");
        Self {
            requests: Some(tx),
            epoch,
            reply,
            thread: Some(thread),
        }
    }
    pub fn request(&self, root: u32, depth: u8) -> u64 {
        let ticket = self.epoch.fetch_add(1, Ordering::Relaxed) + 1;
        *self.reply.lock().expect("graph reply mutex poisoned") = None;
        if let Some(tx) = &self.requests {
            let _ = tx.send((ticket, root, depth));
        }
        ticket
    }
    pub fn take_reply(&self) -> Option<GraphReply> {
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
impl Drop for GraphWorker {
    fn drop(&mut self) {
        self.cancel();
        self.requests = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
