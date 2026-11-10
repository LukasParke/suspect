//! Stateful dependency-graph test generation.
//!
//! Auto-discovers resource dependencies from path semantics
//! (POST /users creates User → GET /users/{userId} reads it), builds a
//! dependency DAG, and generates test sequences that set up required state
//! before exercising dependent operations.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use suspect_ir::{IrSpec, Method};

/// One node in the resource dependency graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceNode {
    /// Resource type name derived from the path (e.g. `users`, `posts`).
    pub name: String,
    /// Operations that create this resource (201 responses).
    pub creators: Vec<OpRef>,
    /// Operations that read this resource.
    pub readers: Vec<OpRef>,
    /// Operations that modify this resource.
    pub mutators: Vec<OpRef>,
    /// Operations that delete this resource.
    pub deleters: Vec<OpRef>,
    /// Parent resources this one depends on (e.g. posts → users).
    pub depends_on: BTreeSet<String>,
}

/// A reference to an operation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OpRef {
    /// operationId when present.
    pub id: Option<String>,
    /// HTTP method.
    pub method: String,
    /// Path template.
    pub path: String,
}

/// The full dependency graph.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DependencyGraph {
    /// Resource nodes keyed by name, in topological order.
    pub nodes: BTreeMap<String, ResourceNode>,
    /// Edges: child → parents.
    pub edges: BTreeMap<String, BTreeSet<String>>,
}

/// One generated test step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestStep {
    /// The step's identifier: `param_sources` reference setup steps as
    /// `$steps.<id>.response.body#/pointer`.
    pub id: String,
    /// The operation to invoke.
    pub op: OpRef,
    /// Purpose: `setup` | `exercise` | `verify` | `teardown`.
    pub phase: String,
    /// Path parameters sourced from earlier steps (param → JSON pointer).
    pub param_sources: BTreeMap<String, String>,
    /// Request body template (schema-derived defaults).
    pub body: Option<serde_json::Value>,
}

/// One generated test sequence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestSequence {
    /// What this sequence exercises.
    pub target: OpRef,
    /// Ordered steps: setups first, then the target, then teardown.
    pub steps: Vec<TestStep>,
}

/// Builds the resource dependency graph from a spec.
#[must_use]
pub fn build_graph(spec: &IrSpec) -> DependencyGraph {
    let mut nodes: BTreeMap<String, ResourceNode> = BTreeMap::new();

    for op in &spec.operations {
        let segments: Vec<&str> = op.path.split('/').filter(|s| !s.is_empty()).collect();
        if segments.is_empty() {
            continue;
        }

        // Identify resource segments (non-parameter) and their depth
        let mut current_resource: Option<String> = None;
        let mut parents: Vec<String> = Vec::new();

        for (i, seg) in segments.iter().enumerate() {
            if seg.starts_with('{') {
                // Parameter segment: the previous non-param segment is the parent
                if let Some(parent) = &current_resource {
                    parents.push(parent.clone());
                }
                continue;
            }
            let resource = singularize(seg);
            let node = nodes
                .entry(resource.clone())
                .or_insert_with(|| ResourceNode {
                    name: resource.clone(),
                    creators: Vec::new(),
                    readers: Vec::new(),
                    mutators: Vec::new(),
                    deleters: Vec::new(),
                    depends_on: BTreeSet::new(),
                });

            // Is this the final resource (the one the operation acts on)?
            let is_final =
                i == segments.len() - 1 || segments[i + 1..].iter().all(|s| s.starts_with('{'));

            if is_final {
                let op_ref = OpRef {
                    id: op.id.clone(),
                    method: op.method.as_str().to_uppercase(),
                    path: op.path.clone(),
                };
                match op.method {
                    Method::Post => node.creators.push(op_ref),
                    Method::Delete => node.deleters.push(op_ref),
                    Method::Put | Method::Patch => node.mutators.push(op_ref),
                    _ => node.readers.push(op_ref),
                }
                for p in &parents {
                    node.depends_on.insert(p.clone());
                }
            }
            current_resource = Some(resource);
        }
    }

    let mut edges = BTreeMap::new();
    for (name, node) in &nodes {
        if !node.depends_on.is_empty() {
            edges.insert(name.clone(), node.depends_on.clone());
        }
    }

    DependencyGraph { nodes, edges }
}

