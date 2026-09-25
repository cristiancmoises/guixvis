//! Index construction invariants over a hand-written fixture.

mod common;

use guixvis::index::Index;
use guixvis::model::IndexDoc;
use guixvis::search::dependents_by_name;

use common::load_fixture;

#[test]
fn exact_numeric_ids_preserve_versions_and_private_variants() {
    let doc: IndexDoc = serde_json::from_str(include_str!("fixtures/identity.json"))
        .expect("numeric identity document parses");
    let index = Index::from_doc(doc, 0).expect("exact identity document validates");
    assert_eq!(index.packages[0].inputs.as_ref(), &[1, 2, 3, 4, 5]);
    assert_eq!(index.packages[0].propagated.as_ref(), &[1]);
    assert_eq!(index.packages[0].native.as_ref(), &[1]);
    assert_eq!(index.packages[0].dep_count(), 5);
    assert_eq!(index.names["variant"], 3);
    assert_eq!(index.dependents[1].as_ref(), &[0]);
    assert_eq!(index.packages[0].relation_count(), 7);
    assert_eq!(
        index.packages[0].dep_kinds(1),
        vec![
            guixvis::index::DepKind::Input,
            guixvis::index::DepKind::Propagated,
            guixvis::index::DepKind::Native,
        ]
    );
    assert!(!index.packages[4].catalog);
    assert!(index.is_complete());
}

#[test]
fn shuffled_ids_are_sorted_and_catalog_wins_legacy_lookup() {
    let mut doc: IndexDoc = serde_json::from_str(include_str!("fixtures/identity.json")).unwrap();
    doc.packages[3].catalog = false;
    doc.packages[4].catalog = true;
    doc.packages.reverse();
    let index = Index::from_doc(doc, 0).unwrap();
    assert_eq!(index.names["variant"], 4);
    for (id, package) in index.packages.iter().enumerate() {
        assert_eq!(package.id as usize, id);
    }
}

#[test]
fn diagnostics_and_identity_survive_binary_round_trip() {
    let mut doc: IndexDoc = serde_json::from_str(include_str!("fixtures/identity.json")).unwrap();
    doc.diagnostics.push(guixvis::model::IndexDiagnostic {
        package_id: 0,
        kind: "native".into(),
        code: "accessor-failed".into(),
        message: "Could not read inputs".into(),
    });
    let index = Index::from_doc(doc, 0).unwrap();
    let bytes = guixvis::blob::encode(&index);
    let loaded = guixvis::blob::decode(&bytes, 1).unwrap();
    assert!(!loaded.is_complete());
    assert_eq!(index.diagnostics, loaded.diagnostics);
    assert!(!loaded.packages[4].catalog);
    assert_eq!(loaded.packages[0].relation_count(), 7);
    let mut trailing = bytes;
    trailing.push(0);
    assert!(guixvis::blob::decode(&trailing, 0).is_err());
}

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

    // Unknown identities are structural corruption, never silent omissions.
    doc.packages[0].inputs.push(99);
    assert!(Index::from_doc(doc, 0).is_err());
}

#[test]
fn untrusted_text_is_safe_in_both_document_and_blob_paths() {
    let mut doc: IndexDoc = serde_json::from_str(include_str!("fixtures/small.json")).unwrap();
    doc.packages[0].name = "emacs\u{1b}]52;c;payload\u{7}".into();
    doc.packages[0].synopsis = "line\nnext\tword\u{1b}[31m".into();
    doc.header.origin = guixvis::model::GuixOrigin {
        executable: "/bin/guix\u{1b}".into(),
        system: "x86_64-linux\u{7}".into(),
        channels: vec![guixvis::model::ChannelPin {
            name: "guix\u{9b}".into(),
            commit: "abc".into(),
        }],
        verified: true,
        mutable_package_path: false,
    };
    doc.diagnostics.push(guixvis::model::IndexDiagnostic {
        package_id: 0,
        kind: "input\u{1b}".into(),
        code: "fail\u{7}".into(),
        message: "oops\u{9b}31m".into(),
    });
    let mut index = Index::from_doc(doc, 0).unwrap();
    assert!(!index.packages[0].name.chars().any(char::is_control));
    assert!(!index.diagnostics[0].message.chars().any(char::is_control));
    assert!(!index.origin.is_verified());
    assert!(!index.origin.executable.chars().any(char::is_control));
    assert!(!index.origin.system.chars().any(char::is_control));
    assert!(!index.origin.channels[0].name.chars().any(char::is_control));
    index.packages[0].name = std::sync::Arc::from("evil\u{1b}\u{7}");
    index.packages[0].inputs = vec![1, 1].into();
    let decoded = guixvis::blob::decode(&guixvis::blob::encode(&index), 0).unwrap();
    assert!(!decoded.packages[0].name.chars().any(char::is_control));
    assert_eq!(decoded.packages[0].inputs.as_ref(), &[1]);
}
