//! `suspect test fuzz` — schema-driven mutant fuzzing of an OpenAPI document's
//! operations against a live server.
//!
//! Every operation's query/path parameters and JSON request-body schema are
//! reduced to scalar fields ([`suspect_test::fuzz::scalar_fields`]); a
//! deterministic run counter cycles pathological values through those fields
//! ([`suspect_test::fuzz::generate_mutants`]). Mutants execute concurrently,
//! at most `IN_FLIGHT` requests in flight. A **crash** is a 5xx or a
//! transport failure/timeout (the client times out at `TIMEOUT`). Any
//! other reply is held to the response the operation declares, at the
//! run's conformance level: one that breaks it is a contract
//! **violation**; one that conforms **survives**. Any crash or violation
//! exits 1.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use suspect_ir::{IrOperation, IrSpec, ParamIn};
use suspect_journal::Journal;
use suspect_source::Uri;
use suspect_test::fuzz::{self, Mutant, ScalarField};
use suspect_test::reporters::{Case, Failure, Outcome};
use suspect_test::{Conformance, HttpClient, HttpRequest, HttpResponse, Schemas, TransportError};
use tokio::task::JoinSet;

use super::http::LiveTransport;

/// Maximum concurrent mutant requests.
const IN_FLIGHT: usize = 16;

/// Per-request timeout; anything slower is reported as a hang crash.
const TIMEOUT: Duration = Duration::from_secs(10);

/// Where a mutated field lands on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Substituted into the `{param}` slot of the path template.
    Path,
    /// Appended as `?name=value`.
    Query,
    /// Placed inside the JSON request body.
    Body,
}

/// One fuzzable input field plus its wire placement.
#[derive(Debug, Clone)]
struct Target {
    field: ScalarField,
    place: Place,
}

/// Everything needed to fire mutants at one operation.
struct OpPlan {
    /// Report label (`operationId` or `METHOD /path`).
    label: String,
    /// HTTP method (uppercase).
    method: String,
    /// OpenAPI path template (`/pets/{petId}`).
    path: String,
    /// Fuzzable fields in cycle order.
    targets: Vec<Target>,
    /// Body payload fields (empty when the operation takes no JSON body).
    body_fields: Vec<ScalarField>,
    /// The responses the operation declares; every reply is held to them.
    responses: Vec<suspect_ir::IrResponse>,
}

/// Outcome of one executed mutant.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    /// Server rejected or tolerated the input without dying, with a reply
    /// the operation declares.
    Survived,
    /// 5xx, timeout, or transport failure.
    Crash(String),
    /// A reply that is not one the operation declares.
    Violation(String),
}

/// What every reply of a run is held to.
#[derive(Clone, Copy)]
struct Judge<'a> {
    schemas: &'a Schemas,
    level: Conformance,
}

/// Crash and violation counts of one operation, with the first of each.
#[derive(Debug, Default)]
struct Tally {
    sent: u32,
    crashes: u32,
    violations: u32,
    first_crash: Option<String>,
    first_violation: Option<String>,
}

impl Tally {
    fn record(&mut self, verdict: Verdict) {
        self.sent += 1;
        match verdict {
            Verdict::Survived => {}
            Verdict::Crash(line) => {
                self.crashes += 1;
                self.first_crash.get_or_insert(line);
            }
            Verdict::Violation(line) => {
                self.violations += 1;
                self.first_violation.get_or_insert(line);
            }
        }
    }

    /// The operation as one reported case.
    fn case(&self, label: &str) -> Case {
        let mut failures = Vec::new();
        if let Some(line) = &self.first_crash {
            failures.push(Failure {
                check: "crash".to_owned(),
                expected: "no 5xx or transport failure".to_owned(),
                actual: format!(
                    "{} of {} requests crashed, first: {line}",
                    self.crashes, self.sent
                ),
            });
        }
        if let Some(line) = &self.first_violation {
            failures.push(Failure {
                check: "contract".to_owned(),
                expected: "every reply to be one the operation declares".to_owned(),
                actual: format!(
                    "{} of {} replies broke the contract, first: {line}",
                    self.violations, self.sent
                ),
            });
        }
        Case {
            outcome: if failures.is_empty() {
                Outcome::Passed
            } else {
                Outcome::Failed { failures }
            },
            ..Case::passed("fuzz", label)
        }
    }
}

