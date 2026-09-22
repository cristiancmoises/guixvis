//! Background fuzzy search over name + synopsis.
//!
//! A dedicated worker thread owns the nucleo haystacks; the UI thread sends
//! queries over a channel (latest-wins) and picks up replies through a
//! single-slot, ticket-stamped mailbox. Highlight ranges are precomputed for
//! the displayed prefix of hits, in *character* offsets.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32String};

use crate::index::{Index, SYN_LIMIT};

/// Render cap for the result list.
pub const RESULT_CAP: usize = 500;
/// How many top hits get precomputed highlight ranges.
pub const HIGHLIGHT_CAP: usize = 100;
/// Keystroke debounce before a query is dispatched.
pub const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(30);

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub id: u32,
    pub score: u32,
    /// The query matched the package name itself (used as a tie-breaker).
    pub name_match: bool,
}

/// Character offsets (half-open) of fuzzy matches within the name and within
/// the synopsis, respectively.
#[derive(Debug, Clone)]
pub struct HighlightedHit {
    pub hit: SearchHit,
    pub name_ranges: Vec<(usize, usize)>,
    pub synopsis_ranges: Vec<(usize, usize)>,
}

#[derive(Debug, Clone)]
pub struct SearchReply {
    pub ticket: u64,
    pub query: String,
    pub hits: Vec<HighlightedHit>,
}

/// Synchronous fuzzy-search engine over one `Index`.
///
/// Owns the nucleo haystacks (name + truncated synopsis per package) and can
/// be called directly — used by the web API handlers — or driven through
/// [`SearchWorker`] on a background thread, as the TUI does.
pub struct SearchEngine {
    hay: Haystacks,
}

impl SearchEngine {
    pub fn new(index: &Index) -> Self {
        SearchEngine {
            hay: build_haystacks(index),
        }
    }

    /// Ranked search with highlight ranges; `limit` caps the result list.
    pub fn search(&self, index: &Index, query: &str, limit: usize) -> Vec<HighlightedHit> {
        scan(index, &self.hay, query, limit)
    }
}

pub struct SearchWorker {
    req_tx: Option<mpsc::Sender<(u64, String)>>,
    reply: Arc<Mutex<Option<SearchReply>>>,
    ticket: Arc<AtomicU64>,
    handle: Option<JoinHandle<()>>,
}

impl SearchWorker {
    /// Spawn the worker for an index. Haystacks are built on the worker
    /// thread so the caller never blocks on it.
    pub fn spawn(index: Arc<Index>) -> Self {
        let (req_tx, req_rx) = mpsc::channel::<(u64, String)>();
        let reply: Arc<Mutex<Option<SearchReply>>> = Arc::new(Mutex::new(None));
        let ticket: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
        let reply2 = Arc::clone(&reply);
        let handle = std::thread::Builder::new()
            .name("guixvis-search".into())
            .spawn(move || worker_loop(index, req_rx, reply2))
            .expect("failed to spawn search worker");
        SearchWorker {
            req_tx: Some(req_tx),
            reply,
            ticket,
            handle: Some(handle),
        }
    }

    /// Dispatch a query; returns its ticket. The worker processes the most
    /// recent request (latest-wins) and stamps the reply with that ticket.
    pub fn send(&self, query: String) -> u64 {
        let ticket = self.ticket.fetch_add(1, Ordering::Relaxed) + 1;
        // Invariant: send() is only called while the worker exists.
        if let Some(tx) = self.req_tx.as_ref() {
            let _ = tx.send((ticket, query));
        }
        ticket
    }

    /// Take the freshest reply if its ticket is newer than `last`.
    pub fn take_reply(&self, last: u64) -> Option<SearchReply> {
        let reply = self.reply.lock().ok()?;
        if reply.as_ref().map(|r| r.ticket) <= Some(last) {
            return None;
        }
        reply.clone()
    }
}