/// Generates test sequences for every operation that mutates state.
#[must_use]
pub fn generate_sequences(spec: &IrSpec) -> Vec<TestSequence> {
    let graph = build_graph(spec);
    let mut sequences = Vec::new();

    for node in graph.nodes.values() {
        // Exercise creators, mutators, and deleters (they need setup)
        let targets: Vec<(&OpRef, String)> = node
            .creators
            .iter()
            .map(|op| (op, "exercise".to_owned()))
            .chain(node.mutators.iter().map(|op| (op, "exercise".to_owned())))
            .chain(node.deleters.iter().map(|op| (op, "exercise".to_owned())))
            .collect();

        for (target, phase) in targets {
            let mut steps = Vec::new();

            // Setup: create parent resources first (topological order)
            for parent_name in topological_parents(&graph, &node.name) {
                let Some(parent_node) = graph.nodes.get(&parent_name) else {
                    continue;
                };
                if let Some(creator) = parent_node.creators.first() {
                    steps.push(TestStep {
                        id: format!("create_{parent_name}"),
                        op: creator.clone(),
                        phase: "setup".to_owned(),
                        param_sources: path_params_from_graph(&graph, &parent_name),
                        body: default_body(spec, creator),
                    });
                }
            }

            // The target operation itself. When the target is the
            // resource's creator, it IS the creation step every later
            // reference points at — name it accordingly.
            let target_is_creator = node.creators.iter().any(|c| c == target);
            // A mutator or deleter needs the resource to exist: create it
            // after its parents, before the target.
            if !target_is_creator && let Some(creator) = node.creators.first() {
                steps.push(TestStep {
                    id: format!("create_{}", node.name),
                    op: creator.clone(),
                    phase: "setup".to_owned(),
                    param_sources: path_params_from_graph(&graph, &node.name),
                    body: default_body(spec, creator),
                });
            }
            steps.push(TestStep {
                id: if target_is_creator {
                    format!("create_{}", node.name)
                } else {
                    format!("exercise_{}", node.name)
                },
                op: target.clone(),
                phase: phase.clone(),
                param_sources: path_params_from_graph(&graph, &node.name),
                body: default_body(spec, target),
            });

            // Teardown: delete what we created (reverse order)
            if !node.deleters.is_empty() && target.method != "DELETE" {
                steps.push(TestStep {
                    id: format!("teardown_{}", node.name),
                    op: node.deleters[0].clone(),
                    phase: "teardown".to_owned(),
                    param_sources: path_params_from_graph(&graph, &node.name),
                    body: None,
                });
            }

            sequences.push(TestSequence {
                target: target.clone(),
                steps,
            });
        }
    }

    sequences
}

/// Returns parent resource names in topological order (roots first).
fn topological_parents(graph: &DependencyGraph, resource: &str) -> Vec<String> {
    let mut order = Vec::new();
    let mut visited = BTreeSet::new();
    visit(graph, resource, &mut visited, &mut order);
    order.reverse(); // roots first
    order.retain(|n| n != resource);
    order
}

fn visit(
    graph: &DependencyGraph,
    node: &str,
    visited: &mut BTreeSet<String>,
    order: &mut Vec<String>,
) {
    if !visited.insert(node.to_owned()) {
        return;
    }
    if let Some(parents) = graph.edges.get(node) {
        for p in parents {
            visit(graph, p, visited, order);
        }
    }
    order.push(node.to_owned());
}

/// Maps path parameters for a resource to JSON pointers into earlier
/// setup-step responses.
fn path_params_from_graph(graph: &DependencyGraph, resource: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    // Own id
    map.insert(
        format!("{resource}Id"),
        format!("$steps.create_{resource}.response.body#/id"),
    );
    // Parent ids
    if let Some(parents) = graph.edges.get(resource) {
        for parent in parents {
            map.insert(
                format!("{parent}Id"),
                format!("$steps.create_{parent}.response.body#/id"),
            );
        }
    }
    map
}

