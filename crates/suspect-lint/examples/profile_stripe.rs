//! Per-rule profiler for the stripe corpus.
//!
//! Run: `SUSPECT_PROFILE=1 cargo run --release -p suspect-lint --example profile_stripe`

use std::path::PathBuf;
use std::time::Instant;

use suspect_ref::WorkspaceBuilder;

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

    // Warm caches once.
    {
        let ws = WorkspaceBuilder::new().root(&dir).build()?;
        ws.load_all("stripe.yaml")?;
        let handle = ws
            .get(&suspect_source::Uri::from_path(&path)?)
            .ok_or("entry not found")?;
        let linter = suspect_lint::Linter::spectral_default();
        let n = linter.run(handle.doc()).len();
        eprintln!("warmup: {n} findings");
    }

    let ws = {
        let ws = WorkspaceBuilder::new().root(&dir).build()?;
        ws.load_all("stripe.yaml")?;
        ws
    };
    let handle = ws
        .get(&suspect_source::Uri::from_path(&path)?)
        .ok_or("entry not found")?;
    let low = handle.doc();

    let linter = suspect_lint::Linter::spectral_default();
    let mut times = Vec::new();
    for _ in 0..7 {
        let t = Instant::now();
        let findings = linter.run(low);
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        println!(
            "run: {:>8.2} ms ({} findings)",
            times.last().unwrap(),
            findings.len()
        );
        if std::env::var_os("SUSPECT_PROFILE").is_some() {
            let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
            for f in &findings {
                *counts.entry(&f.code).or_default() += 1;
            }
            eprintln!("{counts:?}");
        }
    }
    println!(
        "lint median {:.2} ms, p95 {:.2} ms",
        pct(times.clone(), 0.5),
        pct(times, 0.95)
    );
    Ok(())
}