/// Runs `suspect test fuzz` against one OpenAPI document.
///
/// # Errors
/// Propagates workspace/IR compilation failures and transport setup errors;
/// crashes surface through the exit code instead.
#[allow(clippy::too_many_arguments)]
pub fn fuzz(
    spec: &Path,
    base_url: &str,
    runs: usize,
    filter: Option<&str>,
    journal: Option<&Path>,
    evolved: bool,
    rounds: u32,
    per_round: u32,
    conformance: Conformance,
    report_files: &crate::ReportFiles,
) -> anyhow::Result<i32> {
    let ws = super::workspace_for_entry(spec)?;
    let uri = Uri::from_path(spec)?;
    ws.get(&uri)
        .ok_or_else(|| anyhow::anyhow!("spec document not loaded: {uri}"))?;
    let ir = IrSpec::from_workspace(&ws, &uri)
        .map_err(|e| anyhow::anyhow!("IR compilation failed: {e}"))?;

    let plans: Vec<OpPlan> = ir
        .operations
        .iter()
        .filter(|op| match (&filter, &op.id) {
            (Some(want), Some(id)) => id.contains(want),
            (Some(_), None) => false,
            (None, _) => true,
        })
        .map(|op| plan_operation(&ir, op))
        .collect();
    if plans.is_empty() {
        eprintln!("no operations matched filter {filter:?}");
        return Ok(2);
    }

    let started = Instant::now();
    let http = Arc::new(LiveTransport::new(TIMEOUT)?);
    let rt = tokio::runtime::Runtime::new()?;
    let schemas = Schemas::from_spec(&ir);
    let judge = Judge {
        schemas: &schemas,
        level: conformance,
    };
    let mut tallies: Vec<(String, Tally)> = Vec::with_capacity(plans.len());

    // Grammar-evolved, coverage-guided fuzzing: payloads that produce
    // novel response shapes become seeds; the campaign exploits them.
    if evolved {
        let handle = rt.handle();
        for plan in &plans {
            let fields: Vec<ScalarField> = plan.targets.iter().map(|t| t.field.clone()).collect();
            let http = Arc::clone(&http);
            let mut tally = Tally::default();
            let stats = suspect_test::evolved_fuzz::run_campaign(
                fields,
                rounds,
                per_round,
                &mut |payload| {
                    let request = build_evolved_request(base_url, plan, payload);
                    let outcome = handle.block_on(http.execute(request));
                    let (status, body) = match &outcome {
                        Ok(resp) => (resp.status, serde_json::from_slice(&resp.body).ok()),
                        Err(_) => (0, None),
                    };
                    let verdict = classify(outcome, plan, judge);
                    let crash = matches!(verdict, Verdict::Crash(_));
                    tally.record(verdict);
                    suspect_test::evolved_fuzz::RequestOutcome {
                        status,
                        body,
                        crash,
                    }
                },
            );
            println!(
                "{:<44} corpus: {:<4} shapes: {:<4} crashes: {:<4} violations: {}",
                plan.label, stats.corpus_size, stats.shapes, tally.crashes, tally.violations
            );
            print_examples(&tally);
            tallies.push((plan.label.clone(), tally));
        }
        return finish(
            "evolved-fuzz",
            "evolved fuzz complete",
            "requests",
            spec,
            &tallies,
            journal,
            report_files,
            started,
        );
    }

    rt.block_on(async {
        for plan in &plans {
            let fields: Vec<ScalarField> = plan.targets.iter().map(|t| t.field.clone()).collect();
            let mutants = fuzz::generate_mutants(&fields, runs);
            let requests: Vec<HttpRequest> = mutants
                .iter()
                .enumerate()
                .filter_map(|(i, m)| {
                    let place = plan.targets[i % plan.targets.len()].place;
                    (!leaves_the_route(m, place)).then(|| build_request(base_url, plan, m, place))
                })
                .collect();

            let mut tally = Tally::default();
            for chunk in requests.chunks(IN_FLIGHT) {
                let mut set = JoinSet::new();
                for req in chunk {
                    let http = Arc::clone(&http);
                    let req = req.clone();
                    set.spawn(async move { http.execute(req).await });
                }
                while let Some(joined) = set.join_next().await {
                    let outcome = joined
                        .unwrap_or_else(|e| Err(TransportError(format!("task panicked: {e}"))));
                    tally.record(classify(outcome, plan, judge));
                }
            }
            println!(
                "{:<44} mutants: {:<5} crashes: {:<4} violations: {}",
                plan.label, tally.sent, tally.crashes, tally.violations
            );
            print_examples(&tally);
            tallies.push((plan.label.clone(), tally));
        }
    });

    finish(
        "fuzz",
        "fuzz complete",
        "mutants",
        spec,
        &tallies,
        journal,
        report_files,
        started,
    )
}

