//! Source-selected native Python sync/async clients and wheel packaging.
pub use crate::http_contract::HttpDiagnostic;
use crate::{OutFile, http_protocol as protocol, python_codecs};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use suspect_ir::contract::{Contract, SchemaId, SourceId};
mod credential_env;
mod docs;
mod emit;
mod native_examples;
mod planning;

pub use crate::examples::ExamplePlan;
pub use planning::{
    NativeType, PlannedBody, PlannedGroup, PlannedHeader, PlannedMedia, PlannedOperation,
    PlannedParameter, PlannedResponse,
};

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub codecs: python_codecs::CodecConfig,
    pub max_response_bytes: usize,
    pub max_request_bytes: usize,
    pub max_part_bytes: usize,
    pub max_stream_item_bytes: usize,
    pub max_parts: usize,
    /// An explicit, source-backed protocol policy. Unsupported adapter features
    /// remain located refusals even if requested by a caller's capability set.
    pub capabilities: protocol::Capabilities,
    /// Explicit variable names bound to admitted source security declarations.
    pub credential_env: Option<crate::credential_env::CredentialEnv>,
}
impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            codecs: Default::default(),
            max_response_bytes: 8 * 1024 * 1024,
            max_request_bytes: 8 * 1024 * 1024,
            max_part_bytes: 8 * 1024 * 1024,
            max_stream_item_bytes: 1024 * 1024,
            max_parts: 1024,
            capabilities: capabilities(),
            credential_env: None,
        }
    }
}
#[derive(Debug)]
pub struct HttpPlan {
    contract: Arc<Contract>,
    operations: Vec<PlannedOperation>,
    protocol: protocol::ProtocolPlan,
    groups: Vec<PlannedGroup>,
    codecs: python_codecs::CodecPlan,
    symbols: BTreeMap<SchemaId, String>,
    config: HttpConfig,
    examples: ExamplePlan,
    credential_env: Option<crate::credential_env::CredentialEnvPlan>,
}
impl HttpPlan {
    pub fn contract(&self) -> &Arc<Contract> {
        &self.contract
    }
    pub fn operations(&self) -> &[PlannedOperation] {
        &self.operations
    }
    pub fn codecs(&self) -> &python_codecs::CodecPlan {
        &self.codecs
    }
    pub fn symbols(&self) -> &BTreeMap<SchemaId, String> {
        &self.symbols
    }
    pub fn examples(&self) -> &ExamplePlan {
        &self.examples
    }
    pub fn protocol(&self) -> &protocol::ProtocolPlan {
        &self.protocol
    }
    pub fn groups(&self) -> &[PlannedGroup] {
        &self.groups
    }
    pub fn credential_env(&self) -> Option<&crate::credential_env::CredentialEnvPlan> {
        self.credential_env.as_ref()
    }
    pub fn render(&self) -> Vec<OutFile> {
        emit_http(self, &PackageConfig::default()).expect("default package identity")
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageConfig {
    pub name: String,
    pub version: String,
    pub import_name: String,
}
impl Default for PackageConfig {
    fn default() -> Self {
        Self {
            name: "generated-sdk".into(),
            version: "0.0.0".into(),
            import_name: "generated_sdk".into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageError(pub &'static str);
impl std::fmt::Display for PackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for PackageError {}
pub fn emit_http(plan: &HttpPlan, package: &PackageConfig) -> Result<Vec<OutFile>, PackageError> {
    if package.name.is_empty()
        || package.name.len() > 64
        || !package.name.as_bytes()[0].is_ascii_lowercase()
        || !package
            .name
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !package
            .name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    {
        return Err(PackageError("invalid Python distribution name"));
    }
    if semver::Version::parse(&package.version).is_err() {
        return Err(PackageError("invalid exact package version"));
    }
    if package.version.parse::<pep440_rs::Version>().is_err() {
        return Err(PackageError(
            "Python package version must also satisfy PEP 440",
        ));
    }
    if package.import_name.is_empty()
        || package.import_name.len() > 64
        || !package.import_name.as_bytes()[0].is_ascii_lowercase()
        || !package
            .import_name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        || python_name(&package.import_name) != package.import_name
        || [
            "httpx",
            "typing",
            "json",
            "sys",
            "os",
            "codecs",
            "pathlib",
            "types",
            "dataclasses",
            "asyncio",
        ]
        .contains(&package.import_name.as_str())
    {
        return Err(PackageError("invalid import package name"));
    }
    Ok(emit::package(plan, package))
}
fn diagnostic(
    contract: &Contract,
    id: SourceId,
    code: &'static str,
    message: &str,
) -> HttpDiagnostic {
    HttpDiagnostic {
        at: contract.source_span(&id).unwrap_or(0..0),
        source: id,
        code,
        message: message.into(),
    }
}
pub fn plan_http(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: HttpConfig,
) -> Result<HttpPlan, Vec<HttpDiagnostic>> {
    let mut errors = Vec::new();
    let root = SourceId::new(contract.entry().clone(), Default::default());
    if selected.is_empty() {
        errors.push(diagnostic(
            &contract,
            root.clone(),
            "http-no-operations",
            "select an outgoing operation",
        ));
    }
    if config.max_request_bytes == 0
        || config.max_response_bytes == 0
        || config.max_request_bytes > u32::MAX as usize
        || config.max_response_bytes > u32::MAX as usize
        || config.max_part_bytes > u32::MAX as usize
        || config.max_stream_item_bytes > u32::MAX as usize
        || config.max_parts == 0
        || config.max_parts > 65_536
    {
        errors.push(diagnostic(
            &contract,
            root.clone(),
            "http-resource-policy",
            "HTTP byte budgets must be positive 32-bit values",
        ));
    }
    let supported = capabilities();
    for requested in config.capabilities.enabled() {
        if !supported.supports(*requested) {
            errors.push(diagnostic(
                &contract,
                root.clone(),
                "python-http-capability-unsupported",
                &format!("Python does not implement {requested:?}"),
            ));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let wire = protocol::plan(
        &contract,
        selected,
        config
            .capabilities
            .clone()
            .with_limits(protocol::ByteLimits::new(
                config.max_request_bytes.max(config.max_response_bytes) as u64,
                config.max_part_bytes as u64,
                config.max_stream_item_bytes as u64,
            )),
    )
    .into_result()
    .map_err(|findings| {
        findings
            .into_iter()
            .map(|finding| HttpDiagnostic {
                source: finding.source().source().clone(),
                at: finding.source().span(),
                code: finding.code(),
                message: finding.message().into(),
            })
            .collect::<Vec<_>>()
    })?;
    let credential_env =
        crate::credential_env::plan(&contract, &wire, config.credential_env.as_ref())?;
    for id in crate::schema_view::closure(&contract, wire.codec_roots()) {
        for keyword in ["readOnly", "writeOnly"] {
            if contract
                .source(&id)
                .and_then(|raw| raw.get(keyword))
                .is_some_and(|value| value != &serde_json::Value::Bool(false))
            {
                errors.push(diagnostic(
                    &contract,
                    id.child(keyword),
                    "http-directional-codec-unsupported",
                    "Python context projections need their own validated codec profile",
                ));
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let codecs =
        python_codecs::plan_codecs(contract.clone(), wire.codec_roots(), config.codecs.clone())
            .map_err(|errors| {
                errors
                    .into_iter()
                    .map(|e| HttpDiagnostic {
                        source: e.source,
                        at: e.at,
                        code: e.code,
                        message: e.message,
                    })
                    .collect::<Vec<_>>()
            })?;
    let symbols = codecs
        .models()
        .symbols()
        .iter()
        .map(|symbol| (symbol.source().clone(), symbol.name().into()))
        .collect();
    let (operations, groups) = planning::lower(&wire);
    let examples =
        if codecs.validation_program().version == suspect_schema::OwnedProgram::V3_VERSION {
            crate::examples::plan_protocol_examples_v3(contract.clone(), &wire, Default::default())
        } else {
            crate::examples::plan_protocol_examples_v2(contract.clone(), &wire, Default::default())
        };
    Ok(HttpPlan {
        contract,
        operations,
        protocol: wire,
        groups,
        codecs,
        symbols,
        config,
        examples,
        credential_env,
    })
}

/// Capabilities with native implementations in this adapter. Positional and
/// nested/streamed multipart remain explicit shared-planner refusals.
pub fn capabilities() -> protocol::Capabilities {
    use protocol::Capability::*;
    protocol::Capabilities::for_adapter(
        "python-http-protocol-v1",
        [
            UnnamedOperations,
            AdditionalMethods,
            CustomMethods,
            HttpServers,
            RelativeServers,
            DocumentRelativeServers,
            SchemaResources,
            DynamicSchemaReferences,
            MultipleServers,
            ServerVariables,
            AnonymousSecurity,
            SecurityAlternatives,
            ConjunctiveSecurity,
            HttpBasic,
            ApiKeys,
            OAuth2,
            OpenIdConnect,
            SecurityRoles,
            ParameterStyles,
            HeaderParameters,
            CookieParameters,
            QuerystringParameters,
            QuerystringForm,
            ReservedParameters,
            ContentParameters,
            RangeResponses,
            DefaultResponses,
            UndeclaredResponses,
            MultipleMediaTypes,
            MediaRanges,
            MediaTypeParameters,
            StructuredJsonMedia,
            SchemaFreeJson,
            TextBodies,
            BinaryBodies,
            UndeclaredResponseBody,
            ResponseHeaders,
            ResponseLinks,
            FormBodies,
            MultipartBodies,
            PartEncodings,
            ServerSentEvents,
            JsonLines,
            OpenApi30,
            OpenApi32,
        ],
    )
}

/// Complete HTTP adapter and shared protocol inputs to the runtime fingerprint.
pub fn source_assets() -> &'static [(&'static str, &'static [u8])] {
    macro_rules! assets { ($($path:literal),* $(,)?) => { &[$(($path,include_bytes!($path) as &'static [u8])),*] }; }
    assets!(
        "python_http.rs",
        "python_http/planning.rs",
        "python_http/emit.rs",
        "python_http/docs.rs",
        "python_http/native_examples.rs",
        "python_http/credential_env.rs",
        "python_http/credential_env.py",
        "python_http/runtime.py",
        "python_http/types.py",
        "python_http/auth.py",
        "python_http/wire.py",
        "python_http/media.py",
        "python_http/registry.py",
        "python_http/parts.py",
        "python_http/streams.py",
        "python_http/urls.py",
        "python_validation/runtime_v2.py",
        "python_validation/guard.py",
        "python_validation/runtime_v3.py",
        "python_validation/resource_guard.py",
        "http_protocol.rs",
        "http_protocol/planner.rs",
        "http_protocol/model.rs",
        "http_protocol/capabilities.rs",
        "http_protocol/servers.rs",
        "http_protocol/security.rs",
        "http_protocol/parameters.rs",
        "http_protocol/shapes.rs",
        "http_protocol/media.rs",
        "http_protocol/responses.rs",
        "http_protocol/bodies.rs",
        "http_protocol/wire.rs",
        "http_protocol/resource.rs",
        "http_protocol/examples.rs",
    )
}
fn allocate(base: &str, used: &mut BTreeSet<String>) -> String {
    let mut name = base.to_owned();
    let mut suffix = 2;
    while !used.insert(name.clone()) {
        name = format!("{base}_{suffix}");
        suffix += 1;
    }
    name
}
fn python_name(name: &str) -> String {
    let mut value = crate::rust_models::snake(name);
    if [
        "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
        "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is",
        "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with",
        "yield", "self", "cls", "match", "case",
    ]
    .contains(&value.as_str())
        || value.starts_with("__")
    {
        value.push('_');
    }
    value
}
