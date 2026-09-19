//! Micro-benchmark harness: cache load, search latency, graph layout.
//!
//! Run with a warm cache:
//!
//! ```sh
//! cargo run --release --example bench
//! ```

use std::time::{Duration, Instant};

use guixvis::{cache, graph, search};

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1000.0)
}

fn best_of<F: FnMut()>(runs: usize, mut f: F) -> Duration {
    let mut best = Duration::MAX;
    for _ in 0..runs {
        let t = Instant::now();
        f();
        best = best.min(t.elapsed());
    }
    best
}

fn main() {
    let Ok(c) = cache::Cache::new() else {
        eprintln!("no cache directory");
        return;
    };
    let path = c.path();
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!(
            "no snapshot at {} — run guixvis once to build it",
            path.display()
        );
        return;
    };
    println!(
        "snapshot: {} ({:.2} MB)",
        path.display(),
        bytes.len() as f64 / 1e6
    );

    let t = Instant::now();
    let idx = match guixvis::blob::decode(&bytes, 0) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("snapshot decode failed: {e}");
            return;
        }
    };
    let load = t.elapsed();
    println!("  decode : {}", ms(load));
    println!("  packages: {}", idx.packages.len());
    println!("  TOTAL snapshot->usable index: {}", ms(load));

    // ---- search -----------------------------------------------------------
    let engine = search::SearchEngine::new(&idx);
    let queries = [
        "emacs", "python", "rust", "libre", "guile", "tor", "zlib", "x",
    ];
    println!("\nsearch (best of 20, limit 500):");
    let mut all = Duration::ZERO;
    let mut n = 0;
    for q in queries {
        let mut hits = 0;
        let d = best_of(20, || {
            hits = engine.search(&idx, q, 500).len();
        });
        all += d;
        n += 1;
        println!("  {q:<8} {:>10}  ({hits} hits)", ms(d));
    }
    println!("  mean   : {}", ms(all / n));
    let empty = best_of(20, || {
        engine.search(&idx, "", 500);
    });
    println!("  empty query (recent list): {}", ms(empty));

    // ---- graph ------------------------------------------------------------
    println!("\ngraph project + layout (300 iterations):");
    let roots: Vec<&str> = vec!["emacs", "python", "zlib", "glibc"];
    for name in roots {
        let Some(&root) = idx.names.get(name) else {
            eprintln!("  {name}: not in index");
            continue;
        };
        for (label, dir, depth) in [
            ("deps", graph::Dir::Deps, 2u8),
            ("reverse", graph::Dir::Dependents, 1),
        ] {
            let t = Instant::now();
            let proj = graph::project(&idx, root, dir, depth, 200);
            let project_ms = t.elapsed();
            let mut view = graph::GraphView::new(root, depth);
            let t = Instant::now();
            view.rebuild(&idx, root, depth);
            let layout = t.elapsed();
            println!(
                "  {name:<7} {label:<8} nodes={:<4} edges={:<5} project={} layout={}",
                proj.nodes.len(),
                proj.edges.len(),
                ms(project_ms),
                ms(layout)
            );
        }
    }
}