/// Prints the first crash and the first violation of an operation.
fn print_examples(tally: &Tally) {
    if let Some(line) = &tally.first_crash {
        println!("    crash: {line}");
    }
    if let Some(line) = &tally.first_violation {
        println!("    violation: {line}");
    }
}

/// Prints the totals, writes the journal record and the report files, and
/// returns the exit code: `1` when anything crashed or broke the contract.
#[allow(clippy::too_many_arguments)]
fn finish(
    kind: &str,
    headline: &str,
    unit: &str,
    spec: &Path,
    tallies: &[(String, Tally)],
    journal: Option<&Path>,
    report_files: &crate::ReportFiles,
    started: Instant,
) -> anyhow::Result<i32> {
    let sent: u32 = tallies.iter().map(|(_, tally)| tally.sent).sum();
    let crashes: u32 = tallies.iter().map(|(_, tally)| tally.crashes).sum();
    let violations: u32 = tallies.iter().map(|(_, tally)| tally.violations).sum();
    println!();
    println!("{headline}: {sent} {unit}, {crashes} crashes, {violations} contract violations");
    let failed = crashes + violations;
    let mut journal = Journal::new(super::run_sink(journal, false)?);
    journal.run_summary(
        kind,
        sent - failed,
        failed,
        0,
        started.elapsed().as_millis() as f64,
    );
    journal.flush()?;
    let cases: Vec<Case> = tallies
        .iter()
        .map(|(label, tally)| tally.case(label))
        .collect();
    report_files.write_cases("fuzz", &spec.display().to_string(), &cases, started)?;
    Ok(i32::from(failed > 0))
}

/// Reduces one IR operation into its fuzzable targets.
fn plan_operation(ir: &IrSpec, op: &IrOperation) -> OpPlan {
    let method = op.method.as_str();
    let label = op
        .id
        .clone()
        .unwrap_or_else(|| format!("{method} {}", op.path));
    let mut targets = Vec::new();
    for p in &op.parameters {
        let place = match p.location {
            ParamIn::Path => Place::Path,
            ParamIn::Query => Place::Query,
            ParamIn::Header | ParamIn::Cookie => continue,
        };
        let schema = match &p.schema {
            Some(s) if !s.is_null() => s.clone(),
            _ => serde_json::json!({"type": "string"}),
        };
        targets.push(Target {
            field: ScalarField {
                name: p.name.clone(),
                schema,
                required: p.required,
            },
            place,
        });
    }
    let body_fields = resolve_body(ir, op)
        .map(fuzz::scalar_fields)
        .unwrap_or_default();
    // Body fields join the same mutation cycle as path/query parameters;
    // `body_fields` additionally drives whole-payload assembly.
    for field in &body_fields {
        targets.push(Target {
            field: field.clone(),
            place: Place::Body,
        });
    }
    OpPlan {
        label,
        method: method.to_owned(),
        path: op.path.clone(),
        targets,
        body_fields,
        responses: op.responses.clone(),
    }
}

/// Resolves the operation's JSON request-body component schema, if any.
fn resolve_body<'a>(ir: &'a IrSpec, op: &IrOperation) -> Option<&'a Value> {
    let name = op.body_schema.as_ref()?;
    let idx = *ir.schema_index.get(name)?;
    ir.schemas.get(idx as usize).map(|s| &s.json)
}

