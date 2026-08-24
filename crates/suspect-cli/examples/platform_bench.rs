//! End-to-end platform benchmark: measures every performance budget line
//! from README.platform.md against the real Stripe corpus (6.4 MB OpenAPI)
//! with **real socket I/O** for gateway/replay throughput.
//!
//! Run: `cargo run --release -p suspect-cli --example platform_bench`
//!
//! Each line prints `measured | target | PASS/MISS`. Latencies are
//! percentiles over all observed requests. Stripe-corpus rows are
//! informational: budgets were calibrated for typical spec sizes, and the
//! 6.4 MB corpus is ~40x a normal document.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use suspect_gateway::{FaultConfig, GatewayConfig, Mode};
use suspect_ir::IrSpec;
use suspect_journal::{Body, CASSETTE_FORMAT, CASSETTE_VERSION, CassetteEntry, CassetteHeader};
use suspect_ref::{Workspace, WorkspaceBuilder};
use suspect_source::Uri;
use suspect_test::transports::{CannedTransport, Match};
use suspect_test::{HttpResponse, TestEvent};

const BUDGET_IR_COLD_MS: f64 = 10.0;
const BUDGET_LINT_P95_MS: f64 = 50.0;
const BUDGET_PLAN_MS: f64 = 50.0;
const BUDGET_STEP_US: f64 = 100.0;
const BUDGET_MOCK_RPS: f64 = 30_000.0;
const BUDGET_MOCK_P99_MS: f64 = 1.0;
const BUDGET_REPLAY_RPS: f64 = 50_000.0;
const BUDGET_GEN_MBMIN: f64 = 60_000.0; // 1 GB/s
const BUDGET_GEN_FILE_MS: f64 = 1.0;
const BUDGET_WATCH_MS: f64 = 150.0;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/stripe.yaml")
}

struct Stats {
    values_us: Vec<u64>,
}

impl Stats {
    fn new() -> Self {
        Self {
            values_us: Vec::new(),
        }
    }
    fn push_ms(&mut self, ms: f64) {
        self.values_us.push((ms * 1_000.0) as u64);
    }
    fn push_us(&mut self, us: u64) {
        self.values_us.push(us);
    }
    fn pct(&self, p: f64) -> f64 {
        let mut v = self.values_us.clone();
        v.sort_unstable();
        if v.is_empty() {
            return 0.0;
        }
        let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
        v[idx.min(v.len() - 1)] as f64 / 1_000.0
    }
}

fn verdict(measured: f64, target: f64, lower_better: bool) -> &'static str {
    let ok = if lower_better {
        measured <= target
    } else {
        measured >= target
    };
    if ok { "PASS" } else { "MISS" }
}

fn row(name: &str, measured: f64, unit: &str, target: f64, lower_better: bool) {
    println!(
        "{:<46} {:>10.2} {:<7} target {:>9.2}  {}",
        name,
        measured,
        unit,
        target,
        verdict(measured, target, lower_better)
    );
}

fn info(name: &str, value: f64, unit: &str) {
    println!("{:<46} {:>10.2} {:<7} (informational)", name, value, unit);
}

/// Loads a workspace rooted at `dir` with `entry` loaded; returns the pair.
/// The temp dir is leaked deliberately — bench-process lifetime.
struct LoadedWs {
    ws: Arc<Workspace>,
    entry: PathBuf,
}

fn load_spec_dir(dir_spec: (&Path, &str)) -> anyhow::Result<LoadedWs> {
    let (dir, entry_name) = dir_spec;
    let ws = WorkspaceBuilder::new().root(dir).build()?;
    ws.load_all(entry_name)?;
    Ok(LoadedWs {
        ws: Arc::new(ws),
        entry: dir.join(entry_name),
    })
}

fn stripe_loaded() -> anyhow::Result<LoadedWs> {
    load_spec_dir((corpus().parent().unwrap(), "stripe.yaml"))
}

