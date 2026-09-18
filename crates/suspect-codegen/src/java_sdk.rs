//! Source-selected Java SDKs: native immutable models and builders, exact checked
//! codecs, an OwnedProgram runtime, sync/CompletableFuture HTTP, Maven and Javadoc.
//!
//! OpenAPI is the only API-semantic input. This backend explicitly adopts the shared
//! bounded HTTP protocol plan, retaining native bytes, parts and streaming items.
//! Native names, construction signatures, codec bindings, status variants
//! and documentation/example identities are retained together in an immutable plan.

use std::sync::Arc;

use std::collections::BTreeSet;
use suspect_ir::contract::{Contract, SchemaId, SourceId};
use suspect_schema::{Config, OwnedCompiler, OwnedProgram};

pub use crate::http_contract::HttpDiagnostic;

mod docs;
mod emit;
pub mod http;
mod http_emit;
mod incoming;
pub mod json_runtime;
mod model_emit;
pub mod models;
mod oauth;
mod pagination;
pub mod protocol;
pub mod validation;
mod stream;
mod wire_emit;

pub use docs::{JavaExample, JavaOperationExample};

/// Explicit wire policy. Compatibility profiles never activate from API names.
#[derive(Debug, Clone, Default)]
pub struct ProtocolConfig {
    pub limits: crate::http_protocol::ByteLimits,
    pub compatibility_profiles: BTreeSet<crate::http_protocol::CompatibilityProfile>,
    /// Explicit runtime variable names, bound after protocol admission.
    pub credential_env: Option<crate::credential_env::CredentialEnv>,
    /// Golden SDK behavior defaults resolved inside this backend's plan.
    pub sdk_defaults: Option<crate::sdk_defaults::SdkDefaults>,
    /// `ua/v1` attribution constants compiled from package identity and source.
    pub attribution: Option<crate::attribution::AttributionDescriptor>,
}

/// Maven/Java naming identity. No option adds service semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageConfig {
    /// Java namespace, also the default Maven group ID.
    pub package: String,
    /// Exact semantic version.
    pub version: String,
    /// Generated client class name.
    pub api_name: String,
}
impl Default for PackageConfig {
    fn default() -> Self {
        Self {
            package: "com.example.generated".into(),
            version: "0.1.0".into(),
            api_name: "Client".into(),
        }
    }
}

/// Native build identity, separate from the source API and runtime policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MavenConfig {
    /// Explicit Maven group ID. `None` retains `PackageConfig::package` and its
    /// existing artifact bytes; the Java import namespace remains independent.
    pub group_id: Option<String>,
    /// Maven artifact ID.
    pub artifact_id: String,
    /// Declared Java release. The verified compiler profiles are 21 and 25.
    pub java_release: u16,
}
impl Default for MavenConfig {
    fn default() -> Self {
        Self {
            group_id: None,
            artifact_id: "generated-sdk".into(),
            java_release: 21,
        }
    }
}

