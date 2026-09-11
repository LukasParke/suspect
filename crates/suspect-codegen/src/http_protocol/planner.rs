use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};
use suspect_ir::contract::{Contract, ContractSeverity, SchemaDialect, SourceId};

use super::*;

/// Plan exactly the selected outgoing operation sources against an adapter's
/// explicit capabilities. No artifact may be constructed unless `is_admitted()`
/// (or `into_result()`) succeeds. Selection is deterministic and deduplicated.
#[must_use]
pub fn plan(
    contract: &Contract,
    selected: &[SourceId],
    capabilities: Capabilities,
) -> ProtocolPlan {
    let mut p = Planner {
        contract,
        capabilities,
        diagnostics: Vec::new(),
        checked_schemas: BTreeSet::new(),
        checked_versions: BTreeSet::new(),
        reference_uses: BTreeMap::new(),
    };
    let root = SourceId::new(contract.entry().clone(), Default::default());
    if p.capabilities.adapter.trim().is_empty() {
        p.error(
            &root,
            "http-adapter-name",
            "capabilities require an identifiable adapter/profile",
        );
    }
    match contract
        .openapi_version()
        .split('.')
        .take(2)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["3", "0"] => {
            p.require(Capability::OpenApi30, &root.child("openapi"));
        }
        ["3", "1"] => {}
        ["3", "2"] => {
            p.require(Capability::OpenApi32, &root.child("openapi"));
        }
        _ => p.unsupported(
            &root.child("openapi"),
            "http-openapi-version",
            "HTTP protocol planning supports the OAS 3.0, 3.1 and 3.2 feature sets",
        ),
    }
    p.checked_versions.insert(root.clone());
    let wanted: BTreeSet<_> = selected.iter().cloned().collect();
    let mut available = BTreeMap::<_, Vec<_>>::new();
    let mut operation_ids = BTreeMap::<String, BTreeSet<SourceId>>::new();
    for operation in contract
        .operations()
        .chain(contract.callbacks())
        .chain(contract.webhooks())
    {
        if let Some(name) = operation.operation_id() {
            operation_ids
                .entry(name.to_owned())
                .or_default()
                .insert(operation.source().clone());
        }
    }
    for operation in contract.operations() {
        available
            .entry(operation.source().clone())
            .or_default()
            .push(operation);
    }
    let mut operations = Vec::new();
    for source in wanted {
        let Some(mounts) = available.get(&source) else {
            p.unsupported(&source, "http-operation-not-found", "selected source is not an indexed outgoing path operation; callbacks, webhooks and unindexed methods are not client operations");
            continue;
        };
        if mounts.len() != 1 {
            p.error(mounts[1].path_item_source(), "http-operation-mount-ambiguous", "one operation source is mounted at several paths; source-only selection cannot choose a wire path");
            continue;
        }
        let op = mounts[0];
        let Some(raw) = p.object(&source, "operation") else {
            continue;
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
        p.validate_path_item(op.path_item_source());
        let Some(method) = method(op.method().as_str()) else {
            p.unsupported(
                &source,
                "http-method-unsupported",
                "method is not represented by the shared protocol descriptors",
            );
            continue;
        };
        if method.is_custom() {
            p.require(Capability::CustomMethods, &source);
            if !p.is_32(&source) {
                p.unsupported(
                    &source,
                    "http-version-field",
                    "additionalOperations requires OAS 3.2",
                );
            }
            if method.as_str() == "CONNECT" {
                p.unsupported(&source, "http-connect-tunnel-unsupported", "CONNECT requires authority-form targets and a tunnel transport; it is not an ordinary path/body HTTP operation");
            }
        } else if !matches!(
            method,
            Method::Get | Method::Post | Method::Patch | Method::Put | Method::Delete
        ) {
            p.require(Capability::AdditionalMethods, &source);
        }
        if method == Method::Query && !p.is_32(&source) {
            p.unsupported(
                &source,
                "http-version-field",
                "the QUERY Operation Object field requires OAS 3.2",
            );
        }
        let operation_id = p.string(&source, "operationId", false);
        if let Some(id) = &operation_id
            && operation_ids
                .get(&id.value)
                .is_some_and(|sources| sources.len() > 1)
        {
            p.error(&id.source.source, "http-operation-id-duplicate", "operationId must be unique across the OpenAPI description; the source name is ambiguous");
        }
        if operation_id.as_ref().is_none_or(|id| id.value.is_empty()) {
            p.require(Capability::UnnamedOperations, &source);
        }
        let summary = p.string(&source, "summary", false);
        let description = p.string(&source, "description", false);
        let deprecated = p.boolean(&source, "deprecated", false);
        let tags = p.strings(&source, "tags", false).unwrap_or_default();
        if let Some(external) = raw.get("externalDocs") {
            let at = source.child("externalDocs");
            if external.is_object() {
                p.string(&at, "description", false);
                p.string(&at, "url", true);
            } else {
                p.error(
                    &at,
                    "http-metadata-object",
                    "externalDocs must be an object",
                );
            }
        }
        // Callbacks describe provider-initiated operations and remain in Contract.
        if let Some(callbacks) = raw.get("callbacks")
            && !callbacks.is_object()
        {
            p.error(
                &source.child("callbacks"),
                "http-metadata-object",
                "callbacks must be a map",
            );
        }
        let servers = super::servers::plan(&mut p, op.server_source(), &source);
        let security = super::security::plan(&mut p, op.security_source(), &source);
        let parameters = super::parameters::plan(&mut p, &op);
        if raw.contains_key("requestBody") {
            if method == Method::Trace {
                p.error(
                    &source.child("requestBody"),
                    "http-request-body-method",
                    "HTTP TRACE requests must not contain a request body",
                );
            } else if p.is_30(&source)
                && matches!(method, Method::Get | Method::Head | Method::Delete)
            {
                p.unsupported(&source.child("requestBody"), "http-request-body-version", "OAS 3.0 ignores requestBody where HTTP body semantics are undefined; this planner will not activate that declaration");
            }
        }
        let body = super::bodies::plan(&mut p, &source);
        let responses = super::responses::plan(&mut p, &source);
        let path = op.path_template().unwrap_or_default().to_owned();
        match super::servers::placeholders(&path) {
            Ok(names) => {
                let declared: BTreeSet<_> = parameters
                    .iter()
                    .filter(|v| v.location == ParameterLocation::Path)
                    .map(|v| v.name.as_str())
                    .collect();
                if names.into_iter().collect::<BTreeSet<_>>() != declared {
                    p.error(op.path_item_source(), "http-path-parameters", "path template expressions must exactly match the effective required path parameters");
                }
            }
            Err(reason) => p.error(op.path_item_source(), "http-path-template", reason),
        }
        if !path.starts_with('/') || path.contains(['?', '#']) {
            p.error(
                op.path_item_source(),
                "http-path-template",
                "an HTTP path must start with / and contain neither query nor fragment",
            );
        }
        super::security::validate_collisions(&mut p, &security, &parameters);
        let mut origin = p.inline(&source);
        let inline_operation_source = match &method {
            Method::Custom(token) => op
                .path_item_source()
                .child("additionalOperations")
                .child(token),
            _ => op
                .path_item_source()
                .child(&method.as_str().to_ascii_lowercase()),
        };
        if &inline_operation_source != op.source() {
            origin.use_site = p.location(op.path_item_source());
            origin.use_site_resource = p.resource_context(op.path_item_source());
            if let Some(path_origin) = p.resolve(op.path_item_source(), true) {
                origin.references = path_origin.references;
                origin.reference_resources = path_origin.reference_resources;
            }
        }
        let annotations = raw
            .iter()
            .filter(|(key, _)| key.starts_with("x-"))
            .map(|(key, value)| Located {
                source: p.location(&source.child(key)),
                value: value.clone(),
            })
            .collect();
        operations.push(OperationPlan {
            source: origin,
            path_item: p.location(op.path_item_source()),
            method,
            path,
            operation_id,
            summary,
            description,
            deprecated,
            tags,
            servers,
            security,
            parameters,
            body,
            responses,
            annotations,
        });
    }
    let mut roots = BTreeSet::new();
    for op in &operations {
        operation_roots(op, &mut roots);
    }
    // The contract has already recorded unsupported schema semantics. Restrict
    // those findings to actual codec inputs; unrelated components never widen
    // the selected operation's closure, and binary schemas never become JSON.
    let roots: Vec<_> = roots.into_iter().collect();
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
                source: SourceLocation {
                    source: diagnostic.source.clone(),
                    span: diagnostic.at.clone(),
                },
                code: diagnostic.code,
                severity: if diagnostic.severity == ContractSeverity::Error {
                    Severity::Error
                } else {
                    Severity::Warning
                },
                kind: if diagnostic.severity == ContractSeverity::Error {
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
    // A definition may have been encountered before its last selected use.
    // Complete the use-site links after traversing the whole selected plan.
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
    ProtocolPlan {
        version: PROTOCOL_PLAN_VERSION,
        capabilities: p.capabilities,
        operations: if admitted { operations } else { Vec::new() },
        codec_roots: if admitted { roots } else { Vec::new() },
        codec_schema_closure: if admitted { reachable } else { Vec::new() },
        diagnostics: p.diagnostics,
    }
}

pub(super) struct Planner<'a> {
    pub contract: &'a Contract,
    pub capabilities: Capabilities,
    pub diagnostics: Vec<Diagnostic>,
    checked_schemas: BTreeSet<SourceId>,
    checked_versions: BTreeSet<SourceId>,
    reference_uses: BTreeMap<SourceId, BTreeSet<SourceId>>,
}

