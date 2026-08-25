//! Feature benchmark: exercises all ten platform features against the real
//! corpus (stripe-sdk 10.6 MB, api.github.com 9.4 MB) and the live Plex
//! server, timing each hot path.
//!
//! Run: cargo run --release -p suspect-cli --example feature_bench

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use suspect_ir::{IrSchema, IrSpec};

fn load_ir(root: &str, entry: &str) -> Result<(Arc<suspect_ref::Workspace>, IrSpec), String> {
    let ws = suspect_ref::WorkspaceBuilder::new()
        .root(std::path::Path::new(root))
        .build()
        .map_err(|e| e.to_string())?;
    ws.load_all(entry).map_err(|e| e.to_string())?;
    let ws = Arc::new(ws);
    let uri: suspect_source::Uri = ws
        .uris()
        .into_iter()
        .find(|u| u.as_str().ends_with(entry))
        .ok_or("entry not found")?;
    let ir = IrSpec::from_workspace(&ws, &uri)?;
    Ok((ws, ir))
}

fn mean(v: &[Duration]) -> Duration {
    if v.is_empty() {
        Duration::ZERO
    } else {
        v.iter().sum::<Duration>() / v.len() as u32
    }
}

fn fmt_ms(d: Duration) -> String {
    let us = d.as_micros();
    if us < 1_000 {
        format!("{us}µs")
    } else if us < 1_000_000 {
        format!("{:.2}ms", us as f64 / 1_000.0)
    } else {
        format!("{:.2}s", us as f64 / 1_000_000.0)
    }
}

