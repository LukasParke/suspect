//! Compiles Arazzo documents into executable [`Plan`]s.
//!
//! [`compile_plan`] walks the Arazzo workflows/steps, resolves each step's
//! target operation to a canonical [`OpKey`] (via IR snapshots of the
//! `sourceDescriptions` documents), parses parameters into runtime
//! expressions, and converts `successCriteria` condition strings into the
//! pragmatic [`CriterionPlan`] model. Steps without explicit success
//! criteria default to `StatusInRange(2, 2)` per the Arazzo recommendation.

use std::fmt;
use std::sync::Arc;

use suspect_arazzo::{ArazzoDoc, ParameterView, StepView};
use suspect_ir::{IrSpec, Method, OpSelector, ParamIn};
use suspect_low::{LowDoc, NodeRef, ValueKind};
use suspect_ref::Workspace;
use suspect_rex::{Rex, parse_rex};

/// Failure while compiling an Arazzo document into a [`Plan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError(
    /// Human-readable description of what could not be compiled.
    pub String,
);

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CompileError {}

/// Canonical address of a tested operation: HTTP method plus OAS path template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpKey {
    /// HTTP method of the operation.
    pub method: Method,
    /// Path template of the operation (e.g. `/pets/{petId}`).
    pub path: String,
}

/// A compiled Arazzo document: every workflow ready for execution.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// Compiled workflows, in document order.
    pub workflows: Vec<WfPlan>,
}

/// One compiled workflow.
#[derive(Debug, Clone, PartialEq)]
pub struct WfPlan {
    /// The Arazzo `workflowId`.
    pub workflow_id: String,
    /// Workflow-level inputs (plain-valued workflow parameters), read by
    /// `$inputs...` expressions during execution.
    pub inputs: serde_json::Map<String, serde_json::Value>,
    /// Compiled steps, in document order; executed sequentially.
    pub steps: Vec<StepPlan>,
}

/// One compiled step parameter (`name`, `in`, `value`).
#[derive(Debug, Clone, PartialEq)]
pub struct StepParam {
    /// Where the parameter is placed on the request.
    pub location: ParamIn,
    /// Parameter name; also the `{name}` path template token for
    /// [`ParamIn::Path`].
    pub name: String,
    /// Value expression evaluated at execution time.
    pub value: Rex,
}

/// One compiled step.
#[derive(Debug, Clone, PartialEq)]
pub struct StepPlan {
    /// The Arazzo `stepId`; unique within its workflow.
    pub step_id: String,
    /// Resolved target operation.
    pub operation: OpKey,
    /// Step parameters in document order.
    pub parameters: Vec<StepParam>,
    /// Request body expression, when the step declares `requestBody`.
    pub request_body: Option<Rex>,
    /// Success criteria; all must pass for the step to pass. Empty at
    /// compile time means "defaults to 2xx" and is materialized as
    /// [`CriterionPlan::StatusInRange(2, 2)`](CriterionPlan::StatusInRange).
    pub success: Vec<CriterionPlan>,
    /// Step outputs `(name, expression)` captured after a passing step.
    pub outputs: Vec<(String, Rex)>,
    /// Response-body JSON pointers referenced by this step's criteria, so
    /// executors know which parts of parsed bodies are relevant.
    pub body_pointers: Vec<String>,
}

