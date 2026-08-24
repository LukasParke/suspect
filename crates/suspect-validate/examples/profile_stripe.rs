//! Per-phase profiler for the stripe corpus validation.
//!
//! Run: `SUSPECT_PROFILE=1 cargo run --release -p suspect-validate --example profile_stripe`

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use suspect_oas::Session;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/stripe.yaml")
}

fn pct(mut v: Vec<f64>, p: f64) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    let idx = (((v.len() as f64 - 1.0) * p).round()) as usize;
    v[idx]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = corpus();
    let dir = path.parent().unwrap().to_path_buf();
    let entry = Uri::from_path(&path)?;

    // Warm caches once.
    {
        let ws = WorkspaceBuilder::new().root(&dir).build()?;
        ws.load_all("stripe.yaml")?;
        let session = Session::new(Arc::new(ws));
        let n = suspect_validate::validate_entry(&session, entry.as_str())?.len();
        eprintln!("warmup: {n} diagnostics");
    }

    let ws = {
        let ws = WorkspaceBuilder::new().root(&dir).build()?;
        ws.load_all("stripe.yaml")?;
        Arc::new(ws)
    };
    let session = Session::new(ws);

    // Split: model load vs checks.
    for _ in 0..3 {
        let t = Instant::now();
        let api = session.load(entry.as_str())?;
        println!(
            "session.load: {:>8.2} ms",
            t.elapsed().as_secs_f64() * 1000.0
        );
        let t = Instant::now();
        let diags = suspect_validate::validate_openapi(&api);
        println!(
            "validate_openapi: {:>8.2} ms ({} diags)",
            t.elapsed().as_secs_f64() * 1000.0,
            diags.len()
        );
    }

    let mut times = Vec::new();
    for _ in 0..7 {
        let t = Instant::now();
        let diags = suspect_validate::validate_entry(&session, entry.as_str())?;
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        println!(
            "run: {:>8.2} ms ({} diagnostics)",
            times.last().unwrap(),
            diags.len()
        );
    }
    println!(
        "validate median {:.2} ms, p95 {:.2} ms",
        pct(times.clone(), 0.5),
        pct(times, 0.95)
    );
    Ok(())
}
