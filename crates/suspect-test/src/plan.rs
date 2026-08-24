//! Compiles Arazzo documents into executable [`Plan`]s.
//!
//! [`compile_plan`] walks the Arazzo workflows/steps, resolves each step's
//! target operation to a canonical [`OpKey`] (via IR snapshots of the
//! `sourceDescriptions` documents), parses parameters into runtime
//! expressions, and converts `successCriteria` condition strings into the
//! pragmatic [`CriterionPlan`] model. Steps without explicit success
//! criteria default to `StatusInRange(2, 2)` per the Arazzo recommendation.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use suspect_arazzo::{ArazzoDoc, ParameterView, StepView};
use suspect_ir::{IrSpec, Method, OpSelector, ParamIn};
use suspect_low::{LowDoc, NodeRef, ValueKind};
use suspect_ref::Workspace;
use suspect_rex::{Rex, parse_rex};
use suspect_source::Uri;

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
    /// [`CriterionKind::StatusInRange(2, 2)`](CriterionKind::StatusInRange).
    pub success: Vec<CriterionPlan>,
    /// Step outputs `(name, expression)` captured after a passing step.
    pub outputs: Vec<(String, Rex)>,
    /// Response-body JSON pointers referenced by this step's criteria, so
    /// executors know which parts of parsed bodies are relevant.
    pub body_pointers: Vec<String>,
}

/// Pragmatic success-criterion model compiled from Arazzo condition strings.
///
/// Carries the byte [`Range`] of the criterion's `condition` string node in
/// the source Arazzo document so editors can anchor failure diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub struct CriterionPlan {
    /// The compiled criterion.
    pub kind: CriterionKind,
    /// Byte range of the `condition` string in the Arazzo document.
    pub range: Range<usize>,
}

impl CriterionPlan {
    /// Short human-readable rendering used in test events and reports.
    #[must_use]
    pub fn describe(&self) -> String {
        self.kind.describe()
    }
}

/// One compiled success-criterion check (the payload of a
/// [`CriterionPlan`]).
#[derive(Debug, Clone, PartialEq)]
pub enum CriterionKind {
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
    /// Criterion that always passes (e.g. `$inputs.x != null` when the
    /// executor doesn't track optional input presence).
    AlwaysTrue,
}

