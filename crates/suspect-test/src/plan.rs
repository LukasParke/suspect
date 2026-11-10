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

use suspect_arazzo::rex::{Rex, parse_rex};
use suspect_arazzo::{ArazzoDoc, ParameterView, StepDependency, StepView, WorkflowView};
use suspect_ir::{IrSpec, Method, OpSelector, ParamIn};
use suspect_low::{LowDoc, NodeRef, ValueKind};
use suspect_ref::Workspace;
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
    /// The schema table of each OpenAPI source description, in
    /// `sourceDescriptions` order; a step's [`StepPlan::contract`] indexes
    /// it. Each table holds one document's component and inline schemas in
    /// that document's own dialect, so two sources may reuse a schema name
    /// and an OpenAPI 3.0 source may sit beside a 3.1 one.
    pub contracts: Vec<Arc<suspect_runtime::Schemas>>,
}

/// One compiled workflow.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WfPlan {
    /// The Arazzo `workflowId`.
    pub workflow_id: String,
    /// Workflow-level inputs (plain-valued workflow parameters), read by
    /// `$inputs...` expressions during execution.
    pub inputs: serde_json::Map<String, serde_json::Value>,
    /// Schema-declared default values for optional inputs, keyed by name.
    pub input_defaults: serde_json::Map<String, serde_json::Value>,
    /// Compiled steps, in document order; executed sequentially.
    pub steps: Vec<StepPlan>,
    /// Workflow ids that run to completion first, their outputs readable
    /// through `$workflows.<id>...`: the workflow's own `dependsOn`, then
    /// every workflow a step names in a `$workflows.<id>.steps.<step>`
    /// `dependsOn` entry (the step cannot run before that workflow does).
    pub depends_on: Vec<String>,
    /// Workflow-level `outputs` `(name, expression)`, evaluated against the
    /// accumulated step outputs once every step has run.
    pub outputs: Vec<(String, Rex)>,
}

/// Pause inserted after a step completes, declared with the
/// `x-suspect-think` step extension (`2s`, `500ms..2s`, or a bare number of
/// seconds). Only honoured by executors whose
/// [`ThinkPolicy`](crate::exec::ThinkPolicy) asks for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThinkSpec {
    /// Shortest pause (inclusive).
    pub min: std::time::Duration,
    /// Longest pause (inclusive); equal to `min` for a fixed pause.
    pub max: std::time::Duration,
}

impl ThinkSpec {
    /// A fixed pause.
    #[must_use]
    pub fn fixed(d: std::time::Duration) -> Self {
        Self { min: d, max: d }
    }

    /// Parses a think literal (`2s`, `500ms..2s`, `1.5`).
    ///
    /// # Errors
    /// Returns the duration parser's message for malformed literals.
    pub fn parse(text: &str) -> Result<Self, String> {
        let (min, max) = crate::duration::parse_duration_range(text)?;
        Ok(Self { min, max })
    }
}

/// A step of a named workflow in the same Arazzo document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowStepRef {
    /// The `workflowId`.
    pub workflow: String,
    /// The `stepId` within that workflow.
    pub step: String,
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
    /// Request body expression, when the step declares `requestBody`
    /// (its `payload` when the spec wrapper is used, else the whole node).
    /// JSON bodies may carry runtime expressions in string leaves; the
    /// executor resolves them per request.
    pub request_body: Option<Rex>,
    /// `requestBody.contentType` when declared; sent as `content-type`.
    pub request_content_type: Option<String>,
    /// Success criteria; all must pass for the step to pass. Empty at
    /// compile time means "defaults to 2xx" and is materialized as
    /// [`CriterionKind::StatusInRange(2, 2)`](CriterionKind::StatusInRange).
    pub success: Vec<CriterionPlan>,
    /// Step outputs `(name, expression)` captured after a passing step.
    pub outputs: Vec<(String, Rex)>,
    /// Response-body JSON pointers referenced by this step's criteria, so
    /// executors know which parts of parsed bodies are relevant.
    pub body_pointers: Vec<String>,
    /// `onFailure` actions, in document order. When the step fails, the
    /// first one whose criteria hold applies; a `retry` that has used up
    /// its limit is passed over. With none, the workflow ends failed.
    pub on_failure: Vec<ActionPlan>,
    /// `onSuccess` actions, in document order. When the step passes, the
    /// first one whose criteria hold applies. With none, the next step
    /// runs.
    pub on_success: Vec<ActionPlan>,
    /// Arazzo 1.1 `timeout` in milliseconds: the step fails when the HTTP
    /// exchange exceeds it.
    pub timeout_ms: Option<u64>,
    /// Security requirement alternatives for the target operation: each
    /// alternative lists the scheme names in one requirement object
    /// (alternatives ORed, schemes within one alternative ANDed). Empty
    /// when the operation declares no security.
    pub security: Vec<Vec<String>>,
    /// The responses the target operation declares, as the shared
    /// contract runtime reads them; empty when the operation declares none
    /// or is not part of any source description.
    pub responses: Vec<suspect_ir::IrResponse>,
    /// Index into [`Plan::contracts`] of the source description that
    /// declares the target operation. `None` for a message step, and for
    /// an HTTP step whose `operationPath` names an operation no source
    /// description declares.
    pub contract: Option<usize>,
    /// The `Accept` header sent when the step sets none: the media types
    /// the operation's responses declare, JSON first.
    pub accept: String,
    /// Arazzo 1.1 AsyncAPI message step; `None` for ordinary HTTP steps.
    pub message: Option<MessageStep>,
    /// Step ids of the same workflow that must complete before this one
    /// runs. Combines the explicit `dependsOn` list (bare ids and
    /// `$workflows.<this>.steps.<id>`, normalized to `<id>`) with the
    /// implicit dependencies Arazzo 1.1 §5.8.5.2.4 requires: a step that
    /// reads `$steps.<id>.outputs.<name>` depends on `<id>`. Every id names
    /// a step of the workflow (checked at compile time).
    pub depends_on: Vec<String>,
    /// Steps of *other* workflows in this document that must have
    /// completed (passed) before this one runs, from
    /// `$workflows.<workflowId>.steps.<stepId>` `dependsOn` entries. Their
    /// workflows are added to [`WfPlan::depends_on`] so they run first.
    pub workflow_depends_on: Vec<WorkflowStepRef>,
    /// Pause after this step from the `x-suspect-think` extension.
    pub think: Option<ThinkSpec>,
}

/// One compiled `onSuccess`/`onFailure` action.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionPlan {
    /// The action's `name`.
    pub name: String,
    /// What the action does.
    pub kind: ActionKind,
    /// The action applies only when every criterion holds for the step's
    /// exchange; an action without criteria always applies.
    pub criteria: Vec<CriterionPlan>,
}

/// What an action does to the workflow.
#[derive(Debug, Clone, PartialEq)]
pub enum ActionKind {
    /// `type: end`: the workflow ends here.
    End,
    /// `type: goto` with a `stepId`: the workflow continues at that step.
    Goto(
        /// The target step id (a step of the same workflow).
        String,
    ),
    /// `type: retry` (`onFailure` only): the step runs again.
    Retry {
        /// `retryAfter`: the pause before each new attempt. A `Retry-After`
        /// response header counted in seconds overrides it.
        after: std::time::Duration,
        /// `retryLimit`: how many times the step may be retried; `1` when
        /// the action declares none.
        limit: u32,
    },
}