/// Complete immutable native plan for exactly one selected source closure.
/// Planning is admission, not evidence that a native compiler has run.
#[derive(Debug)]
pub struct SdkPlan {
    contract: Arc<Contract>,
    config: PackageConfig,
    maven: MavenConfig,
    models: models::JavaModelPlan,
    operations: Vec<http::JavaOperation>,
    program: OwnedProgram,
    examples: crate::examples::ExamplePlan,
    native_examples: Vec<JavaOperationExample>,
    protocol: crate::http_protocol::ProtocolPlan,
    credential_env: Option<crate::credential_env::CredentialEnvPlan>,
    attribution: Option<crate::attribution::AttributionDescriptor>,
    pagination: Option<crate::http_protocol::PaginationOutcome>,
    oauth: Option<crate::http_protocol::OAuthPlan>,
    stream_semantics: crate::http_protocol::StreamSemanticsPlan,
    incoming: crate::http_protocol::IncomingPlan,
}
impl SdkPlan {
    /// Retained rich source-backed HTTP decisions and actual codec roots.
    #[must_use]
    pub fn protocol(&self) -> &crate::http_protocol::ProtocolPlan {
        &self.protocol
    }
    /// Source-bound runtime environment policy; never contains environment values.
    #[must_use]
    pub fn credential_env(&self) -> Option<&crate::credential_env::CredentialEnvPlan> {
        self.credential_env.as_ref()
    }
    /// Compiled `ua/v1` attribution constants; `None` keeps the disabled sentinel.
    #[must_use]
    pub fn attribution(&self) -> Option<&crate::attribution::AttributionDescriptor> {
        self.attribution.as_ref()
    }
    /// Compiled pagination selection carried from the configured SDK defaults.
    #[must_use]
    pub fn pagination(&self) -> Option<&crate::http_protocol::PaginationOutcome> {
        self.pagination.as_ref()
    }
    /// Compiled OAuth lifecycle selection carried from the configured SDK defaults.
    #[must_use]
    pub fn oauth(&self) -> Option<&crate::http_protocol::OAuthPlan> {
        self.oauth.as_ref()
    }
    /// Compiled typed-stream semantics for every operation stream media,
    /// carried from the infallible shared planner.
    #[must_use]
    pub fn stream_semantics(&self) -> &crate::http_protocol::StreamSemanticsPlan {
        &self.stream_semantics
    }
    /// Compiled incoming webhook/callback receipts over the whole contract,
    /// selection-independent.
    #[must_use]
    pub fn incoming(&self) -> &crate::http_protocol::IncomingPlan {
        &self.incoming
    }
    /// Exact retained native model/declaration/codec graph.
    #[must_use]
    pub const fn models(&self) -> &models::JavaModelPlan {
        &self.models
    }
    /// Actual public method, input, construction and status-variant descriptors.
    #[must_use]
    pub fn operations(&self) -> &[http::JavaOperation] {
        &self.operations
    }
    /// Packaging identity of the planned jar.
    #[must_use]
    pub const fn package(&self) -> &PackageConfig {
        &self.config
    }
    /// Maven artifact/toolchain identity.
    #[must_use]
    pub const fn maven(&self) -> &MavenConfig {
        &self.maven
    }
    /// Effective Maven group, independent of the Java package when configured.
    #[must_use]
    pub fn maven_group_id(&self) -> &str {
        self.maven
            .group_id
            .as_deref()
            .unwrap_or(&self.config.package)
    }
    /// Original retained source contract.
    #[must_use]
    pub const fn contract(&self) -> &Arc<Contract> {
        &self.contract
    }
    /// Checked instruction graph and source bindings.
    #[must_use]
    pub const fn program(&self) -> &OwnedProgram {
        &self.program
    }
    /// Validated examples and original findings, including invalid declarations.
    #[must_use]
    pub const fn examples(&self) -> &crate::examples::ExamplePlan {
        &self.examples
    }
    /// Native source-bound example expressions and callable method identities.
    #[must_use]
    pub fn native_examples(&self) -> &[JavaOperationExample] {
        &self.native_examples
    }
    /// Exact source OpenAPI version.
    #[must_use]
    pub fn openapi_version(&self) -> &str {
        self.contract.openapi_version()
    }
    /// Deterministic Maven sources/resources, native docs and executable examples.
    /// All paths are rooted under `java/`.
    ///
    /// # Errors
    /// A checked-program or artifact resource invariant prevents the whole package.
    pub fn render(&self) -> Result<Vec<crate::OutFile>, Vec<HttpDiagnostic>> {
        emit::render(self)
    }
}

/// Plan exactly the selected outgoing operations and optional additional model
/// roots. The default Maven profile targets Java 21 with no runtime dependencies.
///
/// # Errors
/// Located wire, schema, representation or identity findings prevent a package.
pub fn plan_sdk(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: PackageConfig,
    roots: &[SchemaId],
) -> Result<SdkPlan, Vec<HttpDiagnostic>> {
    plan_sdk_with_maven(contract, selected, config, roots, MavenConfig::default())
}

/// Plan with explicit Maven artifact/toolchain identity.
///
/// # Errors
/// Same source-linked admission as `plan_sdk`; no partial artifacts are returned.
pub fn plan_sdk_with_maven(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: PackageConfig,
    roots: &[SchemaId],
    maven: MavenConfig,
) -> Result<SdkPlan, Vec<HttpDiagnostic>> {
    plan_sdk_with_protocol(
        contract,
        selected,
        config,
        roots,
        maven,
        ProtocolConfig::default(),
    )
}

/// Plan with explicit wire byte ceilings and optional versioned compatibility.
///
/// # Errors
/// Source-linked unsupported, malformed and native resource declarations.
pub fn plan_sdk_with_protocol(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: PackageConfig,
    roots: &[SchemaId],
    maven: MavenConfig,
    protocol_config: ProtocolConfig,
) -> Result<SdkPlan, Vec<HttpDiagnostic>> {
    plan_sdk_internal(
        contract,
        selected,
        config,
        roots,
        maven,
        protocol_config,
        false,
    )
}

