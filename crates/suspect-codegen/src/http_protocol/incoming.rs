//! Provider-initiated operations: OpenAPI webhooks and incoming callbacks.
//!
//! HTTP semantics are INVERTED for incoming operations. For an outgoing client
//! operation the declared method is what WE send and the declared responses are
//! what the server returns. For a webhook or callback the declared method is
//! what the PROVIDER sends to us, and the declared responses are what OUR
//! handler returns. Planning still uses the shared operation machinery, but
//! every consumer of this module must read `method` as the receipt verb and
//! `responses` as the handler's constructed replies.
//!
//! Callback routes carry RFC 6570 runtime expressions (`{$request.body#/id}`).
//! Version 1 carries the declared expression string verbatim and flags it
//! `expression: true`; substituting those expressions at receipt time is a
//! runtime/framework concern, never a generation-time guess.
//!
//! Enumeration walks ALL `Webhook` and `Callback` nodes directly from the
//! Contract. Selection independence is the point: webhooks are document-level
//! in 3.1 and callbacks attach to operations that may never be selected, so a
//! callback on an unselected operation still compiles. Broken declarations
//! produce source-linked `sdk-incoming-*` diagnostics instead of silent skips;
//! constructs the outbound planner refuses reuse the same shared refusals.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::Value;
use suspect_ir::contract::{Contract, SchemaId, SourceId};

use super::planner::{Planner, method, representation_roots};
use super::{
    BodyPlan, Diagnostic, DiagnosticKind, HeaderPlan, MediaPlan, Method, ParameterLocation,
    ParameterPlan, Provenance, Representation, ResponsePlan, ResponseStatus, Severity,
};
use crate::http_contract::HttpDiagnostic;

/// Which declared collection produced one incoming operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum IncomingKind {
    /// A document-level `webhooks` entry (OAS 3.1+).
    Webhook,
    /// A `callbacks` entry attached to an operation (selected or not).
    Callback,
}

/// The declared receipt route, carried verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IncomingRoute {
    /// The webhook key, or the callback expression string, exactly as declared.
    pub(super) route: String,
    /// True when the route carries runtime expression syntax (`{...}`).
    pub(super) expression: bool,
}

/// Declared request facets of one incoming operation: the parameters the
/// provider sends and the body it delivers. The same `MediaPlan` machinery as
/// outgoing request bodies compiles these, so codecs and limits match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IncomingRequestPlan {
    pub(super) parameters: Vec<ParameterPlan>,
    pub(super) body: Option<BodyPlan>,
}

/// One declared handler response: the status our receipt returns and the media
/// used to construct it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IncomingResponsePlan {
    pub(super) status_key: String,
    pub(super) status: ResponseStatus,
    pub(super) media: Vec<MediaPlan>,
    pub(super) headers: Vec<HeaderPlan>,
}

/// One compiled incoming operation. `source` locates the declaration; the
/// explanations record every value-reducing v1 decision applied to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IncomingOperationPlan {
    pub(super) kind: IncomingKind,
    /// The webhook key, or `parentOperationId.callbackName`.
    pub(super) name: String,
    /// The method the PROVIDER sends.
    pub(super) method: Method,
    pub(super) route: IncomingRoute,
    pub(super) request: IncomingRequestPlan,
    pub(super) responses: Vec<IncomingResponsePlan>,
    pub(super) source: Provenance,
    pub(super) explanations: Vec<String>,
}

/// Complete generation-time incoming plan. `codec_roots` are the actual JSON/
/// text codec inputs of the incoming declarations (sorted, deduplicated) so a
/// backend can extend its codec table without re-deriving them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct IncomingPlan {
    pub(super) operations: Vec<IncomingOperationPlan>,
    #[serde(serialize_with = "super::model::serialize_roots")]
    pub(super) codec_roots: Vec<SchemaId>,
    #[serde(serialize_with = "super::model::serialize_roots")]
    pub(super) codec_schema_closure: Vec<SchemaId>,
}

macro_rules! incoming_getters {
    ($($name:ident : $ty:ty),* $(,)?) => { $(
        #[must_use]
        pub fn $name(&self) -> &$ty { &self.$name }
    )* };
}

impl IncomingRoute {
    incoming_getters!(route: String);
    #[must_use]
    pub fn expression(&self) -> bool {
        self.expression
    }
}

impl IncomingRequestPlan {
    incoming_getters!(parameters: Vec<ParameterPlan>, body: Option<BodyPlan>);
}

impl IncomingResponsePlan {
    incoming_getters!(status_key: String, media: Vec<MediaPlan>, headers: Vec<HeaderPlan>);
    #[must_use]
    pub fn status(&self) -> ResponseStatus {
        self.status
    }
}