/// Generates a default request body from an operation's declared schema.
fn default_body(spec: &IrSpec, op: &OpRef) -> Option<serde_json::Value> {
    let ir_op = spec
        .operations
        .iter()
        .find(|o| o.method.as_str().eq_ignore_ascii_case(&op.method) && o.path == op.path)?;
    let schema_name = ir_op.body_schema.as_deref()?;
    let schema = spec.schemas.iter().find(|s| s.name == schema_name)?;
    Some(default_from_schema(spec, &schema.json, 0))
}

fn default_from_schema(
    spec: &IrSpec,
    schema: &serde_json::Value,
    depth: usize,
) -> serde_json::Value {
    if depth > 6 {
        return serde_json::Value::Null;
    }

    // $ref → resolve
    if let Some(r) = schema.get("$ref").and_then(|v| v.as_str()) {
        let name = r.rsplit('/').next().unwrap_or(r);
        if let Some(resolved) = spec.schemas.iter().find(|s| s.name == name) {
            return default_from_schema(spec, &resolved.json, depth + 1);
        }
        return serde_json::Value::Null;
    }

    match schema.get("type").and_then(|t| t.as_str()) {
        Some("object") => {
            let mut obj = serde_json::Map::new();
            if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
                for (key, prop) in props {
                    obj.insert(key.clone(), default_from_schema(spec, prop, depth + 1));
                }
            }
            serde_json::Value::Object(obj)
        }
        Some("array") => {
            let item = schema
                .get("items")
                .map(|i| default_from_schema(spec, i, depth + 1))
                .unwrap_or(serde_json::Value::Null);
            serde_json::Value::Array(vec![item])
        }
        Some("string") => {
            if let Some(enum_vals) = schema.get("enum").and_then(|e| e.as_array())
                && let Some(first) = enum_vals.first()
            {
                return first.clone();
            }
            serde_json::Value::String("example".to_owned())
        }
        Some("integer") | Some("number") => serde_json::json!(1),
        Some("boolean") => serde_json::Value::Bool(true),
        _ => serde_json::Value::Null,
    }
}

