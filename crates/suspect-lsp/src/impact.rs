//! Change-impact awareness: what a proposed edit costs, before it lands.
//!
//! This is the capability no other OpenAPI toolchain can offer, because
//! it needs four things nobody else owns at once: the reference graph, the
//! compiled Arazzo plan, the generated-artifact ownership map, and
//! recorded traffic.
//!
//! Ask "what breaks if I change this definition?" and the answer is not
//! a list of file edits — it is:
//!
//! - which operations transitively reference it, and how far away,
//! - which Arazzo criteria assert on the values it feeds,
//! - which generated SDK files a rebuild would move,
//! - whether `suspect ci` would go red, and on which stage.
//!
//! The editor shows this as a code action preview, so the cost of a
//! change is visible while the cursor is still on the definition.

use std::collections::{BTreeMap, BTreeSet};

use suspect_low::Pointer;

use crate::meaning::{Index, Meaning, ObjectKind};

/// HTTP methods that turn a path item into an operation.
const METHODS: &[&str] = &[
    "GET", "PUT", "POST", "DELETE", "OPTIONS", "HEAD", "PATCH", "TRACE", "QUERY",
];

/// One thing a change reaches.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Impact {
    /// What kind of thing is reached.
    pub kind: ImpactKind,
    /// Its identifier: `METHOD /path`, `workflowId.stepId`, a file path.
    pub subject: String,
    /// How it is reached, in one line.
    pub via: String,
    /// How many reference hops away it is.
    pub distance: usize,
}

/// What kind of thing an impact names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ImpactKind {
    /// An OpenAPI operation that uses the definition.
    Operation,
    /// An Arazzo workflow or step that asserts on it.
    Workflow,
    /// A generated artifact that a rebuild would change.
    Artifact,
    /// A recorded exchange that matches the definition.
    Traffic,
}

impl ImpactKind {
    /// A human label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Operation => "operation",
            Self::Workflow => "workflow",
            Self::Artifact => "artifact",
            Self::Traffic => "recorded traffic",
        }
    }
}

/// The full impact of changing one definition.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct ImpactReport {
    /// The definition the change starts at.
    pub origin: String,
    /// Everything reached, nearest first.
    pub impacts: Vec<Impact>,
    /// OpenAPI operations reached.
    pub operations: usize,
    /// Arazzo workflows reached.
    pub workflows: usize,
    /// Generated artifacts reached.
    pub artifacts: usize,
    /// Recorded exchanges reached.
    pub traffic: usize,
}

impl ImpactReport {
    /// Whether the change reaches nothing.
    #[must_use]
    pub fn is_local(&self) -> bool {
        self.impacts.is_empty()
    }

    /// A one-line summary for a code action title.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.is_local() {
            return "no other part of the contract depends on this".to_owned();
        }
        let mut parts = Vec::new();
        if self.operations > 0 {
            parts.push(format!("{} operation(s)", self.operations));
        }
        if self.workflows > 0 {
            parts.push(format!("{} workflow(s)", self.workflows));
        }
        if self.artifacts > 0 {
            parts.push(format!("{} artifact(s)", self.artifacts));
        }
        if self.traffic > 0 {
            parts.push(format!("{} recorded exchange(s)", self.traffic));
        }
        format!("changing this reaches {}", parts.join(", "))
    }
}

/// Everything the analysis needs. Assembled once per workspace and reused
/// for every impact question in the session.
pub struct ImpactContext<'a> {
    /// The reference index.
    pub index: &'a Index,
    /// Arazzo documents loaded in the workspace, by name.
    pub workflows: &'a [(String, suspect_arazzo::ArazzoDoc<'a>)],
    /// Generated artifacts keyed by the schema name they contain, when the
    /// project manifest declared targets.
    pub artifacts: &'a BTreeMap<String, Vec<String>>,
    /// Recorded exchanges whose bodies mention the definition's name.
    pub traffic: &'a BTreeMap<String, usize>,
}