/// Arazzo 1.1 AsyncAPI message step: `action: send` or `action: receive`
/// on a `channelPath` of an `asyncapi` source description.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageStep {
    /// `send` publishes a payload; `receive` waits for one.
    pub direction: MessageDirection,
    /// The `sourceDescriptions` entry holding the AsyncAPI document.
    pub source: String,
    /// The channel address inside that document.
    pub channel: String,
    /// The addressed message type, when the channel declares several.
    pub message_type: Option<String>,
    /// The payload to publish (send steps) when it is a single expression.
    pub payload: Option<Rex>,
    /// A structured payload whose string leaves are runtime expressions,
    /// materialized at send time.
    pub payload_template: Option<serde_json::Value>,
    /// The correlation id the step matches or stamps (Arazzo 1.1).
    pub correlation_id: Option<Rex>,
    /// The JSON Schema the AsyncAPI message declares, used to validate a
    /// received payload before criteria run.
    pub payload_schema: Option<serde_json::Value>,
}

/// Which direction a message step runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageDirection {
    /// Publish a payload to the channel.
    Send,
    /// Wait for a matching payload on the channel.
    Receive,
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
        /// Expected value: a literal, or a runtime expression
        /// (`$steps.<id>.outputs.<name>`, `$inputs.<name>`) resolved
        /// against the run state at execution time.
        expected: Expected,
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
    /// Ordered numeric comparison of the value at a body pointer against
    /// a literal bound: `$response.body#/MediaContainer/size > 0`.
    Compare {
        /// RFC 6901 body pointer (`"/a/b"`).
        pointer: String,
        /// The comparison to apply.
        comparison: Comparison,
        /// The literal right-hand side.
        bound: serde_json::Value,
    },
    /// An existence-style check over a dot-notation body fragment kept raw
    /// from the condition (e.g. `"pets[0].name"`); passes when the fragment
    /// resolves to a non-null value in the parsed response body.
    JsonPathTrue {
        /// Raw fragment as written after `#/`.
        expr: String,
    },
    /// Criterion that always passes.
    AlwaysTrue,
    /// A `simple` condition in the full Arazzo grammar: comparisons of
    /// runtime expressions and literals combined with `&&`, `||`, `!` and
    /// parentheses. Every condition the shapes above do not cover compiles
    /// to this.
    Condition {
        /// The condition as written.
        text: String,
        /// The parsed condition.
        cond: crate::condition::Cond,
    },
    /// `type: regex`: the pattern is applied to the text its `context`
    /// expression resolves to.
    ContextRegex {
        /// The `context` runtime expression.
        context: Rex,
        /// The `context` as written.
        context_text: String,
        /// Regular-expression source (validated at compile time).
        pattern: String,
    },
    /// `type: jsonpath`: an RFC 9535 query applied to the JSON value its
    /// `context` expression resolves to. Holds when the query selects at
    /// least one node.
    ContextJsonPath {
        /// The `context` runtime expression.
        context: Rex,
        /// The `context` as written.
        context_text: String,
        /// The JSONPath query (validated at compile time).
        query: String,
    },
}

/// The right-hand side of an equality criterion: a literal parsed from
/// the condition text, or a runtime expression resolved against the run
/// state at execution time (Arazzo success criteria may reference
/// earlier step outputs and workflow inputs).
#[derive(Debug, Clone, PartialEq)]
pub enum Expected {
    /// A JSON literal from the condition text.
    Literal(serde_json::Value),
    /// `$steps.<stepId>.outputs.<name>` — the named output of a
    /// completed step.
    StepOutput {
        /// The referenced step's id.
        step: String,
        /// The referenced output's name.
        name: String,
    },
    /// `$inputs.<name>` — the workflow's effective input value.
    Input(String),
}

impl std::fmt::Display for Expected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Literal(value) => write!(f, "{value}"),
            Self::StepOutput { step, name } => write!(f, "$steps.{step}.outputs.{name}"),
            Self::Input(name) => write!(f, "$inputs.{name}"),
        }
    }
}

/// The ordered comparisons Arazzo success criteria support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comparison {
    /// `>`
    Greater,
    /// `>=`
    GreaterOrEqual,
    /// `<`
    Less,
    /// `<=`
    LessOrEqual,
}

impl Comparison {
    /// The operator's source spelling.
    pub fn sign(self) -> &'static str {
        match self {
            Self::Greater => ">",
            Self::GreaterOrEqual => ">=",
            Self::Less => "<",
            Self::LessOrEqual => "<=",
        }
    }

    /// Applies the comparison to two ordered values (numbers, or strings
    /// compared lexicographically).
    pub fn holds<T: PartialOrd>(self, left: T, right: T) -> bool {
        match self {
            Self::Greater => left > right,
            Self::GreaterOrEqual => left >= right,
            Self::Less => left < right,
            Self::LessOrEqual => left <= right,
        }
    }
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
            Self::Compare {
                pointer,
                comparison,
                bound,
            } => format!("body{pointer} {} {bound}", comparison.sign()),
            Self::JsonPathTrue { expr } => format!("body#/{expr} exists"),
            Self::AlwaysTrue => "always passes".to_owned(),
            Self::Condition { text, .. } => text.clone(),
            Self::ContextRegex {
                context_text,
                pattern,
                ..
            } => format!("{context_text} =~ /{pattern}/"),
            Self::ContextJsonPath {
                context_text,
                query,
                ..
            } => format!("{context_text} | {query}"),
        }
    }

    /// Records this criterion's body-pointer target, if any.
    fn note_pointer(&self, out: &mut Vec<String>) {
        match self {
            Self::Equals {
                pointer: Some(p), ..
            } => out.push(p.clone()),
            Self::NotNull { pointer } => out.push(pointer.clone()),
            Self::Compare { pointer, .. } => out.push(pointer.clone()),
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
/// be resolved, a criterion/parameter expression cannot be parsed, or a
/// `dependsOn` entry (workflow or step level) cannot be resolved or forms
/// a workflow dependency cycle.
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
        let mut implied_workflows: Vec<String> = Vec::new();
        let components = arazzo.root().get("components");
        for step in wf.steps() {
            let mut compiled = compile_step(step, &sources, components)?;
            let (local, foreign) = step_dependencies(&doc, wf, step)?;
            compiled.depends_on = local;
            for dependency in &foreign {
                if !implied_workflows.contains(&dependency.workflow) {
                    implied_workflows.push(dependency.workflow.clone());
                }
            }
            compiled.workflow_depends_on = foreign;
            steps.push(compiled);
        }
        // The workflow's own actions apply to every step, after the
        // step's; a step's action of the same name replaces the
        // workflow's.
        let owner = format!("workflow '{}'", wf.workflow_id);
        let wf_failure = compile_actions(
            &owner,
            wf.failure_actions(),
            "failureActions",
            true,
            components,
        )?;
        let wf_success = compile_actions(
            &owner,
            wf.success_actions(),
            "successActions",
            false,
            components,
        )?;
        for step in &mut steps {
            let inherit = |own: &mut Vec<ActionPlan>, shared: &[ActionPlan]| {
                let inherited: Vec<ActionPlan> = shared
                    .iter()
                    .filter(|action| !own.iter().any(|mine| mine.name == action.name))
                    .cloned()
                    .collect();
                own.extend(inherited);
            };
            inherit(&mut step.on_failure, &wf_failure);
            inherit(&mut step.on_success, &wf_success);
        }
        // A `goto` names a step of this workflow; one that names nothing
        // could never be followed.
        for step in &steps {
            for action in step.on_failure.iter().chain(&step.on_success) {
                if let ActionKind::Goto(target) = &action.kind
                    && !steps.iter().any(|candidate| &candidate.step_id == target)
                {
                    return Err(CompileError(format!(
                        "workflow '{}', step '{}': action '{}' goes to '{target}', which matches no step of the workflow",
                        wf.workflow_id, step.step_id, action.name
                    )));
                }
            }
        }
        // Extract schema-declared defaults from `inputs.properties`.
        let mut input_defaults = serde_json::Map::new();
        let wf_node = arazzo.root().get("workflows").and_then(|w| {
            w.items().into_iter().find(|item| {
                item.get("workflowId")
                    .and_then(|n| n.as_str())
                    .is_some_and(|s| s == wf.workflow_id)
            })
        });
        if let Some(inputs_schema) = wf_node.and_then(|n| n.get("inputs"))
            && let Some(props) = inputs_schema.get("properties")
        {
            for e in props.entries() {
                if let Some(default) = e.value.and_then(|v| v.get("default")) {
                    let json_val = materialize_json(default);
                    input_defaults.insert(e.key.to_owned(), json_val);
                }
            }
        }

        // Local workflow ids only: a `$sourceDescriptions.<name>.<id>`
        // entry names a workflow in another Arazzo document, which this
        // plan does not execute (the validator checks that reference).
        let mut depends_on: Vec<String> = wf
            .depends_on()
            .into_iter()
            .filter(|dep| !dep.starts_with('$'))
            .map(str::to_owned)
            .collect();
        for workflow in implied_workflows {
            if !depends_on.contains(&workflow) {
                depends_on.push(workflow);
            }
        }

        let mut outputs = Vec::new();
        for (name, node) in wf.outputs() {
            outputs.push((name.to_owned(), rex_of_node(node)?));
        }

        workflows.push(WfPlan {
            workflow_id: wf.workflow_id.to_owned(),
            inputs,
            input_defaults,
            steps,
            depends_on,
            outputs,
        });
    }
    check_dependencies(&workflows)?;
    let contracts = sources
        .specs
        .iter()
        .map(|(_, ir, _)| Arc::new(suspect_runtime::Schemas::from_spec(ir)))
        .collect();
    Ok(Plan {
        workflows,
        contracts,
    })
}

