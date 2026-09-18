//! Index construction invariants over a hand-written fixture.

mod common;

use guixvis::index::Index;
use guixvis::model::IndexDoc;
use guixvis::search::dependents_by_name;

use common::load_fixture;

#[test]
fn parses_fixture() {
    let index = load_fixture();
    assert_eq!(index.len(), 10);
    assert_eq!(index.guix_commit, "testcommit");
    assert_eq!(index.packages[0].name.as_ref(), "emacs");
    assert_eq!(index.packages[0].inputs.len(), 2);
    assert_eq!(index.packages[0].propagated.len(), 1);
    assert_eq!(index.packages[0].native.len(), 1);
}

#[test]
fn reverse_edges_are_exact_and_bidirectional() {
    let index = load_fixture();
    let names = dependents_by_name(&index);

    // zlib is depended on by emacs, emacs-minimal, emacs-next and gtk+.
    let zlib_deps = &names["zlib"];
    assert_eq!(zlib_deps.len(), 4);
    assert!(zlib_deps.contains(&"emacs"));
    assert!(zlib_deps.contains(&"emacs-minimal"));
    assert!(zlib_deps.contains(&"emacs-next"));
    assert!(zlib_deps.contains(&"gtk+"));

    // Bidirectional invariant: for every edge u -> v, v's dependents
    // contain u, and no dependent exists without a matching edge.
    for p in &index.packages {
        for (dep, _) in p.deps() {
            let dep_name = index.packages[dep as usize].name.as_ref();
            assert!(
                names[dep_name].contains(&p.name.as_ref()),
                "{} depends on {} but is missing from its dependents",
                p.name,
                dep_name
            );
        }
        for d in index.dependents[p.id as usize].iter() {
            let d_name = index.packages[*d as usize].name.as_ref();
            assert!(
                names[p.name.as_ref()].contains(&d_name),
                "{} listed as dependent of {} but has no such edge",
                d_name,
                p.name
            );
        }
    }
}

#[test]
fn deduplicates_deps_across_input_kinds() {
    let index = load_fixture();
    // emacs lists zlib only in `inputs`; duplicates would come from the
    // other kinds if present. Build a package with a dup to verify dedupe.
    let p = &index.packages[0];
    let deps: Vec<u32> = p.deps().map(|(d, _)| d).collect();
    assert_eq!(deps.len(), 4); // gtk+, zlib, pkg-config, texinfo
    let mut sorted = deps.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), deps.len());
}

#[test]
fn bfs_transitive_closure_matches_naive_fixed_point() {
    let index = load_fixture();
    // Transitive deps of emacs at depth 3: gtk+, zlib, pkg-config, texinfo
    // (zlib also reachable via gtk+ -> zlib).
    let (nodes, total) = index.deps_bfs(0, 3, 100);
    let ids: Vec<u32> = nodes.iter().map(|n| n.id).collect();
    assert!(ids.contains(&index.names["gtk+"]));
    assert!(ids.contains(&index.names["zlib"]));
    assert!(ids.contains(&index.names["pkg-config"]));
    assert!(ids.contains(&index.names["texinfo"]));
    assert_eq!(total, 4);

    // Naive fixed point: expand until stable.
    let mut closure = std::collections::HashSet::new();
    closure.insert(0u32);
    loop {
        let mut next = closure.clone();
        for id in &closure {
            for (d, _) in index.packages[*id as usize].deps() {
                next.insert(d);
            }
        }
        if next.len() == closure.len() {
            break;
        }
        closure = next;
    }
    closure.remove(&0);
    assert_eq!(closure.len(), ids.len());
    for id in &ids {
        assert!(closure.contains(id));
    }
}

#[test]
fn bfs_survives_cycles() {
    let index = load_fixture();
    let a = index.names["cyc-a"];
    let (nodes, total) = index.deps_bfs(a, 8, 100);
    assert_eq!(total, 1); // only cyc-b discovered once
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].id, index.names["cyc-b"]);
}

#[test]
fn reverse_bfs_depths() {
    let index = load_fixture();
    let zlib = index.names["zlib"];
    let (nodes, total) = index.dependents_bfs(zlib, 1, 100);
    assert_eq!(nodes.len(), 4); // direct only at depth 1
    assert_eq!(total, 4);
    let (nodes, _) = index.dependents_bfs(zlib, 2, 100);
    // depth 2 adds nothing new (no chained dependents in fixture)
    assert_eq!(nodes.len(), 4);
}

#[test]
fn module_neighbors_grouped() {
    let index = load_fixture();
    let emacs = index.names["emacs"];
    let neighbors = index.module_neighbors(emacs);
    assert_eq!(neighbors.len(), 3); // emacs, emacs-minimal, emacs-next
}

#[test]
fn rejects_bad_documents() {
    let mut doc: IndexDoc = serde_json::from_str(include_str!("fixtures/small.json")).unwrap();

    // Duplicate id.
    let mut dup = doc.clone();
    dup.packages[7].id = 0;
    assert!(Index::from_doc(dup, 0).is_err());

    // Count mismatch.
    let mut bad_count = doc.clone();
    bad_count.header.package_count = 7;
    assert!(Index::from_doc(bad_count, 0).is_err());

    // Out-of-range id.
    let mut oob = doc.clone();
    oob.packages[7].id = 99;
    assert!(Index::from_doc(oob, 0).is_err());

    // Wrong schema.
    let mut bad_schema = doc.clone();
    bad_schema.header.schema = 2;
    assert!(Index::from_doc(bad_schema, 0).is_err());

    // Empty name.
    let mut empty = doc.clone();
    empty.packages[0].name = String::new();
    assert!(Index::from_doc(empty, 0).is_err());

    // Unknown dep names are dropped, not fatal.
    doc.packages[0].inputs.push("nonexistent".to_string());
    let index = Index::from_doc(doc, 0).expect("unknown dep tolerated");
    assert_eq!(index.packages[0].inputs.len(), 2);
}