impl<'a> Planner<'a> {
    pub fn location(&self, source: &SourceId) -> SourceLocation {
        // Missing-field diagnostics point to an existing container, never to a
        // fabricated schema/default node or an unrelated document's byte range.
        let mut at = source.clone();
        loop {
            if let Some(span) = self.contract.source_span(&at) {
                return SourceLocation { source: at, span };
            }
            let Some(parent) = parent(&at) else {
                return SourceLocation {
                    source: source.clone(),
                    span: 0..0,
                };
            };
            at = parent;
        }
    }
    pub fn inline(&self, source: &SourceId) -> Provenance {
        self.provenance(source, source, Vec::new())
    }
    /// Declaration shape errors must survive effective-parameter overrides.
    /// Reuse the canonical metadata validator without applying this adapter's
    /// wire capabilities to a valid declaration that has been overridden.
    pub fn validate_metadata_shape(&mut self, origin: &Provenance) {
        for error in self.contract.diagnostics().iter().filter(|d| {
            d.code == "INVALID_HTTP_SHAPE"
                && (contains_source(&origin.use_site.source, &d.source)
                    || contains_source(&origin.terminal.source, &d.source))
        }) {
            self.diagnostics.push(Diagnostic {
                source: SourceLocation {
                    source: error.source.clone(),
                    span: error.at.clone(),
                },
                code: "http-metadata-declaration",
                severity: Severity::Error,
                kind: DiagnosticKind::InvalidSource,
                message: error.message.clone(),
                capability: None,
                related: self.related(&error.source),
                resource_context: self.resource_context(&error.source),
            });
        }
    }
    pub fn error(&mut self, source: &SourceId, code: &'static str, message: impl Into<String>) {
        self.diagnostic(
            source,
            code,
            Severity::Error,
            DiagnosticKind::InvalidSource,
            message,
        );
    }
    pub fn unsupported(
        &mut self,
        source: &SourceId,
        code: &'static str,
        message: impl Into<String>,
    ) {
        self.diagnostic(
            source,
            code,
            Severity::Error,
            DiagnosticKind::Unsupported,
            message,
        );
    }
    pub fn warn(&mut self, source: &SourceId, code: &'static str, message: impl Into<String>) {
        self.diagnostic(
            source,
            code,
            Severity::Warning,
            DiagnosticKind::Annotation,
            message,
        );
    }
    fn diagnostic(
        &mut self,
        source: &SourceId,
        code: &'static str,
        severity: Severity,
        kind: DiagnosticKind,
        message: impl Into<String>,
    ) {
        self.diagnostics.push(Diagnostic {
            source: self.location(source),
            code,
            severity,
            kind,
            message: message.into(),
            capability: None,
            related: self.related(source),
            resource_context: self.resource_context(source),
        });
    }
    fn related(&self, source: &SourceId) -> Vec<SourceLocation> {
        let mut found = BTreeSet::new();
        let mut pending = vec![source.clone()];
        while let Some(id) = pending.pop() {
            let mut at = Some(id);
            while let Some(id) = at {
                if let Some(uses) = self.reference_uses.get(&id) {
                    for use_site in uses {
                        if use_site != source && found.insert(use_site.clone()) {
                            pending.push(use_site.clone());
                        }
                    }
                }
                at = parent(&id);
            }
        }
        found.iter().map(|id| self.location(id)).collect()
    }
    pub fn require(&mut self, capability: Capability, source: &SourceId) -> bool {
        if self.capabilities.supports(capability) {
            return true;
        }
        self.diagnostics.push(Diagnostic {
            source: self.location(source),
            code: "http-capability-required",
            severity: Severity::Error,
            kind: DiagnosticKind::Capability,
            message: format!(
                "adapter {:?} has not opted in to {capability:?}",
                self.capabilities.adapter
            ),
            capability: Some(capability),
            related: self.related(source),
            resource_context: self.resource_context(source),
        });
        false
    }
    pub fn version(&self, source: &SourceId) -> &str {
        self.contract.openapi_version_at(source)
    }
    pub fn is_32(&self, source: &SourceId) -> bool {
        self.version(source).starts_with("3.2.")
    }
    pub fn is_30(&self, source: &SourceId) -> bool {
        self.version(source).starts_with("3.0.")
    }

