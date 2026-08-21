use std::fs;
use std::path::{Path, PathBuf};
use suspect_ref::WorkspaceBuilder;

fn ws(dir: &Path) -> suspect_ref::Workspace {
    WorkspaceBuilder::new().root(dir).build().unwrap()
}

#[test]
fn probe_edge_decode() {
    let dir = std::env::temp_dir().join("suspect-probe-edge");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let p: PathBuf = dir.join("stripe-style.yaml");
    fs::write(&p, "components:\n  schemas:\n    A:\n      $ref: >-\n        #/components/schemas/B\n    B:\n      type: object\n").unwrap();
    let w = ws(&dir);
    match w.load_all("stripe-style.yaml") {
        Ok(n) => println!("LOADED {n} docs"),
        Err(e) => println!("LOAD ERR: {e:?}"),
    }
    let h = w.open("stripe-style.yaml").unwrap();
    for e in h.edges().iter() {
        println!("EDGE at {:?} raw={:?} parsed={:?}", e.at, e.raw, e.parsed);
    }
}