/// Pragmatic success-criterion model compiled from Arazzo condition strings.
#[derive(Debug, Clone, PartialEq)]
pub enum CriterionPlan {
    /// `$statusCode` falls inside an inclusive range of hundreds classes,
    /// e.g. `(2, 2)` accepts every 2xx response.
    StatusInRange(
        /// Low hundreds digit, inclusive.
        u8,
        /// High hundreds digit, inclusive.
        u8,
    ),
    /// Equality check. With `pointer: None` compares against the status
    /// code; with a pointer compares the JSON value at that RFC 6901
    /// pointer inside the parsed response body.
    Equals {
        /// Body pointer (`"/a/b"`), or `None` for the status code.
        pointer: Option<String>,
        /// Expected value.
        expected: serde_json::Value,
    },
    /// The value at a body pointer exists and is not `null`.
    NotNull {
        /// RFC 6901 body pointer (`"/a/b"`).
        pointer: String,
    },
    /// The raw response body text matches a regular expression.
    Regex {
        /// Regular-expression source.
        pattern: String,
    },
    /// An existence-style check over a dot-notation body fragment kept raw
    /// from the condition (e.g. `"pets[0].name"`); passes when the fragment
    /// resolves to a non-null value in the parsed response body.
    JsonPathTrue {
        /// Raw fragment as written after `#/`.
        expr: String,
    },
}

impl CriterionPlan {
    /// Short human-readable rendering used in test events and reports.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::StatusInRange(lo, hi) if lo == hi => format!("{lo}xx"),
            Self::StatusInRange(lo, hi) => format!("{lo}-{hi}xx"),
            Self::Equals {
                pointer: None,
                expected,
            } => format!("statusCode == {expected}"),
            Self::Equals {
                pointer: Some(p),
                expected,
            } => format!("body{p} == {expected}"),
            Self::NotNull { pointer } => format!("body{pointer} != null"),
            Self::Regex { pattern } => format!("body =~ /{pattern}/"),
            Self::JsonPathTrue { expr } => format!("body#/{expr} exists"),
        }
    }

    /// Records this criterion's body-pointer target, if any.
    fn note_pointer(&self, out: &mut Vec<String>) {
        match self {
            Self::Equals {
                pointer: Some(p), ..
            } => out.push(p.clone()),
            Self::NotNull { pointer } => out.push(pointer.clone()),
            _ => {}
        }
    }
}

/// Compiles an Arazzo document into a [`Plan`], resolving operations through
/// IR snapshots built from the workspace documents named by
/// `sourceDescriptions`.
///
/// # Errors
/// Returns a [`CompileError`] when a workflow/step lacks its id, a source
/// description matches no loaded document, or an operation reference cannot
/// be resolved or a criterion/parameter expression cannot be parsed.
pub fn compile_plan(arazzo: &LowDoc, ws: &Arc<Workspace>) -> Result<Plan, CompileError> {
    let doc = ArazzoDoc::new(arazzo);
    let sources = SourceIndex::load(&doc, ws)?;

    let mut workflows = Vec::with_capacity(doc.workflows().len());
    for wf in doc.workflows() {
        if wf.workflow_id.is_empty() {
            return Err(CompileError("workflow missing workflowId".to_owned()));
        }
        let inputs = workflow_inputs(wf);
        let mut steps = Vec::with_capacity(wf.steps().len());
        for step in wf.steps() {
            steps.push(compile_step(step, &sources)?);
        }
        workflows.push(WfPlan {
            workflow_id: wf.workflow_id.to_owned(),
            inputs,
            steps,
        });
    }
    Ok(Plan { workflows })
}

/// Maps `sourceDescriptions` names to IR snapshots of their documents.
struct SourceIndex {
    specs: Vec<(String, IrSpec)>,
}

impl SourceIndex {
    fn load(doc: &ArazzoDoc<'_>, ws: &Arc<Workspace>) -> Result<Self, CompileError> {
        let uris = ws.uris();
        let mut specs = Vec::new();
        for src in doc.source_descriptions() {
            // Overlays and nested Arazzo descriptions carry no operations.
            if !matches!(src.kind, suspect_arazzo::SourceType::OpenApi) {
                continue;
            }
            let wanted = file_name_of(src.url);
            let uri = uris
                .iter()
                .find(|u| file_name_of(u.as_str()) == wanted)
                .ok_or_else(|| {
                    CompileError(format!(
                        "source description '{}' ({}) matches no loaded document",
                        src.name, src.url
                    ))
                })?;
            let ir = IrSpec::from_workspace(ws, uri)
                .map_err(|e| CompileError(format!("source '{}': {e}", src.name)))?;
            specs.push((src.name.to_owned(), ir));
        }
        Ok(Self { specs })
    }