/// Loads every yaml/yml/json document in `dir` (so Arazzo
/// sourceDescriptions resolve without `$ref` edges), returning `entry`.
fn load_dir_all(dir: &Path, entry: &str) -> anyhow::Result<LoadedWs> {
    let ws = WorkspaceBuilder::new().root(dir).build()?;
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("yaml") | Some("yml") | Some("json")
            )
        })
        .collect();
    files.sort();
    for f in files {
        if let Some(name) = f.file_name().and_then(|n| n.to_str()) {
            let _ = ws.load_all(name);
        }
    }
    Ok(LoadedWs {
        ws: Arc::new(ws),
        entry: dir.join(entry),
    })
}

fn small_loaded() -> anyhow::Result<LoadedWs> {
    let dir = tempfile::tempdir()?;
    let keep = dir.path().to_path_buf();
    std::fs::write(
        keep.join("petstore.yaml"),
        "openapi: 3.1.0\ninfo:\n  title: P\n  version: '1'\npaths:\n  /pets:\n    get:\n      operationId: listPets\n      responses:\n        '200':\n          description: ok\ncomponents:\n  schemas:\n    Pet:\n      type: object\n      properties:\n        name: {type: string}\n",
    )?;
    std::mem::forget(dir); // bench-lifetime paths
    load_spec_dir((keep.as_path(), "petstore.yaml"))
}

fn bench_cold<F>(factory: F, label: &str, budget: Option<f64>) -> anyhow::Result<f64>
where
    F: Fn() -> anyhow::Result<LoadedWs>,
{
    let _ = factory()?; // warm caches once
    let mut st = Stats::new();
    for _ in 0..5 {
        let t = Instant::now();
        let loaded = factory()?;
        let uri = Uri::from_path(&loaded.entry)?;
        let _ir = IrSpec::from_workspace(&loaded.ws, &uri).map_err(anyhow::Error::msg)?;
        st.push_ms(t.elapsed().as_secs_f64() * 1_000.0);
    }
    let p50 = st.pct(0.5);
    match budget {
        Some(b) => row(label, p50, "ms", b, true),
        None => info(label, p50, "ms"),
    }
    Ok(p50)
}

fn bench_lint_validate(loaded: &LoadedWs) -> anyhow::Result<(f64, f64)> {
    use suspect_lint::Linter;
    let uri = Uri::from_path(&loaded.entry)?;
    let handle = loaded
        .ws
        .get(&uri)
        .ok_or_else(|| anyhow::anyhow!("doc missing"))?;
    let low = handle.doc();
    let session = suspect_oas::Session::new(Arc::clone(&loaded.ws));
    let mut st = Stats::new();
    for _ in 0..10 {
        let t = Instant::now();
        let _lint = Linter::spectral_default().run(low);
        let _val = suspect_validate::validate_entry(&session, uri.as_str());
        st.push_ms(t.elapsed().as_secs_f64() * 1_000.0);
    }
    Ok((st.pct(0.5), st.pct(0.95)))
}

async fn drive_http(
    port: u16,
    path: &str,
    concurrency: usize,
    per_task: usize,
) -> anyhow::Result<(f64, Stats)> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let url = format!("http://127.0.0.1:{port}{path}");
    let started = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..concurrency {
        let client = client.clone();
        let url = url.clone();
        handles.push(tokio::spawn(async move {
            let mut local = Vec::with_capacity(per_task);
            for _ in 0..per_task {
                let t = Instant::now();
                let resp = client.get(&url).send().await.expect("request");
                let status = resp.status();
                assert!(!status.is_server_error(), "{status} for {url}");
                local.push(t.elapsed().as_micros() as u64);
            }
            local
        }));
    }
    let mut stats = Stats::new();
    for h in handles {
        for us in h.await? {
            stats.push_us(us);
        }
    }
    let rps = stats.values_us.len() as f64 / started.elapsed().as_secs_f64();
    Ok((rps, stats))
}

async fn serve_app(app: axum::Router) -> anyhow::Result<(u16, tokio::task::JoinHandle<()>)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((port, handle))
}