/// Explicitly admit the verified resource/dynamic V3 profile when required.
/// Ordinary admitted closures retain the complete V1/V2 plan and artifact policy.
///
/// # Errors
/// Original source-linked capability, schema, representation and resource findings.
pub fn plan_sdk_with_protocol_v3(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: PackageConfig,
    roots: &[SchemaId],
    maven: MavenConfig,
    protocol_config: ProtocolConfig,
) -> Result<SdkPlan, Vec<HttpDiagnostic>> {
    match plan_sdk_with_protocol(
        contract.clone(),
        selected,
        config.clone(),
        roots,
        maven.clone(),
        protocol_config.clone(),
    ) {
        Ok(plan) => Ok(plan),
        Err(errors)
            if errors.iter().any(|e| {
                e.code == "http-capability-required"
                    || e.code == "java-validation-compile"
                        && matches!(
                            e.source.pointer().rsplit('/').next(),
                            Some("$id" | "$self" | "$anchor" | "$dynamicAnchor" | "$dynamicRef")
                        )
            }) =>
        {
            plan_sdk_internal(
                contract,
                selected,
                config,
                roots,
                maven,
                protocol_config,
                true,
            )
        }
        Err(errors) => Err(errors),
    }
}

fn plan_sdk_internal(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: PackageConfig,
    roots: &[SchemaId],
    maven: MavenConfig,
    protocol_config: ProtocolConfig,
    resources: bool,
) -> Result<SdkPlan, Vec<HttpDiagnostic>> {
    let mut errors = Vec::new();
    if config.package.is_empty()
        || config.package.len() > 200
        || config.package.split('.').next() == Some("java")
        || !config.package.split('.').all(models::is_java_identifier)
    {
        errors.push(plan_diag(
            &contract,
            "java-package-invalid",
            "package must be a portable dotted Java namespace outside java.*",
        ));
    }
    if !models::is_java_identifier(&config.api_name)
        || models::reserved_types()
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&config.api_name))
    {
        errors.push(plan_diag(
            &contract,
            "java-api-name-invalid",
            "client name must be a Java identifier that does not shadow runtime/JDK types",
        ));
    }
    if semver::Version::parse(&config.version).is_err() {
        errors.push(plan_diag(
            &contract,
            "java-version-invalid",
            "version must be an exact semantic version",
        ));
    }
    if maven.group_id.as_ref().is_some_and(|group| {
        group.is_empty()
            || group.len() > 200
            || !group.starts_with(|c: char| c.is_ascii_alphanumeric())
            || !group
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    }) {
        errors.push(plan_diag(
            &contract,
            "java-maven-group-invalid",
            "Maven group_id must be a nonempty portable ASCII coordinate",
        ));
    }
    if maven.artifact_id.is_empty()
        || maven.artifact_id.len() > 100
        || !maven
            .artifact_id
            .starts_with(|c: char| c.is_ascii_alphanumeric())
        || !maven
            .artifact_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        || !matches!(maven.java_release, 21 | 25)
    {
        errors.push(plan_diag(
            &contract,
            "java-maven-identity-invalid",
            "Maven requires a portable artifact ID and a verified Java release (21 or 25)",
        ));
    }
    if roots.is_empty() && selected.is_empty() {
        errors.push(plan_diag(
            &contract,
            "java-no-roots",
            "select at least one outgoing operation or model root",
        ));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    if protocol_config.limits.body() > 8 * 1024 * 1024
        || protocol_config.limits.part() > 8 * 1024 * 1024
        || protocol_config.limits.stream_item() > 8 * 1024 * 1024
    {
        return Err(vec![plan_diag(
            &contract,
            "java-protocol-resource-limit",
            "Java wire byte ceilings must fit the finite 8 MiB native profile",
        )]);
    }
    let wire = if resources {
        protocol::plan_v3(&contract, selected, &protocol_config)?
    } else {
        protocol::plan(&contract, selected, &protocol_config)?
    };
    let credential_env =
        crate::credential_env::plan_with_defaults(&contract, &wire, protocol_config.credential_env.as_ref(), protocol_config.sdk_defaults.as_ref())?;
    let attribution = protocol_config.attribution;
    // Compiled pagination policy: SDK defaults are required, and a policy
    // failure is a plan failure like every other diagnostic.
    let pagination = match protocol_config.sdk_defaults.as_ref() {
        Some(defaults) => Some(crate::http_protocol::plan_pagination(
            &contract,
            &wire,
            Some(defaults),
        )?),
        None => None,
    };
    // Compiled OAuth lifecycle policy, mirrored from pagination: SDK defaults
    // are required, and a policy failure is a plan failure like every other
    // diagnostic.
    let oauth = match protocol_config.sdk_defaults.as_ref() {
        Some(defaults) => Some(crate::http_protocol::plan_oauth(
            &contract,
            &wire,
            Some(defaults),
        )?),
        None => None,
    };
    // Compiled incoming webhook/callback receipts. Planning walks the whole
    // Contract (selection-independent) and fails the plan on broken incoming
    // declarations like every other diagnostic.
    let incoming = crate::http_protocol::plan_incoming(&contract)?;
    // Compiled typed-stream semantics are infallible and unconditional: they
    // record the declared event kinds, payload codecs, sentinel and completion
    // policies for every operation stream media. Emission stays conditional on
    // discriminated SSE operations and is never gated on SDK defaults.
    let stream_semantics = crate::http_protocol::plan_stream_semantics(&contract, &wire);
    let mut roots = roots.to_vec();
    roots.extend(wire.codec_roots().iter().cloned());
    if !incoming.is_empty() {
        // Incoming receipts extend the codec table only when declared: their
        // body schemas become actual codec inputs beside the selected
        // operations'.
        roots.extend(incoming.codec_roots().iter().cloned());
    }
    roots.sort();
    roots.dedup();
    for root in &roots {
        if contract.schema(root).is_none() {
            errors.push(diagnostic(
                &contract,
                root.clone(),
                "java-unknown-model-root",
                "selected root is not an indexed schema",
            ));
        }
    }
    let reachable = crate::schema_view::closure(&contract, &roots);
    for id in &reachable {
        for keyword in ["readOnly", "writeOnly"] {
            if contract.schema(id).is_some_and(|schema| {
                crate::schema_view::raw(schema)
                    .get(keyword)
                    .is_some_and(|value| value != &serde_json::Value::Bool(false))
            }) {
                errors.push(diagnostic(&contract, id.child(keyword), "java-directional-codec-unsupported", "directional Java projections are not implemented; readOnly/writeOnly must be absent or false"));
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let compiler = OwnedCompiler::new(Config {
        max_depth: 128,
        oas30_nullable_in_31: protocol_config
            .compatibility_profiles
            .contains(&crate::http_protocol::CompatibilityProfile::Oas30NullableIn31V1),
        ..Config::default()
    });
    let compiled = if resources {
        compiler.compile_v3(contract.clone(), &reachable)
    } else {
        compiler.compile_v2(contract.clone(), &reachable)
    }
    .map_err(|errors| {
        errors
            .into_iter()
            .map(|e| HttpDiagnostic {
                source: e.source,
                at: e.span.unwrap_or(0..0),
                code: "java-validation-compile",
                message: format!("{:?}: {}", e.kind, e.message),
            })
            .collect::<Vec<_>>()
    })?;
    let program = compiled.program();
    validation::check(&contract, &program)?;
    // The emitted OAuth class name shares the package namespace with native
    // models; reserve it only while OAuth emission participates, so no-policy
    // allocation behavior is unchanged.
    let mut reserved = BTreeSet::new();
    if let Some(oauth) = &oauth
        && oauth::emittable(oauth)
    {
        reserved.extend(oauth::TOP_LEVEL_NAMES.iter().map(|name| (*name).to_string()));
    }
    if !incoming.is_empty() {
        // The emitted Incoming class name shares the package namespace with
        // native models; reserve it only while receipt emission participates,
        // so receipt-less allocation behavior is unchanged.
        reserved.extend(incoming::TOP_LEVEL_NAMES.iter().map(|name| (*name).to_string()));
    }
    let models = models::plan_models(&contract, &roots, &config, &compiled, &program, reserved)?;
    let operations = http::plan_operations(&wire, &models, &config, credential_env.is_some());
    let examples = if resources {
        crate::examples::plan_protocol_examples_v3(contract.clone(), &wire, Default::default())
    } else {
        crate::examples::plan_protocol_examples_v2(contract.clone(), &wire, Default::default())
    };
    let native_examples = docs::plan_examples(&examples, &operations, &models, &compiled);
    // Receipts the emitted incoming helpers cannot express surface as shared
    // plan errors; emission recomputes the admitted set deterministically.
    incoming::prepare(&contract, &incoming, &models)?;
    Ok(SdkPlan {
        contract,
        config,
        maven,
        models,
        operations,
        program,
        examples,
        native_examples,
        protocol: wire,
        credential_env,
        attribution,
        pagination,
        oauth,
        stream_semantics,
        incoming,
    })
}

pub(crate) fn diagnostic(
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
pub(crate) fn plan_diag(
    contract: &Contract,
    code: &'static str,
    message: impl Into<String>,
) -> HttpDiagnostic {
    diagnostic(
        contract,
        SourceId::new(contract.entry().clone(), Default::default()),
        code,
        message,
    )
}
