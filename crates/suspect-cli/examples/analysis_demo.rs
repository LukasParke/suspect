//! Integration exercise: quality scoring (R10) on the real Plex spec and
//! reverse engineering (R8) of the suspect gateway's own routes.

/// Minimal HTTP/1.1 GET over TcpStream (no TLS needed for localhost Plex).
fn raw_get(host: &str, path: &str) -> Result<(u16, Option<serde_json::Value>), ()> {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(host).map_err(|_| ())?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nAccept: application/json\r\n\r\n"
    )
    .map_err(|_| ())?;
    let mut buf = String::new();
    stream.read_to_string(&mut buf).map_err(|_| ())?;
    let status: u16 = buf
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or(())?;
    let body = buf
        .split("\r\n\r\n")
        .nth(1)
        .and_then(|b| serde_json::from_str(b).ok());
    Ok((status, body))
}

fn main() -> Result<(), String> {
    // --- R10: quality score on the real Plex spec ---
    let ws = suspect_ref::WorkspaceBuilder::new()
        .root(std::path::Path::new("/tmp/plex-api-spec"))
        .build()
        .map_err(|e| e.to_string())?;
    ws.load_all("plex-api-spec.yaml")
        .map_err(|e| e.to_string())?;
    let ws = std::sync::Arc::new(ws);
    let uri: suspect_source::Uri = ws
        .uris()
        .into_iter()
        .find(|u| u.as_str().ends_with("plex-api-spec.yaml"))
        .ok_or("entry not found")?;
    let ir = suspect_ir::IrSpec::from_workspace(&ws, &uri).map_err(|e| e.to_string())?;
    println!(
        "plex spec: {} ops, {} schemas",
        ir.operations.len(),
        ir.schemas.len()
    );

    let report = suspect_lint::quality::score_spec(&ir);
    println!(
        "quality overall: {}/100 ({} actions)",
        report.overall, report.total_actions
    );
    for d in &report.dimensions {
        println!(
            "  {:<28} {:>3}/100  ({} actions)",
            d.label,
            d.score,
            d.actions.len()
        );
    }

    // --- R1: evolution — inject a synthetic undocumented observation ---
    let mut traffic = std::collections::HashMap::new();
    if let Some(op) = ir.operations.first() {
        let key = op
            .id
            .clone()
            .unwrap_or_else(|| format!("{} {}", op.method.as_str().to_uppercase(), op.path));
        traffic.insert(key, vec![serde_json::json!({"ghostField": 1})]);
    }
    let evo = suspect_ir::evolution::analyze_traffic(&ir, &traffic);
    println!(
        "evolution: {} clean ops, {} proposals",
        evo.clean_operations,
        evo.proposals.len()
    );

    // --- R8: reverse engineering on our own gateway source ---
    let routes = suspect_reverse::extract_from_tree(std::path::Path::new(
        "/home/luke/github/suspect/crates/suspect-gateway/src",
    ));
    println!(
        "reverse: {} routes extracted from gateway source",
        routes.len()
    );
    for r in routes.iter().take(3) {
        println!("  {} {} ({}:{})", r.method, r.path, r.framework, r.line);
    }
    let xref = suspect_reverse::cross_reference(&ir, &routes);
    println!(
        "xref: {} undocumented, {} spec-only, {} method mismatches",
        xref.undocumented.len(),
        xref.spec_only.len(),
        xref.method_mismatches.len()
    );

    // --- R5: semantic diff — plex spec against itself is clean ---
    let diff = suspect_codegen::diff::diff_specs(&ir, &ir);
    println!(
        "self-diff: {} deltas, semver {}",
        diff.deltas.len(),
        diff.semver
    );

    // --- R7: consumer impact — no-op version bump is safe ---
    let impact = suspect_codegen::consumer_impact::analyze_impact(&ir, &ir, &[]);
    println!("impact: {}", impact.summary);

    // --- R4: stateful dependency graph on the real spec ---
    let graph = suspect_test::stateful::build_graph(&ir);
    println!(
        "stateful graph: {} resources, {} dependency edges",
        graph.nodes.len(),
        graph.edges.len()
    );
    let seqs = suspect_test::stateful::generate_sequences(&ir);
    let with_setup = seqs.iter().filter(|s| s.steps.len() > 1).count();
    println!(
        "stateful: {} sequences ({} need setup steps)",
        seqs.len(),
        with_setup
    );

    // --- R3: evolved fuzz campaign against the live Plex server ---
    let schema = serde_json::json!({
        "type": "object",
        "properties": {"X-Plex-Token": {"type": "string"}}
    });
    let fields = suspect_test::fuzz::scalar_fields(&schema);
    let mut seq = 0u32;
    let stats = suspect_test::evolved_fuzz::run_campaign(fields, 2, 5, &mut |_payload| {
        seq += 1;
        match raw_get("localhost:32400", "/identity") {
            Ok((status, body)) => suspect_test::evolved_fuzz::RequestOutcome {
                status,
                body,
                crash: status >= 500,
            },
            Err(()) => suspect_test::evolved_fuzz::RequestOutcome {
                status: 0,
                body: None,
                crash: true,
            },
        }
    });
    println!(
        "evolved fuzz: {} requests, {} shapes, {} crashes, corpus {}",
        stats.requests, stats.shapes, stats.crashes, stats.corpus_size
    );

    Ok(())
}