    fn spec_for(&self, name: Option<&str>) -> Option<&IrSpec> {
        match name {
            Some(n) => self
                .specs
                .iter()
                .find(|(key, _)| key == n)
                .map(|(_, ir)| ir),
            None => self.specs.first().map(|(_, ir)| ir),
        }
    }
}

/// Last path segment of a URI-ish string, ignoring any query/fragment.
fn file_name_of(url: &str) -> &str {
    let base = url.split(['?', '#']).next().unwrap_or(url);
    base.rsplit(['/', '\\']).next().unwrap_or(base)
}

/// Workflow-level plain-valued parameters become the workflow's input map.
fn workflow_inputs(
    wf: &suspect_arazzo::WorkflowView<'_>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut inputs = serde_json::Map::new();
    for p in wf.parameters() {
        let Some(name) = p.name() else { continue };
        let Some(node) = p.value() else { continue };
        // Runtime-expression values cannot be static inputs; they are bound
        // by the caller at execution time instead.
        if node.as_str().is_some_and(|s| s.starts_with('$')) {
            continue;
        }
        inputs.insert(name.to_owned(), materialize_json(node));
    }
    inputs
}

fn compile_step(step: &StepView<'_>, sources: &SourceIndex) -> Result<StepPlan, CompileError> {
    if step.step_id.is_empty() {
        return Err(CompileError("step missing stepId".to_owned()));
    }
    let step_id = step.step_id.to_owned();
    let operation = resolve_operation(step, sources)?;
    let mut parameters = Vec::new();
    for p in step.parameters() {
        parameters.push(compile_param(&step_id, p)?);
    }
    let request_body = match step.request_body() {
        Some(node) => Some(rex_of_node(node)?),
        None => None,
    };

    let mut success = Vec::new();
    let mut body_pointers = Vec::new();
    for c in step.success_criteria() {
        let cond = c.condition().ok_or_else(|| {
            CompileError(format!("step '{step_id}': criterion missing condition"))
        })?;
        let crit = parse_condition(cond, c.criterion_type())?;
        crit.note_pointer(&mut body_pointers);
        success.push(crit);
    }
    if success.is_empty() {
        // Spec-recommended default: accept any 2xx.
        success.push(CriterionPlan::StatusInRange(2, 2));
    }

    let mut outputs = Vec::new();
    for (name, node) in step.outputs() {
        outputs.push((name.to_owned(), rex_of_node(node)?));
    }

    Ok(StepPlan {
        step_id,
        operation,
        parameters,
        request_body,
        success,
        outputs,
        body_pointers,
    })
}

fn compile_param(step_id: &str, p: ParameterView<'_>) -> Result<StepParam, CompileError> {
    let name = p
        .name()
        .ok_or_else(|| CompileError(format!("step '{step_id}': parameter missing name")))?
        .to_owned();
    let location = match p.location() {
        Some("path") => ParamIn::Path,
        Some("query") => ParamIn::Query,
        Some("header") => ParamIn::Header,
        Some("cookie") => ParamIn::Cookie,
        other => {
            return Err(CompileError(format!(
                "step '{step_id}', parameter '{name}': unsupported 'in' {other:?}"
            )));
        }
    };
    let node = p.value().ok_or_else(|| {
        CompileError(format!(
            "step '{step_id}', parameter '{name}': missing value"
        ))
    })?;
    let value = rex_of_node(node)?;
    Ok(StepParam {
        location,
        name,
        value,
    })
}

