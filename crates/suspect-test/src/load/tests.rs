//! End-to-end load runs on tokio's paused clock: schedules, caps, closed
//! loops, weights, hooks, data feeding, cancellation and `ensure`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use suspect_low::LowDoc;
use suspect_source::Source;
use tokio::sync::watch;

use super::profile::{
    Condition, DataOrder, DataSource, Ensure, InputSource, LoadProfile, Phase, PhaseKind, Scenario,
};
use super::{IntervalReport, Jitter, LoadConfig, LoadSink, LoadSummary, run_load};
use crate::exec::{HttpClient, HttpRequest, HttpResponse, TransportError};
use crate::plan::{Plan, compile_plan};
use crate::tests::{DEPENDS_ARAZZO, canned_fixture_http, compile_fixture, workspace};
use crate::transports::{CannedTransport, DelayTransport, Match};

fn phase(kind: PhaseKind, secs: f64) -> Phase {
    Phase {
        name: None,
        kind,
        duration: Duration::from_secs_f64(secs),
        max_vusers: None,
        range: 0..0,
    }
}

fn scenario(workflow: &str, weight: f64) -> Scenario {
    Scenario {
        name: workflow.to_owned(),
        workflow: workflow.to_owned(),
        weight,
        think: None,
        inputs: Default::default(),
        data: None,
        iterations: 1,
        continue_on_failure: false,
        range: 0..0,
    }
}

fn profile(phases: Vec<Phase>, scenarios: Vec<Scenario>) -> LoadProfile {
    LoadProfile {
        phases,
        scenarios,
        grace: Duration::from_secs(30),
        ..LoadProfile::default()
    }
}

fn config(http: Arc<dyn HttpClient>) -> LoadConfig {
    LoadConfig {
        base_url: "http://api.test".to_owned(),
        http,
        http_factory: None,
        report_interval: Duration::from_secs(1),
        grace: Duration::from_secs(30),
        seed: 42,
        env: "test".to_owned(),
        jitter: Jitter::None,
        cancel: None,
        auth: Arc::default(),
        validate_responses: false,
    }
}

#[derive(Default)]
struct Collect {
    intervals: Mutex<Vec<IntervalReport>>,
    notes: Mutex<Vec<String>>,
}

impl LoadSink for Collect {
    fn interval(&self, report: &IntervalReport) {
        self.intervals.lock().unwrap().push(report.clone());
    }
    fn note(&self, message: &str) {
        self.notes.lock().unwrap().push(message.to_owned());
    }
}

/// Fixture plan: `create-and-fetch` (2 steps) and `cleanup-user` (1 step).
fn fixture() -> Arc<Plan> {
    Arc::new(compile_fixture())
}

async fn run(
    plan: Arc<Plan>,
    profile: &LoadProfile,
    cfg: LoadConfig,
) -> (LoadSummary, Arc<Collect>) {
    let sink = Arc::new(Collect::default());
    let summary = run_load(plan, profile, cfg, sink.clone())
        .await
        .expect("run completes");
    (summary, sink)
}

