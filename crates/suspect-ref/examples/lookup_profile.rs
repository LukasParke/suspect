//! Coarse reference lookup profile against an explicitly supplied corpus.
//! Run in release mode; preparation is excluded from measured lookup phases.

use std::time::Instant;

use suspect_ref::WorkspaceBuilder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("supply a corpus path")?;
    let workspace = WorkspaceBuilder::new().build()?;
    workspace.load_all(&path)?;
    let document = workspace.open(&path)?;
    let nodes: Vec<_> = document
        .edges()
        .iter()
        .map(|edge| {
            document
                .doc()
                .root()
                .pointer(&edge.path)
                .unwrap()
                .get("$ref")
                .unwrap()
        })
        .collect();
    let started = Instant::now();
    let targets: Vec<_> = nodes
        .iter()
        .map(|node| document.ref_target(*node).unwrap())
        .collect();
    println!("{} direct refs: {:?}", targets.len(), started.elapsed());

    let started = Instant::now();
    let landed: Vec<_> = targets
        .iter()
        .map(|target| {
            workspace
                .get_by_id(target.doc)
                .unwrap()
                .doc()
                .root()
                .pointer(&target.pointer)
                .unwrap()
        })
        .collect();
    println!("target pointer materialization: {:?}", started.elapsed());

    let started = Instant::now();
    for node in &landed {
        let doc = node.syntax().doc();
        let range = node.syntax().byte_range();
        let syntax_id = node.syntax().raw().id();
        let mut raw = doc
            .root()
            .raw()
            .descendant_for_byte_range(range.start, range.end.saturating_sub(1))
            .unwrap();
        while raw.byte_range() != range || raw.id() != syntax_id {
            raw = raw.parent().unwrap();
        }
        assert_eq!(raw.byte_range(), range);
        std::hint::black_box(raw);
    }
    println!(
        "target source-range materialization: {:?}",
        started.elapsed()
    );
    Ok(())
}