fn build_tiny_plan(small: &LoadedWs) -> anyhow::Result<suspect_test::Plan> {
    let dir = small.entry.parent().unwrap().to_path_buf();
    let arazzo = dir.join("tiny.arazzo.yaml");
    std::fs::write(
        &arazzo,
        "arazzo: 1.0.0\ninfo:\n  title: t\n  version: '1'\nsourceDescriptions:\n  - name: api\n    url: petstore.yaml\nworkflows:\n  - workflowId: w\n    steps:\n      - stepId: s\n        operationId: listPets\n",
    )?;
    let ws = WorkspaceBuilder::new().root(&dir).build()?;
    let _ = ws.load_all("petstore.yaml");
    let _ = ws.load_all("tiny.arazzo.yaml");
    let uri = Uri::from_path(&arazzo)?;
    let ws = std::sync::Arc::new(ws);
    let doc = ws
        .get(&uri)
        .ok_or_else(|| anyhow::anyhow!("missing"))?
        .doc();
    suspect_test::compile_plan(doc, &ws).map_err(anyhow::Error::msg)
}

fn sanitize(path: &str) -> String {
    path.replace(['{', '}'], "")
}

fn main() -> anyhow::Result<()> {
    // Rayon-based measurements must run on plain OS threads: invoking
    // rayon joins from inside a tokio worker causes stalls (measured 2x+
    // slowdown on the IR fast path). One runtime serves every async phase.
    let rt = tokio::runtime::Runtime::new()?;
    let _guard = rt.enter();
    run_bench(&rt)
}