/// Rejects `dependsOn` references to unknown workflows and dependency
/// cycles, so executors can schedule prerequisites without re-validating.
fn check_dependencies(workflows: &[WfPlan]) -> Result<(), CompileError> {
    for wf in workflows {
        for dep in &wf.depends_on {
            if dep == &wf.workflow_id {
                return Err(CompileError(format!(
                    "workflow '{}' depends on itself",
                    wf.workflow_id
                )));
            }
            if !workflows.iter().any(|w| &w.workflow_id == dep) {
                return Err(CompileError(format!(
                    "workflow '{}': dependsOn '{dep}' matches no workflow",
                    wf.workflow_id
                )));
            }
        }
    }
    dependency_waves(workflows).map(|_| ())
}

/// Groups workflows into execution waves: every workflow in wave `n` depends
/// only on workflows in earlier waves. Workflows without dependencies form
/// wave 0.
///
/// # Errors
/// Returns a [`CompileError`] naming a workflow on a dependency cycle.
pub fn dependency_waves(workflows: &[WfPlan]) -> Result<Vec<Vec<usize>>, CompileError> {
    let mut placed: Vec<Option<usize>> = vec![None; workflows.len()];
    let mut waves: Vec<Vec<usize>> = Vec::new();
    let mut remaining = workflows.len();
    while remaining > 0 {
        let mut wave = Vec::new();
        for (idx, wf) in workflows.iter().enumerate() {
            if placed[idx].is_some() {
                continue;
            }
            let ready = wf.depends_on.iter().all(|dep| {
                workflows
                    .iter()
                    .position(|w| &w.workflow_id == dep)
                    .is_none_or(|d| placed[d].is_some())
            });
            if ready {
                wave.push(idx);
            }
        }
        if wave.is_empty() {
            let stuck = workflows
                .iter()
                .enumerate()
                .find(|(i, _)| placed[*i].is_none())
                .map(|(_, w)| w.workflow_id.as_str())
                .unwrap_or("?");
            return Err(CompileError(format!(
                "workflow '{stuck}' is part of a dependsOn cycle"
            )));
        }
        for &idx in &wave {
            placed[idx] = Some(waves.len());
        }
        remaining -= wave.len();
        waves.push(wave);
    }
    Ok(waves)
}

/// Maps `sourceDescriptions` names to IR snapshots of their documents.
struct SourceIndex {
    specs: Vec<(String, IrSpec, Uri)>,
    /// Parsed AsyncAPI documents by `sourceDescriptions` name (Arazzo 1.1).
    asyncapi: std::collections::BTreeMap<String, suspect_arazzo::AsyncApiDoc>,
    ws: Arc<Workspace>,
}

impl SourceIndex {
    fn load(doc: &ArazzoDoc<'_>, base: &Uri, ws: &Arc<Workspace>) -> Result<Self, CompileError> {
        let uris = ws.uris();
        let mut specs = Vec::new();
        let mut asyncapi = std::collections::BTreeMap::new();
        for src in doc.source_descriptions() {
            match src.kind {
                // Overlays and nested Arazzo descriptions carry no operations.
                suspect_arazzo::SourceType::OpenApi => {
                    let uri = resolve_source(src, base, &uris)?;
                    let ir = IrSpec::from_workspace(ws, &uri)
                        .map_err(|e| CompileError(format!("source '{}': {e}", src.name)))?;
                    specs.push((src.name.to_owned(), ir, uri));
                }
                suspect_arazzo::SourceType::AsyncApi => {
                    let uri = resolve_source(src, base, &uris)?;
                    let Some(handle) = ws.get(&uri) else {
                        continue;
                    };
                    if let Ok(document) = suspect_arazzo::AsyncApiDoc::parse(handle.doc()) {
                        asyncapi.insert(src.name.to_owned(), document);
                    }
                }
                _ => {}
            }
        }
        Ok(Self {
            specs,
            asyncapi,
            ws: Arc::clone(ws),
        })
    }

    /// The AsyncAPI document declared for a `sourceDescriptions` entry.
    fn asyncapi(&self, name: &str) -> Option<&suspect_arazzo::AsyncApiDoc> {
        self.asyncapi.get(name)
    }

    /// Index of the OpenAPI source description named `name`.
    fn index_of(&self, name: &str) -> Option<usize> {
        self.specs.iter().position(|(key, _, _)| key == name)
    }

    /// The operation `selector` names, with the index of the source
    /// description that declares it: only `source` when one is named, else
    /// the first source description that declares it.
    fn find(
        &self,
        source: Option<usize>,
        selector: OpSelector<'_>,
    ) -> Option<(usize, &suspect_ir::IrOperation)> {
        match source {
            Some(index) => self
                .specs
                .get(index)
                .and_then(|(_, ir, _)| ir.operation(selector))
                .map(|op| (index, op)),
            None => self
                .specs
                .iter()
                .enumerate()
                .find_map(|(index, (_, ir, _))| ir.operation(selector).map(|op| (index, op))),
        }
    }

