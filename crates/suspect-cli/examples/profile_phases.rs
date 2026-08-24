//! Phase-level profiler for the stripe corpus: where do the 705 ms of
//! parse+IR and the 12.8 s of lint+validate actually go?
//!
//! Run: `cargo run --release -p suspect-cli --example profile_phases`

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use suspect_low::NodeRef;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/stripe.yaml")
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

fn main() -> anyhow::Result<()> {
    println!("=== stripe.yaml phase profiler (release) ===\n");
    let path = corpus();
    let dir = path.parent().unwrap().to_path_buf();
    let bytes = std::fs::read(&path)?;
    println!(
        "corpus: {} ({:.1} MB)\n",
        path.display(),
        bytes.len() as f64 / 1024.0 / 1024.0
    );

    // Warm once.
    {
        let ws = WorkspaceBuilder::new().root(&dir).build()?;
        ws.load_all("stripe.yaml")?;
    }

    // ---- parse pipeline breakdown ----
    println!("[workspace load]");
    let mut load_times = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        let ws = WorkspaceBuilder::new().root(&dir).build()?;
        ws.load_all("stripe.yaml")?;
        load_times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    println!(
        "  {:<52} {:>9.2} ms (median of 5)",
        "WorkspaceBuilder + load_all",
        median(load_times)
    );

    // Split load_all internals: syntax parse alone via suspect_syntax.
    {
        use suspect_source::Source;
        let uri = Uri::from_path(&path)?;
        let mut parse_times = Vec::new();
        for _ in 0..5 {
            let t = Instant::now();
            let _doc =
                suspect_syntax::SourceDoc::parse(uri.clone(), Source::from_vec(bytes.clone()));
            parse_times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        println!(
            "  {:<52} {:>9.2} ms (median of 5)",
            "suspect-syntax::SourceDoc::parse",
            median(parse_times)
        );
    }

    // LowDoc construction on top.
    {
        use suspect_source::Source;
        let uri = Uri::from_path(&path)?;
        let mut low_times = Vec::new();
        for _ in 0..5 {
            let t = Instant::now();
            let _low = suspect_low::LowDoc::parse(uri.clone(), Source::from_vec(bytes.clone()));
            low_times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        println!(
            "  {:<52} {:>9.2} ms (median of 5)",
            "LowDoc::parse (lossless wrapper)",
            median(low_times)
        );
    }

    // IR build split: workspace reuse.
    let ws = Arc::new({
        let ws = WorkspaceBuilder::new().root(&dir).build()?;
        ws.load_all("stripe.yaml")?;
        ws
    });
    let entry = Uri::from_path(&path)?;

    println!("\n[IR build on preloaded workspace]");
    {
        let handle = ws.get(&entry).unwrap();
        let doc = handle.doc();
        let root = doc.root();

        // Count nodes to size the walk.
        let t = Instant::now();
        let node_count = count_nodes(root);
        let walk_ms = t.elapsed().as_secs_f64() * 1000.0;
        println!("  {node_count:>9} nodes; plain traversal {walk_ms:.2} ms");

        // Schema materialization cost (overlay roundtrip) per component.
        if let Some(components) = root.get("components")
            && let Some(schemas) = components.get("schemas")
        {
            let entries = schemas.entries();
            let names: Vec<_> = entries.iter().map(|e| e.key.to_owned()).collect();
            let t = Instant::now();
            for name in &names {
                let node = schemas.get(name).unwrap();
                let json_string = suspect_overlay::Value::from_node(node.resolved()).to_json();
                let _value: serde_json::Value =
                    serde_json::from_str(&json_string).unwrap_or(serde_json::Value::Null);
            }
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            println!(
                "  {:<52} {:>9.2} ms  ({} schemas, overlay+string+serde)",
                "schema materialization (current impl)",
                ms,
                names.len()
            );

            // Direct NodeRef->Json conversion prototype timing.
            let t = Instant::now();
            for name in &names {
                let node = schemas.get(name).unwrap();
                let _value = direct_json(node.resolved());
            }
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            println!(
                "  {:<52} {:>9.2} ms  (direct NodeRef->Json)",
                "schema materialization (prototype)", ms
            );
        }

        // Full IrSpec::from_workspace timing.
        let mut ir_times = Vec::new();
        for _ in 0..5 {
            let t = Instant::now();
            let _ir =
                suspect_ir::IrSpec::from_workspace(&ws, &entry).map_err(anyhow::Error::msg)?;
            ir_times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        println!(
            "  {:<52} {:>9.2} ms (median of 5)",
            "IrSpec::from_workspace total",
            median(ir_times)
        );
    }

    // ---- lint/validate split ----
    println!("\n[lint vs validate]");
    {
        let handle = ws.get(&entry).unwrap();
        let low = handle.doc();
        let session = suspect_oas::Session::new(Arc::clone(&ws));

        let t = Instant::now();
        let linter = suspect_lint::Linter::spectral_default();
        let findings = linter.run(low);
        println!(
            "  {:<52} {:>9.2} ms  ({} findings) [single run]",
            "lint total",
            t.elapsed().as_secs_f64() * 1000.0,
            findings.len()
        );

        let t = Instant::now();
        let diags = suspect_validate::validate_entry(&session, entry.as_str())?;
        println!(
            "  {:<52} {:>9.2} ms  ({} diagnostics) [first run]",
            "validate total (cold schema compiles)",
            t.elapsed().as_secs_f64() * 1000.0,
            diags.len()
        );

        let t = Instant::now();
        let diags2 = suspect_validate::validate_entry(&session, entry.as_str())?;
        println!(
            "  {:<52} {:>9.2} ms  ({} diagnostics) [second run]",
            "validate total",
            t.elapsed().as_secs_f64() * 1000.0,
            diags2.len()
        );
    }

    Ok(())
}

fn count_nodes(root: NodeRef<'_>) -> usize {
    let mut n = 0usize;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        n += 1;
        stack.extend(node.entries().into_iter().filter_map(|e| e.value));
    }
    n
}

/// Direct NodeRef -> serde_json::Value converter (prototype of the fast
/// path; no overlay tree, no intermediate string).
fn direct_json(node: NodeRef<'_>) -> serde_json::Value {
    use suspect_low::ValueKind;
    let node = node.resolved();
    match node.kind() {
        ValueKind::Object => {
            let mut map = serde_json::Map::new();
            for e in node.entries() {
                if let Some(v) = e.value {
                    map.insert(e.key.to_owned(), direct_json(v));
                }
            }
            serde_json::Value::Object(map)
        }
        ValueKind::Array => {
            serde_json::Value::Array(node.items().iter().map(|n| direct_json(*n)).collect())
        }
        _ => scalar_json(node),
    }
}

fn scalar_json(node: NodeRef<'_>) -> serde_json::Value {
    if let Some(b) = node.as_bool() {
        return serde_json::Value::Bool(b);
    }
    if let Some(i) = node.as_i64() {
        return serde_json::Value::Number(i.into());
    }
    if let Some(f) = node.as_f64() {
        return serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null);
    }
    match node.as_str() {
        Some(s) => serde_json::Value::String(s.to_owned()),
        None => serde_json::Value::Null,
    }
}
