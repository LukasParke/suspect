//! Public admission review: what the shared admission layer says about a
//! document, without generating anything.
//!
//! This is the read-only front door to the same source-level admission the
//! backends run before emitting artifacts: shared HTTP admission
//! ([`crate::http_contract::plan`]), incoming webhook/callback admission,
//! contract-level limitations, and cross-convention naming-collision
//! analysis. Consumers (CLI pre-flight, the LSP, CI jobs) get one
//! structured report instead of inferring verdicts from per-backend
//! generation failures.
//!
//! Findings carry stable codes, located byte ranges, and per-code fix
//! guidance, mirroring the guidance discipline of the validation battery.

use std::collections::BTreeMap;
use std::ops::Range;

use suspect_ir::contract::{Contract, ContractSeverity, SourceId};

/// How hard a finding blocks generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingKind {
    /// Generation refuses the operation or document; backends return this
    /// as their failure list.
    Refusal,
    /// The document compiles, but the finding predicts degraded or
    /// collision-prone output.
    Advice,
    /// A contract-level limitation that is retained with a warning
    /// (uninterpreted annotations).
    Contract,
}

/// Stable one-line summary for an admission code.
fn summary(code: &str) -> &'static str {
    match code {
        "naming-method-collision" => "Operations whose method names collide",
        "naming-model-collision" => "Schemas whose model names collide",
        "http-operation-id" => "Operation without a usable operationId",
        "http-operation-id-duplicate" => "Duplicate operationId",
        "http-operation-not-found" => "Requested operation does not exist",
        "http-operation-object" => "Operation value is not an object",
        "http-method-unsupported" => "HTTP method not supported by the profile",
        "http-method-token" => "Unrecognized HTTP method token",
        "http-parameters-metadata-invalid" => "Parameters collection is not an array",
        "http-parameter-metadata-invalid" => "Parameter value is not an object",
        "http-body-metadata-invalid" => "Request body value is not an object",
        "http-body-schema-invalid" => "Request body schema is not a schema",
        "http-response-metadata-invalid" => "Response entry is not an object",
        "http-responses-metadata-invalid" => "Responses value is not an object",
        "http-content-metadata-invalid" => "Content value is not an object",
        "http-schema-metadata-invalid" => "Schema value is not an object or boolean",
        "http-security-metadata-invalid" => "Security scheme value is not an object",
        "http-server-metadata-invalid" => "Server value is not an object",
        _ => "",
    }
}

/// Actionable fix guidance for an admission code; empty when the located
/// message already carries the specifics.
fn how_to_fix(code: &str) -> &'static str {
    match code {
        "naming-method-collision" => {
            "Rename one of the operations so the generated method names differ in every convention."
        }
        "naming-model-collision" => {
            "Rename one of the schemas so the generated model names differ in every convention."
        }
        "http-operation-id" => {
            "Add a non-empty `operationId` to every operation the profile selects."
        }
        "http-operation-id-duplicate" => {
            "Make every selected operationId unique; generated method names depend on it."
        }
        "http-operation-not-found" => {
            "Select operations that exist as outgoing path operations in the document."
        }
        "http-method-unsupported" => {
            "Restructure the operation onto GET, POST, PATCH, PUT, or DELETE."
        }
        "http-parameters-metadata-invalid" => {
            "Make `parameters` an array of Parameter Objects or references."
        }
        "http-schema-metadata-invalid" => {
            "Replace the value with a Schema Object or a boolean schema."
        }
        _ => "",
    }
}

/// One admission finding: stable code, located range, message, and fix
/// guidance.
#[derive(Debug, Clone)]
pub struct AdmissionFinding {
    /// Stable machine-readable code (e.g. `http-operation-id`).
    pub code: &'static str,
    /// The finding kind: refusal, advice, or retained-annotation warning.
    pub kind: FindingKind,
    /// Human-readable message.
    pub message: String,
    /// Stable one-line summary of what the code means.
    pub summary: &'static str,
    /// Actionable instruction for resolving the finding.
    pub how_to_fix: &'static str,
    /// Source identity of the offending value, when located.
    pub source: Option<SourceId>,
    /// Byte range of the offending value in its document.
    pub at: Range<usize>,
}

