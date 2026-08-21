use suspect_low::{LowDoc, NodeRef};

/// A parsed Arazzo 1.0 document.
pub struct ArazzoDoc<'d> {
    root: NodeRef<'d>,
    workflows: Vec<WorkflowView<'d>>,
    sources: Vec<SourceDescriptionView<'d>>,
}

/// `sourceDescriptions` entry.
pub struct SourceDescriptionView<'d> {
    pub name: &'d str,
    pub url: &'d str,
    pub kind: SourceType,
    node: NodeRef<'d>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceType {
    OpenApi,
    Overlay,
    Arazzo,
    Other,
}

/// One workflow.
pub struct WorkflowView<'d> {
    pub workflow_id: &'d str,
    node: NodeRef<'d>,
    steps: Vec<StepView<'d>>,
}

/// One step within a workflow.
pub struct StepView<'d> {
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

    #[must_use]
    pub fn root(&self) -> NodeRef<'d> {
        self.root
    }

    #[must_use]
    pub fn version(&self) -> Option<&'d str> {
        self.root.get("arazzo").and_then(|n| n.as_str())
    }

    #[must_use]
    pub fn info_title(&self) -> Option<&'d str> {
        self.root.get("info").and_then(|i| i.get("title")).and_then(|n| n.as_str())
    }

    #[must_use]
    pub fn info_summary(&self) -> Option<&'d str> {
        self.root.get("info").and_then(|i| i.get("summary")).and_then(|n| n.as_str())
    }

    #[must_use]
    pub fn info_description(&self) -> Option<&'d str> {
        self.root.get("info").and_then(|i| i.get("description")).and_then(|n| n.as_str())
    }

    #[must_use]
    pub fn workflows(&self) -> &[WorkflowView<'d>] {
        &self.workflows
    }

    #[must_use]
    pub fn source_descriptions(&self) -> &[SourceDescriptionView<'d>] {
        &self.sources
    }
}

impl SourceDescriptionView<'_> {
    #[must_use]
    pub fn node(&self) -> NodeRef<'_> {
        self.node
    }
}

impl<'d> WorkflowView<'d> {
    #[must_use]
    pub fn node(&self) -> NodeRef<'d> {
        self.node
    }

    #[must_use]
    pub fn summary(&self) -> Option<&'d str> {
        self.node.get("summary").and_then(|n| n.as_str())
    }

    #[must_use]
    pub fn description(&self) -> Option<&'d str> {
        self.node.get("description").and_then(|n| n.as_str())
    }

    #[must_use]
    pub fn steps(&self) -> &[StepView<'d>] {
        &self.steps
    }

    #[must_use]
    pub fn parameters(&self) -> Vec<ParameterView<'d>> {
        params_of(self.node)
    }

    #[must_use]
    pub fn success_actions(&self) -> Vec<ActionView<'d>> {
        actions_of(self.node, "successActions")
    }

    #[must_use]
    pub fn failure_actions(&self) -> Vec<ActionView<'d>> {
        actions_of(self.node, "failureActions")
    }

    #[must_use]
    pub fn outputs(&self) -> Vec<(&'d str, NodeRef<'d>)> {
        outputs_of(self.node)
    }
}

impl<'d> StepView<'d> {
    #[must_use]
    pub fn node(&self) -> NodeRef<'d> {
        self.node
    }

    #[must_use]
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
    pub fn parameters(&self) -> Vec<ParameterView<'d>> {
        params_of(self.node)
    }

    #[must_use]
    pub fn success_criteria(&self) -> Vec<CriterionView<'d>> {
        criteria_of(self.node, "successCriteria")
    }

    #[must_use]
    pub fn request_body(&self) -> Option<NodeRef<'d>> {
        self.node.get("requestBody")
    }

    #[must_use]
    pub fn outputs(&self) -> Vec<(&'d str, NodeRef<'d>)> {
        outputs_of(self.node)
    }

    #[must_use]
    pub fn on_success(&self) -> Vec<ActionView<'d>> {
        actions_of(self.node, "onSuccess")
    }

    #[must_use]
    pub fn on_failure(&self) -> Vec<ActionView<'d>> {
        actions_of(self.node, "onFailure")
    }
}

impl ParameterView<'_> {
    #[must_use]
    pub fn node(&self) -> NodeRef<'_> {
        self.node
    }

    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.node.get("name").and_then(|n| n.as_str())
    }

    #[must_use]
    pub fn location(&self) -> Option<&str> {
        self.node.get("in").and_then(|n| n.as_str())
    }

    /// Reusable-parameter reference from `$components.parameters`.
    #[must_use]
    pub fn reference(&self) -> Option<&str> {
        self.node.get("reference").and_then(|n| n.as_str())
    }

    #[must_use]
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
    pub fn node(&self) -> NodeRef<'_> {
        self.node
    }

    #[must_use]
    pub fn condition(&self) -> Option<&str> {
        self.node.get("condition").and_then(|n| n.as_str())
    }

    #[must_use]
    pub fn context(&self) -> Option<&str> {
        self.node.get("context").and_then(|n| n.as_str())
    }

    #[must_use]
    pub fn criterion_type(&self) -> Option<&str> {
        self.node.get("type").and_then(|n| n.as_str())
    }
}

impl ActionView<'_> {
    #[must_use]
    pub fn node(&self) -> NodeRef<'_> {
        self.node
    }

    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.node.get("name").and_then(|n| n.as_str())
    }

    #[must_use]
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