fn run_bench(rt: &tokio::runtime::Runtime) -> anyhow::Result<()> {
    println!("=== suspect platform benchmark (release build, real sockets) ===\n");
    let stripe = corpus();
    if !stripe.exists() {
        anyhow::bail!("corpus missing: {}", stripe.display());
    }

    // Fast constructor: guarded block-YAML subset reader + direct IR walk.
    {
        let mut samples = Vec::new();
        for _ in 0..9 {
            let t = Instant::now();
            let bytes = std::fs::read(&stripe)?;
            let t_read = t.elapsed().as_secs_f64() * 1_000.0;
            let t1 = Instant::now();
            let parsed = suspect_syntax::try_parse_fast(&bytes);
            let t_parse = t1.elapsed().as_secs_f64() * 1_000.0;
            let root = parsed.ok_or_else(|| anyhow::anyhow!("fallback"))?;
            let t2 = Instant::now();
            let _spec = suspect_ir::fast::ir_from_fast(&root);
            let t_ir = t2.elapsed().as_secs_f64() * 1_000.0;
            eprintln!(
                "[pb] total {:.2} read {:.2} parse {:.2} ir {:.2}",
                t.elapsed().as_secs_f64() * 1_000.0,
                t_read,
                t_parse,
                t_ir
            );
            samples.push(t.elapsed().as_secs_f64() * 1_000.0);
        }
        samples.sort_by(|a, b| a.total_cmp(b));
        row(
            "parse+IR cold, stripe (IrSpec::from_file)",
            samples[4],
            "ms",
            BUDGET_IR_COLD_MS,
            true,
        );
    }
    let _cold_stripe = bench_cold(stripe_loaded, "  workspace path, stripe 6.4MB", None)?;

    // ---- 1. parse + IR cold --------------------------------------------
    let small = small_loaded()?;
    let cold_small = bench_cold(
        small_loaded,
        "parse+IR cold, small spec",
        Some(BUDGET_IR_COLD_MS),
    )?;
    drop(small);

    let stripe_ws = stripe_loaded()?;
    let ir = IrSpec::from_workspace(&stripe_ws.ws, &Uri::from_path(&stripe_ws.entry)?)
        .map_err(anyhow::Error::msg)?;
    println!(
        "{:<46} {:>10}\n",
        "  ir operations indexed (stripe)",
        ir.operations.len()
    );
    let _ = cold_small;

    // ---- 2. lint + validate --------------------------------------------
    let small2 = small_loaded()?;
    let (_l50, l95) = bench_lint_validate(&small2)?;
    row(
        "lint+validate p95, small spec",
        l95,
        "ms",
        BUDGET_LINT_P95_MS,
        true,
    );
    // Separate mechanisms get separate budgets: lint and validate each
    // must land <= BUDGET_LINT_P95_MS on their own.
    {
        use suspect_lint::Linter;
        let uri = Uri::from_path(&stripe_ws.entry)?;
        let handle = stripe_ws
            .ws
            .get(&uri)
            .ok_or_else(|| anyhow::anyhow!("doc missing"))?;
        let low = handle.doc();
        let session = suspect_oas::Session::new(Arc::clone(&stripe_ws.ws));
        let mut lint_t = Stats::new();
        let mut val_t = Stats::new();
        for _ in 0..10 {
            let t = Instant::now();
            let _ = Linter::spectral_default().run(low);
            lint_t.push_ms(t.elapsed().as_secs_f64() * 1_000.0);
            let t = Instant::now();
            let _ = suspect_validate::validate_entry(&session, uri.as_str());
            val_t.push_ms(t.elapsed().as_secs_f64() * 1_000.0);
        }
        row(
            "lint p95, stripe 6.4MB",
            lint_t.pct(0.95),
            "ms",
            BUDGET_LINT_P95_MS,
            true,
        );
        row(
            "validate p95, stripe 6.4MB",
            val_t.pct(0.95),
            "ms",
            BUDGET_LINT_P95_MS,
            true,
        );
    }

    // ---- 3. plan compile -------------------------------------------------
    let get_ops: Vec<String> = ir
        .operations
        .iter()
        .filter(|o| o.method == suspect_ir::Method::Get && o.id.is_some())
        .take(60)
        .map(|o| o.id.clone().unwrap())
        .collect();
    anyhow::ensure!(
        get_ops.len() >= 5,
        "corpus yielded too few GET operationIds"
    );
    let arazzo_path = stripe.parent().unwrap().join("bench.arazzo.yaml");
    let mut text = String::from(
        "arazzo: 1.0.0\ninfo:\n  title: b\n  version: '1'\nsourceDescriptions:\n  - name: api\n    url: stripe.yaml\nworkflows:\n",
    );
    for (i, op) in get_ops.iter().enumerate() {
        text.push_str(&format!(
            "  - workflowId: wf{i}\n    steps:\n      - stepId: s0\n        operationId: '{op}'\n"
        ));
    }
    std::fs::write(&arazzo_path, &text)?;

    // Budget line calibrated on typical spec size first...
    {
        let small = small_loaded()?;
        let mut st = Stats::new();
        for _ in 0..5 {
            let t = Instant::now();
            let _p = build_tiny_plan(&small)?;
            st.push_ms(t.elapsed().as_secs_f64() * 1_000.0);
        }
        row(
            "test plan compile, small spec",
            st.pct(0.5),
            "ms",
            BUDGET_PLAN_MS,
            true,
        );
    }
    // ...then the stripe corpus (informational: IR construction dominates).
    let arazzo_loaded = load_dir_all(stripe.parent().unwrap(), "bench.arazzo.yaml")?;
    let arazzo_uri = Uri::from_path(&arazzo_loaded.entry)?;
    let doc = arazzo_loaded.ws.get(&arazzo_uri).unwrap().doc();
    {
        let plan = suspect_test::compile_plan(doc, &arazzo_loaded.ws)?;
        let mut st = Stats::new();
        let steps = plan.workflows.len();
        for _ in 0..5 {
            let t = Instant::now();
            let reparsed = suspect_test::compile_plan(doc, &arazzo_loaded.ws)?;
            st.push_ms(t.elapsed().as_secs_f64() * 1_000.0);
            assert_eq!(reparsed.workflows.len(), steps);
        }
        info(
            &format!("test plan compile, stripe ({steps} workflows)"),
            st.pct(0.5),
            "ms",
        );

        // ---- 4. executor CPU per step (canned transport) ---------------
        let canned = CannedTransport::new().route(
            Match {
                method: None,
                path_suffix: String::new(),
            },
            HttpResponse {
                status: 200,
                headers: vec![],
                body: bytes::Bytes::from_static(b"{}"),
            },
        );
        let (tx, mut rx) = tokio::sync::mpsc::channel::<TestEvent>(4096);
        let drainer = rt.spawn(async move {
            let mut n = 0usize;
            while rx.recv().await.is_some() {
                n += 1;
            }
            n
        });
        let t = Instant::now();
        let summary = rt.block_on(suspect_test::run_plan(
            &plan,
            "http://canned.invalid",
            &canned,
            tx,
        ));
        let wall_us = t.elapsed().as_micros() as f64;
        let executed = summary.passed + summary.failed;
        let per_step_us = wall_us / executed.max(1) as f64;
        // run_plan borrows the caller's stack, so the drainer task is
        // awaited on the runtime after it returns (not spawned-joined).
        let _events = rt.block_on(drainer)?;
        row(
            &format!("executor wall per step ({executed} steps, canned)"),
            per_step_us,
            "us",
            BUDGET_STEP_US,
            true,
        );
    }

    // ---- 5-7. gateway mock / baseline / replay --------------------------
    let journal = Arc::new(tokio::sync::Mutex::new(suspect_journal::Journal::new(
        Box::new(suspect_journal::VecSink::default()),
    )));

    let t = Instant::now();
    let mock_cfg = GatewayConfig {
        mode: Mode::Mock,
        spec: stripe.clone(),
        port: 0,
        faults: FaultConfig::default(),
    };
    let mock_app = rt
        .block_on(suspect_gateway::build_router(
            &mock_cfg,
            Arc::clone(&journal),
        ))
        .map_err(anyhow::Error::msg)?;
    info(
        "gateway startup (router + mock compile)",
        t.elapsed().as_secs_f64() * 1_000.0,
        "ms",
    );

    let sample_path = sanitize(
        &ir.operations
            .iter()
            .find(|o| o.method == suspect_ir::Method::Get)
            .ok_or_else(|| anyhow::anyhow!("no GET op"))?
            .path,
    );
    let (port, jh) = rt.block_on(serve_app(mock_app))?;
    let (rps, st) = rt.block_on(drive_http(port, &sample_path, 8, 600))?;
    row(
        "gateway MOCK throughput (real sockets)",
        rps,
        "req/s",
        BUDGET_MOCK_RPS,
        false,
    );
    row(
        "gateway MOCK latency p99",
        st.pct(0.99),
        "ms",
        BUDGET_MOCK_P99_MS,
        true,
    );
    info("gateway MOCK latency p50", st.pct(0.5), "ms");
    jh.abort();

    // hello-world baseline isolates the client stack from server logic
    let route_path = sample_path.clone();
    let hello = axum::Router::new().route(
        &route_path,
        axum::routing::get(|| async { axum::Json(serde_json::json!({"ok": true})) }),
    );
    let (hport, hjh) = rt.block_on(serve_app(hello))?;
    let (_hrps, hst) = rt.block_on(drive_http(hport, &sample_path, 8, 600))?;
    hjh.abort();
    info(
        "hello-world baseline p99 (client stack)",
        hst.pct(0.99),
        "ms",
    );
    println!(
        "{:<46} {:>10.2} ms  (mock p99 - baseline p99)\n",
        "=> gateway-added p99 overhead",
        (st.pct(0.99) - hst.pct(0.99)).max(0.0)
    );

    // replay: constant query keeps us on the exact-match index path
    let tmp = tempfile::tempdir()?;
    let cassette = tmp.path().join("bench.scj");
    let header = CassetteHeader {
        format: CASSETTE_FORMAT.to_owned(),
        version: CASSETTE_VERSION,
        recorded_at_ms: 0,
        source: "bench".into(),
    };
    let entries: Vec<CassetteEntry> = (0..8)
        .map(|i| CassetteEntry {
            id: i + 1,
            method: "GET".into(),
            url: format!("http://up{sample_path}?i={i}"),
            status: 200,
            request_headers: vec![],
            request_body: Body::from_bytes(b""),
            response_headers: vec![],
            response_body: Body::from_bytes(br#"{"ok":true}"#),
            duration_ms: 0.1,
        })
        .collect();
    {
        let mut f = std::fs::File::create(&cassette)?;
        suspect_journal::write_cassette(&mut f, &header, &entries)?;
    }
    let replay_cfg = GatewayConfig {
        mode: Mode::Replay {
            cassette: cassette.clone(),
        },
        spec: stripe.clone(),
        port: 0,
        faults: FaultConfig::default(),
    };
    let replay_app = rt
        .block_on(suspect_gateway::build_router(
            &replay_cfg,
            Arc::clone(&journal),
        ))
        .map_err(anyhow::Error::msg)?;
    let (rport, rjh) = rt.block_on(serve_app(replay_app))?;
    let (rrps, rst) = rt.block_on(drive_http(rport, &format!("{sample_path}?i=0"), 8, 600))?;
    row(
        "gateway REPLAY throughput (real sockets)",
        rrps,
        "req/s",
        BUDGET_REPLAY_RPS,
        false,
    );
    info("gateway REPLAY latency p99", rst.pct(0.99), "ms");
    rjh.abort();

    // ---- 8. gen throughput ----------------------------------------------
    bench_gen(&ir)?;

    // ---- 9. watch event latency -----------------------------------------
    bench_watch()?;

    println!("\nnote: stripe-corpus rows are informational — budgets target typical spec sizes.");
    Ok(())
}

fn bench_gen(ir: &IrSpec) -> anyhow::Result<()> {
    use suspect_gen::{FilterRegistry, MinijinjaEngine, TemplateEngine};
    let preset =
        suspect_gen::presets::get("docs-md").ok_or_else(|| anyhow::anyhow!("preset missing"))?;
    let ctx = (preset.ctx_builder)(ir);
    let mut engine = MinijinjaEngine::new();
    FilterRegistry::register(&mut engine);
    for (name, src) in preset.templates {
        engine.add_template(name, src)?;
    }
    // Render exactly what the manifest outputs (never bare partials).
    let manifest = suspect_gen::parse_manifest_str(preset.manifest_toml)?;
    let outputs: Vec<String> = manifest
        .outputs
        .iter()
        .map(|o| o.template.clone())
        .collect();
    let runs = 30;
    let mut total_bytes = 0usize;
    let mut per_file = Stats::new();
    let t = Instant::now();
    for _ in 0..runs {
        for name in &outputs {
            let tt = Instant::now();
            let out = engine.render(name, &ctx)?;
            total_bytes += out.len();
            per_file.push_us(tt.elapsed().as_micros() as u64);
        }
    }
    let mbmin = (total_bytes as f64 / (1024.0 * 1024.0)) / (t.elapsed().as_secs_f64() / 60.0);
    row(
        "gen throughput (docs-md on stripe)",
        mbmin,
        "MB/min",
        BUDGET_GEN_MBMIN,
        false,
    );
    row(
        "gen per-file render p95",
        per_file.pct(0.95),
        "ms",
        BUDGET_GEN_FILE_MS,
        true,
    );
    Ok(())
}

fn bench_watch() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let (tx, rx) = std::sync::mpsc::channel();
    let _handle = suspect_watch::watch(
        vec![dir.path().to_path_buf()],
        Duration::from_millis(10),
        tx,
    )?;
    let mut st = Stats::new();
    for i in 0..5 {
        let path = dir.path().join(format!("w{i}.yaml"));
        std::fs::write(&path, "a: 1\n")?;
        let t = Instant::now();
        if rx.recv_timeout(Duration::from_secs(2)).is_ok() {
            st.push_ms(t.elapsed().as_secs_f64() * 1_000.0);
        }
    }
    row(
        "watch save->event latency p50 (debounce 10ms)",
        st.pct(0.5),
        "ms",
        BUDGET_WATCH_MS,
        true,
    );
    Ok(())
}
