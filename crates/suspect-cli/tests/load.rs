//! `suspect load` end to end: the bundled Petstore example against an
//! in-process mock gateway, clean and with fault injection.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use suspect_cli::ReportFormat;
use suspect_cli::commands::load::{JitterArg, LoadArgs, load};
use suspect_gateway::{FaultConfig, GatewayConfig, Mode};
use suspect_journal::{Journal, VecSink};

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/load")
        .join(name)
        .canonicalize()
        .expect("example exists")
}

/// Serves the Petstore mock on an ephemeral port; returns the port.
async fn spawn_gateway(faults: FaultConfig) -> u16 {
    let cfg = GatewayConfig {
        mode: Mode::Mock,
        spec: example("petstore.openapi.yaml"),
        port: 0,
        faults,
        redact_headers: Vec::new(),
        redact_json_keys: Vec::new(),
    };
    let journal = Arc::new(tokio::sync::Mutex::new(Journal::new(Box::new(
        VecSink::default(),
    ))));
    let app = suspect_gateway::build_router(&cfg, journal)
        .await
        .expect("router");
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    port
}

fn write_profile(dir: &Path, arazzo: &Path) -> PathBuf {
    let path = dir.join("smoke.load.yaml");
    std::fs::write(
        &path,
        format!(
            r#"suspectLoad: "1.0"
arazzo: {arazzo}
phases:
  - {{ name: smoke, duration: 2s, arrivalRate: 15 }}
scenarios:
  - name: browse
    workflow: browse
    weight: 2
  - name: writer
    workflow: create-and-cleanup
    weight: 1
http:
  timeout: 5s
ensure:
  maxErrorRate: 1
  thresholds:
    - http.response_time.p95: 250
  conditions:
    - expression: vusers.completed == vusers.created
report:
  interval: 1s
"#,
            arazzo = arazzo.display()
        ),
    )
    .unwrap();
    path
}

fn args(profile: PathBuf, port: u16, junit: PathBuf, json: PathBuf) -> LoadArgs {
    LoadArgs {
        input: profile,
        base_url: Some(format!("http://127.0.0.1:{port}")),
        env: None,
        workflow: Vec::new(),
        rate: None,
        ramp_to: None,
        vus: None,
        iterations: None,
        duration: None,
        max_vus: None,
        input_pairs: Vec::new(),
        inputs_file: None,
        credentials: None,
        validate_responses: false,
        seed: Some(3),
        jitter: JitterArg::None,
        report: ReportFormat::Ndjson,
        report_interval: None,
        grace: Some("10s".into()),
        timeout: None,
        http1: false,
        insecure: false,
        json: Some(json),
        junit: Some(junit),
        dry_run: false,
    }
}

/// `load` builds its own tokio runtime, so it runs on a plain thread while
/// the gateway lives on the test runtime.
fn run_cli(a: LoadArgs) -> i32 {
    std::thread::spawn(move || load(a).expect("load runs"))
        .join()
        .expect("thread")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn load_passes_against_clean_mock_and_fails_with_faults() {
    let dir = tempfile::tempdir().unwrap();
    let profile = write_profile(dir.path(), &example("flows.arazzo.yaml"));

    // Clean gateway: every check passes, exit 0, reports written.
    let port = spawn_gateway(FaultConfig::default()).await;
    let junit = dir.path().join("ok.xml");
    let json = dir.path().join("ok.json");
    let code = run_cli(args(profile.clone(), port, junit.clone(), json.clone()));
    assert_eq!(code, 0);
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    assert_eq!(report["version"], 1);
    assert_eq!(report["run"]["ensure_ok"], true);
    assert_eq!(report["aggregate"]["vusers"]["created"], 30);
    assert_eq!(report["aggregate"]["vusers"]["failed"], 0);
    assert_eq!(report["aggregate"]["contract"]["failures"], 0);
    assert!(report["aggregate"]["http"]["requests"].as_u64().unwrap() >= 60);
    // The dependsOn prerequisite ran once as a hook.
    assert_eq!(report["hooks"][0]["workflow"], "sign-in");
    assert_eq!(report["hooks"][0]["ok"], true);
    assert!(report["intervals"].as_array().unwrap().len() >= 2);
    let xml = std::fs::read_to_string(&junit).unwrap();
    assert!(xml.contains("failures=\"0\""), "{xml}");

    // Faulted gateway: 20% 503s + 300ms delays break the SLOs, exit 1,
    // and the JUnit names the breached checks.
    let port = spawn_gateway(FaultConfig {
        delay_ms: 300,
        delay_pct: 50,
        error_status: Some(503),
        error_pct: 20,
    })
    .await;
    let junit = dir.path().join("bad.xml");
    let json = dir.path().join("bad.json");
    let code = run_cli(args(profile, port, junit.clone(), json.clone()));
    assert_eq!(code, 1);
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    assert_eq!(report["run"]["ensure_ok"], false);
    assert!(
        report["aggregate"]["contract"]["failures"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(
        report["aggregate"]["http"]["codes"]["503"]
            .as_u64()
            .unwrap()
            > 0
    );
    let xml = std::fs::read_to_string(&junit).unwrap();
    assert!(xml.contains("<failure message=\"actual"), "{xml}");
    assert!(xml.contains("error rate &lt;= 1%"), "{xml}");
}
