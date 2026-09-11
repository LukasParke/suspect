//! Canonical source-selected Swift Package SDK for the admitted HTTP surface.
//!
//! Companion to the Go/Python/Rust backends: admission is shared via
//! `crate::http_protocol::plan`. Native Swift names, transport policy,
//! package identity and docs are planned here. Native models, exact JSON
//! codecs and the URLSession client are emitted from the shared wire
//! contract and the `OwnedProgram` validation roots — no per-API schemas
//! are hand-written and nothing is inferred from operation names.

use std::{collections::BTreeMap, collections::BTreeSet, sync::Arc};

use crate::{OutFile, http_protocol};
use suspect_ir::contract::{Contract, SchemaId, SourceId};

mod emit;
pub(crate) mod models;
mod protocol;
mod protocol_emit;
mod protocol_examples;
mod protocol_metadata;
mod protocol_positional;
mod protocol_query;
mod validation;

#[cfg(test)]
#[allow(dead_code)]
mod validation_v2_support;
#[cfg(test)]
#[allow(dead_code)]
mod validation_v3_support;

#[cfg(test)]
#[path = "swift_sdk/resources_tests.rs"]
mod resources_tests;

#[cfg(test)]
#[path = "swift_sdk/aggregate_examples_tests.rs"]
mod aggregate_examples_tests;

pub use protocol::{
    PlannedBody, PlannedHeader, PlannedMedia, PlannedPart, PlannedParts, PlannedPositionalParts,
    PlannedQueryForm, PlannedResponse,
};

pub use crate::http_contract::HttpDiagnostic;

// Matches the existing native bearer/API-key attachment policy. This is a
// runtime value limit, distinct from the shared variable-name syntax limits.
pub(crate) const CREDENTIAL_ENV_MAX_BYTES: usize = 8192;

/// Finite transport policy embedded in the generated package.
#[derive(Debug, Clone)]
pub struct SwiftConfig {
    /// Shared source validation policy. Unsupported assertions fail planning.
    pub validation: suspect_schema::Config,
    /// Generated response ceiling; callers may only lower it.
    pub max_response_bytes: usize,
    /// Ceiling for an assembled URL and for a serialized request body.
    pub max_request_bytes: usize,
    /// Maximum raw bytes retained with response/item failures, independently
    /// of transport queues and per-item decoding ceilings.
    pub max_stream_capture_bytes: usize,
    /// Independent ceiling for each in-memory form/multipart part.
    pub max_part_bytes: usize,
    /// Maximum framed SSE envelope or JSON-lines item before decoding.
    pub max_stream_item_bytes: usize,
    /// Maximum queued transport bytes while a stream consumer is paused.
    pub max_stream_buffer_bytes: usize,
    /// Explicit, versioned compatibility profiles; none are inferred.
    pub compatibility_profiles: BTreeSet<http_protocol::CompatibilityProfile>,
    /// Explicit runtime environment variable names, bound to used source schemes.
    /// The generator never reads their credential values.
    pub credential_env: Option<crate::credential_env::CredentialEnv>,
}

impl Default for SwiftConfig {
    fn default() -> Self {
        Self {
            validation: suspect_schema::Config {
                max_depth: 128,
                ..Default::default()
            },
            max_response_bytes: 8 * 1024 * 1024,
            max_request_bytes: 8 * 1024 * 1024,
            max_stream_capture_bytes: 8 * 1024 * 1024,
            max_part_bytes: 8 * 1024 * 1024,
            max_stream_item_bytes: 1024 * 1024,
            max_stream_buffer_bytes: 64 * 1024,
            compatibility_profiles: BTreeSet::new(),
            credential_env: None,
        }
    }
}