/// Naive singularization: `users` → `user`, `ies` → `y`.
fn singularize(word: &str) -> String {
    if let Some(stem) = word.strip_suffix("ies") {
        return format!("{stem}y");
    }
    if let Some(stem) = word.strip_suffix('s')
        && !stem.is_empty()
    {
        return stem.to_owned();
    }
    word.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use suspect_ir::{IrOperation, IrResponse, IrSchema};

    fn petstore() -> IrSpec {
        IrSpec {
            operations: vec![
                IrOperation {
                    id: Some("createUser".to_owned()),
                    method: Method::Post,
                    path: "/users".to_owned(),
                    summary: None,
                    description: None,
                    tags: Vec::new(),
                    deprecated: false,
                    parameters: Vec::new(),
                    body_schema: Some("User".to_owned()),
                    responses: vec![IrResponse {
                        status: Some(201),
                        description: None,
                        schema: Some("User".to_owned()),
                        ..Default::default()
                    }],
                },
                IrOperation {
                    id: Some("createPost".to_owned()),
                    method: Method::Post,
                    path: "/users/{userId}/posts".to_owned(),
                    summary: None,
                    description: None,
                    tags: Vec::new(),
                    deprecated: false,
                    parameters: Vec::new(),
                    body_schema: Some("Post".to_owned()),
                    responses: vec![IrResponse {
                        status: Some(201),
                        description: None,
                        schema: Some("Post".to_owned()),
                        ..Default::default()
                    }],
                },
                IrOperation {
                    id: Some("deletePost".to_owned()),
                    method: Method::Delete,
                    path: "/users/{userId}/posts/{postId}".to_owned(),
                    summary: None,
                    description: None,
                    tags: Vec::new(),
                    deprecated: false,
                    parameters: Vec::new(),
                    body_schema: None,
                    responses: vec![IrResponse {
                        status: Some(204),
                        description: None,
                        schema: None,
                        ..Default::default()
                    }],
                },
            ],
            schemas: vec![
                IrSchema {
                    name: "User".to_owned(),
                    json: serde_json::json!({"type":"object","properties":{"name":{"type":"string"}}}),
                },
                IrSchema {
                    name: "Post".to_owned(),
                    json: serde_json::json!({"type":"object","properties":{"title":{"type":"string"}}}),
                },
            ],
            ..IrSpec::default()
        }
    }

    #[test]
    fn graph_discovers_parent_dependency() {
        let graph = build_graph(&petstore());
        let posts = graph.nodes.get("post").expect("post node");
        assert!(posts.depends_on.contains("user"), "posts depend on users");
        assert_eq!(posts.creators.len(), 1);
        assert_eq!(posts.deleters.len(), 1);
    }

    #[test]
    fn sequences_order_setup_before_target() {
        let seqs = generate_sequences(&petstore());
        let post_seq = seqs
            .iter()
            .find(|s| s.target.id.as_deref() == Some("createPost"))
            .expect("createPost sequence");
        assert_eq!(post_seq.steps[0].phase, "setup", "user creation first");
        assert_eq!(post_seq.steps[0].op.id.as_deref(), Some("createUser"));
        let target = post_seq
            .steps
            .iter()
            .find(|s| s.phase == "exercise")
            .unwrap();
        assert_eq!(target.op.id.as_deref(), Some("createPost"));
    }

    /// Serves POSTs with a fresh id and everything else with 2xx,
    /// recording request paths.
    #[derive(Default)]
    struct RecordingServer {
        paths: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl crate::exec::HttpClient for RecordingServer {
        async fn execute(
            &self,
            req: crate::exec::HttpRequest,
        ) -> Result<crate::exec::HttpResponse, crate::exec::TransportError> {
            self.paths.lock().unwrap().push(req.url.clone());
            let body = match req.method.as_str() {
                "POST" => br#"{"id": 7}"#.to_vec(),
                _ => br#"{}"#.to_vec(),
            };
            Ok(crate::exec::HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: body.into(),
            })
        }
    }

    #[test]
    fn sequences_thread_created_ids_through_later_steps() {
        let seqs = generate_sequences(&petstore());
        let delete_seq = seqs
            .iter()
            .find(|s| s.target.id.as_deref() == Some("deletePost"))
            .expect("deletePost sequence");
        // The mutator target needs the resource first: its setup creates
        // the post, after the user.
        let setup_ids: Vec<&str> = delete_seq.steps.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            setup_ids,
            vec!["create_user", "create_post", "exercise_post"],
            "setup creates the resource before the delete target runs; a delete target needs no teardown"
        );
        let server = RecordingServer::default();
        let outcome = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(run_sequence(delete_seq, "http://api.test", &server));
        assert!(outcome.passed, "sequence must pass: {outcome:?}");
        let paths = server.paths.lock().unwrap().clone();
        assert_eq!(
            paths,
            vec![
                "http://api.test/users",
                "http://api.test/users/7/posts",
                "http://api.test/users/7/posts/7",
            ],
            "created ids thread through every later path"
        );
    }

    /// A description whose `createWidget` answers `201` with a `Widget`.
    const WIDGETS: &str = r##"
openapi: 3.1.0
info: { title: Widgets, version: "1" }
paths:
  /widgets:
    post:
      operationId: createWidget
      requestBody:
        content:
          application/json:
            schema: { $ref: '#/components/schemas/Widget' }
      responses:
        '201':
          description: created
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Widget' }
  /widgets/{widgetId}:
    delete:
      operationId: deleteWidget
      parameters:
        - { name: widgetId, in: path, required: true, schema: { type: string } }
      responses:
        '204': { description: gone }
components:
  schemas:
    Widget:
      type: object
      required: [id]
      properties:
        id: { type: string }