#[tokio::test(start_paused = true)]
async fn constant_rate_creates_users_on_schedule_and_reports_intervals() {
    let http: Arc<dyn HttpClient> = Arc::new(DelayTransport::new(
        canned_fixture_http(),
        Duration::from_millis(100),
    ));
    let p = profile(
        vec![
            phase(
                PhaseKind::ArrivalRate {
                    rate: 10.0,
                    ramp_to: None,
                },
                5.0,
            ),
            phase(PhaseKind::Pause, 1.0),
        ],
        vec![scenario("create-and-fetch", 1.0)],
    );
    let (summary, sink) = run(fixture(), &p, config(http)).await;

    let m = &summary.aggregate;
    assert_eq!(m.vusers.created, 50, "{m:#?}");
    assert_eq!(m.vusers.completed, 50);
    assert_eq!(m.vusers.failed, 0);
    assert_eq!(m.http.requests, 100, "two steps per user");
    assert_eq!(m.http.codes[&201], 50);
    assert_eq!(m.http.codes[&200], 50);
    assert_eq!(m.contract.failures, 0);
    // DelayTransport sleeps 100ms of virtual time per request.
    assert!(
        (m.http.response_time.median - 100.0).abs() < 1.0,
        "{:?}",
        m.http.response_time
    );
    assert!(m.vusers.session_length.median >= 200.0);
    assert_eq!(m.steps.len(), 2);
    assert_eq!(m.steps[0].step, "create-pet");
    assert_eq!(m.steps[0].operation, "POST /pets");
    assert_eq!(summary.phases.len(), 2);
    assert_eq!(summary.phases[0].expected_arrivals, 50);
    assert!(!summary.interrupted);
    assert!(summary.ensure.ok(), "no checks configured");

    // ~6s of run at 1s intervals.
    let intervals = sink.intervals.lock().unwrap();
    assert!((5..=7).contains(&intervals.len()), "{}", intervals.len());
    assert_eq!(intervals[0].interval.vusers.created, 10);
    assert_eq!(intervals.last().unwrap().cumulative.vusers.created, 50);
}

#[tokio::test(start_paused = true)]
async fn max_vusers_skips_arrivals_instead_of_queueing() {
    // 100/s arrivals, each user busy for 2 × 500ms, cap 10 concurrent.
    let http: Arc<dyn HttpClient> = Arc::new(DelayTransport::new(
        canned_fixture_http(),
        Duration::from_millis(500),
    ));
    let mut ph = phase(
        PhaseKind::ArrivalRate {
            rate: 100.0,
            ramp_to: None,
        },
        1.0,
    );
    ph.max_vusers = Some(10);
    let p = profile(vec![ph], vec![scenario("create-and-fetch", 1.0)]);
    let (summary, _) = run(fixture(), &p, config(http)).await;
    let m = &summary.aggregate;
    assert_eq!(m.vusers.created + m.vusers.skipped, 100);
    assert_eq!(m.vusers.created, 10, "cap holds for the whole second");
    assert_eq!(m.vusers.skipped, 90);
    assert_eq!(m.vusers.completed, 10, "drain lets them finish");
}

#[tokio::test(start_paused = true)]
async fn closed_loop_vus_loop_until_the_phase_ends() {
    let http: Arc<dyn HttpClient> = Arc::new(DelayTransport::new(
        canned_fixture_http(),
        Duration::from_millis(100),
    ));
    // 4 users × (2 steps × 100ms = 200ms per iteration) over 2s ≈ 10 each.
    let p = profile(
        vec![phase(
            PhaseKind::ConstantVus {
                vus: 4,
                iterations: None,
            },
            2.0,
        )],
        vec![scenario("create-and-fetch", 1.0)],
    );
    let (summary, _) = run(fixture(), &p, config(http)).await;
    let m = &summary.aggregate;
    assert!(
        (36..=44).contains(&m.vusers.created),
        "{}",
        m.vusers.created
    );
    assert_eq!(m.vusers.created, m.vusers.completed);
    assert_eq!(m.http.requests, m.vusers.created * 2);
    assert_eq!(summary.phases[0].kind, "vus 4");

    // Iteration budget caps the total regardless of duration.
    let p = profile(
        vec![phase(
            PhaseKind::ConstantVus {
                vus: 4,
                iterations: Some(6),
            },
            60.0,
        )],
        vec![scenario("create-and-fetch", 1.0)],
    );
    let http: Arc<dyn HttpClient> = Arc::new(DelayTransport::new(
        canned_fixture_http(),
        Duration::from_millis(100),
    ));
    let (summary, _) = run(fixture(), &p, config(http)).await;
    assert_eq!(summary.aggregate.vusers.created, 6);
    // The phase ends when the budget is spent, not after its 60s clock.
    assert!(summary.duration_s < 5.0, "{}", summary.duration_s);
    assert_eq!(summary.phases[0].kind, "vus 4 × 6 iterations");
}