/// Resolves a step's target to a canonical [`OpKey`].
///
/// Preference order mirrors `operationId` first, then `operationPath`
/// (`$sourceDescriptions.<name>#/paths/~1pets/get`, `<name>#/users/{userId}/delete`,
/// or a literal `GET /pets`).
fn resolve_operation(step: &StepView<'_>, sources: &SourceIndex) -> Result<OpKey, CompileError> {
    if let Some(id) = step.operation_id() {
        for (_, ir) in &sources.specs {
            if let Some(op) = ir.operation(OpSelector::Id(id)) {
                return Ok(OpKey {
                    method: op.method,
                    path: op.path.clone(),
                });
            }
        }
        return Err(CompileError(format!(
            "step '{}': operationId '{}' not found in any source description",
            step.step_id, id
        )));
    }
    let Some(path_expr) = step.operation_path() else {
        return Err(CompileError(format!(
            "step '{}': needs operationId or operationPath",
            step.step_id
        )));
    };
    let (name, method, raw_path) = parse_operation_path(path_expr).ok_or_else(|| {
        CompileError(format!(
            "step '{}': unparseable operationPath '{path_expr}'",
            step.step_id
        ))
    })?;

    // Canonicalize against the referenced spec's IR when possible so the key
    // uses the exact path template from the OpenAPI document.
    let canonical = sources
        .spec_for(name)
        .and_then(|ir| ir.operation(OpSelector::MethodPath(method, &raw_path)))
        .map(|op| op.path.clone());
    Ok(OpKey {
        method,
        path: canonical.unwrap_or(raw_path),
    })
}

/// Parses an `operationPath` spelling into
/// `(source-name, Method, raw-path)`, following the accepted shapes:
/// `$sourceDescriptions.<name>#/paths/~1pets/get`, `<name>#/users/{userId}/delete`,
/// and literal `GET /pets`.
fn parse_operation_path(path: &str) -> Option<(Option<&str>, Method, String)> {
    if let Some(rest) = path.strip_prefix("$sourceDescriptions.") {
        let (name, frag) = rest.split_once('#')?;
        let name = name.strip_suffix('.').unwrap_or(name);
        let (m, p) = fragment_to_method_path(frag)?;
        return Some((Some(name), m, p));
    }
    if let Some((left, frag)) = path.split_once('#') {
        let name = if left.is_empty() { None } else { Some(left) };
        let (m, p) = fragment_to_method_path(frag)?;
        return Some((name, m, p));
    }
    // Literal "METHOD /path" form.
    let mut parts = path.split_whitespace();
    let method_word = parts.next()?;
    let method = Method::from_key(&method_word.to_lowercase())?;
    let target = parts.next()?;
    Some((None, method, format!("/{}", target.trim_matches('/'))))
}

/// Parses the fragment half of an `operationPath` into `(Method, path)`.
fn fragment_to_method_path(frag: &str) -> Option<(Method, String)> {
    let trimmed = frag.trim_start_matches('/');
    let tokens: Vec<String> = trimmed
        .split('/')
        .filter(|t| !t.is_empty())
        .map(|t| t.replace("~1", "/").replace("~0", "~"))
        .collect();
    if tokens.is_empty() {
        return None;
    }
    if tokens[0] == "paths" && tokens.len() >= 3 {
        // Canonical `/paths/~1pets/get`: token 1 already decodes to "/pets".
        let m = Method::from_key(&tokens[2].to_lowercase())?;
        let mut path = tokens[1].clone();
        if !path.starts_with('/') {
            path.insert(0, '/');
        }
        return Some((m, path));
    }
    let m = Method::from_key(&tokens[tokens.len() - 1].to_lowercase())?;
    let segments = &tokens[..tokens.len() - 1];
    if segments.is_empty() {
        return None;
    }
    // Segments may already carry a decoded leading slash; normalize.
    Some((
        m,
        format!("/{}", segments.join("/").trim_start_matches('/')),
    ))
}

fn rex_of_node(node: NodeRef<'_>) -> Result<Rex, CompileError> {
    match node.kind() {
        // Structured nodes serialize to JSON text verbatim.
        ValueKind::Object | ValueKind::Array => Ok(Rex::Text(materialize_json(node).to_string())),
        _ => match node.as_str() {
            Some(s) => parse_rex(s).map_err(|e| CompileError(e.to_string())),
            None => Ok(Rex::Text(String::new())),
        },
    }
}