    /// Security requirement alternatives for `key` in source description
    /// `source`: operation-level `security` overrides path-item, which
    /// overrides root. Each alternative is the list of scheme names in
    /// that requirement object.
    fn security_for(&self, key: &OpKey, source: Option<usize>) -> Vec<Vec<String>> {
        let Some((_, _, uri)) = source.and_then(|index| self.specs.get(index)) else {
            return Vec::new();
        };
        let Some(handle) = self.ws.get(uri) else {
            return Vec::new();
        };
        let root = handle.doc().root();
        let read_alternatives = |node: suspect_low::NodeRef<'_>| -> Vec<Vec<String>> {
            let mut out = Vec::new();
            if let Some(security) = node.get("security")
                && security.kind() == suspect_low::ValueKind::Array
            {
                for requirement in security.items() {
                    let names: Vec<String> = requirement
                        .entries()
                        .iter()
                        .map(|e| e.key.to_owned())
                        .collect();
                    if !names.is_empty() {
                        out.push(names);
                    }
                }
            }
            out
        };
        // operation > path item > root precedence.
        if let Some(paths) = root.get("paths")
            && let Some(path_item) = paths.get(&key.path)
            && let Some(op) = path_item.get(&key.method.as_str().to_ascii_lowercase())
        {
            let operation_level = read_alternatives(op);
            if !operation_level.is_empty() {
                return operation_level;
            }
            let item_level = read_alternatives(path_item);
            if !item_level.is_empty() {
                return item_level;
            }
        }
        read_alternatives(root)
    }
}

/// The `Accept` header for an operation: every media type its responses
/// declare, JSON-compatible types first and the rest at a lower weight, so
/// a server that negotiates answers in a form body criteria can read.
/// Wildcard ranges say nothing a server could act on and are left out; an
/// operation that declares no concrete type asks for `application/json`.
fn accept_header(responses: &[suspect_ir::IrResponse]) -> String {
    let mut json: Vec<String> = Vec::new();
    let mut other: Vec<String> = Vec::new();
    for media in responses.iter().flat_map(|response| &response.content) {
        let essence = suspect_ir::media_essence(&media.media_type);
        if essence.contains('*') || json.contains(&essence) || other.contains(&essence) {
            continue;
        }
        if suspect_ir::is_json_media_type(&essence) {
            json.push(essence);
        } else {
            other.push(essence);
        }
    }
    if json.is_empty() && other.is_empty() {
        return "application/json".to_owned();
    }
    json.into_iter()
        .chain(other.into_iter().map(|essence| format!("{essence};q=0.5")))
        .collect::<Vec<_>>()
        .join(", ")
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

/// The steps that must run before `step`: its declared `dependsOn` plus
/// every step whose outputs it reads (Arazzo 1.1 §5.8.5.2.4), split into
/// steps of `wf` (by id) and steps of other workflows of `doc`.
///
/// Per the Step Object, an entry is a bare `stepId` of the current
/// workflow, `$workflows.<workflowId>.steps.<stepId>` for a workflow of
/// this document (normalized to a bare id when it names `wf` itself), or
/// `$sourceDescriptions.<name>.<workflowId>.steps.<stepId>` for another
/// Arazzo document. That last form is dropped, like a cross-document
/// workflow `dependsOn`: this plan never executes other documents.
///
/// # Errors
/// A [`CompileError`] naming the workflow and step when an entry is
/// malformed, names the step itself, or names a workflow or step that
/// does not exist, since such a step could never become ready.
fn step_dependencies(
    doc: &ArazzoDoc<'_>,
    wf: &WorkflowView<'_>,
    step: &StepView<'_>,
) -> Result<(Vec<String>, Vec<WorkflowStepRef>), CompileError> {
    let located = |message: String| {
        CompileError(format!(
            "workflow '{}', step '{}': {message}",
            wf.workflow_id, step.step_id
        ))
    };
    let has_step = |workflow: &WorkflowView<'_>, id: &str| {
        workflow
            .steps()
            .iter()
            .any(|candidate| candidate.step_id == id)
    };
    let mut local: Vec<String> = Vec::new();
    let mut foreign: Vec<WorkflowStepRef> = Vec::new();
    for (entry, dependency) in step.depends_on().into_iter().zip(step.dependencies()) {
        let id = match dependency {
            StepDependency::Local(id) => id,
            StepDependency::Workflow { workflow, step: id } if workflow == wf.workflow_id => id,
            StepDependency::Workflow { workflow, step: id } => {
                let Some(other) = doc.workflows().iter().find(|w| w.workflow_id == workflow) else {
                    return Err(located(format!("dependsOn '{entry}' matches no workflow")));
                };
                if !has_step(other, id) {
                    return Err(located(format!(
                        "dependsOn '{entry}' matches no step of workflow '{workflow}'"
                    )));
                }
                let reference = WorkflowStepRef {
                    workflow: workflow.to_owned(),
                    step: id.to_owned(),
                };
                if !foreign.contains(&reference) {
                    foreign.push(reference);
                }
                continue;
            }
            StepDependency::Source { .. } => continue,
            StepDependency::Invalid(_) => {
                return Err(located(format!(
                    "dependsOn '{entry}' is not a stepId, $workflows.<workflowId>.steps.<stepId>, \
                     or $sourceDescriptions.<name>.<workflowId>.steps.<stepId>"
                )));
            }
        };
        if id == step.step_id {
            return Err(located(format!(
                "dependsOn '{entry}' names the step itself"
            )));
        }
        if !has_step(wf, id) {
            return Err(located(format!(
                "dependsOn '{entry}' matches no step of workflow '{}'",
                wf.workflow_id
            )));
        }
        if !local.iter().any(|known| known == id) {
            local.push(id.to_owned());
        }
    }

    let mut expressions: Vec<String> = Vec::new();
    for parameter in step.parameters() {
        if let Some(value) = parameter.value().and_then(|node| node.as_str()) {
            expressions.push(value.to_owned());
        }
    }
    for criterion in step.success_criteria() {
        if let Some(condition) = criterion.condition() {
            expressions.push(condition.to_owned());
        }
    }
    for (_, value) in step.outputs() {
        if let Some(text) = value.as_str() {
            expressions.push(text.to_owned());
        }
    }
    for expression in &expressions {
        let mut rest: &str = expression;
        while let Some(after) = rest.split_once("$steps.").map(|(_, tail)| tail) {
            rest = after;
            let step_id: String = after
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if step_id.is_empty() {
                break;
            }
            // A step reading its own captured outputs is not an ordering
            // constraint.
            if step_id == step.step_id {
                continue;
            }
            if !has_step(wf, &step_id) {
                return Err(located(format!(
                    "reads '$steps.{step_id}', which matches no step of workflow '{}'",
                    wf.workflow_id
                )));
            }
            if !local.contains(&step_id) {
                local.push(step_id);
            }
        }
    }
    Ok((local, foreign))
}