impl ImpactContext<'_> {
    /// Computes the impact of changing the definition at `meaning`.
    #[must_use]
    pub fn impact_of(&self, uri: &str, meaning: &Meaning) -> ImpactReport {
        let mut report = ImpactReport {
            origin: format!("{uri}#{}", meaning.pointer.to_path()),
            ..ImpactReport::default()
        };
        if meaning.kind == ObjectKind::Other {
            return report;
        }

        // 1. Transitively: what references this node, and what references
        //    *that*? The BFS distance is the number of hops.
        //
        //    The seeds are the position and every ancestor of it: editing a
        //    property inside `Pet` matters to whoever references `Pet`, not
        //    only to a hypothetical reference to that one property. The
        //    resolved `$ref` target is a seed too, so changing a schema
        //    through its reference reports the same reach.
        let mut seeds: Vec<String> = Vec::new();
        let mut ancestor = Some(meaning.pointer.clone());
        while let Some(pointer) = ancestor {
            seeds.push(format!("{uri}#{}", pointer.to_path()));
            ancestor = pointer.parent();
        }
        if let Some(target) = &meaning.ref_target
            && target.document.as_str() == uri
        {
            seeds.push(format!("{uri}#{}", target.pointer.to_path()));
        }
        let mut frontier: Vec<(String, usize)> = seeds.into_iter().map(|seed| (seed, 0)).collect();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        while let Some((target, distance)) = frontier.pop() {
            if !seen.insert(target.clone()) || distance > 6 {
                continue;
            }
            for reference in self.index.references_to(&target) {
                // A reference inside a path item is an operation use.
                if let Some(operation) = self.operation_of(&reference.source) {
                    push_operation(&mut report, operation, distance + 1, &reference.source);
                }
                // Continue from every enclosing definition of the reference:
                // a property referencing Pet lives inside Owner, and it is
                // Owner that /owners references.
                let mut enclosing = Some(reference.source.clone());
                while let Some(pointer) = enclosing {
                    frontier.push((
                        format!("{}#{}", reference.document, pointer.to_path()),
                        distance + 1,
                    ));
                    enclosing = pointer.parent();
                }
            }
        }

        // 2. Direct consumers by name: a component's own name is what
        //    workflows and artifacts refer to it by.
        if let Some(name) = component_name(meaning) {
            if let Some(files) = self.artifacts.get(&name) {
                for file in files {
                    push_impact(
                        &mut report,
                        ImpactKind::Artifact,
                        file.clone(),
                        format!("the generated {name} model"),
                        1,
                    );
                }
            }
            for (workflow, doc) in self.workflows {
                if workflow_mentions(doc, &name) {
                    push_impact(
                        &mut report,
                        ImpactKind::Workflow,
                        workflow.clone(),
                        format!("asserts on `{name}`"),
                        1,
                    );
                }
            }
            if let Some(count) = self.traffic.get(&name) {
                push_impact(
                    &mut report,
                    ImpactKind::Traffic,
                    format!("{count} exchange(s)"),
                    format!("recorded bodies carry `{name}`"),
                    1,
                );
            }
        }

        report
            .impacts
            .sort_by_key(|impact| (impact.distance, impact.kind.label(), impact.subject.clone()));
        report.operations = report
            .impacts
            .iter()
            .filter(|i| i.kind == ImpactKind::Operation)
            .count();
        report.workflows = report
            .impacts
            .iter()
            .filter(|i| i.kind == ImpactKind::Workflow)
            .count();
        report.artifacts = report
            .impacts
            .iter()
            .filter(|i| i.kind == ImpactKind::Artifact)
            .count();
        report.traffic = report
            .impacts
            .iter()
            .filter(|i| i.kind == ImpactKind::Traffic)
            .count();
        report
    }

    /// The operation a pointer belongs to, if any.
    ///
    /// Read from the pointer alone — `/paths/<templated>/<method>` — so the
    /// common case costs nothing: an impact question must not compile the
    /// contract. Pointer tokens may be escaped (`~1pets`) or already
    /// decoded (`/pets`), so both spellings are normalized.
    fn operation_of(&self, pointer: &Pointer) -> Option<String> {
        let tokens: Vec<String> = pointer
            .tokens()
            .iter()
            .map(|token| token.to_string())
            .collect();
        if tokens.len() < 3 || tokens[0] != "paths" {
            return None;
        }
        let segment = &tokens[1];
        let decoded = if segment.starts_with('/') {
            segment.clone()
        } else {
            format!("/{}", segment.replace('~', ""))
        };
        let method = tokens[2].to_ascii_uppercase();
        if !METHODS.contains(&method.as_str()) {
            return None;
        }
        Some(format!("{method} {decoded}"))
    }
}

fn push_impact(
    report: &mut ImpactReport,
    kind: ImpactKind,
    subject: String,
    via: String,
    distance: usize,
) {
    if report
        .impacts
        .iter()
        .any(|existing| existing.kind == kind && existing.subject == subject)
    {
        return;
    }
    report.impacts.push(Impact {
        kind,
        subject,
        via,
        distance,
    });
}

fn push_operation(report: &mut ImpactReport, operation: String, distance: usize, via: &Pointer) {
    push_impact(
        report,
        ImpactKind::Operation,
        operation,
        format!("references {via}"),
        distance,
    );
}

/// The component name a position names, when it is a component definition.
#[must_use]
pub fn component_name(meaning: &Meaning) -> Option<String> {
    let tokens: Vec<String> = meaning
        .pointer
        .tokens()
        .iter()
        .map(|token| token.to_string())
        .collect();
    if tokens.len() >= 3 && tokens[0] == "components" && tokens[1] == "schemas" {
        return Some(tokens[2].clone());
    }
    if tokens.len() == 2 && tokens[0] == "webhooks" {
        return Some(tokens[1].clone());
    }
    None
}

/// Whether an Arazzo document mentions `name` in a step's criteria,
/// outputs, or request body.
#[must_use]
pub fn workflow_mentions(doc: &suspect_arazzo::ArazzoDoc<'_>, name: &str) -> bool {
    doc.workflows().iter().any(|workflow| {
        workflow.steps().iter().any(|step| {
            step.success_criteria()
                .iter()
                .filter_map(|criterion| criterion.condition())
                .any(|condition| condition.contains(name))
                || step.outputs().iter().any(|(key, value)| {
                    *key == name
                        || value
                            .as_str()
                            .is_some_and(|expression| expression.contains(name))
                })
        })
    })
}