/// One admitted operation with allocated native Swift names.
#[derive(Debug, Clone)]
pub struct PlannedOperation {
    pub source: SourceId,
    pub operation_id: String,
    pub method: String,
    pub path: String,
    /// lowerCamel method name on the client, collision-free and not a
    /// Swift keyword.
    pub method_name: String,
    /// UpperCamel input struct name, e.g. `CreateKeysInput`.
    pub input_type: String,
    /// UpperCamel success result payload type.
    pub success_type: String,
    /// Per-operation source-declared API error enum, retaining actual status.
    pub error_type: String,
    /// Native parameter names in wire order.
    pub parameters: Vec<PlannedParameter>,
    pub(crate) body: Option<PlannedBody>,
    pub(crate) responses: Vec<PlannedResponse>,
    pub(crate) wire: http_protocol::OperationPlan,
    pub(crate) metadata_name: String,
    pub(crate) description: String,
}

impl PlannedOperation {
    /// Original admitted descriptors, including security and server choices.
    pub fn protocol(&self) -> &http_protocol::OperationPlan {
        &self.wire
    }
    pub fn body(&self) -> Option<&PlannedBody> {
        self.body.as_ref()
    }
    pub fn responses(&self) -> &[PlannedResponse] {
        &self.responses
    }
    /// Inputs with no required fields can be omitted at the call site.
    pub fn default_input(&self) -> bool {
        self.parameters.iter().all(|p| !p.wire.required())
            && self.body.as_ref().is_none_or(|b| !b.wire.required())
    }
}

#[derive(Debug, Clone)]
pub struct PlannedParameter {
    pub(crate) wire: http_protocol::ParameterPlan,
    /// Swift property name on the input struct.
    pub field_name: String,
    /// A whole-query form encoder over the parameter's real aggregate JSON codec.
    pub query_form: Option<PlannedQueryForm>,
}

impl PlannedParameter {
    #[must_use]
    pub const fn wire(&self) -> &http_protocol::ParameterPlan {
        &self.wire
    }
}

/// Immutable complete selected-operation plan for one Swift package.
pub struct SdkPlan {
    contract: Arc<Contract>,
    operations: Vec<PlannedOperation>,
    /// Native model symbol names keyed by schema source.
    symbols: BTreeMap<SchemaId, String>,
    /// Security scheme name to native client credential property.
    credentials: BTreeMap<String, String>,
    credential_bindings: Vec<protocol::PlannedCredential>,
    credential_env: Option<crate::credential_env::CredentialEnvPlan>,
    protocol: http_protocol::ProtocolPlan,
    config: SwiftConfig,
    models: models::ModelPlan,
    program: suspect_schema::OwnedProgram,
    validation_data: String,
    validator: suspect_schema::OwnedSchema,
    examples: crate::examples::ExamplePlan,
}

impl std::fmt::Debug for SdkPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SdkPlan")
            .field("operations", &self.operations)
            .field("protocol", &self.protocol)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl SdkPlan {
    #[must_use]
    pub fn contract(&self) -> &Arc<Contract> {
        &self.contract
    }
    #[must_use]
    pub fn operations(&self) -> &[PlannedOperation] {
        &self.operations
    }
    #[must_use]
    pub fn symbols(&self) -> &BTreeMap<SchemaId, String> {
        &self.symbols
    }
    /// Immutable typed native declarations shared with compatibility analysis.
    pub(crate) fn models(&self) -> &models::ModelPlan {
        &self.models
    }
    #[must_use]
    pub fn credentials(&self) -> &BTreeMap<String, String> {
        &self.credentials
    }
    /// Source-bound runtime environment policy; values are variable names only.
    #[must_use]
    pub fn credential_env(&self) -> Option<&crate::credential_env::CredentialEnvPlan> {
        self.credential_env.as_ref()
    }
    /// Native credential member for one source-document-scoped scheme declaration.
    /// Scheme names alone are not identities in a multi-document contract.
    pub fn credential_property(&self, scheme: &SourceId) -> Option<&str> {
        self.credential_bindings
            .iter()
            .find(|c| c.requirement.scheme().use_site().source() == scheme)
            .map(|c| c.property.as_str())
    }
    /// The admitted shared HTTP plan used by emission, never reconstructed from source text.
    pub fn protocol(&self) -> &http_protocol::ProtocolPlan {
        &self.protocol
    }
    #[must_use]
    pub const fn config(&self) -> &SwiftConfig {
        &self.config
    }
    /// Checked portable instructions actually executed by the Swift runtime.
    #[must_use]
    pub fn program(&self) -> &suspect_schema::OwnedProgram {
        &self.program
    }
    /// Shared source-validated example values and their origins.
    #[must_use]
    pub fn examples(&self) -> &crate::examples::ExamplePlan {
        &self.examples
    }
    /// Render the complete Swift package with the default identity.
    #[must_use]
    pub fn render(&self) -> Vec<OutFile> {
        emit::package(self, &PackageConfig::default())
    }
}