/// Compiles an Arazzo 1.1 AsyncAPI step into a [`MessageStep`], or
/// `None` for an ordinary HTTP step.
fn message_step(step: &StepView<'_>, sources: &SourceIndex) -> Option<MessageStep> {
    let channel_path = step.channel_path()?;
    let channel_ref = suspect_arazzo::parse_channel_path(channel_path)?;
    let direction = match step.action() {
        Some("send") => MessageDirection::Send,
        // A channel step without an explicit action subscribes when it
        // declares a correlation id, otherwise it publishes.
        _ => {
            if step.correlation_id().is_some() {
                MessageDirection::Receive
            } else {
                MessageDirection::Send
            }
        }
    };
    // The AsyncAPI document for the referenced source, when it is loaded.
    let flow = match direction {
        MessageDirection::Send => suspect_arazzo::asyncapi::MessageDirection::Outgoing,
        MessageDirection::Receive => suspect_arazzo::asyncapi::MessageDirection::Incoming,
    };
    let resolved = sources
        .asyncapi(&channel_ref.source)
        .as_ref()
        .and_then(|doc| doc.channel(&channel_ref.address))
        .and_then(|channel| channel.message(flow, None))
        .cloned();
    Some(MessageStep {
        direction,
        source: channel_ref.source,
        channel: channel_ref.address,
        message_type: resolved.as_ref().map(|m| m.name.clone()),
        // A send step's payload is either a single runtime expression or a
        // structured template whose string leaves are expressions.
        payload: step.request_body().and_then(|body| {
            let payload = body.get("payload").unwrap_or(body);
            payload
                .as_str()
                .and_then(|text| suspect_arazzo::rex::parse_rex(text).ok())
        }),
        payload_template: step.request_body().map(|body| {
            let payload = body.get("payload").unwrap_or(body);
            if payload.kind() == suspect_low::ValueKind::Str {
                serde_json::Value::Null
            } else {
                serde_json::from_str(&suspect_overlay::Value::from_node(payload).to_json())
                    .unwrap_or(serde_json::Value::Null)
            }
        }),
        correlation_id: step
            .correlation_id()
            .and_then(|expr| suspect_arazzo::rex::parse_rex(expr).ok()),
        payload_schema: resolved.and_then(|m| m.payload),
    })
}

