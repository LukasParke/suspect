use suspect_low::{LowDoc, NodeRef};

/// A parsed Arazzo 1.0 document.
pub struct ArazzoDoc<'d> {
    root: NodeRef<'d>,
    workflows: Vec<WorkflowView<'d>>,
    sources: Vec<SourceDescriptionView<'d>>,
}

/// `sourceDescriptions` entry.
pub struct SourceDescriptionView<'d> {
    /// The entry's `name` (referenced by `$sourceDescriptions.<name>`
    /// expressions).
    pub name: &'d str,
    /// The entry's `url` (where the described document lives).
    pub url: &'d str,
    /// The entry's declared `type`; `OpenApi` when absent (the default).
    pub kind: SourceType,
    node: NodeRef<'d>,
}

/// Declared `type` of a `sourceDescriptions` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceType {
    /// `openapi` — the entry (and the default when `type` is absent).
    OpenApi,
    /// `overlay` — an OpenAPI Overlay document.
    Overlay,
    /// `arazzo` — another Arazzo document.
    Arazzo,
    /// Any unrecognized `type` value.
    Other,
}

/// One workflow.
pub struct WorkflowView<'d> {
    /// The workflow's `workflowId`; empty when the field is missing or not a
    /// string.
    pub workflow_id: &'d str,
    node: NodeRef<'d>,
    steps: Vec<StepView<'d>>,
}

/// One step within a workflow.
pub struct StepView<'d> {
    /// The step's `stepId`; empty when the field is missing or not a string.
    pub step_id: &'d str,
    node: NodeRef<'d>,
}

/// A reusable or inline parameter (`name`, `in`, `value`/`target`).
pub struct ParameterView<'d> {
    node: NodeRef<'d>,
}

/// A success/failure criterion (`condition` + optional `context` + `type`).
pub struct CriterionView<'d> {
    node: NodeRef<'d>,
}

/// An `onSuccess`/`onFailure` action.
pub struct ActionView<'d> {
    node: NodeRef<'d>,
}

impl<'d> ArazzoDoc<'d> {
    /// Views an already-parsed document (call [`LowDoc::sniff_family`] first
    /// if you need to gate on family).
    #[must_use]
    pub fn new(doc: &'d LowDoc) -> Self {
        let root = doc.root();
        let workflows = root
            .get("workflows")
            .map(|n| {
                n.items()
                    .into_iter()
                    .map(|w| {
                        let workflow_id =
                            w.get("workflowId").and_then(|n| n.as_str()).unwrap_or("");
                        let steps = w
                            .get("steps")
                            .map(|s| {
                                s.items()
                                    .into_iter()
                                    .map(|st| StepView {
                                        step_id: st
                                            .get("stepId")
                                            .and_then(|n| n.as_str())
                                            .unwrap_or(""),
                                        node: st,
                                    })
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        WorkflowView { workflow_id, node: w, steps }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let sources = root
            .get("sourceDescriptions")
            .map(|n| {
                n.items()
                    .into_iter()
                    .map(|s| SourceDescriptionView {
                        name: s.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                        url: s.get("url").and_then(|v| v.as_str()).unwrap_or(""),
                        kind: match s.get("type").and_then(|v| v.as_str()) {
                            Some("overlay") => SourceType::Overlay,
                            Some("arazzo") => SourceType::Arazzo,
                            // absent or "openapi": Arazzo 1.0 default
                            Some("openapi") | None => SourceType::OpenApi,
                            Some(_) => SourceType::Other,
                        },
                        node: s,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Self { root, workflows, sources }
    }

    /// The document root node, for fields not covered by the typed views.
    #[must_use]
    pub fn root(&self) -> NodeRef<'d> {
        self.root
    }

    /// The declared Arazzo version (the `arazzo` field), e.g. `"1.0.0"`.
    #[must_use]
    pub fn version(&self) -> Option<&'d str> {
        self.root.get("arazzo").and_then(|n| n.as_str())
    }

    /// `info.title` of the document.
    #[must_use]
    pub fn info_title(&self) -> Option<&'d str> {
        self.root.get("info").and_then(|i| i.get("title")).and_then(|n| n.as_str())
    }

    /// `info.summary` of the document.
    #[must_use]
    pub fn info_summary(&self) -> Option<&'d str> {
        self.root.get("info").and_then(|i| i.get("summary")).and_then(|n| n.as_str())
    }

    /// `info.description` of the document.
    #[must_use]
    pub fn info_description(&self) -> Option<&'d str> {
        self.root.get("info").and_then(|i| i.get("description")).and_then(|n| n.as_str())
    }

    /// All top-level workflows, in document order.
    #[must_use]
    pub fn workflows(&self) -> &[WorkflowView<'d>] {
        &self.workflows
    }

    /// All `sourceDescriptions` entries, in document order.
    #[must_use]
    pub fn source_descriptions(&self) -> &[SourceDescriptionView<'d>] {
        &self.sources
    }
}

impl SourceDescriptionView<'_> {
    #[must_use]
    /// The raw `sourceDescriptions` entry node.
    pub fn node(&self) -> NodeRef<'_> {
        self.node
    }
}

impl<'d> WorkflowView<'d> {
    #[must_use]
    /// The raw workflow node.
    pub fn node(&self) -> NodeRef<'d> {
        self.node
    }

    #[must_use]
    /// The workflow's `summary`.
    pub fn summary(&self) -> Option<&'d str> {
        self.node.get("summary").and_then(|n| n.as_str())
    }

    #[must_use]
    /// The workflow's `description`.
    pub fn description(&self) -> Option<&'d str> {
        self.node.get("description").and_then(|n| n.as_str())
    }

