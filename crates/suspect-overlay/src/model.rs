use suspect_low::{LowDoc, NodeRef, ValueKind};

use crate::error::OverlayError;

/// A parsed Overlay 1.0 document.
pub struct OverlayDoc<'d> {
    root: NodeRef<'d>,
    actions: Vec<ActionView<'d>>,
}

/// One action of an overlay.
pub struct ActionView<'d> {
    /// Raw JSONPath expression text.
    pub target: &'d str,
    /// Compiled query (None when the expression failed to parse — reported
    /// by [`validate_overlay`] / surfaced at apply time).
    pub parsed: Option<suspect_jsonpath::Path>,
    /// The `update` value node, when present.
    pub update: Option<NodeRef<'d>>,
    /// The `remove` flag.
    pub remove: bool,
    /// Optional human description.
    pub description: Option<&'d str>,
}

impl<'d> OverlayDoc<'d> {
    /// Parses and structurally validates an overlay document.
    ///
    /// # Errors
    /// [`OverlayError::NotAnObject`] / [`OverlayError::MissingField`] /
    /// [`OverlayError::InvalidTarget`] on malformed documents.
    pub fn parse(doc: &'d LowDoc) -> Result<Self, OverlayError> {
        let root = doc.root();
        if root.kind() != ValueKind::Object {
            return Err(OverlayError::NotAnObject);
        }
        let version = root.get("overlay").and_then(|n| n.as_str());
        if version.is_none() {
            return Err(OverlayError::MissingField { field: "overlay" });
        }
        if root.get("info").and_then(|i| i.get("title")).is_none() {
            return Err(OverlayError::MissingField { field: "info.title" });
        }
        if root.get("info").and_then(|i| i.get("version")).is_none() {
            return Err(OverlayError::MissingField { field: "info.version" });
        }
        let actions_node = root.get("actions").ok_or(OverlayError::MissingField { field: "actions" })?;
        if actions_node.kind() != ValueKind::Array {
            return Err(OverlayError::MissingField { field: "actions" });
        }
        let mut actions = Vec::new();
        for (i, item) in actions_node.items().into_iter().enumerate() {
            let target = item
                .get("target")
                .and_then(|n| n.as_str())
                .ok_or(OverlayError::InvalidAction { index: i, reason: "missing `target`".into() })?;
            let parsed = Some(
                suspect_jsonpath::Path::parse(target).map_err(|e| OverlayError::InvalidTarget {
                    index: i,
                    input: target.to_owned(),

                    reason: e.to_string(),
                })?,
            );
            let update = item.get("update");
            let remove = item.get("remove").and_then(|n| n.as_bool()).unwrap_or(false);
            if update.is_none() && !remove {
                return Err(OverlayError::InvalidAction {
                    index: i,
                    reason: "must set `update` or `remove: true`".into(),
                });
            }
            let description = item.get("description").and_then(|n| n.as_str());
            actions.push(ActionView { target, parsed, update, remove, description });
        }
        Ok(Self { root, actions })
    }

    #[must_use]
    /// The `overlay` version string from the document root, if present.
    pub fn version(&self) -> Option<&'d str> {
        self.root.get("overlay").and_then(|n| n.as_str())
    }

    #[must_use]
    /// Human-readable title from `info.title`, if present.
    pub fn title(&self) -> Option<&'d str> {
        self.root.get("info").and_then(|i| i.get("title")).and_then(|n| n.as_str())
    }

    #[must_use]
    /// The overlay's own revision from `info.version`, if present.
    pub fn overlay_version(&self) -> Option<&'d str> {
        self.root.get("info").and_then(|i| i.get("version")).and_then(|n| n.as_str())
    }

    /// Target document URI from `extends`, when declared.
    #[must_use]
    pub fn extends(&self) -> Option<&'d str> {
        self.root.get("extends").and_then(|n| n.as_str())
    }

    #[must_use]
    /// The parsed actions, in document order.
    pub fn actions(&self) -> &[ActionView<'d>] {
        &self.actions
    }

    #[must_use]
    /// The overlay document's root node.
    pub fn root(&self) -> NodeRef<'d> {
        self.root
    }
}

impl std::fmt::Debug for OverlayDoc<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OverlayDoc")
            .field("version", &self.version())
            .field("actions", &self.actions.len())
            .finish()
    }
}

/// Non-fatal structural findings (spec-conformance linting).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayDiagnostic {
    /// Stable machine-readable identifier (e.g. `empty-actions`).
    pub code: &'static str,
    /// Human-readable explanation of the finding.
    pub message: String,
}

/// Structural checks beyond hard errors: empty action lists, `update` paired
/// with `remove: true` (update is ignored per spec), non-string targets.
#[must_use]
pub fn validate_overlay(doc: &OverlayDoc<'_>) -> Vec<OverlayDiagnostic> {
    let mut out = Vec::new();
    if doc.actions().is_empty() {
        out.push(OverlayDiagnostic {
            code: "overlay-empty-actions",
            message: "`actions` must contain at least one value".into(),
        });
    }
    for (i, a) in doc.actions().iter().enumerate() {
        if a.remove && a.update.is_some() {
            out.push(OverlayDiagnostic {
                code: "overlay-update-with-remove",
                message: format!("action #{i}: `update` has no effect while `remove` is true"),
            });
        }
        if a.description.is_none() {
            out.push(OverlayDiagnostic {
                code: "overlay-action-missing-description",
                message: format!("action #{i}: `description` recommended"),
            });
        }
    }
    out
}