/// Swift Package identity, independent of every API semantic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageConfig {
    /// Package name, e.g. `GeneratedSDK` (used for the product name).
    pub name: String,
    /// Exact SemVer, no pre-release or build metadata.
    pub version: String,
    /// Importable module name under `Sources/`, a legal Swift identifier
    /// that is not a keyword.
    pub module_name: String,
}

impl Default for PackageConfig {
    fn default() -> Self {
        Self {
            name: "GeneratedSDK".into(),
            version: "0.0.0".into(),
            module_name: "GeneratedSDK".into(),
        }
    }
}

/// Package identity admission failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageError {
    InvalidName,
    InvalidVersion,
    InvalidModuleName,
}

impl std::fmt::Display for PackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidName => "invalid Swift package name",
            Self::InvalidVersion => "invalid exact package version",
            Self::InvalidModuleName => "invalid Swift module name",
        })
    }
}
impl std::error::Error for PackageError {}

/// Emit a Swift Package: `Package.swift`, module sources, DocC catalog,
/// examples and manifest.
///
/// # Errors
/// Invalid name, module name or version fails before any artifact is
/// returned.
pub fn emit_sdk(plan: &SdkPlan, package: &PackageConfig) -> Result<Vec<OutFile>, Vec<String>> {
    let mut errors = Vec::new();
    if !valid_identifier(&package.name) {
        errors.push(PackageError::InvalidName.to_string());
    }
    if !valid_identifier(&package.module_name)
        || [
            "Swift",
            "Foundation",
            "FoundationNetworking",
            "PackageDescription",
            "XCTest",
            "Testing",
        ]
        .contains(&package.module_name.as_str())
    {
        errors.push(PackageError::InvalidModuleName.to_string());
    }
    if !semver::Version::parse(&package.version)
        .is_ok_and(|v| v.pre.is_empty() && v.build.is_empty())
    {
        errors.push(PackageError::InvalidVersion.to_string());
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(emit::package(plan, package))
}

/// Swift identifiers: ASCII letter or underscore first, then alphanumerics
/// or underscores; keywords are rejected outright.
#[must_use]
pub fn valid_identifier(name: &str) -> bool {
    let head_ok = name
        .as_bytes()
        .first()
        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_');
    head_ok
        && name != "_"
        && name.len() <= 64
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !keywords().contains(&name)
}

/// Reserved Swift keywords the emitter must never allocate.
#[must_use]
pub fn keywords() -> &'static [&'static str] {
    &[
        "associatedtype",
        "class",
        "deinit",
        "enum",
        "extension",
        "fileprivate",
        "func",
        "import",
        "init",
        "inout",
        "internal",
        "let",
        "open",
        "operator",
        "private",
        "precedencegroup",
        "protocol",
        "public",
        "rethrows",
        "static",
        "struct",
        "subscript",
        "typealias",
        "var",
        "break",
        "case",
        "catch",
        "continue",
        "default",
        "defer",
        "do",
        "else",
        "fallthrough",
        "for",
        "guard",
        "if",
        "in",
        "repeat",
        "return",
        "throw",
        "switch",
        "where",
        "while",
        "as",
        "false",
        "is",
        "nil",
        "self",
        "Self",
        "super",
        "true",
        "try",
        "throws",
        "throw",
        "Type",
        "Protocol",
        "actor",
        "async",
        "await",
        "some",
        "any",
        "nonisolated",
        "isolated",
        "sending",
        "borrowing",
        "consuming",
        "package",
        "macro",
        "get",
        "set",
        "willSet",
        "didSet",
        "repeat",
        "each",
    ]
}