/// Builds one mutant request: path/query fields default except the targeted
/// one, body assembled via [`fuzz::payload`] when the operation has a body.
fn build_request(base_url: &str, plan: &OpPlan, m: &Mutant, place: Place) -> HttpRequest {
    let mut url = plan.path.clone();
    for t in &plan.targets {
        let value = if place == Place::Path && t.field.name == m.field {
            render_scalar(&m.value)
        } else {
            render_scalar(&fuzz::default_value(&t.field.schema, &t.field.name))
        };
        if t.place == Place::Path {
            url = url.replace(&format!("{{{}}}", t.field.name), &encode_component(&value));
        }
    }
    let mut query = String::new();
    for t in &plan.targets {
        if t.place != Place::Query {
            continue;
        }
        let value = if place == Place::Query && t.field.name == m.field {
            render_scalar(&m.value)
        } else {
            render_scalar(&fuzz::default_value(&t.field.schema, &t.field.name))
        };
        if !query.is_empty() {
            query.push('&');
        }
        query.push_str(&encode_component(&t.field.name));
        query.push('=');
        query.push_str(&encode_component(&value));
    }
    if !query.is_empty() {
        url.push('?');
        url.push_str(&query);
    }
    if !url.starts_with('/') {
        url.insert(0, '/');
    }
    let full_url = format!("{}/{}", base_url.trim_end_matches('/'), &url[1..]);

    let (body, headers) = if plan.body_fields.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        // Only apply the mutation inside the body when it targets a body
        // field; otherwise every field keeps its benign default.
        let effective = if place == Place::Body {
            m.clone()
        } else {
            Mutant {
                field: String::new(),
                kind: fuzz::MUTANT_KINDS[0],
                value: Value::Null,
            }
        };
        let payload = fuzz::payload(&plan.body_fields, &effective);
        (
            serde_json::to_vec(&payload).unwrap_or_default(),
            vec![("content-type".into(), "application/json".into())],
        )
    };

    HttpRequest {
        method: plan.method.clone(),
        url: full_url,
        headers,
        body: body.into(),
    }
}

/// Whether a mutant would address another operation instead of this one:
/// an empty path segment turns `/pets/{petId}` into `/pets/`, the
/// collection. Whatever answers there says nothing about this operation,
/// so the mutant is not sent.
fn leaves_the_route(m: &Mutant, place: Place) -> bool {
    place == Place::Path && render_scalar(&m.value).is_empty()
}

/// Renders a JSON scalar into its wire string form.
fn render_scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        other => other.to_string(),
    }
}

/// Percent-encodes one URL component (RFC 3986 unreserved set kept raw).
fn encode_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Builds one evolved-campaign request: the payload carries a value for
/// every field, distributed by place — path substitution, query params, and
/// a JSON body merged over schema defaults.
fn build_evolved_request(base_url: &str, plan: &OpPlan, payload: &Value) -> HttpRequest {
    let field_value = |name: &str| -> Value { payload.get(name).cloned().unwrap_or(Value::Null) };
    let render = |value: &Value| -> String {
        match value {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        }
    };
    let mut url = plan.path.clone();
    for t in &plan.targets {
        if t.place != Place::Path {
            continue;
        }
        let value = render(&field_value(&t.field.name));
        url = url.replace(&format!("{{{}}}", t.field.name), &encode_component(&value));
    }
    let mut query = String::new();
    for t in &plan.targets {
        if t.place != Place::Query {
            continue;
        }
        if !query.is_empty() {
            query.push('&');
        }
        query.push_str(&encode_component(&t.field.name));
        query.push('=');
        query.push_str(&encode_component(&render(&field_value(&t.field.name))));
    }
    if !query.is_empty() {
        url.push('?');
        url.push_str(&query);
    }
    if !url.starts_with('/') {
        url.insert(0, '/');
    }
    let full_url = format!("{}/{}", base_url.trim_end_matches('/'), &url[1..]);
    let (body, headers) = if plan.body_fields.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        let mut merged = serde_json::Map::new();
        for field in &plan.body_fields {
            merged.insert(
                field.name.clone(),
                payload
                    .get(&field.name)
                    .cloned()
                    .unwrap_or_else(|| fuzz::default_value(&field.schema, &field.name)),
            );
        }
        (
            serde_json::to_vec(&Value::Object(merged)).unwrap_or_default(),
            vec![("content-type".into(), "application/json".into())],
        )
    };
    HttpRequest {
        method: plan.method.clone(),
        url: full_url,
        headers,
        body: body.into(),
    }
}