    pub fn object(&mut self, source: &SourceId, name: &str) -> Option<&'a Map<String, Value>> {
        self.guard_document_version(source);
        if let Some(diagnostic) =
            self.contract.diagnostics().iter().find(|d| {
                d.code == "AMBIGUOUS_OPENAPI_CONTEXT" && contains_source(&d.source, source)
            })
        {
            self.unsupported(&diagnostic.source, diagnostic.code, &diagnostic.message);
            return None;
        }
        match self.contract.source(source).and_then(Value::as_object) {
            Some(raw) => self.ensure_resource_scope(source).then_some(raw),
            None => {
                self.error(
                    source,
                    "http-metadata-object",
                    format!("{name} must be an object"),
                );
                None
            }
        }
    }
    fn guard_document_version(&mut self, source: &SourceId) {
        let root = SourceId::new(source.document().clone(), Default::default());
        if !self.checked_versions.insert(root.clone()) {
            return;
        }
        let version = self.contract.source(&root).and_then(|v| v.get("openapi"));
        match version.and_then(Value::as_str) {
            Some(version) if version.starts_with("3.0.") => {
                self.require(Capability::OpenApi30, &root.child("openapi"));
            }
            Some(version) if version.starts_with("3.2.") => {
                self.require(Capability::OpenApi32, &root.child("openapi"));
            }
            Some(version) if version.starts_with("3.1.") => {}
            Some(_) => self.unsupported(
                &root.child("openapi"),
                "http-openapi-version",
                "referenced HTTP document uses an unsupported OpenAPI feature set",
            ),
            None if version.is_some() => self.error(
                &root.child("openapi"),
                "http-openapi-version",
                "referenced document openapi version must be a string",
            ),
            None => {} // Fragment documents inherit the entry's supported version.
        }
    }
    pub fn known(&mut self, source: &SourceId, raw: &Map<String, Value>, fields: &[&str]) {
        for key in raw.keys() {
            if key.starts_with("x-") {
                self.warn(&source.child(key), "http-extension-uninterpreted", "extension is retained as annotation; no wire, authentication, pagination, retry or sentinel behavior is inferred");
            } else if !fields.contains(&key.as_str()) {
                self.error(
                    &source.child(key),
                    "http-field-unknown",
                    format!("unknown HTTP metadata field {key:?}"),
                );
            }
        }
    }
    pub fn string(
        &mut self,
        source: &SourceId,
        key: &str,
        required: bool,
    ) -> Option<Located<String>> {
        match self.contract.source(source).and_then(|v| v.get(key)) {
            Some(Value::String(s)) => Some(Located {
                source: self.location(&source.child(key)),
                value: s.clone(),
            }),
            Some(_) => {
                self.error(
                    &source.child(key),
                    "http-metadata-string",
                    format!("{key} must be a string"),
                );
                None
            }
            None if required => {
                self.error(
                    source,
                    "http-metadata-required",
                    format!("{key} is required"),
                );
                None
            }
            None => None,
        }
    }
    pub fn text(
        &mut self,
        origin: &Provenance,
        key: &str,
        required: bool,
    ) -> Option<Located<String>> {
        if matches!(key, "description" | "summary") {
            for hop in &origin.references {
                if !self.is_30(&hop.source)
                    && self
                        .contract
                        .source(&hop.source)
                        .is_some_and(|v| v.get(key).is_some())
                {
                    return self.string(&hop.source, key, required);
                }
            }
        }
        self.string(&origin.terminal.source, key, required)
    }
    pub fn boolean(&mut self, source: &SourceId, key: &str, default: bool) -> bool {
        match self.contract.source(source).and_then(|v| v.get(key)) {
            Some(Value::Bool(b)) => *b,
            Some(_) => {
                self.error(
                    &source.child(key),
                    "http-metadata-boolean",
                    format!("{key} must be a boolean"),
                );
                default
            }
            None => default,
        }
    }
    pub fn strings(
        &mut self,
        source: &SourceId,
        key: &str,
        required: bool,
    ) -> Option<Vec<Located<String>>> {
        let field = source.child(key);
        match self.contract.source(&field) {
            Some(Value::Array(values)) => {
                let mut result = Vec::new();
                for (i, value) in values.iter().enumerate() {
                    let at = field.child(&i.to_string());
                    if let Some(s) = value.as_str() {
                        result.push(Located {
                            source: self.location(&at),
                            value: s.to_owned(),
                        });
                    } else {
                        self.error(
                            &at,
                            "http-metadata-string",
                            format!("{key} entries must be strings"),
                        );
                    }
                }
                Some(result)
            }
            Some(_) => {
                self.error(
                    &field,
                    "http-metadata-array",
                    format!("{key} must be an array"),
                );
                None
            }
            None if required => {
                self.error(
                    source,
                    "http-metadata-required",
                    format!("{key} is required"),
                );
                None
            }
            None => None,
        }
    }
    pub fn integer(&mut self, source: &SourceId, key: &str) -> Option<Located<u64>> {
        match self.contract.source(source).and_then(|v| v.get(key)) {
            Some(value) => match finite_counter(value) {
                Some(n) => Some(Located {
                    source: self.location(&source.child(key)),
                    value: n,
                }),
                None => {
                    self.unsupported(&source.child(key), "http-bound-unsupported", format!("{key} must be a non-negative integer representable by the finite transport counter"));
                    None
                }
            },
            None => None,
        }
    }

    /// Only the canonical contract graph resolves references. No URL fetching,
    /// bundling, ref-string guessing, or synthetic schema registration occurs.
    pub fn resolve(&mut self, source: &SourceId, path_item: bool) -> Option<Provenance> {
        let mut current = source.clone();
        let mut seen = BTreeSet::new();
        let mut references = Vec::new();
        loop {
            if !seen.insert(current.clone()) {
                self.error(
                    &current,
                    "http-reference-cycle",
                    "HTTP reference must resolve to a finite object",
                );
                return None;
            }
            let raw = self.object(&current, "HTTP declaration/reference target")?;
            if let Some(reference) = raw.get("$ref") {
                if !reference.is_string() {
                    self.error(
                        &current.child("$ref"),
                        "http-reference-invalid",
                        "$ref must be a URI-reference string",
                    );
                    return None;
                }
                if !path_item {
                    for key in raw.keys().filter(|k| k.as_str() != "$ref") {
                        if !self.is_30(&current)
                            && matches!(key.as_str(), "description" | "summary")
                        {
                            self.string(&current, key, false);
                        } else {
                            self.warn(&current.child(key), "http-reference-sibling-ignored", "Reference Object siblings do not override the target's wire fields");
                        }
                    }
                }
                let Some(target) = self.contract.reference_target(&current) else {
                    let base = self
                        .contract
                        .resource_scope(&current)
                        .map(|scope| scope.base_uri())
                        .unwrap_or("<unresolved resource scope>");
                    let detail = match self.contract.resolve_resource_reference(
                        &current,
                        reference.as_str().expect("reference string checked"),
                    ) {
                        Ok(target) => format!(
                            "the URI locates {}#{} but there is no indexed semantic $ref edge",
                            target.document(),
                            target.pointer()
                        ),
                        Err(error) => format!("{}: {}", error.code(), error.message()),
                    };
                    self.unsupported(
                        &current.child("$ref"),
                        "http-reference-unresolved",
                        format!("reference {} from logical base {base:?} has no resolved Contract edge: {detail}", reference),
                    );
                    return None;
                };
                references.push(self.location(&current));
                self.reference_uses
                    .entry(target.clone())
                    .or_default()
                    .insert(current.clone());
                current = target.clone();
            } else {
                break;
            }
        }
        Some(self.provenance(source, &current, references))
    }

    pub fn schema(&mut self, source: &SourceId) -> Option<SchemaUse> {
        let Some(raw) = self.contract.source(source) else {
            self.error(
                &parent(source).unwrap_or_else(|| source.clone()),
                "http-schema-required",
                "a schema is required for this representation",
            );
            return None;
        };
        if !raw.is_object() && !raw.is_boolean() {
            self.error(
                source,
                "http-schema-invalid",
                "schema must be an object or a boolean allowed by its dialect",
            );
            return None;
        }
        if self.contract.schema(source).is_none() {
            self.unsupported(source, "http-schema-unindexed", "schema is not indexed by Contract; extend the canonical schema walk rather than inventing a codec root");
            return None;
        }
        let mut current = source.clone();
        let mut references = Vec::new();
        let mut seen = BTreeSet::new();
        while seen.insert(current.clone()) {
            let Some(schema) = self.contract.schema(&current) else {
                self.unsupported(
                    &current,
                    "http-schema-unindexed",
                    "referenced schema is absent from the canonical schema index",
                );
                return None;
            };
            if !self.ensure_resource_scope(&current) {
                return None;
            }
            if self.checked_schemas.insert(current.clone()) {
                match schema.dialect() {
                    SchemaDialect::OpenApi30 => {
                        self.require(Capability::OpenApi30, &current);
                        let additional = current.pointer().ends_with("/additionalProperties")
                            && parent(&current)
                                .is_some_and(|id| self.contract.schema(&id).is_some());
                        if schema.raw().is_boolean() && !additional {
                            self.error(&current, "http-schema-dialect", "boolean schemas require OAS 3.1 or later, except additionalProperties");
                        }
                    }
                    SchemaDialect::Uri(uri)
                        if !matches!(
                            uri.trim_end_matches('#'),
                            "https://spec.openapis.org/oas/3.1/dialect/base"
                                | "https://json-schema.org/draft/2020-12/schema"
                        ) =>
                    {
                        self.unsupported(
                            &current,
                            "http-schema-dialect",
                            format!("unsupported schema dialect {uri:?}"),
                        );
                    }
                    _ => {
                        if schema.raw().get("nullable").is_some() {
                            self.warn(&current.child("nullable"), "http-nullable-annotation", "nullable is an OAS 3.0 keyword, not a 3.1/3.2 null-type declaration; no wire null convention is inferred");
                        }
                    }
                }
            }
            if schema.raw().get("$ref").is_none() {
                break;
            }
            let Some(target) = schema
                .references()
                .iter()
                .find(|r| r.keyword == "$ref")
                .and_then(|r| r.target.as_ref())
            else {
                let reference = schema
                    .raw()
                    .get("$ref")
                    .and_then(Value::as_str)
                    .unwrap_or("<invalid $ref>");
                let base = self
                    .contract
                    .resource_scope(&current)
                    .map(|scope| scope.base_uri())
                    .unwrap_or("<unresolved resource scope>");
                let detail = self
                    .contract
                    .resolve_resource_reference(&current, reference)
                    .map(|target| {
                        format!(
                            "URI names {}#{} but its schema edge is unresolved",
                            target.document(),
                            target.pointer()
                        )
                    })
                    .unwrap_or_else(|error| format!("{}: {}", error.code(), error.message()));
                self.error(
                    &current.child("$ref"),
                    "http-schema-reference",
                    format!("schema reference {reference:?} from logical base {base:?} has no indexed target: {detail}"),
                );
                return None;
            };
            references.push(self.location(&current));
            self.reference_uses
                .entry(target.clone())
                .or_default()
                .insert(current.clone());
            current = target.clone();
        }
        Some(SchemaUse {
            id: source.clone(),
            source: self.provenance(source, &current, references),
        })
    }
    pub fn codec(&mut self, source: &SourceId, input: CodecInput) -> Option<CodecRef> {
        self.schema(source).map(|schema| CodecRef { schema, input })
    }
    pub fn validate_path_item(&mut self, source: &SourceId) {
        let Some(origin) = self.resolve(source, true) else {
            return;
        };
        let mut definitions = BTreeMap::<String, SourceId>::new();
        let chain = origin
            .references
            .iter()
            .chain(std::iter::once(&origin.terminal));
        for hop in chain.rev() {
            let Some(raw) = self.object(&hop.source, "Path Item") else {
                continue;
            };
            self.known(
                &hop.source,
                raw,
                &[
                    "$ref",
                    "summary",
                    "description",
                    "get",
                    "put",
                    "post",
                    "delete",
                    "options",
                    "head",
                    "patch",
                    "trace",
                    "query",
                    "additionalOperations",
                    "servers",
                    "parameters",
                ],
            );
            for (field, token) in FIXED_METHODS {
                if let Some(operation) = raw.get(field) {
                    if token == "QUERY" && !self.is_32(&hop.source) {
                        self.unsupported(
                            &hop.source.child(field),
                            "http-version-field",
                            "QUERY requires OAS 3.2",
                        );
                    }
                    if !operation.is_object() {
                        self.error(
                            &hop.source.child(field),
                            "http-operation-object",
                            "operation declarations must be objects",
                        );
                    }
                }
            }
            if let Some(additional) = raw.get("additionalOperations") {
                let at = hop.source.child("additionalOperations");
                if !self.is_32(&at) {
                    self.unsupported(
                        &at,
                        "http-version-field",
                        "additionalOperations requires OAS 3.2",
                    );
                }
                if let Some(map) = additional.as_object() {
                    for (token, value) in map {
                        let source = at.child(token);
                        if FIXED_METHODS.iter().any(|(_, fixed)| *fixed == token) {
                            self.error(&source, "http-additional-method-fixed", "additionalOperations must not use an uppercase fixed-method spelling, whether or not its fixed field is present");
                        } else if suspect_ir::contract::HttpMethod::parse(token).is_none() {
                            self.error(
                                &source,
                                "http-method-token",
                                "HTTP method names must be nonempty case-sensitive ASCII tokens",
                            );
                        }
                        if !value.is_object() {
                            self.error(
                                &source,
                                "http-operation-object",
                                "additional operation declarations must be objects",
                            );
                        }
                    }
                } else {
                    self.error(
                        &at,
                        "http-additional-methods-map",
                        "additionalOperations must be a token to Operation Object map",
                    );
                }
            }
            for key in raw
                .keys()
                .filter(|k| k.as_str() != "$ref" && !k.starts_with("x-"))
            {
                if definitions
                    .insert(key.clone(), hop.source.child(key))
                    .is_some()
                {
                    self.error(&hop.source.child(key), "http-path-item-reference-conflict", "the same Path Item field occurs beside $ref and in its target; OpenAPI leaves this overlap undefined");
                }
            }
        }
    }
}