fn diagnostic(
    contract: &Contract,
    source: SourceId,
    code: &'static str,
    message: impl Into<String>,
) -> HttpDiagnostic {
    HttpDiagnostic {
        at: contract.source_span(&source).unwrap_or(0..0),
        source,
        code,
        message: message.into(),
    }
}

/// UpperCamel Swift type identifier from a wire/source name.
#[must_use]
pub fn exported(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper_next = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if upper_next {
                out.extend(c.to_uppercase());
                upper_next = false;
            } else {
                out.push(c);
            }
        } else {
            upper_next = true;
        }
    }
    if out.is_empty() {
        out.push_str("Value");
    }
    if out.as_bytes()[0].is_ascii_digit() {
        out.insert_str(0, "Value");
    }
    if keywords().contains(&out.as_str()) {
        out.push('_');
    }
    out
}

/// lowerCamel Swift member identifier.
#[must_use]
fn member(name: &str) -> String {
    let upper = exported(name);
    if upper.is_empty() {
        return "op".into();
    }
    let mut c = upper.chars();
    let mut result = match c.next() {
        Some(first) => first.to_lowercase().collect::<String>() + c.as_str(),
        None => "op".into(),
    };
    if keywords().contains(&result.as_str()) {
        result.push('_');
    }
    result
}

fn allocate(base: &str, used: &mut BTreeSet<String>) -> String {
    let mut name = base.to_owned();
    let mut suffix = 2;
    while !used.insert(name.clone()) {
        name = format!("{base}{suffix}");
        suffix += 1;
    }
    name
}

