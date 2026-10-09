//! `suspect/renderSdk`: render a native SDK from the live editor state.
//!
//! The incremental reparse keeps the open document's tree current on every
//! keystroke; this request compiles the live buffer (scoped to the
//! requested operations, when any) and renders the requested backend —
//! so a preview of generated code tracks unsaved edits. Artifacts come
//! back as JSON; the client diffs them against what it has.

use serde::{Deserialize, Serialize};
use suspect_codegen::backend::{Backend, GenerationOptions, TargetConfig};
use suspect_ir::contract::{Contract, OperationSelection};

/// Params for `suspect/renderSdk`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderSdkParams {
    /// The document to generate from (open documents use the live buffer).
    pub uri: String,
    /// The native profile id (`typescript-http`, …).
    pub profile: String,
    /// Package identity for the generated SDK.
    pub package_name: String,
    /// Package version for the generated SDK.
    pub package_version: String,
    /// Explicit import/module identity where the profile needs one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_name: Option<String>,
    /// Exact operationId selectors; empty attempts every operation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operation_ids: Vec<String>,
}

/// One rendered artifact, portable relative path + full text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderedArtifact {
    /// Output-relative artifact path.
    pub path: String,
    /// Full artifact text.
    pub content: String,
}

/// Result for `suspect/renderSdk`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderSdkResult {
    /// Whether rendering produced artifacts (false = planning refused).
    pub rendered: bool,
    /// Every rendered artifact, in output order.
    pub artifacts: Vec<RenderedArtifact>,
    /// Planning diagnostics (codes and messages, no secret material).
    pub diagnostics: Vec<RenderDiagnostic>,
    /// How many operations the render covered.
    pub operations: usize,
}

/// One planning diagnostic, for client presentation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderDiagnostic {
    /// Stable machine-readable code.
    pub code: String,
    /// Human-readable message.
    pub message: String,
}

/// Renders `params`' backend from `contract`, scoped to the selected
/// operations when any are named. Pure: the workspace/compile side is the
/// caller's; this is the shared rendering core.
///
/// # Errors
/// Never: refusals come back as `rendered: false` plus diagnostics.
#[must_use]
pub fn render_sdk(contract: std::sync::Arc<Contract>, params: &RenderSdkParams) -> RenderSdkResult {
    let backend = Backend::ALL
        .iter()
        .copied()
        .find(|b| b.name() == params.profile);
    let Some(backend) = backend else {
        return RenderSdkResult {
            rendered: false,
            artifacts: Vec::new(),
            diagnostics: vec![RenderDiagnostic {
                code: "sdk-profile-unknown".to_owned(),
                message: format!("unknown profile {}", params.profile),
            }],
            operations: 0,
        };
    };
    let selected: Vec<_> = if params.operation_ids.is_empty() {
        contract
            .operations()
            .map(|op| op.source().clone())
            .collect()
    } else {
        let selection = OperationSelection::new(params.operation_ids.iter().map(String::as_str));
        contract
            .operations()
            .filter(|op| {
                selection.keeps_operation(
                    op.operation_id(),
                    op.method().as_str(),
                    op.path_template().unwrap_or_default(),
                )
            })
            .map(|op| op.source().clone())
            .collect()
    };
    let target = TargetConfig {
        backend,
        package_name: params.package_name.clone(),
        package_version: params.package_version.clone(),
        import_name: params.import_name.clone(),
    };
    match suspect_codegen::backend::generate_with_options(
        contract.clone(),
        &selected,
        &target,
        &GenerationOptions::default(),
    ) {
        Ok(files) => RenderSdkResult {
            rendered: true,
            operations: selected.len(),
            artifacts: files
                .into_iter()
                .map(|file| RenderedArtifact {
                    path: file.path,
                    content: file.content,
                })
                .collect(),
            diagnostics: Vec::new(),
        },
        Err(errors) => RenderSdkResult {
            rendered: false,
            operations: selected.len(),
            artifacts: Vec::new(),
            diagnostics: errors
                .iter()
                .map(|error| RenderDiagnostic {
                    code: error.code.to_string(),
                    message: error.message.clone(),
                })
                .collect(),
        },
    }
}

/// Marker type naming the `suspect/renderSdk` custom request.
pub enum RenderSdkRequest {
    /// Never constructed; the enum exists only to name the request type.
    #[allow(dead_code)]
    Marker,
}

impl tower_lsp::lsp_types::request::Request for RenderSdkRequest {
    type Params = RenderSdkParams;
    type Result = RenderSdkResult;
    const METHOD: &'static str = "suspect/renderSdk";
}