#[tokio::test(start_paused = true)]
async fn weights_distribute_users_and_runs_are_seeded() {
    let http: Arc<dyn HttpClient> = Arc::new(canned_fixture_http());
    let mut browse = scenario("create-and-fetch", 6.0);
    browse.name = "browse".into();
    let mut cleanup = scenario("cleanup-user", 3.0);
    cleanup.name = "cleanup".into();
    let mut rare = scenario("cleanup-user", 1.0);
    rare.name = "rare".into();
    let p = profile(
        vec![phase(PhaseKind::ArrivalCount { count: 1000 }, 1.0)],
        vec![browse, cleanup, rare],
    );
    let (a, _) = run(fixture(), &p, config(http.clone())).await;
    let by = &a.aggregate.vusers.by_scenario;
    let share = |n: &str| by[n].created as f64 / 10.0;
    assert!((share("browse") - 60.0).abs() < 3.0, "{by:?}");
    assert!((share("cleanup") - 30.0).abs() < 3.0, "{by:?}");
    assert!((share("rare") - 10.0).abs() < 3.0, "{by:?}");
    assert_eq!(a.scenarios[0].share_pct, 60.0);

    let (b, _) = run(fixture(), &p, config(http)).await;
    assert_eq!(
        a.aggregate.vusers.by_scenario, b.aggregate.vusers.by_scenario,
        "same seed, same picks"
    );
}

#[tokio::test(start_paused = true)]
async fn prerequisites_run_once_and_feed_users() {
    let ws = workspace();
    let doc = LowDoc::parse(
        "mem://deps.arazzo.yaml".into(),
        Source::from_vec(DEPENDS_ARAZZO.as_bytes().to_vec()),
    );
    let plan = Arc::new(compile_plan(&doc, &ws).expect("compiles"));

    struct Count(Mutex<Vec<String>>);
    #[async_trait::async_trait]
    impl HttpClient for Count {
        async fn execute(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
            self.0
                .lock()
                .unwrap()
                .push(format!("{} {}", req.method, req.url));
            let body = if req.method == "POST" {
                r#"{"id":"7","name":"Rex"}"#
            } else {
                r#"{"id":"7"}"#
            };
            Ok(HttpResponse {
                status: if req.method == "POST" { 201 } else { 200 },
                headers: Vec::new(),
                body: Bytes::from_static(body.as_bytes()),
            })
        }
    }
    let counter = Arc::new(Count(Mutex::new(Vec::new())));
    let http: Arc<dyn HttpClient> = counter.clone();
    let p = profile(
        vec![phase(PhaseKind::ArrivalCount { count: 5 }, 1.0)],
        vec![scenario("use-pet", 1.0)],
    );
    let (summary, sink) = run(plan, &p, config(http)).await;

    assert_eq!(summary.hooks.len(), 1);
    assert_eq!(summary.hooks[0].workflow, "seed");
    assert_eq!(summary.hooks[0].role, "dependsOn");
    assert!(summary.hooks[0].ok);
    assert_eq!(summary.aggregate.vusers.completed, 5);
    let calls = counter.0.lock().unwrap();
    assert_eq!(
        calls.iter().filter(|c| c.starts_with("POST")).count(),
        1,
        "seed ran once: {calls:?}"
    );
    assert_eq!(
        calls
            .iter()
            .filter(|c| c.ends_with("/pets/7?via=7"))
            .count(),
        5,
        "{calls:?}"
    );
    assert!(
        sink.notes
            .lock()
            .unwrap()
            .iter()
            .any(|n| n.contains("dependsOn workflow `seed` passed"))
    );
}