impl Drop for SearchWorker {
    fn drop(&mut self) {
        // Close the channel FIRST so the worker's recv() returns Err and it
        // exits; then join (bounded, because the exit is immediate).
        self.req_tx = None;
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Haystacks prepared by the worker: one `Utf32String` per package for the
/// combined name+synopsis, and one for the name alone.
struct Haystacks {
    combined: Vec<Utf32String>,
    names: Vec<Utf32String>,
    name_len: Vec<u32>, // char length of each package name
    /// Lowercased name + synopsis, for the candidate prefilter.
    lower: Vec<String>,
    /// Lowercased names, for the substring boost.
    lower_names: Vec<String>,
    /// Global byte frequencies over the folded haystacks: the prefilter
    /// checks only the rarest byte of each term, which skips the most
    /// candidates for a single memchr per candidate.
    byte_freq: [u32; 256],
    /// Package ids ordered for the empty-query browse: the highest-fan-in
    /// hubs first (name breaks ties), because that is what people want to
    /// find before they type anything.
    by_hub: Vec<u32>,
}

fn build_haystacks(index: &Index) -> Haystacks {
    let mut combined = Vec::with_capacity(index.len());
    let mut names = Vec::with_capacity(index.len());
    let mut name_len = Vec::with_capacity(index.len());
    let mut lower = Vec::with_capacity(index.len());
    let mut lower_names = Vec::with_capacity(index.len());
    for p in &index.packages {
        let name_len_c = p.name.chars().count();
        let name = Utf32String::from(p.name.as_ref());
        let syn = &p.synopsis;
        let syn_trunc: String = syn.chars().take(SYN_LIMIT).collect();
        let mut hay = String::with_capacity(p.name.len() + 1 + syn_trunc.len());
        hay.push_str(&p.name);
        hay.push(' ');
        hay.push_str(&syn_trunc);
        name_len.push(name_len_c as u32);
        names.push(name);
        lower_names.push(p.name.to_lowercase());
        lower.push(hay.to_lowercase());
        combined.push(Utf32String::from(hay));
    }
    let mut freq = [0u32; 256];
    for hay in &lower {
        for &b in hay.as_bytes() {
            freq[b as usize] += 1;
        }
    }
    let mut by_hub: Vec<u32> = (0..index.packages.len() as u32).collect();
    by_hub.sort_by(|a, b| {
        let pa = &index.packages[*a as usize];
        let pb = &index.packages[*b as usize];
        index
            .dependents_count(*b)
            .cmp(&index.dependents_count(*a))
            .then_with(|| pa.name.cmp(&pb.name))
    });
    Haystacks {
        combined,
        names,
        name_len,
        lower,
        lower_names,
        byte_freq: freq,
        by_hub,
    }
}

/// Chunk boundaries for the parallel scan.
fn chunk_ranges(len: usize, threads: usize) -> Vec<(usize, usize)> {
    let threads = threads.clamp(1, 8);
    let chunk = len.div_ceil(threads);
    (0..len)
        .step_by(chunk)
        .map(|s| (s, (s + chunk).min(len)))
        .collect()
}

fn worker_loop(
    index: Arc<Index>,
    req_rx: mpsc::Receiver<(u64, String)>,
    reply: Arc<Mutex<Option<SearchReply>>>,
) {
    let engine = SearchEngine::new(&index);
    while let Ok((mut ticket, mut query)) = req_rx.recv() {
        // Latest-wins: drain any newer requests that arrived meanwhile.
        while let Ok(newer) = req_rx.try_recv() {
            (ticket, query) = newer;
        }
        let hits = engine.search(&index, &query, RESULT_CAP);
        let mut out = reply.lock().expect("reply mutex poisoned");
        *out = Some(SearchReply {
            ticket,
            query,
            hits,
        });
    }
}

fn scan(index: &Index, hay: &Haystacks, query: &str, limit: usize) -> Vec<HighlightedHit> {
    // Empty query: browse the hubs (highest fan-in first), not the alphabet.
    if query.trim().is_empty() {
        return hay
            .by_hub
            .iter()
            .take(limit)
            .map(|id| HighlightedHit {
                hit: SearchHit {
                    id: *id,
                    score: 0,
                    name_match: false,
                },
                name_ranges: Vec::new(),
                synopsis_ranges: Vec::new(),
            })
            .collect();
    }

    // Terms are AND-ed: a package must match every one of them. Per-term
    // scores combine as their geometric mean, so one weak term can pull a
    // strong one down but never erase it entirely.
    let terms: Vec<&str> = query.split_whitespace().collect();
    let patterns: Vec<Pattern> = terms
        .iter()
        .map(|t| Pattern::parse(t, CaseMatching::Smart, Normalization::Smart))
        .collect();
    let query_folded = query.to_lowercase();

    // Prefilter: a fuzzy match needs every byte of every (ASCII) term in the
    // haystack, so checking just the rarest byte of each term rejects the
    // most candidates for one memchr each. Non-ASCII terms take the safe
    // path, because Smart normalization can rewrite their bytes.
    // The check only pays for itself when the rare byte actually rejects:
    // a byte present in almost every haystack costs a memchr and filters
    // nothing, so those terms skip the prefilter altogether.
    let total = index.len() as u32;
    let rarest: Vec<Option<u8>> = terms
        .iter()
        .map(|t| {
            let best = t
                .as_bytes()
                .iter()
                .filter(|b| b.is_ascii())
                .copied()
                .min_by_key(|b| hay.byte_freq[*b as usize])?;
            (hay.byte_freq[best as usize] * 100 < total * 80).then_some(best)
        })
        .collect();

    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let ranges = chunk_ranges(index.len(), threads);

    let partials: Vec<Vec<SearchHit>> = std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(ranges.len());
        for (start, end) in &ranges {
            let index = &index;
            let patterns = &patterns;
            let rarest = &rarest;
            let query_folded = &query_folded;
            handles.push(scope.spawn(move || {
                let mut matcher = Matcher::new(Config::DEFAULT);
                let mut local: Vec<SearchHit> = Vec::new();
                'candidates: for i in *start..*end {
                    for &b in rarest {
                        if let Some(b) = b {
                            if !hay.lower[i].as_bytes().contains(&b) {
                                continue 'candidates;
                            }
                        }
                    }

                    // Every term must score; combine as a geometric mean.
                    let mut log_sum = 0.0f64;
                    let mut name_match = true;
                    for (pi, pattern) in patterns.iter().enumerate() {
                        let Some(score) = pattern.score(hay.combined[i].slice(..), &mut matcher)
                        else {
                            continue 'candidates;
                        };
                        log_sum += (score.max(1) as f64).ln();
                        if name_match && pi < patterns.len() {
                            name_match = pattern
                                .score(hay.names[i].slice(..), &mut matcher)
                                .is_some();
                        }
                    }
                    let score = (log_sum / patterns.len() as f64).exp() as u32;

                    // Substring boosts: an exact hit on the name beats a fuzzy
                    // one; a hit in the synopsis beats a scattered match.
                    let score = if hay.lower_names[i].contains(query_folded.as_str()) {
                        score.saturating_add(700)
                    } else if hay.lower[i].contains(query_folded.as_str()) {
                        score.saturating_add(200)
                    } else {
                        score
                    };

                    local.push(SearchHit {
                        id: i as u32,
                        score,
                        name_match,
                    });
                }
                local.sort_unstable_by(|a, b| hit_cmp(index, a, b));
                local.truncate(limit);
                local
            }));
        }
        handles
            .into_iter()
            .map(|h| h.join().expect("search chunk panicked"))
            .collect()
    });

    let mut hits: Vec<SearchHit> = partials.into_iter().flatten().collect();
    hits.sort_unstable_by(|a, b| hit_cmp(index, a, b));
    hits.truncate(limit);

    // Precompute highlight ranges for the displayed prefix: the union of the
    // per-term match indices, merged into runs.
    let mut matcher = Matcher::new(Config::DEFAULT);
    hits.into_iter()
        .enumerate()
        .map(|(pos, hit)| {
            if pos >= HIGHLIGHT_CAP {
                return HighlightedHit {
                    hit,
                    name_ranges: Vec::new(),
                    synopsis_ranges: Vec::new(),
                };
            }
            let id = hit.id as usize;
            let name_len = hay.name_len[id] as usize;
            let mut indices: Vec<u32> = Vec::new();
            for pattern in &patterns {
                let mut part = Vec::new();
                let _ = pattern.indices(hay.combined[id].slice(..), &mut matcher, &mut part);
                indices.extend(part);
            }
            indices.sort_unstable();
            indices.dedup();
            let mut name_ranges = Vec::new();
            let mut synopsis_ranges = Vec::new();
            let mut run: Option<(usize, usize)> = None;
            for idx in indices {
                let idx = idx as usize;
                match run {
                    Some((st, e)) if idx == e => run = Some((st, e + 1)),
                    _ => {
                        if let Some((st, e)) = run.take() {
                            push_range(name_len, st, e, &mut name_ranges, &mut synopsis_ranges);
                        }
                        run = Some((idx, idx + 1));
                    }
                }
            }
            if let Some((st, e)) = run {
                push_range(name_len, st, e, &mut name_ranges, &mut synopsis_ranges);
            }
            HighlightedHit {
                hit,
                name_ranges,
                synopsis_ranges,
            }
        })
        .collect()
}