impl IncomingOperationPlan {
    incoming_getters!(
        name: String,
        route: IncomingRoute,
        request: IncomingRequestPlan,
        responses: Vec<IncomingResponsePlan>,
        source: Provenance,
        explanations: Vec<String>,
    );
    #[must_use]
    pub fn kind(&self) -> IncomingKind {
        self.kind
    }
    /// The method the provider sends to this receipt.
    #[must_use]
    pub fn method(&self) -> &Method {
        &self.method
    }
}

impl IncomingPlan {
    incoming_getters!(operations: Vec<IncomingOperationPlan>);
    #[must_use]
    pub fn codec_roots(&self) -> &[SchemaId] {
        &self.codec_roots
    }
    /// Candidate-aware effective closure supplied by Contract.
    #[must_use]
    pub fn codec_schema_closure(&self) -> &[SchemaId] {
        &self.codec_schema_closure
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }
}

fn diagnostic(d: Diagnostic) -> HttpDiagnostic {
    HttpDiagnostic {
        source: d.source().source().clone(),
        at: d.source().span(),
        code: d.code(),
        message: d.message().to_owned(),
    }
}

/// Plan every declared webhook and callback against the complete planner
/// feature vocabulary. Broken declarations surface as `sdk-incoming-*` (or
/// shared `http-*`) errors; any error refuses the whole incoming plan.
///
/// # Errors
/// Source-linked diagnostics when any incoming declaration is malformed or
/// unrepresentable. No operations survive a refused plan.
pub fn plan_incoming(contract: &Contract) -> Result<IncomingPlan, Vec<HttpDiagnostic>> {
    let mut p = Planner::admission(contract);
    let root = SourceId::new(contract.entry().clone(), Default::default());

    // OAS 3.0 has no webhooks collection: the Contract index skips it, so the
    // declaration would otherwise vanish silently from every generated SDK.
    if contract.openapi_version().starts_with("3.0.")
        && contract
            .source(&root)
            .and_then(|value| value.get("webhooks"))
            .is_some_and(|value| !value.is_null())
    {
        p.unsupported(
            &root.child("webhooks"),
            "sdk-incoming-version",
            "webhooks require OpenAPI 3.1 or later; the declared collection is not represented",
        );
    }

    let mut operations = Vec::new();
    let mut used_names = BTreeMap::<String, SourceId>::new();
    for operation in contract.webhooks() {
        compile_incoming(
            &mut p,
            IncomingKind::Webhook,
            operation,
            &mut operations,
            &mut used_names,
        );
    }
    for operation in contract.callbacks() {
        compile_incoming(
            &mut p,
            IncomingKind::Callback,
            operation,
            &mut operations,
            &mut used_names,
        );
    }

    let mut roots = BTreeSet::new();
    for plan in &operations {
        incoming_roots(plan, &mut roots);
    }
    let roots: Vec<_> = roots.into_iter().collect();
    // The contract has already recorded unsupported schema semantics. As with
    // the outbound plan, only actual codec inputs widen the checked closure.
    let reachable = contract.effective_schema_closure(&roots);
    p.codec_resource_capabilities(&reachable);
    for diagnostic in contract
        .diagnostics()
        .iter()
        .filter(|d| contract.schema_diagnostic_applies(&reachable, d))
    {
        if diagnostic.code.starts_with("unsupported-")
            || diagnostic.code == "invalid-schema"
            || diagnostic.code == "invalid-reference"
            || diagnostic.code == "unknown-schema-keyword"
            || diagnostic.code == "AMBIGUOUS_OPENAPI_CONTEXT"
            || super::resource::resource_finding(diagnostic.code)
        {
            p.diagnostics.push(Diagnostic {
                source: super::SourceLocation {
                    source: diagnostic.source.clone(),
                    span: diagnostic.at.clone(),
                },
                code: diagnostic.code,
                severity: if diagnostic.severity == suspect_ir::contract::ContractSeverity::Error {
                    Severity::Error
                } else {
                    Severity::Warning
                },
                kind: if diagnostic.severity == suspect_ir::contract::ContractSeverity::Error {
                    DiagnosticKind::Unsupported
                } else {
                    DiagnosticKind::Annotation
                },
                message: diagnostic.message.clone(),
                capability: None,
                related: p.related(&diagnostic.source),
                resource_context: p.resource_context(&diagnostic.source),
            });
        }
    }
    // A definition may have been encountered before its last incoming use.
    let related: Vec<_> = p
        .diagnostics
        .iter()
        .map(|d| p.related(&d.source.source))
        .collect();
    for (diagnostic, related) in p.diagnostics.iter_mut().zip(related) {
        diagnostic.related.extend(related);
        diagnostic.related.sort_by(|a, b| a.source.cmp(&b.source));
        diagnostic.related.dedup_by(|a, b| a.source == b.source);
    }
    p.diagnostics.sort_by(|a, b| {
        (&a.source.source, a.source.span.start, a.code).cmp(&(
            &b.source.source,
            b.source.span.start,
            b.code,
        ))
    });
    p.diagnostics
        .dedup_by(|a, b| a.source == b.source && a.code == b.code && a.capability == b.capability);
    let admitted = !p.diagnostics.iter().any(|d| d.severity == Severity::Error);
    if !admitted {
        return Err(p.diagnostics.into_iter().map(diagnostic).collect());
    }
    Ok(IncomingPlan {
        operations,
        codec_roots: roots,
        codec_schema_closure: reachable,
    })
}

