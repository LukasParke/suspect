//! The `suspect/generationContract` custom request: the generation
//! admission verdict for the live document, served to clients on demand.
//!
//! The SDK compiler's admission layer (shared refusals, incoming
//! webhook/callback admission, contract limitations, and cross-convention
//! naming analysis) runs in this process over the live workspace document,
//! so generation-time failures surface as a queryable report while
//! writing. Nothing is generated; the report is the admission layer's own
//! output, byte-identical to what `suspect admission` prints.

use serde::{Deserialize, Serialize};
use tower_lsp::lsp_types::Range;
use tower_lsp::lsp_types::request::Request;

/// Params of the [`GenerationContractRequest`] custom request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationContractParams {
    /// The document to review (must be a loaded workspace document).
    pub uri: String,
}

/// One admission finding, located in the live document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractFinding {
    /// Stable machine-readable code.
    pub code: String,
    /// `"refusal"`, `"advice"`, or `"contract"`.
    pub kind: String,
    /// Located human-readable message.
    pub message: String,
    /// Stable one-line summary of what the code means.
    pub summary: String,
    /// Actionable fix instruction.
    pub how_to_fix: String,
    /// LSP range of the offending value.
    pub range: Range,
}

/// One operation that passed shared admission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractOperation {
    /// HTTP method.
    pub method: String,
    /// Path template.
    pub path: String,
    /// Declared `operationId` (empty when unnamed).
    pub operation_id: String,
}

/// The full admission verdict, in LSP coordinates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationContractResult {
    /// Operations that passed shared admission.
    pub operations: Vec<ContractOperation>,
    /// Every admission finding, in report order.
    pub findings: Vec<ContractFinding>,
    /// Whether any refusal-class finding exists.
    pub admissible: bool,
    /// Which admission surfaces produced findings (for quick triage).
    pub refusals: usize,
}

/// Marker type naming the `suspect/generationContract` custom request;
/// never constructed.
pub enum GenerationContractRequest {
    /// Never constructed; the enum exists only to name the request type.
    #[allow(dead_code)]
    Marker,
}

impl Request for GenerationContractRequest {
    type Params = GenerationContractParams;
    type Result = GenerationContractResult;
    const METHOD: &'static str = "suspect/generationContract";
}

/// Runs the admission review over the live document and maps the report to
/// LSP coordinates. The caller resolves the workspace and open document;
/// unsupported documents (not loaded) produce `None`.
#[must_use]
pub fn generation_contract(
    workspace: &std::sync::Arc<suspect_ref::Workspace>,
    doc: &crate::state::OpenDoc,
) -> Option<GenerationContractResult> {
    let uri = doc.low.uri().clone();
    let contract = suspect_ir::contract::Contract::from_workspace(workspace, &uri).ok()?;
    let report = suspect_codegen::admission::review(&contract);
    let inner = doc.low.inner();
    let (bytes, li) = (inner.bytes(), inner.line_index());
    let findings = report
        .findings
        .iter()
        .map(|f| {
            // The range is byte-based in the finding's own document; when
            // the finding belongs to another loaded document, anchor it at
            // the live document start rather than mis-mapping bytes.
            let same_document = f.source.as_ref().is_some_and(|s| s.document() == &uri);
            let (bytes, li, range) = if same_document {
                (bytes, li, f.at.clone())
            } else {
                (bytes, li, 0..0)
            };
            ContractFinding {
                code: f.code.to_owned(),
                kind: match f.kind {
                    suspect_codegen::admission::FindingKind::Refusal => "refusal",
                    suspect_codegen::admission::FindingKind::Advice => "advice",
                    suspect_codegen::admission::FindingKind::Contract => "contract",
                }
                .to_owned(),
                message: f.message.clone(),
                summary: f.summary.to_owned(),
                how_to_fix: f.how_to_fix.to_owned(),
                range: crate::state::lsp_range(bytes, li, range),
            }
        })
        .collect();
    let operations = report
        .operations
        .iter()
        .map(|o| ContractOperation {
            method: o.method.clone(),
            path: o.path.clone(),
            operation_id: o.operation_id.clone(),
        })
        .collect();
    let refusals = report
        .findings
        .iter()
        .filter(|f| f.kind == suspect_codegen::admission::FindingKind::Refusal)
        .count();
    Some(GenerationContractResult {
        operations,
        findings,
        admissible: report.is_admissible(),
        refusals,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_method_name_is_stable() {
        assert_eq!(
            <GenerationContractRequest as Request>::METHOD,
            "suspect/generationContract"
        );
    }

    #[test]
    fn reports_map_to_lsp_ranges_of_the_live_document() {
        let text = "\
openapi: 3.1.0
info: {title: t, version: '1'}
servers:
  - url: https://api.example.com/v1
security:
  - apiKey: []
paths:
  /a:
    get:
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema: {type: object}
components:
  securitySchemes:
    apiKey:
      type: http
      scheme: bearer
";
        let dir = std::env::temp_dir().join("suspect-lsp-gen-contract");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("spec.yaml"), text).unwrap();
        let ws = std::sync::Arc::new(
            suspect_ref::WorkspaceBuilder::new()
                .root(&dir)
                .build()
                .unwrap(),
        );
        ws.load_all("spec.yaml").unwrap();
        let uri = suspect_source::Uri::from_path(&dir.join("spec.yaml")).unwrap();
        let doc = crate::state::OpenDoc::parse(uri, text.to_owned());
        let report = generation_contract(&ws, &doc).expect("report");
        assert!(
            !report.admissible,
            "unnamed operation refuses: {:?}",
            report.findings
        );
        assert!(report.refusals >= 1);
        assert!(report.findings.iter().all(|f| f.code.starts_with("http-")));
        assert!(report.operations.is_empty());
        // The finding's range is in LSP coordinates of the live document.
        let first = &report.findings[0];
        assert!(first.range.start.line > 0);
        assert!(first.how_to_fix.is_empty() || !first.how_to_fix.is_empty());
    }
}