/// `suspect test load` shares the plan's dependency resolution: a scenario step
/// whose `dependsOn` names `$workflows.seed.steps.make` makes `seed` a
/// once-per-run prerequisite, and every user's step then runs (before the
/// fix it never became ready, so every user's iteration failed).
#[tokio::test(start_paused = true)]
async fn cross_workflow_step_dependency_runs_its_workflow_first() {
    let text = DEPENDS_ARAZZO.replace(
        "    dependsOn: [seed]\n    steps:\n      - stepId: fetch\n",
        "    steps:\n      - stepId: fetch\n        dependsOn: ['$workflows.seed.steps.make']\n",
    );
    assert_ne!(text, DEPENDS_ARAZZO);
    let doc = LowDoc::parse(
        "mem://deps.arazzo.yaml".into(),
        Source::from_vec(text.into_bytes()),
    );
    let plan = Arc::new(compile_plan(&doc, &workspace()).expect("compiles"));
    let p = profile(
        vec![phase(PhaseKind::ArrivalCount { count: 3 }, 1.0)],
        vec![scenario("use-pet", 1.0)],
    );
    let (summary, _) = run(plan, &p, config(Arc::new(canned_fixture_http()))).await;
    assert_eq!(summary.hooks.len(), 1);
    assert_eq!(summary.hooks[0].workflow, "seed");
    assert!(summary.hooks[0].ok);
    assert_eq!(summary.aggregate.vusers.completed, 3);
    assert_eq!(summary.aggregate.vusers.failed, 0);
}

#[tokio::test(start_paused = true)]
async fn failing_before_hook_aborts_the_run() {
    let http: Arc<dyn HttpClient> = Arc::new(CannedTransport::new());
    let mut p = profile(
        vec![phase(PhaseKind::ArrivalCount { count: 5 }, 1.0)],
        vec![scenario("create-and-fetch", 1.0)],
    );
    p.before = Some("cleanup-user".into());
    let err = run_load(fixture(), &p, config(http), Arc::new(super::SilentSink))
        .await
        .expect_err("hook failure is fatal");
    assert!(
        err.0.contains("before workflow `cleanup-user` failed"),
        "{err}"
    );
}