/// One operation that passed shared admission.
#[derive(Debug, Clone)]
pub struct AdmittedOperation {
    /// Stable source identity of the operation.
    pub source: SourceId,
    /// Declared `operationId`, or the empty string when unnamed (such
    /// operations are selected as `METHOD /path`).
    pub operation_id: String,
    /// HTTP method.
    pub method: String,
    /// Path template.
    pub path: String,
}

/// The complete admission verdict for a document.
#[derive(Debug, Clone)]
pub struct AdmissionReport {
    /// Outgoing operations that passed shared admission, in document order.
    pub operations: Vec<AdmittedOperation>,
    /// Every admission finding, sorted by `(document, range, code)`.
    pub findings: Vec<AdmissionFinding>,
}

impl AdmissionReport {
    /// Whether no [`FindingKind::Refusal`] findings exist.
    #[must_use]
    pub fn is_admissible(&self) -> bool {
        !self.findings.iter().any(|f| f.kind == FindingKind::Refusal)
    }

    /// Whether the report carries no findings at all.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
}

/// Reviews every outgoing operation plus the document-wide admission
/// surfaces (incoming webhooks/callbacks, contract limitations, naming).
#[must_use]
pub fn review(contract: &Contract) -> AdmissionReport {
    let selected: Vec<SourceId> = contract.operations().map(|o| o.source().clone()).collect();
    review_selected(contract, &selected)
}

/// Reviews exactly `selected` outgoing operations plus the document-wide
/// admission surfaces. Unknown sources produce `http-operation-not-found`.
#[must_use]
pub fn review_selected(contract: &Contract, selected: &[SourceId]) -> AdmissionReport {
    let mut findings: Vec<AdmissionFinding> = Vec::new();
    let mut operations = Vec::new();

    // Shared source-level admission for the selected outgoing operations.
    let admitted = crate::http_contract::plan(contract, selected);
    let admitted = match admitted {
        Ok(admitted) => admitted,
        Err(errors) => {
            findings.extend(errors.into_iter().map(refusal));
            crate::http_contract::HttpContract {
                operations: Vec::new(),
                roots: Vec::new(),
            }
        }
    };
    for op in &admitted.operations {
        operations.push(AdmittedOperation {
            source: op.source.clone(),
            operation_id: op.operation_id.clone(),
            method: op.method.clone(),
            path: op.path.clone(),
        });
    }

    // Incoming webhooks and callbacks: selection-independent admission.
    if let Err(errors) = crate::http_protocol::plan_incoming(contract) {
        findings.extend(errors.into_iter().map(refusal));
    }

    // Contract-level limitations: invalid references block lowering;
    // unrecognized annotations are retained with a warning.
    for d in contract.diagnostics() {
        findings.push(AdmissionFinding {
            code: d.code,
            kind: match d.severity {
                ContractSeverity::Error => FindingKind::Refusal,
                ContractSeverity::Warning => FindingKind::Contract,
            },
            message: d.message.clone(),
            summary: contract_summary(d.code),
            how_to_fix: contract_how_to_fix(d.code),
            source: Some(d.source.clone()),
            at: d.at.clone(),
        });
    }

    // Cross-convention naming analysis.
    findings.extend(naming_collisions(contract));

    findings.sort_by(|a, b| {
        (a.source.as_ref().map(|s| s.pointer()), a.at.start, a.code).cmp(&(
            b.source.as_ref().map(|s| s.pointer()),
            b.at.start,
            b.code,
        ))
    });

    AdmissionReport {
        operations,
        findings,
    }
}

fn refusal(d: crate::http_contract::HttpDiagnostic) -> AdmissionFinding {
    AdmissionFinding {
        code: d.code,
        kind: FindingKind::Refusal,
        message: d.message,
        summary: summary(d.code),
        how_to_fix: how_to_fix(d.code),
        source: Some(d.source),
        at: d.at,
    }
}