"##;

    /// Answers `POST` with `created` and everything else with `204`.
    struct WidgetServer {
        created: (u16, &'static str),
    }

    #[async_trait::async_trait]
    impl crate::exec::HttpClient for WidgetServer {
        async fn execute(
            &self,
            req: crate::exec::HttpRequest,
        ) -> Result<crate::exec::HttpResponse, crate::exec::TransportError> {
            let (status, body) = match req.method.as_str() {
                "POST" => self.created,
                _ => (204, ""),
            };
            Ok(crate::exec::HttpResponse {
                status,
                headers: vec![("content-type".to_owned(), "application/json".to_owned())],
                body: bytes::Bytes::from(body),
            })
        }
    }

    fn run_widgets(
        created: (u16, &'static str),
        level: suspect_runtime::Conformance,
    ) -> SequenceResult {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("widgets.yaml");
        std::fs::write(&path, WIDGETS).expect("write");
        let spec = IrSpec::from_file(&path).expect("spec");
        let schemas = suspect_runtime::Schemas::from_spec(&spec);
        let sequence = generate_sequences(&spec)
            .into_iter()
            .find(|seq| seq.target.id.as_deref() == Some("createWidget"))
            .expect("createWidget sequence");
        let server = WidgetServer { created };
        tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(run_sequence_checked(
                &sequence,
                "http://api.test",
                &server,
                SequenceContract {
                    spec: &spec,
                    schemas: &schemas,
                    level,
                },
            ))
    }

    #[test]
    fn a_conforming_sequence_passes_the_contract_check() {
        let outcome = run_widgets(
            (201, r#"{"id": "w1"}"#),
            suspect_runtime::Conformance::Strict,
        );
        assert!(outcome.passed, "{outcome:?}");
    }

    #[test]
    fn a_step_whose_body_breaks_its_schema_fails_the_sequence() {
        let outcome = run_widgets((201, r#"{"id": 7}"#), suspect_runtime::Conformance::Strict);
        assert!(!outcome.passed, "{outcome:?}");
        let message = outcome.message.expect("a diagnosis");
        assert!(
            message.contains("createWidget") || message.contains("POST /widgets"),
            "{message}"
        );
        assert!(
            message.contains("/id"),
            "the violation is located: {message}"
        );
    }

    #[test]
    fn a_step_with_an_undeclared_success_status_fails_when_strict() {
        let strict = run_widgets(
            (200, r#"{"id": "w1"}"#),
            suspect_runtime::Conformance::Strict,
        );
        assert!(!strict.passed, "{strict:?}");
        assert!(
            strict.message.as_deref().is_some_and(|m| m.contains("200")),
            "{strict:?}"
        );
        let lenient = run_widgets(
            (200, r#"{"id": "w1"}"#),
            suspect_runtime::Conformance::Schema,
        );
        assert!(lenient.passed, "{lenient:?}");
    }

    #[test]
    fn singularize_variants() {
        assert_eq!(singularize("users"), "user");
        assert_eq!(singularize("entries"), "entry");
        assert_eq!(singularize("status"), "statu"); // acceptable: graph is heuristic
    }
}

/// Outcome of running one generated sequence against a live server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SequenceResult {
    /// The exercised target: operationId when present, else `METHOD path`.
    pub target: String,
    /// Steps executed before a failure, or all of them on success.
    pub steps: usize,
    /// Whether every step returned a 2xx.
    pub passed: bool,
    /// First failure message, when the sequence failed.
    pub message: Option<String>,
}

/// The contract every reply of a sequence is held to.
#[derive(Clone, Copy)]
pub struct SequenceContract<'a> {
    /// The description the sequence was generated from.
    pub spec: &'a IrSpec,
    /// Its schema table.
    pub schemas: &'a suspect_runtime::Schemas,
    /// How much of each declared response applies.
    pub level: suspect_runtime::Conformance,
}

/// Runs one sequence: setup → exercise → teardown, threading each step's
/// response body into later steps' `$steps.<id>.response.body#/ptr`
/// parameter sources. Only the status class is judged; see
/// [`run_sequence_checked`] for the contract.
///
/// # Errors
/// Never: transport failures are reported as a failed [`SequenceResult`],
/// the same way a non-2xx response is.
pub async fn run_sequence(
    seq: &TestSequence,
    base: &str,
    http: &dyn crate::exec::HttpClient,
) -> SequenceResult {
    run_sequence_in(seq, base, http, None).await
}

/// [`run_sequence`], with every 2xx reply also held to the response its
/// operation declares: a step whose status, content type or body breaks
/// the contract fails the sequence with the violations.
///
/// # Errors
/// Never: transport failures are reported as a failed [`SequenceResult`].
pub async fn run_sequence_checked(
    seq: &TestSequence,
    base: &str,
    http: &dyn crate::exec::HttpClient,
    contract: SequenceContract<'_>,
) -> SequenceResult {
    run_sequence_in(seq, base, http, Some(contract)).await
}

async fn run_sequence_in(
    seq: &TestSequence,
    base: &str,
    http: &dyn crate::exec::HttpClient,
    contract: Option<SequenceContract<'_>>,
) -> SequenceResult {
    let target = seq
        .target
        .id
        .clone()
        .unwrap_or_else(|| format!("{} {}", seq.target.method, seq.target.path));
    let fail = |steps: usize, message: String| SequenceResult {
        target: target.clone(),
        steps,
        passed: false,
        message: Some(message),
    };
    // Collected response bodies by step id — what param sources read.
    let mut context: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for (index, step) in seq.steps.iter().enumerate() {
        // Path parameters resolve from earlier steps' bodies; parameters
        // the path does not carry (a creator's own id, say) are ignored.
        let mut path = step.op.path.clone();
        for (param, source) in &step.param_sources {
            let placeholder = format!("{{{param}}}");
            if !path.contains(&placeholder) {
                continue;
            }
            let Some(value) = resolve_reference(source, &context) else {
                return fail(
                    index,
                    format!(
                        "step `{}`: parameter {param} unresolvable: {source}",
                        step.id
                    ),
                );
            };
            path = path.replace(&placeholder, &value);
        }
        let request = crate::exec::HttpRequest {
            method: step.op.method.to_uppercase(),
            url: format!(
                "{}/{}",
                base.trim_end_matches('/'),
                path.trim_start_matches('/')
            ),
            headers: vec![
                ("accept".to_owned(), "application/json".to_owned()),
                ("content-type".to_owned(), "application/json".to_owned()),
            ],
            body: step
                .body
                .as_ref()
                .map(|b| serde_json::to_vec(b).unwrap_or_default())
                .unwrap_or_default()
                .into(),
        };
        match http.execute(request).await {
            Ok(response) => {
                if !(200..300).contains(&response.status) {
                    return fail(
                        index,
                        format!(
                            "step `{}`: {} {} returned {}",
                            step.id, step.op.method, step.op.path, response.status
                        ),
                    );
                }
                if let Some(violations) =
                    contract.and_then(|contract| contract_violations(contract, step, &response))
                {
                    return fail(
                        index,
                        format!(
                            "step `{}`: {} {} returned {} but breaks its contract: {violations}",
                            step.id, step.op.method, step.op.path, response.status
                        ),
                    );
                }
                let body: serde_json::Value =
                    serde_json::from_slice(&response.body).unwrap_or(serde_json::Value::Null);
                context.insert(step.id.clone(), body);
            }
            Err(e) => {
                return fail(
                    index,
                    format!(
                        "step `{}`: {} {}: {e}",
                        step.id, step.op.method, step.op.path
                    ),
                );
            }
        }
    }
    SequenceResult {
        target,
        steps: seq.steps.len(),
        passed: true,
        message: None,
    }
}

/// The violations of one step's reply against the response its operation
/// declares, joined for a message; `None` when it conforms.
fn contract_violations(
    contract: SequenceContract<'_>,
    step: &TestStep,
    response: &crate::exec::HttpResponse,
) -> Option<String> {
    let method = Method::from_key(&step.op.method.to_ascii_lowercase())?;
    let operation = contract
        .spec
        .operation(suspect_ir::OpSelector::MethodPath(method, &step.op.path))?;
    let check = suspect_runtime::check_response(
        &operation.responses,
        contract.schemas,
        suspect_runtime::Reply {
            method: method.as_str(),
            status: response.status,
            headers: &response.headers,
            body: &response.body,
        },
        contract.level,
    );
    (!check.conforms()).then(|| {
        check
            .violations
            .iter()
            .map(suspect_runtime::Violation::located)
            .collect::<Vec<_>>()
            .join("; ")
    })
}

/// Resolves one `$steps.<id>.response.body#/pointer` reference against the
/// collected response bodies.
fn resolve_reference(
    source: &str,
    context: &BTreeMap<String, serde_json::Value>,
) -> Option<String> {
    let rest = source.strip_prefix("$steps.")?;
    let (id, pointer) = rest.split_once(".response.body")?;
    let body = context.get(id)?;
    let mut value = body;
    for segment in pointer
        .strip_prefix('#')?
        .split('/')
        .filter(|s| !s.is_empty())
    {
        let segment = segment.replace("~1", "/").replace("~0", "~");
        value = match value {
            serde_json::Value::Object(map) => map.get(&segment)?,
            serde_json::Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    })
}
