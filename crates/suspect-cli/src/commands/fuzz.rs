//! `suspect fuzz` — schema-driven mutant fuzzing of an OpenAPI document's
//! operations against a live server.
//!
//! Every operation's query/path parameters and JSON request-body schema are
//! reduced to scalar fields ([`suspect_test::fuzz::scalar_fields`]); a
//! deterministic run counter cycles pathological values through those fields
//! ([`suspect_test::fuzz::generate_mutants`]). Mutants execute concurrently,
//! at most `IN_FLIGHT` requests in flight. A mutant **survives** when the
//! server answers 4xx or any ok-style response; a **crash** is a 5xx or a
//! transport failure/timeout (the client times out at `TIMEOUT`). Any
//! crash exits 1.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use suspect_ir::{IrOperation, IrSpec, ParamIn};
use suspect_journal::Journal;
use suspect_source::Uri;
use suspect_test::fuzz::{self, Mutant, ScalarField};
use suspect_test::{HttpClient, HttpRequest, HttpResponse, TransportError};
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
}

/// Outcome of one executed mutant.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    /// Server rejected or tolerated the input without dying.
    Survived,
    /// 5xx, timeout, or transport failure.
    Crash(String),
}

/// Runs `suspect fuzz` against one OpenAPI document.
///
/// # Errors
/// Propagates workspace/IR compilation failures and transport setup errors;
/// crashes surface through the exit code instead.
pub fn fuzz(spec: &Path, base_url: &str, runs: usize, filter: Option<&str>) -> anyhow::Result<i32> {
    let ws = super::workspace_dir_all(spec)?;
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

    let mut total_sent: u32 = 0;
    let mut survivors: u32 = 0;
    let mut total_crashes: u32 = 0;

    rt.block_on(async {
        for plan in &plans {
            let fields: Vec<ScalarField> = plan.targets.iter().map(|t| t.field.clone()).collect();
            let mutants = fuzz::generate_mutants(&fields, runs);
            let requests: Vec<HttpRequest> = mutants
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let place = plan.targets[i % plan.targets.len()].place;
                    build_request(base_url, plan, m, place)
                })
                .collect();

            let mut sent: u32 = 0;
            let mut crashes: u32 = 0;
            let mut example_crash: Option<String> = None;
            for chunk in requests.chunks(IN_FLIGHT) {
                let mut set = JoinSet::new();
                for req in chunk {
                    let http = Arc::clone(&http);
                    let req = req.clone();
                    set.spawn(async move { http.execute(req).await });
                }
                while let Some(joined) = set.join_next().await {
                    sent += 1;
                    let outcome = joined
                        .unwrap_or_else(|e| Err(TransportError(format!("task panicked: {e}"))));
                    if let Verdict::Crash(line) = classify(outcome) {
                        crashes += 1;
                        if example_crash.is_none() {
                            example_crash = Some(line);
                        }
                    }
                }
            }
            survivors += sent - crashes;
            total_sent += sent;
            total_crashes += crashes;
            println!("{:<44} mutants: {sent:<5} crashes: {crashes}", plan.label);
            if let Some(line) = example_crash {
                println!("    crash: {line}");
            }
        }
    });

    println!();
    println!("fuzz complete: {total_sent} mutants, {total_crashes} crashes");
    let elapsed_ms = started.elapsed().as_millis() as f64;
    let mut journal = Journal::new(Box::new(suspect_journal::StdoutSink));
    journal.run_summary("fuzz", survivors, total_crashes, 0, elapsed_ms);
    Ok(i32::from(total_crashes > 0))
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
            render_scalar(&fuzz::default_value(&t.field.schema))
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
            render_scalar(&fuzz::default_value(&t.field.schema))
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

/// Classifies one executed mutant: any response below 500 survives; 5xx and
/// transport failures (including the 10s timeout) crash.
fn classify(outcome: Result<HttpResponse, TransportError>) -> Verdict {
    match outcome {
        Ok(resp) if resp.status >= 500 => Verdict::Crash(format!("status {}", resp.status)),
        Ok(_) => Verdict::Survived,
        Err(e) => Verdict::Crash(e.to_string()),
    }
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

    #[test]
    fn classifies_status_bands() {
        let ok = Ok(HttpResponse {
            status: 400,
            headers: Vec::new(),
            body: Bytes::new(),
        });
        assert_eq!(classify(ok), Verdict::Survived);
        let ok2 = Ok(HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: Bytes::new(),
        });
        assert_eq!(classify(ok2), Verdict::Survived);
        let boom = Ok(HttpResponse {
            status: 503,
            headers: Vec::new(),
            body: Bytes::new(),
        });
        assert!(matches!(classify(boom), Verdict::Crash(_)));
        assert!(matches!(
            classify(Err(TransportError("timed out".into()))),
            Verdict::Crash(_)
        ));
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
            format!("http://localhost:8080/pets/suspect?limit={expected_fill}")
        );
        assert_eq!(req.method, "POST");
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(body["name"], Value::String("suspect".into()));
        assert_eq!(body["tag"], Value::String("suspect".into()));

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
    let ws = super::workspace_dir_all(&spec_path).unwrap();
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