fn push_range(
    name_len: usize,
    start: usize,
    end: usize,
    name_ranges: &mut Vec<(usize, usize)>,
    synopsis_ranges: &mut Vec<(usize, usize)>,
) {
    if end <= name_len {
        name_ranges.push((start, end));
    } else if start > name_len {
        synopsis_ranges.push((start - name_len - 1, end - name_len - 1));
    }
    // Ranges straddling the space separator are clamped out.
}

/// Ranking: higher score first; name matches win ties; shorter names next;
/// lexicographic order last, so results are deterministic.
fn hit_cmp(index: &Index, a: &SearchHit, b: &SearchHit) -> std::cmp::Ordering {
    b.score
        .cmp(&a.score)
        .then_with(|| b.name_match.cmp(&a.name_match))
        .then_with(|| {
            let na = &index.packages[a.id as usize].name;
            let nb = &index.packages[b.id as usize].name;
            na.len().cmp(&nb.len()).then_with(|| na.cmp(nb))
        })
}

/// Map of dependent-name -> dependent ids used by tests.
pub fn dependents_by_name(index: &Index) -> HashMap<&str, Vec<&str>> {
    let mut out: HashMap<&str, Vec<&str>> = HashMap::new();
    for p in &index.packages {
        for d in index.dependents[p.id as usize].iter() {
            out.entry(p.name.as_ref())
                .or_default()
                .push(index.packages[*d as usize].name.as_ref());
        }
    }
    out
}