fn compile_step(
    step: &StepView<'_>,
    sources: &SourceIndex,
    components: Option<NodeRef<'_>>,
) -> Result<StepPlan, CompileError> {
    if step.step_id.is_empty() {
        return Err(CompileError("step missing stepId".to_owned()));
    }
    let step_id = step.step_id.to_owned();
    let message = message_step(step, sources);
    let (operation, contract) = match &message {
        // A message step addresses a channel, not an HTTP operation.
        Some(_) => (
            OpKey {
                method: Method::Post,
                path: step.channel_path().unwrap_or_default().to_owned(),
            },
            None,
        ),
        None => resolve_operation(step, sources)?,
    };
    let security = sources.security_for(&operation, contract);
    let responses = sources
        .find(
            contract,
            OpSelector::MethodPath(operation.method, &operation.path),
        )
        .filter(|_| contract.is_some())
        .map(|(_, op)| op.responses.clone())
        .unwrap_or_default();
    let accept = accept_header(&responses);
    let mut parameters = Vec::new();
    for p in step.parameters() {
        parameters.push(compile_param(&step_id, p)?);
    }
    // Arazzo wraps the body as `{contentType, payload, replacements}`; a
    // bare value is accepted leniently as the payload itself.
    let (request_body, request_content_type) = match step.request_body() {
        Some(node) => {
            let content_type = node
                .get("contentType")
                .and_then(|n| n.as_str())
                .map(str::to_owned);
            let payload = node.get("payload");
            let is_wrapper = node.kind() == ValueKind::Object
                && (payload.is_some() || content_type.is_some())
                && node
                    .entries()
                    .iter()
                    .all(|e| matches!(e.key, "contentType" | "payload" | "replacements"));
            let replacements = node
                .get("replacements")
                .filter(|_| is_wrapper)
                .map(|list| list.items())
                .unwrap_or_default();
            let body = match (is_wrapper, payload) {
                (true, Some(p)) if !replacements.is_empty() => {
                    Some(patched_payload(&step_id, p, &replacements)?)
                }
                (true, Some(p)) => Some(rex_of_node(p)?),
                (true, None) => None,
                (false, _) => Some(rex_of_node(node)?),
            };
            (body, content_type)
        }
        None => (None, None),
    };

    let mut success = Vec::new();
    let mut body_pointers = Vec::new();
    let owner = format!("step '{step_id}'");
    for c in step.success_criteria() {
        let criterion = compile_criterion(&owner, &c)?;
        criterion.kind.note_pointer(&mut body_pointers);
        success.push(criterion);
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

    let on_failure = compile_actions(&owner, step.on_failure(), "onFailure", true, components)?;
    let on_success = compile_actions(&owner, step.on_success(), "onSuccess", false, components)?;

    let think = match step.node().get("x-suspect-think") {
        None => None,
        Some(node) => {
            let text = match node.as_str() {
                Some(s) => s.to_owned(),
                None => String::from_utf8_lossy(node.raw_text()).trim().to_owned(),
            };
            Some(
                ThinkSpec::parse(&text)
                    .map_err(|e| CompileError(format!("step '{step_id}': x-suspect-think: {e}")))?,
            )
        }
    };

    Ok(StepPlan {
        step_id,
        operation,
        parameters,
        request_body,
        request_content_type,
        message,
        // Resolved against the enclosing workflow by `compile_plan`.
        depends_on: Vec::new(),
        workflow_depends_on: Vec::new(),
        timeout_ms: step.timeout_ms(),
        success,
        outputs,
        body_pointers,
        on_failure,
        on_success,
        security,
        responses,
        contract,
        accept,
        think,
    })
}

/// A request payload with its `replacements` applied: each one sets the
/// value at a JSON Pointer `target` inside the payload. A replacement
/// value may be a runtime expression; like any string leaf of the body it
/// is resolved when the request is built.
fn patched_payload(
    step_id: &str,
    payload: NodeRef<'_>,
    replacements: &[NodeRef<'_>],
) -> Result<Rex, CompileError> {
    let located =
        |message: String| CompileError(format!("step '{step_id}', requestBody: {message}"));
    if !matches!(payload.kind(), ValueKind::Object | ValueKind::Array) {
        return Err(located(
            "replacements apply to a JSON payload written as an object or array".to_owned(),
        ));
    }
    let mut body = materialize_json(payload);
    for replacement in replacements {
        let target = replacement
            .get("target")
            .and_then(|n| n.as_str())
            .ok_or_else(|| located("a replacement needs a `target`".to_owned()))?;
        let value = replacement
            .get("value")
            .map(materialize_json)
            .ok_or_else(|| located(format!("replacement `{target}` needs a `value`")))?;
        let tokens: Vec<String> = match target.strip_prefix('/') {
            Some(rest) if !rest.split('/').any(str::is_empty) => rest
                .split('/')
                .map(|token| token.replace("~1", "/").replace("~0", "~"))
                .collect(),
            _ => {
                return Err(located(format!(
                    "replacement target `{target}` is not a JSON Pointer into the payload"
                )));
            }
        };
        let (last, parents) = tokens.split_last().expect("a pointer has a token");
        let mut parent = &mut body;
        for token in parents {
            parent = match parent {
                serde_json::Value::Object(map) => map.get_mut(token),
                serde_json::Value::Array(items) => {
                    token.parse::<usize>().ok().and_then(|i| items.get_mut(i))
                }
                _ => None,
            }
            .ok_or_else(|| {
                located(format!(
                    "replacement target `{target}` does not exist in the payload"
                ))
            })?;
        }
        match parent {
            serde_json::Value::Object(map) => {
                map.insert(last.clone(), value);
            }
            serde_json::Value::Array(items) => match last.parse::<usize>() {
                Ok(index) if index < items.len() => items[index] = value,
                _ if last == "-" => items.push(value),
                _ => {
                    return Err(located(format!(
                        "replacement target `{target}` does not exist in the payload"
                    )));
                }
            },
            _ => {
                return Err(located(format!(
                    "replacement target `{target}` does not exist in the payload"
                )));
            }
        }
    }
    Ok(Rex::Text(body.to_string()))
}

/// Compiles one Criterion Object owned by `owner` (`step 's'`, `workflow
/// 'w'`): a `regex` or `jsonpath` criterion with its `context`, or a
/// `simple` condition.
fn compile_criterion(
    owner: &str,
    criterion: &suspect_arazzo::CriterionView<'_>,
) -> Result<CriterionPlan, CompileError> {
    let located = |message: String| CompileError(format!("{owner}: {message}"));
    let condition = criterion
        .condition_text()
        .ok_or_else(|| located("criterion missing condition".to_owned()))?;
    let condition = condition.as_str();
    let range = criterion
        .node()
        .get("condition")
        .map(|n| n.byte_range())
        .unwrap_or_else(|| criterion.node().byte_range());
    let context = |kind: &str| -> Result<(Rex, String), CompileError> {
        let text = criterion
            .context()
            .ok_or_else(|| located(format!("a `type: {kind}` criterion needs a `context`")))?;
        // `{$expr}` and `$expr` name the same expression.
        let bare = text
            .trim()
            .strip_prefix('{')
            .and_then(|inner| inner.strip_suffix('}'))
            .unwrap_or(text.trim());
        let rex =
            parse_rex(bare).map_err(|e| located(format!("criterion context `{text}`: {e}")))?;
        Ok((rex, bare.to_owned()))
    };
    let kind = match (criterion.criterion_type(), criterion.context()) {
        (Some("regex"), Some(_)) => {
            let (context, context_text) = context("regex")?;
            regex::Regex::new(condition).map_err(|e| {
                located(format!(
                    "criterion `{condition}` is not a valid regular expression: {e}"
                ))
            })?;
            CriterionKind::ContextRegex {
                context,
                context_text,
                pattern: condition.to_owned(),
            }
        }
        (Some("jsonpath"), Some(_)) => {
            let (context, context_text) = context("jsonpath")?;
            suspect_jsonpath::Path::parse(condition).map_err(|e| {
                located(format!(
                    "criterion `{condition}` is not a valid JSONPath query: {e}"
                ))
            })?;
            CriterionKind::ContextJsonPath {
                context,
                context_text,
                query: condition.to_owned(),
            }
        }
        (Some("xpath"), _) => {
            return Err(located(format!(
                "criterion `{condition}` is `type: xpath`, which this runner does not evaluate"
            )));
        }
        // A typed criterion without a context, like an untyped one, is a
        // condition over runtime expressions.
        (None | Some("simple" | "regex" | "jsonpath"), _) => {
            parse_condition(condition).map_err(|CompileError(message)| located(message))?
        }
        (Some(other), _) => {
            return Err(located(format!(
                "criterion `{condition}` has unknown type `{other}` (simple, regex, jsonpath)"
            )));
        }
    };
    Ok(CriterionPlan { kind, range })
}

/// The longest pause before a retry, whoever asks for it: a `retryAfter`
/// or a server's `Retry-After` beyond it is held to it, so a run cannot be
/// stalled for hours by one header.
pub(crate) const MAX_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(300);

/// Compiles an action list of `owner` (`step 's'`, `workflow 'w'`):
/// `onSuccess`/`onFailure` of a step, `successActions`/`failureActions`
/// of a workflow. `retries` says whether the list may hold a `retry`. A
/// Reusable Object (`reference: $components.<kind>.<name>`) is replaced by
/// the action it names.
fn compile_actions(
    owner: &str,
    actions: Vec<suspect_arazzo::ActionView<'_>>,
    list: &str,
    retries: bool,
    components: Option<NodeRef<'_>>,
) -> Result<Vec<ActionPlan>, CompileError> {
    let located = |message: String| CompileError(format!("{owner}, {list}: {message}"));
    let mut out = Vec::with_capacity(actions.len());
    for action in actions {
        let node = match action.node().get("reference").and_then(|n| n.as_str()) {
            None => action.node(),
            Some(reference) => {
                let target = reference
                    .strip_prefix("$components.")
                    .and_then(|rest| rest.split_once('.'))
                    .and_then(|(kind, name)| components?.get(kind)?.get(name));
                target.ok_or_else(|| {
                    located(format!("reference `{reference}` names no reusable action"))
                })?
            }
        };
        let text = |key: &str| node.get(key).and_then(|n| n.as_str());
        let name = text("name").unwrap_or("(unnamed)").to_owned();
        let kind = match text("type") {
            Some("end") => ActionKind::End,
            Some("goto") => match (text("stepId"), text("workflowId")) {
                (Some(step), _) => ActionKind::Goto(step.to_owned()),
                (None, Some(workflow)) => {
                    return Err(located(format!(
                        "action '{name}' goes to workflowId '{workflow}'; only a `stepId` of \
                         the same workflow can be a goto target (order workflows with `dependsOn`)"
                    )));
                }
                (None, None) => {
                    return Err(located(format!(
                        "action '{name}' is a goto without a `stepId`"
                    )));
                }
            },
            Some("retry") if retries => {
                // Arazzo lets a retry run a step or workflow first (refresh
                // a token, then try again). Retrying without it would test
                // something else than the suite describes.
                for key in ["stepId", "workflowId"] {
                    if let Some(target) = text(key) {
                        return Err(located(format!(
                            "action '{name}' is a retry that first runs {key} '{target}', \
                             which this runner does not do"
                        )));
                    }
                }
                let number = |key: &str| {
                    node.get(key).and_then(|n| {
                        n.as_f64()
                            .or_else(|| n.as_i64().map(|value| value as f64))
                            .or_else(|| n.as_str().and_then(|text| text.parse().ok()))
                    })
                };
                let after = number("retryAfter").unwrap_or(0.0);
                if !after.is_finite() || after < 0.0 {
                    return Err(located(format!(
                        "action '{name}': retryAfter must be a non-negative number of seconds"
                    )));
                }
                let limit = match number("retryLimit") {
                    None => 1,
                    Some(limit) if limit >= 0.0 && limit.fract() == 0.0 => limit as u32,
                    Some(_) => {
                        return Err(located(format!(
                            "action '{name}': retryLimit must be a non-negative integer"
                        )));
                    }
                };
                ActionKind::Retry {
                    after: std::time::Duration::from_secs_f64(
                        after.min(MAX_RETRY_DELAY.as_secs_f64()),
                    ),
                    limit,
                }
            }
            Some("retry") => {
                return Err(located(format!(
                    "action '{name}' is a retry, which only a failure can ask for"
                )));
            }
            Some(other) => {
                return Err(located(format!(
                    "action '{name}' has unknown type `{other}` (end, goto, retry)"
                )));
            }
            None => return Err(located(format!("action '{name}' has no `type`"))),
        };
        let mut criteria = Vec::new();
        if let Some(list) = node.get("criteria") {
            for item in list.items() {
                criteria.push(compile_criterion(
                    owner,
                    &suspect_arazzo::CriterionView::of(item),
                )?);
            }
        }
        out.push(ActionPlan {
            name,
            kind,
            criteria,
        });
    }
    Ok(out)
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

/// Resolves a step's target to a canonical [`OpKey`] and the index of the
/// source description that declares it.
///
/// Preference order mirrors `operationId` first, then `operationPath`
/// (`{$sourceDescriptions.<name>.url}#/paths/~1pets/get`, the same without
/// braces or `.url`, `<name>#/users/{userId}/delete`, or a literal
/// `GET /pets`). A reference that names its source description resolves
/// in that description only; one that does not resolves in the first
/// description that declares the operation. An `operationPath` no
/// description declares still compiles, with no source: the request can be
/// sent, but there is no contract to hold its response to.
fn resolve_operation(
    step: &StepView<'_>,
    sources: &SourceIndex,
) -> Result<(OpKey, Option<usize>), CompileError> {
    let key_of = |(index, op): (usize, &suspect_ir::IrOperation)| {
        (
            OpKey {
                method: op.method,
                path: op.path.clone(),
            },
            Some(index),
        )
    };
    if let Some(id) = step.operation_id() {
        // `$sourceDescriptions.<name>.<operationId>` (Arazzo spec §4.2.4)
        // addresses the operation within that document.
        let qualified = id
            .strip_prefix("$sourceDescriptions.")
            .and_then(|rest| rest.split_once('.'));
        if let Some((source, op_id)) = qualified {
            // The name is a promise about where the operation is declared:
            // it is not looked for anywhere else.
            let Some(index) = sources.index_of(source) else {
                return Err(CompileError(format!(
                    "step '{}': operationId '{id}' names source description '{source}', \
                     which is not an OpenAPI description this document declares",
                    step.step_id
                )));
            };
            return sources
                .find(Some(index), OpSelector::Id(op_id))
                .map(key_of)
                .ok_or_else(|| {
                    CompileError(format!(
                        "step '{}': operationId '{op_id}' not found in source description \
                         '{source}'",
                        step.step_id
                    ))
                });
        }
        if let Some(found) = sources.find(None, OpSelector::Id(id)) {
            return Ok(key_of(found));
        }
        return Err(CompileError(format!(
            "step '{}': operationId '{}' not found in any source description",
            step.step_id, id
        )));
    }
    let Some(path_expr) = step.operation_path() else {
        if let Some(workflow) = step.node().get("workflowId").and_then(|n| n.as_str()) {
            return Err(CompileError(format!(
                "step '{}' runs workflow '{workflow}', which this runner does not do; make \
                 that workflow a prerequisite with `dependsOn` and read its outputs through \
                 `$workflows.{workflow}.outputs`",
                step.step_id
            )));
        }
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
    // `$sourceDescriptions.<name>.url` is the specification's spelling;
    // the name as written wins, so a source called `x.url` still resolves.
    let source = name.and_then(|name| {
        sources.index_of(name).or_else(|| {
            name.strip_suffix(".url")
                .and_then(|bare| sources.index_of(bare))
        })
    });
    let selector = OpSelector::MethodPath(method, &raw_path);
    let found = match (name, source) {
        // A named source that is not an OpenAPI description declares
        // nothing to resolve against.
        (Some(_), None) => None,
        _ => sources.find(source, selector),
    };
    Ok(found.map_or_else(
        || {
            (
                OpKey {
                    method,
                    path: raw_path.clone(),
                },
                None,
            )
        },
        key_of,
    ))
}

/// Parses an `operationPath` spelling into
/// `(source-name, Method, raw-path)`, following the accepted shapes:
/// `{$sourceDescriptions.<name>.url}#/paths/~1pets/get` (the
/// specification's own, where the name comes back as `<name>.url`), the
/// same without braces, `$sourceDescriptions.<name>#/paths/~1pets/get`,
/// `<name>#/users/{userId}/delete`, and literal `GET /pets`. Absolute-URL
/// targets (`GET https://...`) are rejected so compilation fails with a
/// clear error instead of a mangled path.
fn parse_operation_path(path: &str) -> Option<(Option<&str>, Method, String)> {
    if let Some((left, frag)) = path.split_once('#') {
        let left = left.trim();
        // `{$expr}` and `$expr` name the same expression.
        let left = left
            .strip_prefix('{')
            .and_then(|inner| inner.strip_suffix('}'))
            .unwrap_or(left);
        let name = left
            .strip_prefix("$sourceDescriptions.")
            .map_or(left, |name| name.strip_suffix('.').unwrap_or(name));
        let name = if name.is_empty() { None } else { Some(name) };
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
#[derive(Clone, Copy)]
enum Op {
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `/=` or `=~` (regular-expression match)
    Re,
    /// `>=`
    Ge,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `<`
    Lt,
}

/// LHS target of a criterion condition.
enum Target {
    /// `{$statusCode}`
    Status,
    /// `$response.body#/<pointer>`
    Body(String),
}

/// Compiles one `simple` condition string.
///
/// Conditions of a few common shapes compile to dedicated criteria, whose
/// failures read as a named check:
///
/// - `'{$statusCode} == 200'` — status equality.
/// - `'{$statusCode} /= /^2../'` — class match when the pattern has the
///   `^N..` shape, otherwise a regex held to the status (`/=`, also spelt
///   `=~`, is this runner's regex-match operator; a body pointer on the
///   left is matched by its value, the bare body as it arrived).
/// - `'$response.body#/x == literal'` — body-pointer equality (JSON literal
///   or quoted/plain string), also against `$steps.<id>.outputs.<name>` or
///   `$inputs.<name>`.
/// - `'$response.body#/x != null'` — not-null existence check.
/// - `'$response.body#/x > 3'` — ordered comparison with a literal.
/// - `'$response.body#/x'` without any operator — existence check via
///   [`CriterionKind::JsonPathTrue`], keeping the raw fragment after `#/`.
///
/// Every other condition — anything with `&&`, `||`, `!` or parentheses,
/// a comparison of the status other than equality, a header, an input,
/// two runtime expressions — is parsed by the full grammar of
/// [`crate::condition`] into [`CriterionKind::Condition`].
pub(crate) fn parse_condition(condition: &str) -> Result<CriterionKind, CompileError> {
    let trimmed = condition.trim();
    // `/=` belongs to the dedicated shapes alone, and the pattern after it
    // may hold parentheses of its own.
    if (is_pattern_match(trimmed) || !crate::condition::is_compound(trimmed))
        && let Some(kind) = condition_shape(trimmed)
    {
        return Ok(kind);
    }
    let unsupported = |reason: String| {
        CompileError(format!(
            "unsupported success criterion: '{condition}' ({reason})"
        ))
    };
    let cond = crate::condition::parse(trimmed).map_err(unsupported)?;
    // A literal, the status or the method standing alone is the same
    // whatever the server answers: `$statusCode == 200 || 201` always
    // holds. A condition with such an operand is a mistake, not a check.
    if let Some(operand) = constant_operand(&cond) {
        return Err(unsupported(format!(
            "`{operand}` compares nothing, so it is the same for every response"
        )));
    }
    Ok(CriterionKind::Condition {
        text: trimmed.to_owned(),
        cond,
    })
}

/// The first operand of `cond` that stands alone and cannot vary with the
/// response body: a literal, `$statusCode` or `$method`.
fn constant_operand(cond: &crate::condition::Cond) -> Option<String> {
    use crate::condition::{Cond, Operand};
    match cond {
        Cond::Or(members) | Cond::And(members) => members.iter().find_map(constant_operand),
        Cond::Not(member) => constant_operand(member),
        Cond::Compare { .. } => None,
        Cond::Truthy(Operand::Literal(value)) => Some(value.to_string()),
        Cond::Truthy(Operand::Expr {
            rex: Rex::StatusCode | Rex::Method,
            text,
        }) => Some(text.clone()),
        Cond::Truthy(Operand::Expr { .. }) => None,
    }
}

/// Whether the condition is `target /= pattern` and nothing more: its
/// first operator is `/=`, and a `/…/flags` pattern is followed by nothing
/// but its flags.
fn is_pattern_match(trimmed: &str) -> bool {
    let Some((_, Op::Re, rhs)) = split_op(trimmed) else {
        return false;
    };
    match rhs.strip_prefix('/').and_then(|rest| rest.rsplit_once('/')) {
        Some((_, flags)) => flags.chars().all(|flag| flag.is_ascii_alphabetic()),
        None => true,
    }
}

/// The dedicated criterion for a condition of one of the common shapes;
/// `None` when the condition needs the full grammar.
fn condition_shape(trimmed: &str) -> Option<CriterionKind> {
    let Some((lhs, op, rhs)) = split_op(trimmed) else {
        // An operator-less condition that is one body expression is an
        // existence check. Anything after the expression is not part of
        // its pointer: the full grammar says what it is.
        let bare = unbraced(trimmed);
        return match classify_target(bare) {
            Some(Target::Body(_)) if !bare.contains(char::is_whitespace) => {
                Some(CriterionKind::JsonPathTrue {
                    expr: fragment_after_hash(bare).unwrap_or_default(),
                })
            }
            _ => None,
        };
    };
    // The dedicated shapes compare one target with a literal, a step
    // output or an input; any other runtime expression on the right needs
    // the full grammar.
    let expression_rhs = rhs.trim_start().starts_with(['$', '{'])
        && matches!(expected_value(rhs), Expected::Literal(_));
    Some(match (classify_target(lhs)?, op) {
        (Target::Status | Target::Body(_), Op::Eq) if expression_rhs => return None,
        (Target::Status, Op::Eq) => CriterionKind::Equals {
            pointer: None,
            expected: expected_value(rhs),
        },
        (target, Op::Re) => return pattern_criterion(lhs, &target, rhs),
        (Target::Body(pointer), Op::Eq) => CriterionKind::Equals {
            pointer: Some(pointer),
            expected: expected_value(rhs),
        },
        (Target::Body(pointer), Op::Ne) if rhs.trim() == "null" => {
            CriterionKind::NotNull { pointer }
        }
        (Target::Body(pointer), Op::Gt | Op::Ge | Op::Lt | Op::Le) => {
            // A bound that is itself a runtime expression is not a literal.
            if rhs.trim_start().starts_with(['$', '{']) {
                return None;
            }
            CriterionKind::Compare {
                pointer,
                comparison: match op {
                    Op::Gt => Comparison::Greater,
                    Op::Ge => Comparison::GreaterOrEqual,
                    Op::Lt => Comparison::Less,
                    _ => Comparison::LessOrEqual,
                },
                bound: literal_value(rhs),
            }
        }
        // Inputs, headers, status inequalities and orderings, body
        // inequalities: the full grammar evaluates them.
        _ => return None,
    })
}

/// The criterion for `target /= pattern`: the pattern is held to the value
/// the target names — the status, the value at a body pointer, or the body
/// as it arrived. `None` when the pattern is not a regular expression, so
/// the condition is reported instead of failing every run.
fn pattern_criterion(lhs: &str, target: &Target, rhs: &str) -> Option<CriterionKind> {
    let pattern = regex_pattern(rhs);
    regex::Regex::new(&pattern).ok()?;
    match target {
        Target::Status => {
            // `^N..` is a status class, reported as one.
            if let range @ CriterionKind::StatusInRange(..) = status_range_or_regex(pattern.clone())
            {
                return Some(range);
            }
        }
        Target::Body(pointer) if pointer.is_empty() => {
            return Some(CriterionKind::Regex { pattern });
        }
        Target::Body(_) => {}
    }
    let text = unbraced(lhs);
    Some(CriterionKind::ContextRegex {
        context: parse_rex(text).ok()?,
        context_text: text.to_owned(),
        pattern,
    })
}

/// `{$expr}` and `$expr` name the same expression: the text without the
/// optional braces.
fn unbraced(text: &str) -> &str {
    let text = text.trim();
    text.strip_prefix('{')
        .and_then(|inner| inner.strip_suffix('}'))
        .map_or(text, str::trim)
}

/// Splits `lhs op rhs` at the earliest operator occurrence that lies outside
/// the pointer text following a `#/` marker, so operators inside a
/// `$response.body#/...` fragment never yield split points.
fn split_op(cond: &str) -> Option<(&str, Op, &str)> {
    let frag_start = cond.find("#/");
    let mut best: Option<(usize, Op, usize)> = None;
    for (needle, op) in [
        ("==", Op::Eq),
        ("!=", Op::Ne),
        ("/=", Op::Re),
        ("=~", Op::Re),
        (">=", Op::Ge),
        ("<=", Op::Le),
        (">", Op::Gt),
        ("<", Op::Lt),
    ] {
        let mut from = 0;
        while let Some(rel) = cond[from..].find(needle) {
            let idx = from + rel;
            from = idx + needle.len();
            if in_pointer_text(frag_start, cond, idx) {
                continue;
            }
            let better = best
                .as_ref()
                .is_none_or(|(i, _, w)| idx < *i || (idx == *i && needle.len() > *w));
            if better {
                best = Some((idx, op, needle.len()));
                break;
            }
        }
    }
    let (idx, op, width) = best?;
    Some((cond[..idx].trim(), op, cond[idx + width..].trim()))
}

/// True when operator position `idx` sits inside the contiguous (whitespace-
/// free) pointer text introduced by a `#/` marker.
fn in_pointer_text(frag_start: Option<usize>, cond: &str, idx: usize) -> bool {
    matches!(frag_start, Some(fs) if idx >= fs && !cond[fs..idx].contains(char::is_whitespace))
}

/// Classifies a condition LHS as status or body-pointer target.
fn classify_target(lhs: &str) -> Option<Target> {
    let lhs = unbraced(lhs);
    if lhs.contains("$statusCode") {
        return Some(Target::Status);
    }
    // `$response.body#/x` and the Arazzo 1.1 `$message.payload#/x` address
    // the same current-message body through a JSON pointer.
    if lhs.contains("$response.body") || lhs.contains("$message.payload") {
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
    // A doubled quote inside a quoted string is one quote, as in the
    // condition grammar.
    let dequoted = |quote: char| {
        rhs.strip_prefix(quote)
            .and_then(|s| s.strip_suffix(quote))
            .map(|inner| inner.replace(&format!("{quote}{quote}"), &quote.to_string()))
    };
    serde_json::Value::String(
        dequoted('\'')
            .or_else(|| dequoted('"'))
            .unwrap_or_else(|| rhs.to_owned()),
    )
}

/// Parses an equality RHS: a runtime expression reference
/// (`$steps.<id>.outputs.<name>`, `$inputs.<name>`) or a JSON literal.
fn expected_value(rhs: &str) -> Expected {
    let rhs = rhs.trim();
    if let Some(rest) = rhs.strip_prefix("$steps.")
        && let Some((step, name)) = rest.split_once(".outputs.")
        && !step.is_empty()
        && !name.is_empty()
    {
        return Expected::StepOutput {
            step: step.to_owned(),
            name: name.trim().to_owned(),
        };
    }
    if let Some(name) = rhs.strip_prefix("$inputs.")
        && !name.is_empty()
    {
        return Expected::Input(name.trim().to_owned());
    }
    Expected::Literal(literal_value(rhs))
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
                expected: Expected::Literal(serde_json::json!(3)),
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
                expected: Expected::Literal(serde_json::json!(3)),
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