#[tokio::test(start_paused = true)]
async fn ensure_catches_contract_and_transport_failures() {
    // POST passes, GET returns 500 → every user fails its second step.
    let http: Arc<dyn HttpClient> = Arc::new(
        CannedTransport::new()
            .route(
                Match {
                    method: Some("POST".into()),
                    path_suffix: "/pets".into(),
                },
                HttpResponse {
                    status: 201,
                    headers: Vec::new(),
                    body: Bytes::from_static(br#"{"id":"7"}"#),
                },
            )
            .route(
                Match {
                    method: Some("GET".into()),
                    path_suffix: "/pets/7".into(),
                },
                HttpResponse {
                    status: 500,
                    ..Default::default()
                },
            ),
    );
    let mut p = profile(
        vec![phase(PhaseKind::ArrivalCount { count: 20 }, 1.0)],
        vec![scenario("create-and-fetch", 1.0)],
    );
    p.ensure = Ensure {
        max_error_rate: Some(1.0),
        thresholds: vec![("http.response_time.p99".into(), 1000.0, 0..0)],
        conditions: vec![Condition {
            expression: "http.codes.500 == 0".into(),
            strict: false,
            range: 0..0,
        }],
    };
    let (summary, _) = run(fixture(), &p, config(http)).await;
    let m = &summary.aggregate;
    assert_eq!(m.http.requests, 40);
    assert_eq!(m.contract.failures, 20);
    assert!((m.http.error_rate - 50.0).abs() < 1e-9);
    assert_eq!(m.vusers.failed, 20);
    assert!(!summary.ensure.ok());
    assert_eq!(summary.ensure.failed(), 2, "{:#?}", summary.ensure);
    let junit = super::report::junit(&summary);
    assert!(junit.contains("failures=\"1\""), "{junit}");
    assert!(junit.contains("<skipped message=\"non-strict"), "{junit}");
    let text = super::report::console_final(&summary);
    assert!(text.contains("ENSURE FAILED"), "{text}");
    assert!(text.contains("✗ error rate <= 1%"), "{text}");
    let json = super::report::json(&summary);
    assert_eq!(json["version"], 1);
    assert_eq!(json["run"]["ensure_ok"], false);
    assert_eq!(json["aggregate"]["contract"]["failures"], 20);
}

#[tokio::test(start_paused = true)]
async fn unique_data_rows_feed_users_then_fail_fast() {
    let dir = tempfile::tempdir().unwrap();
    let csv = dir.path().join("users.csv");
    std::fs::write(&csv, "name\nalice\nbob\n").unwrap();
    struct Seen(Mutex<Vec<String>>);
    #[async_trait::async_trait]
    impl HttpClient for Seen {
        async fn execute(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
            self.0.lock().unwrap().push(req.url);
            Ok(HttpResponse {
                status: 204,
                ..Default::default()
            })
        }
    }
    let seen = Arc::new(Seen(Mutex::new(Vec::new())));
    let mut sc = scenario("cleanup-user", 1.0);
    sc.data = Some(DataSource {
        path: csv,
        fields: vec!["userName".into()],
        order: DataOrder::Unique,
        skip_header: true,
        delimiter: b',',
        range: 0..0,
    });
    sc.inputs.insert(
        "extra".into(),
        InputSource::Pool(vec![serde_json::json!(1), serde_json::json!(2)]),
    );
    let p = profile(
        vec![phase(PhaseKind::ArrivalCount { count: 4 }, 1.0)],
        vec![sc],
    );
    let (summary, sink) = run(fixture(), &p, config(seen.clone())).await;
    let urls = seen.0.lock().unwrap();
    assert_eq!(urls.len(), 2, "{urls:?}");
    assert!(urls.iter().any(|u| u.ends_with("/users/alice")));
    assert!(urls.iter().any(|u| u.ends_with("/users/bob")));
    assert_eq!(summary.aggregate.vusers.created, 4);
    assert_eq!(summary.aggregate.vusers.completed, 2);
    assert_eq!(summary.aggregate.vusers.failed, 2);
    assert!(
        sink.notes
            .lock()
            .unwrap()
            .iter()
            .any(|n| n.contains("unique data rows exhausted"))
    );
}

#[tokio::test(start_paused = true)]
async fn cancellation_stops_arrivals_and_drains() {
    let http: Arc<dyn HttpClient> = Arc::new(DelayTransport::new(
        canned_fixture_http(),
        Duration::from_millis(100),
    ));
    let p = profile(
        vec![phase(
            PhaseKind::ArrivalRate {
                rate: 10.0,
                ramp_to: None,
            },
            60.0,
        )],
        vec![scenario("create-and-fetch", 1.0)],
    );
    let (tx, rx) = watch::channel(false);
    let mut cfg = config(http);
    cfg.cancel = Some(rx);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(2050)).await;
        let _ = tx.send(true);
    });
    let (summary, _) = run(fixture(), &p, cfg).await;
    assert!(summary.interrupted);
    assert_eq!(
        summary.aggregate.vusers.created, 21,
        "arrivals at t=0..2.0s inclusive"
    );
    assert_eq!(
        summary.aggregate.vusers.completed, 21,
        "in-flight users drained"
    );
}

#[tokio::test(start_paused = true)]
async fn think_time_from_profile_applies_between_steps() {
    let http: Arc<dyn HttpClient> = Arc::new(canned_fixture_http());
    let mut sc = scenario("create-and-fetch", 1.0);
    sc.think = Some(crate::plan::ThinkSpec::fixed(Duration::from_secs(3)));
    let p = profile(
        vec![phase(PhaseKind::ArrivalCount { count: 2 }, 1.0)],
        vec![sc],
    );
    let (summary, _) = run(fixture(), &p, config(http)).await;
    // Two steps → one 3s pause per user; session length reflects it
    // (HDR histograms keep 3 significant figures, hence the slack).
    assert!(
        summary.aggregate.vusers.session_length.min >= 2990.0,
        "{:?}",
        summary.aggregate.vusers.session_length
    );
}

const PROFILE_YAML: &str = r#"
suspectLoad: "1.0"
arazzo: ./flows.arazzo.yaml
target: http://localhost:8080
environments:
  staging:
    target: https://staging.example.com
    phases:
      - { duration: 1m, arrivalRate: 1 }
phases:
  - { name: warmup, duration: 30s, arrivalRate: 5 }
  - { name: ramp, duration: 2m, arrivalRate: 5, rampTo: 100, maxVusers: 500 }
  - { pause: 10s }
  - { name: spike, duration: 20s, arrivalCount: 2000 }
  - { name: soak, duration: 10m, vus: 50, iterations: 1000 }