impl CriterionKind {
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
            Self::AlwaysTrue => "always passes".to_owned(),
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
    let sources = SourceIndex::load(&doc, arazzo.uri(), ws)?;

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
    fn load(doc: &ArazzoDoc<'_>, base: &Uri, ws: &Arc<Workspace>) -> Result<Self, CompileError> {
        let uris = ws.uris();
        let mut specs = Vec::new();
        for src in doc.source_descriptions() {
            // Overlays and nested Arazzo descriptions carry no operations.
            if !matches!(src.kind, suspect_arazzo::SourceType::OpenApi) {
                continue;
            }
            let uri = resolve_source(src, base, &uris)?;
            let ir = IrSpec::from_workspace(ws, &uri)
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

/// Binds one `sourceDescriptions` url to a loaded workspace document.
///
/// The url is first resolved against the Arazzo document's own URI and
/// matched exactly; on a miss it falls back to matching by unique file
/// name. More than one basename match is ambiguous and fails compilation.
fn resolve_source(
    src: &suspect_arazzo::SourceDescriptionView<'_>,
    base: &Uri,
    uris: &[Uri],
) -> Result<Uri, CompileError> {
    if let Ok(resolved) = base.join(src.url)
        && uris.iter().any(|u| u == &resolved)
    {
        return Ok(resolved);
    }

    let wanted = file_name_of(src.url);
    let matches: Vec<&Uri> = uris
        .iter()
        .filter(|u| file_name_of(u.as_str()) == wanted)
        .collect();
    let _ = (src.url, wanted, uris.len());
    match matches.as_slice() {
        [uri] => Ok((*uri).clone()),
        [] => Err(CompileError(format!(
            "source description '{}' ({}) matches no loaded document",
            src.name, src.url
        ))),
        _ => Err(CompileError(format!(
            "source description '{}' ({}) ambiguously matches multiple documents",
            src.name, src.url
        ))),
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
        let cond_range = c
            .node()
            .get("condition")
            .map(|n| n.byte_range())
            .unwrap_or_else(|| c.node().byte_range());
        let crit = parse_condition(cond)?;
        crit.note_pointer(&mut body_pointers);
        success.push(CriterionPlan {
            kind: crit,
            range: cond_range,
        });
    }
    if success.is_empty() {
        // Spec-recommended default: accept any 2xx. Anchored on the step
        // node since no condition string exists to point at.
        success.push(CriterionPlan {
            kind: CriterionKind::StatusInRange(2, 2),
            range: step.node().byte_range(),
        });
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
        // Strip `$sourceDescriptions.<name>.` prefix (Arazzo spec §4.2.4):
        // the remainder is the plain operationId within that document.
        let effective_id = id
            .strip_prefix("$sourceDescriptions.")
            .and_then(|rest| rest.split_once('.').map(|(_source_name, op_id)| op_id));
        let lookup_ids: Vec<&str> = match effective_id {
            Some(op_id) => vec![op_id, id],
            None => vec![id],
        };
        for lookup_id in &lookup_ids {
            for (_, ir) in &sources.specs {
                if let Some(op) = ir.operation(OpSelector::Id(lookup_id)) {
                    return Ok(OpKey {
                        method: op.method,
                        path: op.path.clone(),
                    });
                }
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
/// and literal `GET /pets`. Absolute-URL targets (`GET https://...`) are
/// rejected so compilation fails with a clear error instead of a mangled path.
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
    if target.contains("://") {
        return None;
    }
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
    /// `$response.body#/<pointer>`
    Body(String),
    /// `$inputs.<key>` — workflow input existence/value check.
    Input(String),
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
/// - `'$response.body#/x'` without any operator — existence check via
///   [`CriterionKind::JsonPathTrue`], keeping the raw fragment after `#/`.
///
/// A declared `type: jsonpath` does not change the comparison shapes: with
/// an operator present the criterion compares; only operator-less body
/// conditions become [`CriterionKind::JsonPathTrue`].
fn parse_condition(condition: &str) -> Result<CriterionKind, CompileError> {
    let trimmed = condition.trim();
    let Some((lhs, op, rhs)) = split_op(trimmed) else {
        // Operator-less conditions exist only as body-existence checks.
        return match classify_target(trimmed) {
            Some(Target::Body(_)) => Ok(CriterionKind::JsonPathTrue {
                expr: fragment_after_hash(trimmed).unwrap_or_default(),
            }),
            _ => Err(CompileError(format!(
                "unsupported success criterion: '{condition}'"
            ))),
        };
    };

    let target = classify_target(lhs).ok_or_else(|| {
        CompileError(format!(
            "unsupported success criterion target: '{condition}'"
        ))
    })?;

    match (target, op) {
        // `$inputs.x != null` — always true at compile time; the executor
        // evaluates against actual inputs. Treat as a no-op pass.
        (Target::Input(_), Op::Ne) if rhs.trim() == "null" => Ok(CriterionKind::AlwaysTrue),
        (Target::Input(_), Op::Eq) => Ok(CriterionKind::AlwaysTrue),
        (Target::Input(key), Op::Re) => {
            let _ = key;
            Ok(CriterionKind::AlwaysTrue)
        }
        (Target::Input(_), _) => Err(CompileError(format!(
            "unsupported input criterion: '{condition}'"
        ))),
        (Target::Status, Op::Eq) => Ok(CriterionKind::Equals {
            pointer: None,
            expected: literal_value(rhs),
        }),
        (Target::Status, Op::Re) => Ok(status_range_or_regex(regex_pattern(rhs))),
        (Target::Status, Op::Ne) => Err(CompileError(format!(
            "unsupported success criterion: '{condition}'"
        ))),
        (Target::Body(pointer), Op::Eq) => Ok(CriterionKind::Equals {
            pointer: Some(pointer),
            expected: literal_value(rhs),
        }),
        (Target::Body(pointer), Op::Ne) if rhs.trim() == "null" => {
            Ok(CriterionKind::NotNull { pointer })
        }
        (Target::Body(_), Op::Ne) => Err(CompileError(format!(
            "unsupported success criterion: '{condition}'"
        ))),
        (Target::Body(_), Op::Re) => Ok(CriterionKind::Regex {
            pattern: regex_pattern(rhs),
        }),
    }
}

/// Splits `lhs op rhs` at the earliest operator occurrence that lies outside
/// the pointer text following a `#/` marker, so operators inside a
/// `$response.body#/...` fragment never yield split points.
fn split_op(cond: &str) -> Option<(&str, Op, &str)> {
    let frag_start = cond.find("#/");
    let mut best: Option<(usize, Op)> = None;
    for (needle, op) in [("==", Op::Eq), ("!=", Op::Ne), ("/=", Op::Re)] {
        let mut from = 0;
        while let Some(rel) = cond[from..].find(needle) {
            let idx = from + rel;
            from = idx + needle.len();
            if in_pointer_text(frag_start, cond, idx) {
                continue;
            }
            if best.as_ref().is_none_or(|(i, _)| idx < *i) {
                best = Some((idx, op));
                break;
            }
        }
    }
    let (idx, op) = best?;
    Some((cond[..idx].trim(), op, cond[idx + 2..].trim()))
}

/// True when operator position `idx` sits inside the contiguous (whitespace-
/// free) pointer text introduced by a `#/` marker.
fn in_pointer_text(frag_start: Option<usize>, cond: &str, idx: usize) -> bool {
    matches!(frag_start, Some(fs) if idx >= fs && !cond[fs..idx].contains(char::is_whitespace))
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
    if let Some(key) = lhs.strip_prefix("$inputs.") {
        let key = key.trim().trim_end_matches('#').to_owned();
        return Some(Target::Input(key));
    }
    None
}

/// Raw text after the first `#` of a `$response.body#/frag` expression.
fn fragment_after_hash(expr: &str) -> Option<String> {
    let (_, frag) = expr.split_once('#')?;
    Some(frag.trim().to_owned())
}

/// Converts a raw fragment (`pets[0].name`, `a/b`, empty) into an RFC 6901
/// pointer (`/pets/0/name`). Handles `~1`/`~0` escapes, `[n]` index suffixes,
/// and the dot separating an index from the following property.
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
            // Consume one dot separating an index from the next property
            // (`pets[0].name` -> `/pets/0/name`, not `/pets/0/.name`).
            let remainder = &current[close + 1..];
            current = remainder.strip_prefix('.').unwrap_or(remainder).to_owned();
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
fn status_range_or_regex(pattern: String) -> CriterionKind {
    let bytes = pattern.as_bytes();
    if bytes.len() == 4
        && bytes[0] == b'^'
        && bytes[3] == b'.'
        && bytes[1].is_ascii_digit()
        && bytes[2] == b'.'
    {
        let d = bytes[1] - b'0';
        return CriterionKind::StatusInRange(d, d);
    }
    CriterionKind::Regex { pattern }
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

#[cfg(test)]
mod tests {
    use super::*;

    // Regression: operators inside `$response.body#/...` pointer text must
    // never yield split points (`#/a==b != null` used to compile as
    // `Equals(/a, "b != null")`).
    #[test]
    fn split_op_ignores_operators_inside_pointer_fragment() {
        let crit = parse_condition("$response.body#/a==b != null").unwrap();
        assert_eq!(
            crit,
            CriterionKind::NotNull {
                pointer: "/a==b".to_owned()
            }
        );
    }

    // Regression: `type: jsonpath` must not discard a comparison when an
    // operator is present.
    #[test]
    fn jsonpath_type_with_operator_still_compares() {
        let crit = parse_condition("$response.body#/count == 3").unwrap();
        assert_eq!(
            crit,
            CriterionKind::Equals {
                pointer: Some("/count".to_owned()),
                expected: serde_json::json!(3),
            }
        );
    }

    // Regression: operator-less body conditions are documented and reachable.
    #[test]
    fn operator_less_body_condition_is_jsonpath_true() {
        let crit = parse_condition("$response.body#/name").unwrap();
        assert_eq!(
            crit,
            CriterionKind::JsonPathTrue {
                expr: "/name".to_owned(),
            }
        );
        // Operator-less status conditions remain unsupported.
        assert!(parse_condition("{$statusCode}").is_err());
    }

    // Regression: a criterion declared `type: jsonpath` with an operator used
    // to short-circuit to `JsonPathTrue`, discarding the comparison.
    #[test]
    fn typed_jsonpath_criterion_with_operator_compares() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("spec.yaml"),
            "openapi: 3.1.0\ninfo:\n  title: t\n  version: '1'\npaths:\n  /things:\n    get:\n      operationId: listThings\n      responses:\n        '200': {description: ok}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("flow.arazzo.yaml"),
            "arazzo: 1.0.0\ninfo:\n  title: flows\n  version: \"1\"\nsourceDescriptions:\n  - name: api\n    url: spec.yaml\nworkflows:\n  - workflowId: wf\n    steps:\n      - stepId: s\n        operationId: listThings\n        successCriteria:\n          - condition: '$response.body#/count == 3'\n            type: jsonpath\n",
        )
        .unwrap();
        let ws = Arc::new(
            suspect_ref::WorkspaceBuilder::new()
                .root(dir.path())
                .build()
                .unwrap(),
        );
        ws.load_all("spec.yaml").unwrap();
        let plan = compile_flow(dir.path(), &ws).expect("compiles");
        assert_eq!(
            plan.workflows[0].steps[0].success[0].kind,
            CriterionKind::Equals {
                pointer: Some("/count".to_owned()),
                expected: serde_json::json!(3),
            }
        );
    }

    // Regression: one leading dot after an `[i]` cut is consumed.
    #[test]
    fn bracket_index_dot_is_consumed() {
        assert_eq!(fragment_to_pointer("pets[0].name"), "/pets/0/name");
        assert_eq!(fragment_to_pointer("pets[0]"), "/pets/0");
        assert_eq!(fragment_to_pointer("a[0].b[1].c"), "/a/0/b/1/c");
    }

    // Regression: absolute-URL literal targets are rejected instead of
    // mangled into `/https:/...`.
    #[test]
    fn literal_absolute_url_operation_path_is_rejected() {
        assert!(parse_operation_path("GET https://api.example.com/pets").is_none());
        assert!(parse_operation_path("GET /pets").is_some());
    }

    /// Writes two specs sharing the basename `petstore.yaml`, each declaring
    /// its own `operationId` (`op0`, `op1`), plus an Arazzo document whose
    /// `sourceDescriptions` reference them via `(path, url-spelling)` pairs.
    fn two_sources_workspace(dir: &std::path::Path, urls: &[(&str, &str)]) -> Arc<Workspace> {
        let oas = |op_id: &str| {
            format!(
                "openapi: 3.1.0\ninfo:\n  title: t\n  version: '1'\npaths:\n  /{op_id}:\n    get:\n      operationId: {op_id}\n      responses:\n        '200': {{description: ok}}\n"
            )
        };
        let mut sources = String::new();
        for (i, (path, spelling)) in urls.iter().enumerate() {
            let file = dir.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, oas(&format!("op{i}"))).unwrap();
            sources.push_str(&format!("  - name: src{i}\n    url: {spelling}\n"));
        }
        std::fs::write(
            dir.join("flow.arazzo.yaml"),
            format!(
                "arazzo: 1.0.0\ninfo:\n  title: flows\n  version: \"1\"\nsourceDescriptions:\n{sources}workflows:\n  - workflowId: wf\n    steps:\n      - stepId: s0\n        operationId: op0\n      - stepId: s1\n        operationId: op1\n"
            ),
        )
        .unwrap();

        let ws = suspect_ref::WorkspaceBuilder::new()
            .root(dir)
            .build()
            .expect("ws");
        for (path, _) in urls {
            ws.load_all(path).expect("load spec");
        }
        Arc::new(ws)
    }

    fn compile_flow(dir: &std::path::Path, ws: &Arc<Workspace>) -> Result<Plan, CompileError> {
        let uri = Uri::from_path(&dir.join("flow.arazzo.yaml")).expect("uri");
        let bytes = std::fs::read(dir.join("flow.arazzo.yaml")).unwrap();
        let doc = LowDoc::parse(uri, suspect_source::Source::from_vec(bytes));
        compile_plan(&doc, ws)
    }

    // Regression: sources sharing a basename bind by resolved URL, not the
    // first basename match (used to mis-bind both names to one document).
    #[test]
    fn shared_basename_binds_by_resolved_url_when_exact() {
        let dir = tempfile::tempdir().unwrap();
        let ws = two_sources_workspace(
            dir.path(),
            &[
                ("alpha/petstore.yaml", "alpha/petstore.yaml"),
                ("beta/petstore.yaml", "beta/petstore.yaml"),
            ],
        );
        let plan = compile_flow(dir.path(), &ws).expect("compiles");
        assert_eq!(plan.workflows[0].steps[0].operation.path, "/op0");
        assert_eq!(plan.workflows[0].steps[1].operation.path, "/op1");
    }

    // Regression: same-basename docs with no exact URL match are ambiguous,
    // not silently bound to the first match.
    #[test]
    fn shared_basename_without_exact_match_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        // Both urls spell plain `petstore.yaml`: no exact resolved-URI match,
        // and the basename alone matches two documents.
        let ws = two_sources_workspace(
            dir.path(),
            &[
                ("alpha/petstore.yaml", "petstore.yaml"),
                ("beta/petstore.yaml", "petstore.yaml"),
            ],
        );
        let err = compile_flow(dir.path(), &ws).expect_err("ambiguous sources must fail");
        assert!(err.to_string().contains("ambiguously"), "{err}");
    }
}