/// Folds an identifier the way every common SDK naming convention ends up:
/// only alphanumeric characters remain, case-folded. Two identifiers that
/// agree under this fold produce the same method or model name under
/// camel, Pascal, and snake conventions alike.
fn fold(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Groups operationIds and component schema names by folded identity and
/// reports each group beyond its first member.
fn naming_collisions(contract: &Contract) -> Vec<AdmissionFinding> {
    let mut findings = Vec::new();
    let mut methods: BTreeMap<String, (String, Range<usize>)> = BTreeMap::new();
    for op in contract
        .operations()
        .chain(contract.webhooks())
        .chain(contract.callbacks())
    {
        let Some(id) = op.operation_id().filter(|id| !id.is_empty()) else {
            continue;
        };
        let folded = fold(id);
        let span = contract.source_span(op.source()).unwrap_or(0..0);
        match methods.get(&folded) {
            Some((first, _)) if *first != id => {
                findings.push(AdmissionFinding {
                    code: "naming-method-collision",
                    kind: FindingKind::Advice,
                    message: format!(
                        "operationIds `{first}` and `{id}` collapse to the same method name under common SDK naming conventions",
                    ),
                    summary: summary("naming-method-collision"),
                    how_to_fix: how_to_fix("naming-method-collision"),
                    source: Some(op.source().clone()),
                    at: span,
                });
            }
            Some(_) => {}
            None => {
                methods.insert(folded, (id.to_owned(), span));
            }
        }
    }
    let mut models: BTreeMap<String, (String, Range<usize>)> = BTreeMap::new();
    for schema in contract.schemas() {
        let pointer = schema.id().pointer();
        let Some(name) = pointer.strip_prefix("/components/schemas/") else {
            continue;
        };
        // Nested pointers (`/components/schemas/A/properties/x`) are inline
        // schemas, not named component entries.
        if name.contains('/') {
            continue;
        }
        let folded = fold(name);
        let span = schema.span();
        match models.get(&folded) {
            Some((first, _)) if *first != name => {
                findings.push(AdmissionFinding {
                    code: "naming-model-collision",
                    kind: FindingKind::Advice,
                    message: format!(
                        "schema names `{first}` and `{name}` collapse to the same model name under common SDK naming conventions",
                    ),
                    summary: summary("naming-model-collision"),
                    how_to_fix: how_to_fix("naming-model-collision"),
                    source: Some(schema.id().clone()),
                    at: span,
                });
            }
            Some(_) => {}
            None => {
                models.insert(folded, (name.to_owned(), span));
            }
        }
    }
    findings
}

/// Contract-level limitation codes (from the contract compiler), mapped to
/// the same guidance discipline.
fn contract_summary(code: &str) -> &'static str {
    match code {
        "DUPLICATE_OPERATION_ID" => "Duplicate operationId",
        "AMBIGUOUS_OPERATION_MOUNT" => "Operations cannot be disambiguated by method and path",
        code if code.starts_with("DUPLICATE_") => "Duplicate declaration",
        code if code.starts_with("INVALID_") => "Contract semantics the compiler cannot interpret",
        code if code.starts_with("HTTP_REFERENCE") => "Reference does not resolve in the contract",
        code if code.starts_with("unsupported-") || code.starts_with("UNSUPPORTED_") => {
            "Semantics not supported by this compiler version"
        }
        code if code.starts_with("invalid-") => "Contract semantics the compiler cannot interpret",
        _ => "Contract limitation",
    }
}

fn contract_how_to_fix(code: &str) -> &'static str {
    match code {
        "DUPLICATE_OPERATION_ID" => {
            "Make every operationId unique; generated method names depend on it."
        }
        "AMBIGUOUS_OPERATION_MOUNT" => {
            "Give the operations unique `operationId`s or distinct method/path combinations."
        }
        code if code.starts_with("DUPLICATE_") => {
            "Remove the duplicated declaration so every (name, location) pair is unique."
        }
        code if code.starts_with("INVALID_") => {
            "Correct the value so the compiler can materialize it faithfully."
        }
        code if code.starts_with("HTTP_REFERENCE") => {
            "Point the reference at a schema inside the loaded documents."
        }
        code if code.starts_with("unsupported-") || code.starts_with("UNSUPPORTED_") => {
            "Restructure the construct, or annotate it as an extension the compiler may ignore."
        }
        code if code.starts_with("invalid-") => {
            "Correct the schema value so the compiler can materialize it faithfully."
        }
        _ => "",
    }
}