fn compile_incoming<'a>(
    p: &mut Planner<'a>,
    kind: IncomingKind,
    operation: suspect_ir::contract::Operation<'a>,
    operations: &mut Vec<IncomingOperationPlan>,
    used_names: &mut BTreeMap<String, SourceId>,
) {
    let source = operation.source().clone();
    let Some(raw) = p.object(&source, "operation") else {
        return;
    };
    p.known(
        &source,
        raw,
        &[
            "operationId",
            "summary",
            "description",
            "tags",
            "externalDocs",
            "deprecated",
            "parameters",
            "requestBody",
            "responses",
            "callbacks",
            "servers",
            "security",
        ],
    );
    p.validate_path_item(operation.path_item_source());
    let Some(method) = method(operation.method().as_str()) else {
        p.unsupported(
            &source,
            "http-method-unsupported",
            "method is not represented by the shared protocol descriptors",
        );
        return;
    };
    // Version-field semantics stay shared with the outbound planner.
    if method.is_custom() && !p.is_32(&source) {
        p.unsupported(
            &source,
            "http-version-field",
            "additionalOperations requires OAS 3.2",
        );
    }
    if method == Method::Query && !p.is_32(&source) {
        p.unsupported(
            &source,
            "http-version-field",
            "the QUERY Operation Object field requires OAS 3.2",
        );
    }
    if method.as_str() == "CONNECT" {
        p.unsupported(&source, "http-connect-tunnel-unsupported", "CONNECT requires authority-form targets and a tunnel transport; it is not an ordinary path/body HTTP operation");
    }
    let _operation_id = p.string(&source, "operationId", false);
    if raw.contains_key("requestBody") {
        if method == Method::Trace {
            p.error(
                &source.child("requestBody"),
                "http-request-body-method",
                "HTTP TRACE requests must not contain a request body",
            );
        } else if p.is_30(&source) && matches!(method, Method::Get | Method::Head | Method::Delete)
        {
            p.unsupported(&source.child("requestBody"), "http-request-body-version", "OAS 3.0 ignores requestBody where HTTP body semantics are undefined; this planner will not activate that declaration");
        }
    }
    let mut explanations = Vec::new();
    if raw.contains_key("servers") {
        explanations.push(
            "declared servers describe the provider's call context and are not compiled into v1 receipt helpers".into(),
        );
    }
    if raw.contains_key("security") {
        explanations.push(
            "declared security describes how the provider authenticates its call; receipt authorization policy is a runtime/framework concern and is not compiled in v1".into(),
        );
    }
    let parameters = super::parameters::plan(p, &operation);
    let body = super::bodies::plan(p, &source);
    let responses = super::responses::plan(p, &source);
    for parameter in &parameters {
        if parameter.location() != ParameterLocation::Header {
            let location = match parameter.location() {
                ParameterLocation::Path => "path",
                ParameterLocation::Query => "query",
                ParameterLocation::Querystring => "querystring",
                ParameterLocation::Header => "header",
                ParameterLocation::Cookie => "cookie",
            };
            explanations.push(format!(
                "the declared {location} parameter {:?} is recorded but not validated by the v1 receipt helpers; only header parameters are checked",
                parameter.name()
            ));
        }
    }

    let route_text = match kind {
        IncomingKind::Webhook => operation.webhook_name().unwrap_or_default().to_owned(),
        IncomingKind::Callback => operation
            .callback_expression()
            .unwrap_or_default()
            .to_owned(),
    };
    if route_text.is_empty() {
        p.error(
            &source,
            "sdk-incoming-name",
            "the incoming operation has an empty receipt route; a webhook key or callback expression is required",
        );
        return;
    }
    // Runtime expression syntax is a `{...}` occurrence; fixed literal paths
    // carry no braces. v1 never substitutes either form.
    let expression = route_text.contains('{');
    if expression {
        explanations.push(
            "the declared route carries runtime expressions; v1 carries the expression string verbatim and its substitution is a runtime/framework concern".into(),
        );
    }
    let callback_name = match kind {
        IncomingKind::Webhook => None,
        IncomingKind::Callback => Some(operation.callback_name().unwrap_or_default().to_owned()),
    };
    if let Some(callback_name) = &callback_name
        && callback_name.is_empty()
    {
        p.error(
            &source,
            "sdk-incoming-name",
            "the callback expression key is empty; a named callback entry is required",
        );
        return;
    }
    let parent_id = match kind {
        IncomingKind::Webhook => None,
        IncomingKind::Callback => operation
            .callback_parent()
            .and_then(|parent| contract_operation_id(p.contract, parent)),
    };
    let base = match kind {
        IncomingKind::Webhook => route_text.clone(),
        IncomingKind::Callback => {
            let parent = parent_id.clone().unwrap_or_else(|| {
                operation
                    .operation_id()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{} receipt", method.as_str().to_ascii_lowercase()))
            });
            format!("{parent}.{}", callback_name.as_deref().unwrap_or_default())
        }
    };
    // Multiple receipt methods may share one webhook key or callback entry
    // (challenge GETs beside delivery POSTs). The first keeps the plain name;
    // later ones carry a method suffix so every receipt stays addressable.
    let mut name = base.clone();
    if let Some(previous) = used_names.get(&name) {
        if previous != &source {
            name = format!("{}.{}", base, method.as_str().to_ascii_lowercase());
            explanations.push(format!(
                "another receipt already uses the name {base:?}; this one carries a method suffix"
            ));
            if let Some(previous) = used_names.insert(name.clone(), source.clone())
                && previous != source
            {
                p.error(
                        &source,
                        "sdk-incoming-name",
                        format!("two incoming operations compiled to the same name {name:?}; receipts cannot be distinguished"),
                    );
                return;
            }
        }
    } else {
        used_names.insert(name.clone(), source.clone());
    }

    let responses: Vec<IncomingResponsePlan> = responses
        .into_iter()
        .map(|response: ResponsePlan| IncomingResponsePlan {
            status_key: response.status_key().to_owned(),
            status: response.status(),
            media: response.media().to_vec(),
            headers: response.headers().to_vec(),
        })
        .collect();

    // Provenance mirrors the outbound planner: use-site at the operation,
    // with reference chains attached through the path item when they differ.
    let mut origin = p.inline(&source);
    let inline_operation_source = match &method {
        Method::Custom(token) => operation
            .path_item_source()
            .child("additionalOperations")
            .child(token),
        _ => operation
            .path_item_source()
            .child(&method.as_str().to_ascii_lowercase()),
    };
    if &inline_operation_source != operation.source() {
        origin.use_site = p.location(operation.path_item_source());
        origin.use_site_resource = p.resource_context(operation.path_item_source());
        if let Some(path_origin) = p.resolve(operation.path_item_source(), true) {
            origin.references = path_origin.references;
            origin.reference_resources = path_origin.reference_resources;
        }
    }

    operations.push(IncomingOperationPlan {
        kind,
        name,
        method,
        route: IncomingRoute {
            route: route_text,
            expression,
        },
        request: IncomingRequestPlan { parameters, body },
        responses,
        source: origin,
        explanations,
    });
}