/// Materializes any node into owned JSON, dispatching on the node's kind so
/// mappings/sequences are never mistaken for their raw scalar text.
fn materialize_json(node: NodeRef<'_>) -> serde_json::Value {
    match node.kind() {
        ValueKind::Object => {
            let mut map = serde_json::Map::new();
            for entry in node.entries() {
                if let Some(value) = entry.value {
                    map.insert(entry.key.to_owned(), materialize_json(value));
                }
            }
            serde_json::Value::Object(map)
        }
        ValueKind::Array => {
            serde_json::Value::Array(node.items().into_iter().map(materialize_json).collect())
        }
        ValueKind::Bool => serde_json::Value::Bool(node.as_bool().unwrap_or(false)),
        ValueKind::Int => serde_json::Value::from(node.as_i64().unwrap_or_default()),
        ValueKind::Float => serde_json::Value::from(node.as_f64().unwrap_or_default()),
        ValueKind::Str => serde_json::Value::String(node.as_str().unwrap_or_default().to_owned()),
        ValueKind::Null => serde_json::Value::Null,
    }
}

/// Comparison operator found in a criterion condition.
enum Op {
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `/=` (regular-expression match)
    Re,
}

/// LHS target of a criterion condition.
enum Target {
    /// `{$statusCode}`
    Status,
    /// `$response.body#/...` with the decoded RFC 6901 pointer.
    Body(String),
}

/// Compiles one success-criterion condition string.
///
/// Supported shapes:
/// - `'{$statusCode} == 200'` — status equality.
/// - `'{$statusCode} /= /^2../'` — class match when the pattern has the
///   `^N..` shape, otherwise a regex criterion.
/// - `'$response.body#/x == literal'` — body-pointer equality (JSON literal
///   or quoted/plain string).
/// - `'$response.body#/x != null'` — not-null existence check.
/// - `'$response.body#/x'` without operator (or `type: jsonpath`) —
///   existence check via [`CriterionPlan::JsonPathTrue`], keeping the raw
///   fragment after `#/`.
fn parse_condition(condition: &str, ty: Option<&str>) -> Result<CriterionPlan, CompileError> {
    let (lhs, op, rhs) = split_op(condition.trim())
        .ok_or_else(|| CompileError(format!("unsupported success criterion: '{condition}'")))?;

    let target = classify_target(lhs).ok_or_else(|| {
        CompileError(format!(
            "unsupported success criterion target: '{condition}'"
        ))
    })?;

    if ty.is_some_and(|t| t.eq_ignore_ascii_case("jsonpath"))
        && let Target::Body(_) = target
    {
        // Keep the raw fragment; existence semantics apply.
        let frag = fragment_after_hash(lhs);
        return Ok(CriterionPlan::JsonPathTrue {
            expr: frag.unwrap_or_default(),
        });
    }

    match (target, op) {
        (Target::Status, Op::Eq) => Ok(CriterionPlan::Equals {
            pointer: None,
            expected: literal_value(rhs),
        }),
        (Target::Status, Op::Re) => Ok(status_range_or_regex(regex_pattern(rhs))),
        (Target::Status, Op::Ne) => Err(CompileError(format!(
            "unsupported success criterion: '{condition}'"
        ))),
        (Target::Body(pointer), Op::Eq) => Ok(CriterionPlan::Equals {
            pointer: Some(pointer),
            expected: literal_value(rhs),
        }),
        (Target::Body(pointer), Op::Ne) if rhs.trim() == "null" => {
            Ok(CriterionPlan::NotNull { pointer })
        }
        (Target::Body(_), Op::Ne) => Err(CompileError(format!(
            "unsupported success criterion: '{condition}'"
        ))),
        (Target::Body(_), Op::Re) => Ok(CriterionPlan::Regex {
            pattern: regex_pattern(rhs),
        }),
    }
}