fn raw_get(host: &str, path: &str) -> Result<(u16, Option<serde_json::Value>), ()> {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(host).map_err(|_| ())?;
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nAccept: application/json\r\n\r\n")
                .as_bytes(),
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

/// Builds a mutated copy of the IR simulating a minor version bump:
/// adds a schema, adds an optional field to every schema, deprecates 5% of
/// operations.
fn mutated_version(ir: &IrSpec) -> IrSpec {
    let mut new = ir.clone();
    for (i, schema) in &mut new.schemas.iter_mut().enumerate() {
        if let Some(props) = schema
            .json
            .as_object_mut()
            .and_then(|obj| obj.get_mut("properties"))
            .and_then(|p| p.as_object_mut())
        {
            props.insert(
                format!("newField{i}"),
                serde_json::json!({"type": "string"}),
            );
        }
    }
    new.schemas.push(IrSchema {
        name: "BrandNewSchema".to_owned(),
        json: serde_json::json!({"type": "object", "properties": {}}),
    });
    let total = new.operations.len();
    for (i, op) in new.operations.iter_mut().enumerate() {
        if i % 20 == 0 && total > 0 {
            op.deprecated = true;
        }
    }
    new
}

/// Builds a mutated copy simulating a major (breaking) bump.
fn breaking_version(ir: &IrSpec) -> IrSpec {
    let mut new = mutated_version(ir);
    // Remove the last property from the first 10 schemas
    for schema in new.schemas.iter_mut().take(10) {
        if let Some(obj) = schema.json.as_object_mut()
            && let Some(props) = obj.get_mut("properties").and_then(|p| p.as_object_mut())
            && let Some(key) = props.keys().next().cloned()
        {
            props.remove(&key);
        }
    }
    // Drop the last operation
    new.operations.pop();
    new
}

fn bench_r5_diff(name: &str, ir: &IrSpec, iters: usize) {
    let new_ir = mutated_version(ir);
    let breaking = breaking_version(ir);

    let mut t_minor = Vec::new();
    for _ in 0..iters {
        let s = Instant::now();
        let d = suspect_codegen::diff::diff_specs(ir, &new_ir);
        t_minor.push(s.elapsed());
        assert!(d.additive > 0);
    }
    let mut t_major = Vec::new();
    let mut last_breaking = 0usize;
    for _ in 0..iters {
        let s = Instant::now();
        let d = suspect_codegen::diff::diff_specs(ir, &breaking);
        t_major.push(s.elapsed());
        last_breaking = d.breaking;
    }
    println!("  R5  diff {name:<16} {iters} iters \u{b7} breaking deltas: {last_breaking}",);
    println!(
        "      └─ minor bump: {}, major bump: {}",
        fmt_ms(mean(&t_minor)),
        fmt_ms(mean(&t_major))
    );
}

fn bench_r10_quality(name: &str, ir: &IrSpec, iters: usize) {
    let mut times = Vec::new();
    let mut actions = 0usize;
    for _ in 0..iters {
        let s = Instant::now();
        let r = suspect_lint::quality::score_spec(ir);
        times.push(s.elapsed());
        actions = r.total_actions;
    }
    println!(
        "  R10 quality {name:<14} {} ({} actions across 5 dimensions)",
        fmt_ms(mean(&times)),
        actions
    );
}

fn bench_r1_evolution(name: &str, ir: &IrSpec, bodies_per_op: usize) {
    // Synthetic traffic: N bodies per operation, each with 2 undocumented fields
    let mut traffic: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    for op in &ir.operations {
        let key = op
            .id
            .clone()
            .unwrap_or_else(|| format!("{} {}", op.method.as_str(), op.path));
        let bodies: Vec<serde_json::Value> = (0..bodies_per_op)
            .map(|i| serde_json::json!({"ghostA": i, "ghostB": true}))
            .collect();
        traffic.insert(key, bodies);
    }
    let total_bodies: usize = traffic.values().map(|v| v.len()).sum();

    let s = Instant::now();
    let report = suspect_ir::evolution::analyze_traffic(ir, &traffic);
    let elapsed = s.elapsed();

    let per_sec = if elapsed.as_secs_f64() > 0.0 {
        total_bodies as f64 / elapsed.as_secs_f64()
    } else {
        0.0
    };
    println!(
        "  R1  evolution {name:<13} {} bodies in {} ({:.0} bodies/s), {} proposals",
        total_bodies,
        fmt_ms(elapsed),
        per_sec,
        report.proposals.len()
    );
}

fn bench_r7_impact(name: &str, ir: &IrSpec, traffic_size: usize) {
    let new_ir = breaking_version(ir);
    // Synthetic traffic: consumers hitting the first N operations
    let mut traffic = Vec::new();
    for i in 0..traffic_size {
        let op = &ir.operations[i % ir.operations.len()];
        traffic.push(suspect_codegen::consumer_impact::RecordedExchange {
            method: op.method.as_str().to_owned(),
            path: op.path.clone(),
            body: Some(serde_json::json!({"x": i})),
            consumer: format!("consumer-{}", i % 12),
            source_ip: Some(format!("10.0.{}.{}", i / 255, i % 255)),
        });
    }
    let s = Instant::now();
    let report = suspect_codegen::consumer_impact::analyze_impact(ir, &new_ir, &traffic);
    let elapsed = s.elapsed();
    println!(
        "  R7  impact {name:<15} {} exchanges vs {} consumers in {} → {} affected",
        traffic.len(),
        12,
        fmt_ms(elapsed),
        report.affected.len()
    );
}

fn bench_r4_stateful(name: &str, ir: &IrSpec) {
    let s = Instant::now();
    let graph = suspect_test::stateful::build_graph(ir);
    let graph_t = s.elapsed();

    let s = Instant::now();
    let seqs = suspect_test::stateful::generate_sequences(ir);
    let seq_t = s.elapsed();

    let with_setup = seqs.iter().filter(|s| s.steps.len() > 1).count();
    println!(
        "  R4  stateful {name:<14} graph {} ({} resources, {} edges), sequences {} ({} need setup, {} steps total)",
        fmt_ms(graph_t),
        graph.nodes.len(),
        graph.edges.len(),
        fmt_ms(seq_t),
        with_setup,
        seqs.iter().map(|s| s.steps.len()).sum::<usize>()
    );
}

fn bench_r8_reverse(name: &str, ir: &IrSpec) {
    // Extract routes from our own workspace source tree
    let s = Instant::now();
    let routes = suspect_reverse::extract_from_tree(std::path::Path::new(
        "/home/luke/github/suspect/crates",
    ));
    let extract_t = s.elapsed();

    let s = Instant::now();
    let xref = suspect_reverse::cross_reference(ir, &routes);
    let xref_t = s.elapsed();

    let files: usize = routes
        .iter()
        .map(|r| r.file.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    println!(
        "  R8  reverse {name:<14} {} routes from {files} files in {} ({} µs/file), xref {} in {}",
        routes.len(),
        fmt_ms(extract_t),
        if files > 0 {
            extract_t.as_micros() / files as u128
        } else {
            0
        },
        fmt_ms(xref_t),
        fmt_ms(xref_t)
    );
    let _ = (&xref.undocumented, &xref.spec_only);
}

fn bench_r2_causal(_name: &str, _ir: &IrSpec, iters: usize) {
    let spec_path = std::path::PathBuf::from("/tmp/plex-api-spec/plex-api-spec.yaml")
        .canonicalize()
        .unwrap_or_else(|_| {
            std::path::PathBuf::from("/home/luke/github/suspect/corpus/stripe-sdk.yaml")
        });
    let mut times = Vec::new();
    for i in 0..iters {
        let offset = (i * 997) % 100_000;
        let s = Instant::now();
        let trace = suspect_journal::causal::trace_failure(
            "bench.constraint",
            &spec_path,
            Some(offset),
            None,
        );
        times.push(s.elapsed());
        assert!(!trace.steps.is_empty());
    }
    // Breakdown: pure in-process offset→line/col (no subprocess)
    let mut lookup_only = Vec::new();
    for i in 0..iters {
        let offset = (i * 997) % 100_000;
        let s = Instant::now();
        let _ = suspect_journal::causal::offset_to_line_col(&spec_path, offset);
        lookup_only.push(s.elapsed());
    }
    println!(
        "  R2  causal          {} per full trace, {} pure offset lookup ({} traces; diff = git subprocess)",
        fmt_ms(mean(&times)),
        fmt_ms(mean(&lookup_only)),
        iters
    );
}

fn bench_r6_bridge(spec_path: &std::path::Path, ir: &IrSpec, ticks: usize) {
    let mut bridge = suspect_gateway::bridge::ContractBridge::new(spec_path);
    // Baseline tick
    let _ = bridge.tick(|_| Some(ir.clone()));

    // Observation-heavy ticks
    for i in 0..50 {
        bridge.observe("bench-op", serde_json::json!({"ghost": i}));
    }
    let mut times = Vec::new();
    for _ in 0..ticks {
        let s = Instant::now();
        let r = bridge.tick(|_| Some(ir.clone()));
        times.push(s.elapsed());
        let _ = r.new_conflicts;
    }
    println!(
        "  R6  bridge          {} per tick ({} ticks, 50 observations each, IR clone included)",
        fmt_ms(mean(&times)),
        ticks
    );
}

fn bench_r9_playground(spec: &IrSpec, name: &str, iters: usize) {
    let mut times = Vec::new();
    let mut size = 0usize;
    for _ in 0..iters {
        let s = Instant::now();
        let html = suspect_gateway::playground::playground_html(spec);
        times.push(s.elapsed());
        size = html.len();
    }
    println!(
        "  R9  playground {name:<13} {} to render {} KB UI ({} ops embedded)",
        fmt_ms(mean(&times)),
        size / 1024,
        spec.operations.len()
    );
}

fn bench_r3_fuzz(requests: u32) {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "X-Plex-Token": {"type": "string"},
            "count": {"type": "integer"}
        }
    });
    let fields = suspect_test::fuzz::scalar_fields(&schema);
    let mut seq = 0u32;
    let s = Instant::now();
    let stats = suspect_test::evolved_fuzz::run_campaign(fields, requests / 5, 5, &mut |_p| {
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
    let elapsed = s.elapsed();
    let rps = stats.requests as f64 / elapsed.as_secs_f64().max(0.001);
    println!(
        "  R3  evolved fuzz    {} requests in {} ({:.0} req/s live HTTP), {} shapes, {} crashes, corpus {}",
        stats.requests,
        fmt_ms(elapsed),
        rps,
        stats.shapes,
        stats.crashes,
        stats.corpus_size
    );
}

fn main() -> Result<(), String> {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║        TEN REVOLUTIONARY FEATURES — BENCHMARK SUITE          ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // --- Load corpus ---
    let (_, stripe) = load_ir("/home/luke/github/suspect/corpus", "stripe-sdk.yaml")?;
    let (_, github) = load_ir("/home/luke/github/suspect/corpus", "api.github.com.yaml")?;
    let (plex_ws, plex) = load_ir("/tmp/plex-api-spec", "plex-api-spec.yaml")?;
    println!(
        "corpus: stripe-sdk {} ops/{} schemas, github {} ops/{} schemas, plex {} ops/{} schemas",
        stripe.operations.len(),
        stripe.schemas.len(),
        github.operations.len(),
        github.schemas.len(),
        plex.operations.len(),
        plex.schemas.len()
    );
    println!();

    // --- R5 semantic diff ---
    println!("── R5: Wire-format semantic diff ──");
    bench_r5_diff("stripe-sdk", &stripe, 20);
    bench_r5_diff("api.github.com", &github, 20);
    bench_r5_diff("plex", &plex, 20);
    println!();

    // --- R10 quality ---
    println!("── R10: Quality scoring ──");
    bench_r10_quality("stripe-sdk", &stripe, 20);
    bench_r10_quality("api.github.com", &github, 20);
    bench_r10_quality("plex", &plex, 20);
    println!();

    // --- R1 evolution ---
    println!("── R1: Traffic-informed evolution ──");
    bench_r1_evolution("stripe-sdk", &stripe, 10);
    bench_r1_evolution("api.github.com", &github, 10);
    bench_r1_evolution("plex", &plex, 10);
    println!();

    // --- R7 consumer impact ---
    println!("── R7: Consumer impact analysis ──");
    bench_r7_impact("stripe-sdk", &stripe, 5_000);
    bench_r7_impact("api.github.com", &github, 5_000);
    bench_r7_impact("plex", &plex, 5_000);
    println!();

    // --- R4 stateful ---
    println!("── R4: Stateful dependency graph ──");
    bench_r4_stateful("stripe-sdk", &stripe);
    bench_r4_stateful("api.github.com", &github);
    bench_r4_stateful("plex", &plex);
    println!();

    // --- R8 reverse ---
    println!("── R8: Handler reverse engineering ──");
    bench_r8_reverse("vs stripe-sdk", &stripe);
    println!();

    // --- R2 causal ---
    println!("── R2: Causal debugger ──");
    bench_r2_causal("plex", &plex, 100);
    println!();

    // --- R6 bridge ---
    println!("── R6: Contract bridge ──");
    bench_r6_bridge(
        std::path::Path::new("/tmp/plex-api-spec/plex-api-spec.yaml"),
        &plex,
        20,
    );
    println!();

    // --- R9 playground ---
    println!("── R9: Contract playground ──");
    let _ = &plex_ws;
    bench_r9_playground(&plex, "plex", 10);
    println!();

    // --- R3 fuzz (live) ---
    println!("── R3: Grammar-evolved fuzzing (live Plex) ──");
    bench_r3_fuzz(50);
    println!();

    println!("════════ all ten features benchmarked ════════");
    Ok(())
}