/// The parent operation's `operationId`, when it declares a non-empty one.
fn contract_operation_id(contract: &Contract, source: &SourceId) -> Option<String> {
    contract
        .source(source)
        .and_then(|raw| raw.get("operationId"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Actual codec inputs of one incoming declaration. The same rule as the
/// outbound plan: body-less statuses and binary representations contribute no
/// JSON codec roots.
fn incoming_roots(plan: &IncomingOperationPlan, roots: &mut BTreeSet<SourceId>) {
    for parameter in &plan.request.parameters {
        roots.insert(parameter.codec().schema().id().clone());
        if let Some(media) = parameter.content_media() {
            representation_roots(media.representation(), roots);
        }
    }
    if let Some(body) = &plan.request.body {
        for media in body.media() {
            representation_roots(media.representation(), roots);
        }
    }
    for response in &plan.responses {
        if plan.method.as_str() != "HEAD"
            && !matches!(
                response.status,
                ResponseStatus::Exact(100..=199 | 204 | 205 | 304) | ResponseStatus::Range(1)
            )
        {
            for media in &response.media {
                representation_roots(media.representation(), roots);
            }
        }
        for header in &response.headers {
            roots.insert(header.codec().schema().id().clone());
        }
    }
}

/// Whether one representation admits the v1 receipt decode: JSON (with or
/// without a codec), or opaque bytes. Form/multipart/stream receipts need
/// native part/framing capabilities the helpers do not have yet.
pub fn admits_receipt_decode(media: &MediaPlan) -> bool {
    matches!(
        media.representation(),
        Representation::Json { .. } | Representation::Binary { .. }
    )
}