/// Splits `lhs op rhs` at the earliest top-level operator occurrence.
fn split_op(cond: &str) -> Option<(&str, Op, &str)> {
    let mut best: Option<(usize, Op)> = None;
    for (needle, op) in [("==", Op::Eq), ("!=", Op::Ne), ("/=", Op::Re)] {
        if let Some(idx) = cond.find(needle)
            && best.as_ref().is_none_or(|(i, _)| idx < *i)
        {
            best = Some((idx, op));
        }
    }
    let (idx, op) = best?;
    Some((cond[..idx].trim(), op, cond[idx + 2..].trim()))
}

/// Classifies a condition LHS as status or body-pointer target.
fn classify_target(lhs: &str) -> Option<Target> {
    let lhs = lhs.trim();
    if lhs.contains("$statusCode") {
        return Some(Target::Status);
    }
    if lhs.contains("$response.body") {
        let frag = fragment_after_hash(lhs).unwrap_or_default();
        return Some(Target::Body(fragment_to_pointer(&frag)));
    }
    None
}

/// Raw text after the first `#` of a `$response.body#/frag` expression.
fn fragment_after_hash(expr: &str) -> Option<String> {
    let (_, frag) = expr.split_once('#')?;
    Some(frag.trim().to_owned())
}

/// Converts a raw fragment (`pets[0].name`, `a/b`, empty) into an RFC 6901
/// pointer (`/pets/0/name`). Handles `~1`/`~0` escapes and `[n]` index
/// suffixes.
#[must_use]
pub(crate) fn fragment_to_pointer(fragment: &str) -> String {
    let normalized = fragment.trim().trim_start_matches('/');
    let mut tokens: Vec<String> = Vec::new();
    for seg in normalized.split('/') {
        let mut current = seg.replace("~1", "/").replace("~0", "~");
        // Split trailing `[i]` index suffixes into their own tokens.
        while let Some(open) = current.find('[') {
            let close = current[open..].find(']').map(|p| open + p);
            let Some(close) = close else { break };
            let base = current[..open].to_owned();
            if !base.is_empty() {
                tokens.push(base);
            }
            tokens.push(current[open + 1..close].to_owned());
            current = current[close + 1..].to_owned();
        }
        if !current.is_empty() {
            tokens.push(current);
        }
    }
    if tokens.is_empty() {
        String::new()
    } else {
        format!("/{}", tokens.join("/"))
    }
}

/// Extracts the pattern body from an `/pattern/flags` regex literal.
fn regex_pattern(rhs: &str) -> String {
    let rhs = rhs.trim();
    let body = rhs
        .strip_prefix('/')
        .and_then(|r| r.rsplit_once('/'))
        .map(|(body, _flags)| body)
        .unwrap_or(rhs);
    body.to_owned()
}

/// `^N..`-shaped patterns collapse to a status-class range; anything else is
/// kept as a regex criterion.
fn status_range_or_regex(pattern: String) -> CriterionPlan {
    let bytes = pattern.as_bytes();
    if bytes.len() == 4
        && bytes[0] == b'^'
        && bytes[3] == b'.'
        && bytes[1].is_ascii_digit()
        && bytes[2] == b'.'
    {
        let d = bytes[1] - b'0';
        return CriterionPlan::StatusInRange(d, d);
    }
    CriterionPlan::Regex { pattern }
}

/// Parses a condition RHS literal: JSON when it parses, otherwise a dequoted
/// plain string.
fn literal_value(rhs: &str) -> serde_json::Value {
    let rhs = rhs.trim();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(rhs) {
        return v;
    }
    let inner = rhs
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .or_else(|| rhs.strip_prefix('"').and_then(|s| s.strip_suffix('"')));
    serde_json::Value::String(inner.unwrap_or(rhs).to_owned())
}