/// Plan the Swift HTTP surface for exactly the selected operation sources.
///
/// # Errors
/// Unsupported declarations, directional models and resource policies
/// produce source-linked findings before package emission. Validation uses
/// the shared `OwnedCompiler` program through the emitted codec roots.
pub fn plan_sdk(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: SwiftConfig,
) -> Result<SdkPlan, Vec<HttpDiagnostic>> {
    let capabilities = protocol::capabilities(&config);
    let mut errors = Vec::new();
    let fallback = selected
        .first()
        .cloned()
        .unwrap_or_else(|| SourceId::new(contract.entry().clone(), Default::default()));
    if selected.is_empty() {
        errors.push(diagnostic(
            &contract,
            fallback.clone(),
            "http-no-operations",
            "select at least one outgoing operation",
        ));
    }
    if config.max_response_bytes == 0
        || config.max_request_bytes == 0
        || config.max_stream_capture_bytes == 0
        || config.max_response_bytes > i32::MAX as usize
        || config.max_request_bytes > i32::MAX as usize
        || config.max_stream_capture_bytes > i32::MAX as usize
        || [
            config.max_part_bytes,
            config.max_stream_item_bytes,
            config.max_stream_buffer_bytes,
        ]
        .iter()
        .any(|v| *v == 0 || *v > i32::MAX as usize)
    {
        errors.push(diagnostic(
            &contract,
            fallback,
            "http-resource-policy",
            "HTTP byte ceilings must be positive and at most Int32.max",
        ));
    }
    // Keep the native URI-path policy source-linked, including malformed paths
    // for which the shared planner cannot construct an OperationPlan.
    for op in contract
        .operations()
        .filter(|op| selected.contains(op.source()))
    {
        if !op.path_template().is_some_and(valid_path) {
            errors.push(diagnostic(&contract, op.source().clone(), "swift-path-unsupported", "the Swift URL profile requires an ASCII URI path template with balanced placeholders, valid percent escapes, and no dot segments; encode non-ASCII static path text as URI bytes"));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let wire = http_protocol::plan(&contract, selected, capabilities)
        .into_result()
        .map_err(|findings| {
            findings
                .into_iter()
                .filter(|d| d.severity() == http_protocol::Severity::Error)
                .map(|d| HttpDiagnostic {
                    source: d.source().source().clone(),
                    at: d.source().span(),
                    code: d.code(),
                    message: d.message().to_owned(),
                })
                .collect::<Vec<_>>()
        })?;
    protocol::admit(&contract, &wire)?;
    let credential_env =
        crate::credential_env::plan(&contract, &wire, config.credential_env.as_ref())?;
    for id in wire.codec_schema_closure() {
        if let Some(schema) = contract.schema(id) {
            let raw = crate::schema_view::raw(schema);
            for keyword in ["readOnly", "writeOnly"] {
                if raw
                    .get(keyword)
                    .is_some_and(|value| value != &serde_json::Value::Bool(false))
                {
                    errors.push(diagnostic(&contract, id.child(keyword), "http-directional-codec-unsupported", "the Swift HTTP profile requires directional annotations to be absent or false throughout its neutral codec closure"));
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let reachable = wire.codec_schema_closure().to_vec();
    let compiler = suspect_schema::OwnedCompiler::new(config.validation.clone());
    // Prefer the frozen base/scoped envelopes. Only a closure declined by v2
    // needs explicit v3 admission; unrelated unsupported assertions still fail
    // the v3 compiler at their original sources before any artifacts are emitted.
    let validator = compiler
        .compile_v2(contract.clone(), &reachable)
        .or_else(|_| compiler.compile_v3(contract.clone(), &reachable))
        .map_err(|findings| {
            findings
                .into_iter()
                .map(|finding| HttpDiagnostic {
                    source: finding.source,
                    at: finding.span.unwrap_or(0..0),
                    code: "swift-validation-unsupported",
                    message: finding.message,
                })
                .collect::<Vec<_>>()
        })?;
    let program = validator.program();
    let validation_data = validation::emit(&contract, &program)?;
    let mut models = models::plan(&contract, wire.codec_roots(), &validator, &program)?;
    protocol::reserve_runtime_names(&mut models);
    let symbols = models.symbols();
    let mut type_names = models.reserved_names();
    let mut methods: BTreeSet<String> = [
        "client",
        "send",
        "close",
        "decodeResponse",
        "open",
        "transportFailure",
        "responseValue",
        "credentials",
        "transport",
        "options",
        "maxRequestBytes",
        "maxResponseBytes",
        "maxCaptureBytes",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    if credential_env.is_some() {
        methods.insert("fromEnvironment".into());
    }
    let mut credentials = BTreeMap::new();
    let mut credential_props = BTreeSet::from(["serverURL".into(), "value".into()]);
    let mut credential_bindings = Vec::new();
    let mut credential_sources = BTreeSet::new();
    type_names.extend(protocol::RUNTIME_NAMES.iter().map(|s| (*s).to_owned()));
    let mut operations = Vec::new();
    for op in wire.operations() {
        let operation_id = op
            .operation_id()
            .map(|id| id.value().clone())
            .unwrap_or_else(|| {
                format!(
                    "{}_{}",
                    op.method().as_str().to_ascii_lowercase(),
                    op.path()
                )
            });
        let stem = exported(&operation_id);
        let method_name = allocate(&member(&operation_id), &mut methods);
        let mut members = BTreeSet::from(["body".into()]);
        let parameters = op
            .parameters()
            .iter()
            .map(|p| {
                let base = member(p.name());
                let proposed = if members.contains(&base) {
                    format!(
                        "{}{base}",
                        match p.location() {
                            http_protocol::ParameterLocation::Path => "path",
                            http_protocol::ParameterLocation::Query => "query",
                            http_protocol::ParameterLocation::Header => "header",
                            http_protocol::ParameterLocation::Cookie => "cookie",
                            _ => "querystring",
                        }
                    )
                } else {
                    base
                };
                PlannedParameter {
                    wire: p.clone(),
                    field_name: allocate(&proposed, &mut members),
                    query_form: protocol::query_form(p, &stem, &mut type_names),
                }
            })
            .collect();
        for requirement in op
            .security()
            .alternatives()
            .iter()
            .flat_map(|a| a.requirements())
        {
            if credential_sources.insert(requirement.scheme().use_site().source().clone()) {
                let property = allocate(&member(requirement.name()), &mut credential_props);
                credentials
                    .entry(requirement.name().to_owned())
                    .or_insert_with(|| property.clone());
                credential_bindings.push(protocol::PlannedCredential {
                    property,
                    requirement: requirement.clone(),
                });
            }
        }
        let body = op
            .body()
            .map(|b| protocol::body(b, &stem, &models, &mut type_names));
        let responses = op
            .responses()
            .iter()
            .map(|r| protocol::response(r, op.method().as_str(), &stem, &models, &mut type_names))
            .collect();
        operations.push(PlannedOperation {
            source: op.source().use_site().source().clone(),
            operation_id,
            method: op.method().as_str().to_owned(),
            path: op.path().to_owned(),
            method_name,
            input_type: allocate(&format!("{stem}Input"), &mut type_names),
            success_type: allocate(&format!("{stem}Result"), &mut type_names),
            error_type: allocate(&format!("{stem}APIError"), &mut type_names),
            parameters,
            body,
            responses,
            wire: op.clone(),
            metadata_name: allocate(&format!("{stem}HTTP"), &mut type_names),
            description: op
                .description()
                .map(|d| d.value().clone())
                .or_else(|| op.summary().map(|s| s.value().clone()))
                .unwrap_or_default(),
        });
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let examples = match program.version {
        suspect_schema::OwnedProgram::V3_VERSION => {
            crate::examples::plan_protocol_examples_v3(contract.clone(), &wire, Default::default())
        }
        suspect_schema::OwnedProgram::V2_VERSION => {
            crate::examples::plan_protocol_examples_v2(contract.clone(), &wire, Default::default())
        }
        _ => crate::examples::plan_protocol_examples(contract.clone(), &wire, Default::default()),
    };
    Ok(SdkPlan {
        contract,
        operations,
        symbols,
        credentials,
        credential_bindings,
        credential_env,
        protocol: wire,
        config,
        models,
        program,
        validation_data,
        validator,
        examples,
    })
}

fn valid_path(path: &str) -> bool {
    if !path.starts_with('/') {
        return false;
    }
    let mut static_path = String::new();
    let mut characters = path.chars();
    while let Some(ch) = characters.next() {
        if ch == '{' {
            let mut name = String::new();
            let mut closed = false;
            for ch in characters.by_ref() {
                if ch == '}' {
                    closed = true;
                    break;
                }
                if ch == '{' || ch.is_control() {
                    return false;
                }
                name.push(ch);
            }
            if !closed || name.is_empty() {
                return false;
            }
            static_path.push('x');
        } else if ch.is_ascii_alphanumeric() || "/-._~!$&'()*+,;=:@%".contains(ch) {
            static_path.push(ch);
        } else {
            return false;
        }
    }
    for segment in static_path.split('/') {
        let mut decoded = Vec::new();
        let mut bytes = segment.as_bytes().iter().copied();
        while let Some(byte) = bytes.next() {
            if byte == b'%' {
                let Some(a) = bytes.next().and_then(|b| (b as char).to_digit(16)) else {
                    return false;
                };
                let Some(b) = bytes.next().and_then(|b| (b as char).to_digit(16)) else {
                    return false;
                };
                decoded.push((a * 16 + b) as u8);
            } else {
                decoded.push(byte);
            }
        }
        if decoded == b"." || decoded == b".." {
            return false;
        }
    }
    true
}