before: seed
after: purge
scenarios:
  - name: browse
    workflow: browse-and-search
    weight: 6
    think: { between_steps: 500ms..2s }
  - workflow: create-order
    weight: 3
    inputs:
      currency: [USD, EUR]
      region: eu
    iterations: 3
    continue_on_failure: true
http:
  timeout: 5s
  http2: false
  per_vu_connections: true
ensure:
  maxErrorRate: 1
  thresholds:
    - http.response_time.p95: 250
    - scenario.browse.step.list.response_time.p99: 800
  conditions:
    - expression: http.request_rate > 90 and vusers.failed == 0
    - expression: http.codes.503 == 0
      strict: false
report:
  interval: 5s
  json: ./out/report.json
seed: 7
grace: 15s
"#;

#[test]
fn profile_reader_covers_the_schema() {
    let doc = LowDoc::parse(
        "mem://p.load.yaml".into(),
        Source::from_vec(PROFILE_YAML.as_bytes().to_vec()),
    );
    assert!(super::profile::is_load_profile(&doc));
    let mut p =
        super::read_profile(&doc, std::path::Path::new("/tmp/x/p.load.yaml")).expect("valid");
    assert_eq!(
        p.arazzo.as_deref(),
        Some(std::path::Path::new("/tmp/x/./flows.arazzo.yaml"))
    );
    assert_eq!(p.target.as_deref(), Some("http://localhost:8080"));
    assert_eq!(p.phases.len(), 5);
    assert_eq!(p.phases[0].name.as_deref(), Some("warmup"));
    assert_eq!(p.phases[0].duration, Duration::from_secs(30));
    assert_eq!(
        p.phases[1].kind,
        PhaseKind::ArrivalRate {
            rate: 5.0,
            ramp_to: Some(100.0)
        }
    );
    assert_eq!(p.phases[1].max_vusers, Some(500));
    assert_eq!(p.phases[2].kind, PhaseKind::Pause);
    assert_eq!(p.phases[2].duration, Duration::from_secs(10));
    assert_eq!(p.phases[3].kind, PhaseKind::ArrivalCount { count: 2000 });
    assert_eq!(
        p.phases[4].kind,
        PhaseKind::ConstantVus {
            vus: 50,
            iterations: Some(1000)
        }
    );
    assert_eq!(p.before.as_deref(), Some("seed"));
    assert_eq!(p.after.as_deref(), Some("purge"));
    assert_eq!(p.scenarios.len(), 2);
    assert_eq!(p.scenarios[0].name, "browse");
    assert_eq!(p.scenarios[0].weight, 6.0);
    assert_eq!(
        p.scenarios[0].think,
        Some(crate::plan::ThinkSpec {
            min: Duration::from_millis(500),
            max: Duration::from_secs(2)
        })
    );
    assert_eq!(
        p.scenarios[1].name, "create-order",
        "defaults to workflow id"
    );
    assert_eq!(p.scenarios[1].iterations, 3);
    assert!(p.scenarios[1].continue_on_failure);
    assert_eq!(
        p.scenarios[1].inputs["currency"],
        InputSource::Pool(vec![serde_json::json!("USD"), serde_json::json!("EUR")])
    );
    assert_eq!(
        p.scenarios[1].inputs["region"],
        InputSource::Literal(serde_json::json!("eu"))
    );
    assert_eq!(p.http.timeout, Duration::from_secs(5));
    assert!(!p.http.http2);
    assert!(p.http.per_vu_connections);
    assert_eq!(p.ensure.max_error_rate, Some(1.0));
    assert_eq!(p.ensure.thresholds.len(), 2);
    assert_eq!(p.ensure.conditions.len(), 2);
    assert!(!p.ensure.conditions[1].strict);
    assert_eq!(p.report.interval, Duration::from_secs(5));
    assert_eq!(
        p.report.json.as_deref(),
        Some(std::path::Path::new("/tmp/x/./out/report.json"))
    );
    assert_eq!(p.seed, Some(7));
    assert_eq!(p.grace, Duration::from_secs(15));
    assert_eq!(p.total_weight(), 9.0);

    p.apply_environment("staging").unwrap();
    assert_eq!(p.target.as_deref(), Some("https://staging.example.com"));
    assert_eq!(p.phases.len(), 1);
    assert!(p.apply_environment("prod").is_err());
}