    #[must_use]
    /// The workflow's steps, in document order.
    pub fn steps(&self) -> &[StepView<'d>] {
        &self.steps
    }

    #[must_use]
    /// Workflow-level `parameters`, in document order.
    pub fn parameters(&self) -> Vec<ParameterView<'d>> {
        params_of(self.node)
    }

    #[must_use]
    /// The workflow's `successActions`.
    pub fn success_actions(&self) -> Vec<ActionView<'d>> {
        actions_of(self.node, "successActions")
    }

    #[must_use]
    /// The workflow's `failureActions`.
    pub fn failure_actions(&self) -> Vec<ActionView<'d>> {
        actions_of(self.node, "failureActions")
    }

    #[must_use]
    /// The workflow's declared `outputs` as `(name, value-expression)` pairs.
    pub fn outputs(&self) -> Vec<(&'d str, NodeRef<'d>)> {
        outputs_of(self.node)
    }
}

impl<'d> StepView<'d> {
    #[must_use]
    /// The raw step node.
    pub fn node(&self) -> NodeRef<'d> {
        self.node
    }

    #[must_use]
    /// The step's `description`.
    pub fn description(&self) -> Option<&'d str> {
        self.node.get("description").and_then(|n| n.as_str())
    }

    /// `operationId` when the step references by id.
    #[must_use]
    pub fn operation_id(&self) -> Option<&'d str> {
        self.node.get("operationId").and_then(|n| n.as_str())
    }

    /// `operationPath` when the step references by expression
    /// (`$sourceDescriptions.<name>.#/paths/...`).
    #[must_use]
    pub fn operation_path(&self) -> Option<&'d str> {
        self.node.get("operationPath").and_then(|n| n.as_str())
    }

    #[must_use]
    /// Step-level `parameters`, in document order.
    pub fn parameters(&self) -> Vec<ParameterView<'d>> {
        params_of(self.node)
    }

    #[must_use]
    /// The step's `successCriteria`.
    pub fn success_criteria(&self) -> Vec<CriterionView<'d>> {
        criteria_of(self.node, "successCriteria")
    }

    #[must_use]
    /// The step's `requestBody` node, when present.
    pub fn request_body(&self) -> Option<NodeRef<'d>> {
        self.node.get("requestBody")
    }

    #[must_use]
    /// The step's declared `outputs` as `(name, value-expression)` pairs.
    pub fn outputs(&self) -> Vec<(&'d str, NodeRef<'d>)> {
        outputs_of(self.node)
    }

    #[must_use]
    /// The step's `onSuccess` actions.
    pub fn on_success(&self) -> Vec<ActionView<'d>> {
        actions_of(self.node, "onSuccess")
    }

    #[must_use]
    /// The step's `onFailure` actions.
    pub fn on_failure(&self) -> Vec<ActionView<'d>> {
        actions_of(self.node, "onFailure")
    }
}

