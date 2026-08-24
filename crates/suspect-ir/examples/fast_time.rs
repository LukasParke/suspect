//! Times suspect_ir::from_file on the stripe corpus.
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/stripe.yaml");
    let _ = suspect_ir::IrSpec::from_file(&path).map_err(anyhow::Error::msg)?;
    let mut times = Vec::new();
    for _ in 0..9 {
        let t = Instant::now();
        let bytes = std::fs::read(&path)?;
        let t_read = t.elapsed();
        let t1 = Instant::now();
        let root = suspect_syntax::try_parse_fast(&bytes);
        let t_parse = t1.elapsed();
        let root = root.ok_or_else(|| anyhow::anyhow!("fallback"))?;
        let t2 = Instant::now();
        let _spec = suspect_ir::fast::ir_from_fast(&root);
        let t_ir = t2.elapsed();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        eprintln!(
            "[split] total {:?} read {:?} parse {:?} ir {:?}",
            t.elapsed(),
            t_read,
            t_parse,
            t_ir
        );
    }
    times.sort_by(|a, b| a.total_cmp(b));
    eprintln!(
        "from_file median {:.2} ms (min {:.2}, max {:.2})",
        times[4], times[0], times[8]
    );
    Ok(())
}