#[test]
fn profile_reader_reports_located_errors() {
    let bad = r#"
suspectLoad: "1.0"
phases:
  - { duration: 10s, arrivalRate: 5, arrivalCount: 3 }
  - { duration: 0s, arrivalRate: 5 }
  - { duration: 10s }
  - { pause: fast }
scenarios:
  - name: nameless
  - workflow: w
    weight: -1
  - workflow: ok
    data: { path: ./missing.csv }
ensure:
  thresholds:
    - { a: 1, b: 2 }
  conditions:
    - expression: "http.x <"
"#;
    let doc = LowDoc::parse(
        "mem://bad.load.yaml".into(),
        Source::from_vec(bad.as_bytes().to_vec()),
    );
    let (_, diags) = super::read_profile_lenient(&doc, std::path::Path::new("bad.load.yaml"));
    let codes: Vec<&str> = diags.iter().map(|d| d.code).collect();
    for want in [
        "load-phase-kind",
        "load-phase-duration",
        "load-duration",
        "load-scenario-workflow",
        "load-scenario-weight",
        "load-data-missing",
        "load-data-fields",
        "load-threshold-shape",
        "load-condition-syntax",
    ] {
        assert!(codes.contains(&want), "missing {want} in {codes:?}");
    }
    assert!(diags.iter().all(|d| d.range.end >= d.range.start));
    // The `pause: fast` error points inside the document, not at 0..0.
    let pause = diags
        .iter()
        .find(|d| d.message.contains("`pause`"))
        .unwrap();
    assert!(pause.range.start > 0);
    assert!(super::read_profile(&doc, std::path::Path::new("bad.load.yaml")).is_err());
}

#[test]
fn check_against_plan_flags_unknown_workflows() {
    let plan = compile_fixture();
    let mut p = profile(vec![], vec![scenario("ghost", 1.0)]);
    p.before = Some("create-and-fetch".into());
    p.after = Some("nope".into());
    let diags = super::profile::check_against_plan(&p, &plan);
    assert!(
        diags
            .iter()
            .any(|d| d.code == "load-unknown-workflow" && d.message.contains("ghost") && d.error)
    );
    assert!(
        diags
            .iter()
            .any(|d| d.code == "load-unknown-workflow" && d.message.contains("nope"))
    );
    let mut p2 = profile(vec![], vec![scenario("create-and-fetch", 1.0)]);
    p2.before = Some("create-and-fetch".into());
    let diags = super::profile::check_against_plan(&p2, &plan);
    assert!(
        diags
            .iter()
            .any(|d| d.code == "load-hook-is-scenario" && !d.error)
    );
}

const SECURED_OAS: &str = r#"
openapi: 3.1.0
info: { title: Secured, version: "1" }
security: [{ tokenAuth: [] }]
components:
  securitySchemes:
    tokenAuth: { type: http, scheme: bearer }
  schemas:
    Pet:
      type: object
      required: [name]
      properties:
        name: { type: string }
paths:
  /pet:
    get:
      operationId: getPet
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Pet' }
"#;

const SECURED_ARAZZO: &str = r#"
arazzo: 1.0.0
info: { title: secured, version: "1" }
sourceDescriptions:
  - { name: api, url: spec.yaml, type: openapi }
workflows:
  - workflowId: fetch
    steps:
      - stepId: pet
        operationId: getPet
"#;

/// Answers 401 without `Authorization: Bearer s3cret`, otherwise 200 with a
/// body that violates the declared `Pet` schema (`name` is a number).
struct GuardedPet;