// JSON Schema's integer is mathematical (50, 50.0 and 5e1 are the same bound).
// Preserve exactness without float rounding or exponent-sized allocation.
fn finite_counter(value: &Value) -> Option<u64> {
    let text = value.as_number()?.to_string();
    let (mantissa, exponent) = text.split_once(['e', 'E']).unwrap_or((&text, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    if digits.bytes().all(|b| b == b'0') {
        return Some(0);
    }
    if mantissa.starts_with('-') {
        return None;
    }
    let fraction = mantissa
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    let trimmed = digits.trim_end_matches('0');
    let power = exponent
        .parse::<i128>()
        .ok()?
        .checked_add((digits.len() - trimmed.len()) as i128)?
        .checked_sub(fraction as i128)?;
    if !(0..=19).contains(&power) {
        return None;
    }
    let coefficient = trimmed.trim_start_matches('0').parse::<u64>().ok()?;
    coefficient.checked_mul(10u64.checked_pow(power as u32)?)
}

pub(super) fn parent(source: &SourceId) -> Option<SourceId> {
    let (path, _) = source.pointer().rsplit_once('/')?;
    let mut id = SourceId::new(source.document().clone(), Default::default());
    for token in path.split('/').skip(1) {
        id = id.child(&token.replace("~1", "/").replace("~0", "~"));
    }
    Some(id)
}
fn method(value: &str) -> Option<Method> {
    Some(match value {
        "GET" => Method::Get,
        "PUT" => Method::Put,
        "POST" => Method::Post,
        "DELETE" => Method::Delete,
        "OPTIONS" => Method::Options,
        "HEAD" => Method::Head,
        "PATCH" => Method::Patch,
        "TRACE" => Method::Trace,
        "QUERY" => Method::Query,
        _ => {
            suspect_ir::contract::HttpMethod::parse(value)?;
            Method::Custom(value.to_owned())
        }
    })
}

const FIXED_METHODS: [(&str, &str); 9] = [
    ("get", "GET"),
    ("put", "PUT"),
    ("post", "POST"),
    ("delete", "DELETE"),
    ("options", "OPTIONS"),
    ("head", "HEAD"),
    ("patch", "PATCH"),
    ("trace", "TRACE"),
    ("query", "QUERY"),
];

fn contains_source(parent: &SourceId, child: &SourceId) -> bool {
    parent.document() == child.document()
        && (parent.pointer() == child.pointer()
            || child
                .pointer()
                .strip_prefix(parent.pointer())
                .is_some_and(|suffix| suffix.starts_with('/')))
}

fn operation_roots(op: &OperationPlan, roots: &mut BTreeSet<SourceId>) {
    for parameter in &op.parameters {
        roots.insert(parameter.codec.schema.id.clone());
        if let Some(media) = &parameter.content_media {
            representation_roots(&media.representation, roots);
        }
    }
    if let Some(body) = &op.body {
        for media in &body.media {
            representation_roots(&media.representation, roots);
        }
    }
    for response in &op.responses {
        if op.method != Method::Head
            && !matches!(
                response.status,
                ResponseStatus::Exact(100..=199 | 204 | 205 | 304) | ResponseStatus::Range(1)
            )
        {
            for media in &response.media {
                representation_roots(&media.representation, roots);
            }
        }
        for header in &response.headers {
            roots.insert(header.codec.schema.id.clone());
        }
    }
}
fn representation_roots(value: &Representation, roots: &mut BTreeSet<SourceId>) {
    match value {
        Representation::Json { codec } | Representation::Text { codec, .. } => {
            if let Some(c) = codec {
                roots.insert(c.schema.id.clone());
            }
        }
        Representation::Binary { .. } => {}
        Representation::Form { form } => {
            for field in &form.fields {
                part_roots(field, roots);
            }
            additional_roots(&form.additional, roots);
        }
        Representation::Multipart { multipart } => match multipart {
            MultipartPlan::Named {
                parts, additional, ..
            } => {
                for part in parts {
                    part_roots(part, roots);
                }
                additional_roots(additional, roots);
            }
            MultipartPlan::Positional { prefix, items, .. } => {
                for part in prefix {
                    part_roots(part, roots);
                }
                additional_roots(items, roots);
            }
        },
        Representation::Stream { stream } => {
            roots.insert(stream.item_codec.schema.id.clone());
        }
    }
}
fn additional_roots(value: &AdditionalParts, roots: &mut BTreeSet<SourceId>) {
    if let AdditionalParts::Allowed(part) = value {
        part_roots(part, roots);
    }
}
fn part_roots(part: &PartPlan, roots: &mut BTreeSet<SourceId>) {
    match &part.representation {
        PartRepresentation::Json { codec, .. }
        | PartRepresentation::Text { codec, .. }
        | PartRepresentation::Style { codec, .. } => {
            roots.insert(codec.schema.id.clone());
        }
        PartRepresentation::Binary { .. } => {}
    }
    for header in &part.headers {
        roots.insert(header.codec.schema.id.clone());
    }
}