/// Classifies one executed mutant: a 5xx or a transport failure (including
/// the 10s timeout) is a crash; any other reply either is one the
/// operation declares, and survives, or breaks the contract.
fn classify(
    outcome: Result<HttpResponse, TransportError>,
    plan: &OpPlan,
    judge: Judge<'_>,
) -> Verdict {
    let response = match outcome {
        Ok(response) if response.status >= 500 => {
            return Verdict::Crash(format!("status {}", response.status));
        }
        Ok(response) => response,
        Err(e) => return Verdict::Crash(e.to_string()),
    };
    let check = suspect_runtime::check_response(
        &plan.responses,
        judge.schemas,
        suspect_runtime::Reply {
            method: &plan.method,
            status: response.status,
            headers: &response.headers,
            body: &response.body,
        },
        judge.level,
    );
    if check.conforms() {
        return Verdict::Survived;
    }
    Verdict::Violation(format!(
        "status {}: {}",
        response.status,
        check
            .violations
            .iter()
            .map(suspect_runtime::Violation::located)
            .collect::<Vec<_>>()
            .join("; ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn str_target(name: &str, place: Place) -> Target {
        Target {
            field: ScalarField {
                name: name.to_owned(),
                schema: serde_json::json!({"type": "string"}),
                required: true,
            },
            place,
        }
    }

    #[test]
    fn encodes_unsafe_components() {
        assert_eq!(encode_component("a b/c?d"), "a%20b%2Fc%3Fd");
        assert_eq!(encode_component("ok-_.~1"), "ok-_.~1");
    }

    /// An operation declaring `200` with a JSON object that requires `id`.
    fn judged_plan() -> (OpPlan, Schemas) {
        let schemas: std::collections::BTreeMap<String, Value> = [(
            "Pet".to_owned(),
            serde_json::json!({"type": "object", "required": ["id"]}),
        )]
        .into();
        let plan = OpPlan {
            label: "showPet".into(),
            method: "GET".into(),
            path: "/pets/{petId}".into(),
            targets: Vec::new(),
            body_fields: Vec::new(),
            responses: vec![suspect_ir::IrResponse {
                status: Some(200),
                schema: Some("Pet".to_owned()),
                content: vec![suspect_ir::IrMedia {
                    media_type: "application/json".to_owned(),
                    schema: Some("Pet".to_owned()),
                    unresolved: None,
                }],
                ..Default::default()
            }],
        };
        (plan, Schemas::from_any_map(schemas.iter()))
    }

    fn reply(status: u16, body: &'static str) -> Result<HttpResponse, TransportError> {
        Ok(HttpResponse {
            status,
            headers: Vec::new(),
            body: Bytes::from_static(body.as_bytes()),
        })
    }

    #[test]
    fn an_empty_path_segment_is_not_a_mutant_of_this_operation() {
        let empty = Mutant {
            field: "petId".into(),
            kind: fuzz::MutantKind::Empty,
            value: Value::String(String::new()),
        };
        assert!(leaves_the_route(&empty, Place::Path));
        // The same value in the query or the body still addresses the
        // operation.
        assert!(!leaves_the_route(&empty, Place::Query));
        assert!(!leaves_the_route(&empty, Place::Body));
        let null = Mutant {
            field: "petId".into(),
            kind: fuzz::MutantKind::NullRequired,
            value: Value::Null,
        };
        assert!(!leaves_the_route(&null, Place::Path), "`null` is a segment");
    }

    #[test]
    fn classifies_crashes_before_anything_else() {
        let (plan, schemas) = judged_plan();
        let judge = Judge {
            schemas: &schemas,
            level: Conformance::Strict,
        };
        assert!(matches!(
            classify(reply(503, ""), &plan, judge),
            Verdict::Crash(_)
        ));
        assert!(matches!(
            classify(Err(TransportError("timed out".into())), &plan, judge),
            Verdict::Crash(_)
        ));
    }

    #[test]
    fn a_declared_conforming_reply_survives() {
        let (plan, schemas) = judged_plan();
        let judge = Judge {
            schemas: &schemas,
            level: Conformance::Strict,
        };
        assert_eq!(
            classify(reply(200, r#"{"id": "p1"}"#), &plan, judge),
            Verdict::Survived
        );
    }

    #[test]
    fn a_reply_that_breaks_the_contract_is_a_violation() {
        let (plan, schemas) = judged_plan();
        let strict = Judge {
            schemas: &schemas,
            level: Conformance::Strict,
        };
        // A body that is not the declared schema.
        let Verdict::Violation(line) = classify(reply(200, "{}"), &plan, strict) else {
            panic!("a missing required property is a violation");
        };
        assert!(line.contains("status 200") && line.contains("id"), "{line}");
        // A rejection the description does not declare.
        assert!(matches!(
            classify(reply(400, "{}"), &plan, strict),
            Verdict::Violation(_)
        ));

        // Below strict, an undeclared rejection survives...
        let schema_only = Judge {
            schemas: &schemas,
            level: Conformance::Schema,
        };
        assert_eq!(
            classify(reply(400, "{}"), &plan, schema_only),
            Verdict::Survived
        );
        // ...and with conformance off only crashes count.
        let off = Judge {
            schemas: &schemas,
            level: Conformance::Off,
        };
        assert_eq!(classify(reply(200, "{}"), &plan, off), Verdict::Survived);
    }

    #[test]
    fn builds_request_with_path_query_and_body_mutation() {
        let plan = OpPlan {
            label: "createPet".into(),
            method: "POST".into(),
            path: "/pets/{petId}".into(),
            targets: vec![
                str_target("petId", Place::Path),
                str_target("limit", Place::Query),
            ],
            body_fields: vec![
                ScalarField {
                    name: "/name".into(),
                    schema: serde_json::json!({"type": "string"}),
                    required: true,
                },
                ScalarField {
                    name: "/tag".into(),
                    schema: serde_json::json!({"type": "string"}),
                    required: false,
                },
            ],
            responses: Vec::new(),
        };
        // A query-targeted oversize mutant: path/body stay defaulted.
        let m = Mutant {
            field: "limit".into(),
            kind: fuzz::MutantKind::Oversize,
            value: Value::String("a".repeat(512)),
        };
        let req = build_request("http://localhost:8080/", &plan, &m, Place::Query);
        let expected_fill = "a".repeat(512);
        assert_eq!(
            req.url,
            format!("http://localhost:8080/pets/1?limit={expected_fill}")
        );
        assert_eq!(req.method, "POST");
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        // Field names are JSON pointers; the faker reads their words, so a
        // `name` field gets a realistic name rather than filler text.
        assert_eq!(body["name"], Value::String("Ada Lovelace".into()));
        assert_eq!(body["tag"], Value::String("sample text".into()));

        // A body-targeted null mutant lands null inside the JSON payload.
        let m = Mutant {
            field: "/name".into(),
            kind: fuzz::MutantKind::NullRequired,
            value: Value::Null,
        };
        let req = build_request("http://localhost:8080", &plan, &m, Place::Body);
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(body["name"], Value::Null);
        assert_eq!(req.headers[0].0, "content-type");
    }

    #[test]
    fn bodyless_operations_send_no_body() {
        let plan = OpPlan {
            label: "listPets".into(),
            method: "GET".into(),
            path: "/pets".into(),
            targets: vec![str_target("limit", Place::Query)],
            body_fields: Vec::new(),
            responses: Vec::new(),
        };
        let m = Mutant {
            field: "limit".into(),
            kind: fuzz::MutantKind::Negative,
            value: Value::from(-1),
        };
        let req = build_request("http://x.test", &plan, &m, Place::Query);
        assert_eq!(req.url, "http://x.test/pets?limit=-1");
        assert!(req.body.is_empty());
        assert_eq!(req.headers.len(), 0);
    }
}

#[test]
fn plan_operation_resolves_body_fields_from_ir() {
    let dir = std::env::temp_dir().join(format!("suspect-fuzz-plan-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let spec_path = dir.join("spec.yaml");
    std::fs::write(
        &spec_path,
        r###"
openapi: 3.0.0
info:
  title: T
  version: "1"
paths:
  /pets/{petId}:
    post:
      operationId: createPet
      parameters:
        - name: petId
          in: path
          required: true
          schema:
            type: string
      requestBody:
        content:
          application/json:
            schema:
              $ref: "#/components/schemas/Pet"
      responses:
        "200":
          description: ok
components:
  schemas:
    Pet:
      type: object
      required: [name]
      properties:
        name:
          type: string
"###,
    )
    .unwrap();
    let ws = super::workspace_for_entry(&spec_path).unwrap();
    let uri = Uri::from_path(&spec_path).unwrap();
    let ir = IrSpec::from_workspace(&ws, &uri).unwrap();
    let op = ir
        .operation(suspect_ir::OpSelector::Id("createPet"))
        .unwrap();
    let plan = plan_operation(&ir, op);
    assert_eq!(op.body_schema.as_deref(), Some("Pet"));
    // path param + query-less body field => two targets, one of them body.
    assert_eq!(plan.targets.len(), 2);
    assert!(plan.targets.iter().any(|t| t.place == Place::Body));
    assert_eq!(
        plan.body_fields
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        vec!["/name"]
    );
    std::fs::remove_dir_all(&dir).ok();
}