#[async_trait::async_trait]
impl HttpClient for GuardedPet {
    async fn execute(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        let authorized = req
            .headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("authorization") && v == "Bearer s3cret");
        Ok(if authorized {
            HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: Bytes::from_static(br#"{"name": 7}"#),
            }
        } else {
            HttpResponse {
                status: 401,
                headers: Vec::new(),
                body: Bytes::new(),
            }
        })
    }
}

fn secured_plan() -> Arc<Plan> {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("spec.yaml"), SECURED_OAS).expect("spec");
    let ws = suspect_ref::WorkspaceBuilder::new()
        .root(dir.path())
        .build()
        .expect("ws");
    ws.load_all("spec.yaml").expect("load");
    std::mem::forget(dir);
    let doc = LowDoc::parse(
        "mem://secured.arazzo.yaml".into(),
        Source::from_vec(SECURED_ARAZZO.as_bytes().to_vec()),
    );
    Arc::new(compile_plan(&doc, &Arc::new(ws)).expect("compiles"))
}

#[tokio::test(start_paused = true)]
async fn credentials_are_injected_and_schema_validation_is_opt_in() {
    let p = profile(
        vec![phase(PhaseKind::ArrivalCount { count: 5 }, 1.0)],
        vec![scenario("fetch", 1.0)],
    );
    let http: Arc<dyn HttpClient> = Arc::new(GuardedPet);
    let bearer: crate::auth::AuthConfig = serde_json::from_value(serde_json::json!({
        "schemes": { "tokenAuth": { "kind": "bearer", "token": "s3cret" } }
    }))
    .expect("auth config");

    // No credentials: every request is rejected and fails its 2xx default.
    let (summary, _) = run(secured_plan(), &p, config(Arc::clone(&http))).await;
    assert_eq!(summary.aggregate.http.codes[&401], 5);
    assert_eq!(summary.aggregate.contract.failures, 5);

    // Credentials from the run config reach every virtual user.
    let mut cfg = config(Arc::clone(&http));
    cfg.auth = Arc::new(crate::exec::RunAuth::new(bearer.clone()));
    let (summary, _) = run(secured_plan(), &p, cfg).await;
    assert_eq!(summary.aggregate.http.codes[&200], 5);
    assert_eq!(
        summary.aggregate.contract.failures, 0,
        "response bodies are not schema-checked unless asked"
    );

    // Opting in turns the non-conforming body into contract failures.
    let mut cfg = config(http);
    cfg.auth = Arc::new(crate::exec::RunAuth::new(bearer));
    cfg.validate_responses = true;
    let (summary, _) = run(secured_plan(), &p, cfg).await;
    assert_eq!(summary.aggregate.http.codes[&200], 5);
    assert_eq!(summary.aggregate.contract.failures, 5);
}

/// Generator-side cost per request with no network: 8 closed-loop users
/// pushing 200k two-step iterations through `CannedTransport`. Run with
/// `cargo test --release -p suspect-test load_generator_overhead -- --ignored --nocapture`.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "perf measurement; run explicitly in release mode"]
async fn load_generator_overhead() {
    let http: Arc<dyn HttpClient> = Arc::new(canned_fixture_http());
    let iterations = 100_000u64;
    let p = profile(
        vec![phase(
            PhaseKind::ConstantVus {
                vus: 8,
                iterations: Some(iterations),
            },
            600.0,
        )],
        vec![scenario("create-and-fetch", 1.0)],
    );
    let mut cfg = config(http);
    cfg.report_interval = Duration::from_secs(3600);
    let started = std::time::Instant::now();
    let (summary, _) = run(fixture(), &p, cfg).await;
    let wall = started.elapsed();
    let requests = summary.aggregate.http.requests;
    assert_eq!(requests, iterations * 2);
    assert_eq!(summary.aggregate.dropped, 0);
    eprintln!(
        "load generator overhead: {requests} requests in {:.2}s = {:.1} µs/request wall ({:.0} req/s), {:.1} µs CPU-ish per request across 8 workers",
        wall.as_secs_f64(),
        wall.as_secs_f64() * 1e6 / requests as f64,
        requests as f64 / wall.as_secs_f64(),
        wall.as_secs_f64() * 1e6 * 8.0 / requests as f64,
    );
}