impl ParameterView<'_> {
    #[must_use]
    /// The raw parameter node.
    pub fn node(&self) -> NodeRef<'_> {
        self.node
    }

    #[must_use]
    /// The parameter's `name` (inline parameters only).
    pub fn name(&self) -> Option<&str> {
        self.node.get("name").and_then(|n| n.as_str())
    }

    #[must_use]
    /// The parameter's `in` location (`path`, `query`, `header`, `cookie`).
    pub fn location(&self) -> Option<&str> {
        self.node.get("in").and_then(|n| n.as_str())
    }

    /// Reusable-parameter reference from `$components.parameters`.
    #[must_use]
    pub fn reference(&self) -> Option<&str> {
        self.node.get("reference").and_then(|n| n.as_str())
    }

    #[must_use]
    /// The parameter's `value` node (string expressions render via
    /// [`render_embedded`](crate::render_embedded)).
    pub fn value(&self) -> Option<NodeRef<'_>> {
        self.node.get("value")
    }

    /// Step-parameter target expression (`$request.body#/x`, `$inputs...`).
    #[must_use]
    pub fn target(&self) -> Option<&str> {
        self.node.get("target").and_then(|n| n.as_str())
    }
}

impl CriterionView<'_> {
    #[must_use]
    /// The raw criterion node.
    pub fn node(&self) -> NodeRef<'_> {
        self.node
    }

    #[must_use]
    /// The criterion's `condition` (a runtime expression).
    pub fn condition(&self) -> Option<&str> {
        self.node.get("condition").and_then(|n| n.as_str())
    }

    #[must_use]
    /// The criterion's `context` expression binding, when present.
    pub fn context(&self) -> Option<&str> {
        self.node.get("context").and_then(|n| n.as_str())
    }

    #[must_use]
    /// The criterion's `type` (e.g. `regex`, `jsonpath`), when declared.
    pub fn criterion_type(&self) -> Option<&str> {
        self.node.get("type").and_then(|n| n.as_str())
    }
}

impl ActionView<'_> {
    #[must_use]
    /// The raw action node.
    pub fn node(&self) -> NodeRef<'_> {
        self.node
    }

    #[must_use]
    /// The action's `name`.
    pub fn name(&self) -> Option<&str> {
        self.node.get("name").and_then(|n| n.as_str())
    }

    #[must_use]
    /// The action's `type` (`goto`, `retry`, or `end`).
    pub fn action_type(&self) -> Option<&str> {
        self.node.get("type").and_then(|n| n.as_str())
    }

    /// `workflowId` for `goto` actions.
    #[must_use]
    pub fn workflow_id(&self) -> Option<&str> {
        self.node.get("workflowId").and_then(|n| n.as_str())
    }

    /// `stepId` for `goto` actions.
    #[must_use]
    pub fn step_id(&self) -> Option<&str> {
        self.node.get("stepId").and_then(|n| n.as_str())
    }

    #[must_use]
    /// The action's success/failure `criteria`.
    pub fn criteria(&self) -> Vec<CriterionView<'_>> {
        self.node
            .get("criteria")
            .map(|n| n.items().into_iter().map(|c| CriterionView { node: c }).collect())
            .unwrap_or_default()
    }
}

fn params_of<'d>(node: NodeRef<'d>) -> Vec<ParameterView<'d>> {
    node.get("parameters")
        .map(|n| n.items().into_iter().map(|p| ParameterView { node: p }).collect())
        .unwrap_or_default()
}

fn criteria_of<'d>(node: NodeRef<'d>, key: &str) -> Vec<CriterionView<'d>> {
    node.get(key)
        .map(|n| n.items().into_iter().map(|c| CriterionView { node: c }).collect())
        .unwrap_or_default()
}

fn actions_of<'d>(node: NodeRef<'d>, key: &str) -> Vec<ActionView<'d>> {
    node.get(key)
        .map(|n| n.items().into_iter().map(|a| ActionView { node: a }).collect())
        .unwrap_or_default()
}

fn outputs_of<'d>(node: NodeRef<'d>) -> Vec<(&'d str, NodeRef<'d>)> {
    node.get("outputs")
        .map(|n| {
            n.entries()
                .into_iter()
                .filter_map(|e| e.value.map(|v| (e.key, v)))
                .collect()
        })
        .unwrap_or_default()
}
