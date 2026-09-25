//! Shared helpers for integration tests.
#![allow(dead_code)]

use guixvis::index::Index;
use guixvis::model::IndexDoc;

pub fn load_fixture() -> Index {
    let doc: IndexDoc =
        serde_json::from_str(include_str!("../fixtures/small.json")).expect("fixture parses");
    Index::from_doc(doc, 0).expect("fixture validates")
}

pub fn identity_fixture() -> Index {
    let doc = serde_json::from_str(include_str!("../fixtures/identity.json")).unwrap();
    Index::from_doc(doc, 0).unwrap()
}

pub fn chain_fixture(count: u32) -> Index {
    let packages: Vec<_> = (0..count)
        .map(|id| {
            serde_json::json!({
                "id": id, "name": format!("p{id:05}"), "version": "1", "catalog": true,
                "inputs": if id + 1 < count { vec![id + 1] } else { vec![] }
            })
        })
        .collect();
    let doc = serde_json::from_value(serde_json::json!({
        "header": {"schema": 4, "package_count": count}, "packages": packages,
    }))
    .unwrap();
    Index::from_doc(doc, 0).unwrap()
}
