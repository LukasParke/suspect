//! Source-selected native Ruby SDKs, backed by the canonical Contract and
//! checked `OwnedCompiler` → `OwnedProgram` validation instructions.
//!
//! Ruby naming, keyword constructors, codecs and transport policy live here.
//! HTTP admission is shared with the other native backends. Model shapes are
//! lowered from compiled instructions; neither the emitter nor generated Ruby
//! interprets OpenAPI/JSON Schema. Unsupported representations block emission.

use std::{collections::BTreeMap, sync::Arc};

use suspect_ir::contract::{Contract, SchemaId, SourceId};
use suspect_schema::{Config, OwnedCompiler, OwnedProgram};

use crate::{OutFile, credential_env, examples, http_protocol};

pub use crate::http_contract::HttpDiagnostic;

mod emit;
mod models;
mod protocol;
mod samples;

pub use models::{
    ExtraFields, ModelField, ModelPlan, ModelShape, ModelSymbol, PatternExtra, ScalarKind,
};
pub use protocol::{
    NativeRecord, NativeType, PlannedBody, PlannedHeader, PlannedMedia, PlannedOperation,
    PlannedParameter, PlannedResponse, RecordBinding, RecordField,
};
pub use samples::{NativeExample, SampleValue};

/// Complete native planning/runtime closure for session and compatibility fingerprints.
pub fn source_assets() -> &'static [(&'static str, &'static [u8])] {
    &[
        ("ruby_sdk.rs", include_bytes!("ruby_sdk.rs")),
        (
            "ruby_sdk/protocol.rs",
            include_bytes!("ruby_sdk/protocol.rs"),
        ),
        ("ruby_sdk/emit.rs", include_bytes!("ruby_sdk/emit.rs")),
        ("ruby_sdk/models.rs", include_bytes!("ruby_sdk/models.rs")),
        ("ruby_sdk/samples.rs", include_bytes!("ruby_sdk/samples.rs")),
        ("ruby_sdk/json.rb", include_bytes!("ruby_sdk/json.rb")),
        (
            "ruby_sdk/resource_guard.rb",
            include_bytes!("ruby_sdk/resource_guard.rb"),
        ),
        (
            "ruby_sdk/validation_v3.rb",
            include_bytes!("ruby_sdk/validation_v3.rb"),
        ),
        (
            "ruby_sdk/program_guard.rb",
            include_bytes!("ruby_sdk/program_guard.rb"),
        ),
        (
            "ruby_sdk/validation_v2.rb",
            include_bytes!("ruby_sdk/validation_v2.rb"),
        ),
        (
            "ruby_sdk/validation.rb",
            include_bytes!("ruby_sdk/validation.rb"),
        ),
        ("ruby_sdk/codecs.rb", include_bytes!("ruby_sdk/codecs.rb")),
        ("ruby_sdk/wire.rb", include_bytes!("ruby_sdk/wire.rb")),
        ("ruby_sdk/payload.rb", include_bytes!("ruby_sdk/payload.rb")),
        ("ruby_sdk/streams.rb", include_bytes!("ruby_sdk/streams.rb")),
        ("ruby_sdk/http.rb", include_bytes!("ruby_sdk/http.rb")),
        (
            "ruby_sdk/credential_env.rb",
            include_bytes!("ruby_sdk/credential_env.rb"),
        ),
        (
            "ruby_sdk/runtime.rbs",
            include_bytes!("ruby_sdk/runtime.rbs"),
        ),
        ("ruby_sdk/GUIDE.md", include_bytes!("ruby_sdk/GUIDE.md")),
    ]
}

/// Finite per-exchange and per-codec resource policy, independent of API semantics.
#[derive(Debug, Clone)]
pub struct RubyConfig {
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub max_capture_bytes: usize,
    pub max_header_bytes: usize,
    pub max_url_bytes: usize,
    pub max_response_chunks: usize,
    pub max_json_depth: usize,
    pub max_json_work: usize,
    pub max_conversion_steps: usize,
    pub max_part_bytes: usize,
    pub max_parts: usize,
    pub max_stream_item_bytes: usize,
    pub max_stream_items: usize,
    pub legacy_binary_strings: bool,
    /// Admission of source-document-relative server URLs after native witnessing.
    pub document_relative_servers: bool,
    /// Admission of the checked canonical-resource v3 profile.
    pub schema_resources: bool,
    /// Admission of entered-resource dynamic binding in the checked v3 profile.
    pub dynamic_schema_references: bool,
    /// Explicit variable-name defaults, read only by configured clients at creation.
    pub credential_env: Option<credential_env::CredentialEnv>,
    pub schema: Config,
}

impl Default for RubyConfig {
    fn default() -> Self {
        Self {
            max_request_bytes: 8 * 1024 * 1024,
            max_response_bytes: 8 * 1024 * 1024,
            max_capture_bytes: 16 * 1024,
            max_header_bytes: 64 * 1024,
            max_url_bytes: 64 * 1024,
            max_response_chunks: 65_536,
            max_json_depth: 128,
            max_json_work: 64 * 1024 * 1024,
            max_conversion_steps: 32 * 1024 * 1024,
            max_part_bytes: 1024 * 1024,
            max_parts: 1024,
            max_stream_item_bytes: 1024 * 1024,
            max_stream_items: 10_000,
            legacy_binary_strings: false,
            document_relative_servers: true,
            schema_resources: true,
            dynamic_schema_references: true,
            credential_env: None,
            schema: Config {
                max_depth: 128,
                ..Config::default()
            },
        }
    }
}

/// Gem identity and presentation. These options never change wire semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageConfig {
    pub name: String,
    pub version: String,
    pub require_name: String,
    pub namespace: String,
}

impl Default for PackageConfig {
    fn default() -> Self {
        Self {
            name: "generated-sdk".into(),
            version: "0.0.0".into(),
            require_name: "generated_sdk".into(),
            namespace: "GeneratedSDK".into(),
        }
    }
}

/// Complete immutable SDK plan. Construction discharges every emitted layer.
#[derive(Debug)]
pub struct SdkPlan {
    contract: Arc<Contract>,
    config: RubyConfig,
    program: OwnedProgram,
    models: ModelPlan,
    operations: Vec<PlannedOperation>,
    examples: examples::ExamplePlan,
    native_examples: Vec<NativeExample>,
    protocol: http_protocol::ProtocolPlan,
    records: Vec<NativeRecord>,
    credential_env: Option<credential_env::CredentialEnvPlan>,
}

impl SdkPlan {
    #[must_use]
    pub fn credential_env(&self) -> Option<&credential_env::CredentialEnvPlan> {
        self.credential_env.as_ref()
    }
    pub fn protocol(&self) -> &http_protocol::ProtocolPlan {
        &self.protocol
    }
    pub fn records(&self) -> &[NativeRecord] {
        &self.records
    }
    pub fn schema_index(&self, id: &SchemaId) -> Option<usize> {
        self.models.source_symbol(id).map(|s| s.schema_index)
    }
    #[must_use]
    pub fn contract(&self) -> &Arc<Contract> {
        &self.contract
    }
    #[must_use]
    pub fn config(&self) -> &RubyConfig {
        &self.config
    }
    #[must_use]
    pub fn program(&self) -> &OwnedProgram {
        &self.program
    }
    #[must_use]
    pub fn models(&self) -> &ModelPlan {
        &self.models
    }
    #[must_use]
    pub fn operations(&self) -> &[PlannedOperation] {
        &self.operations
    }
    #[must_use]
    pub fn examples(&self) -> &examples::ExamplePlan {
        &self.examples
    }
    /// Typed construction trees for validated examples, including source slots.
    #[must_use]
    pub fn native_examples(&self) -> &[NativeExample] {
        &self.native_examples
    }
    /// Source identity for a retained, checked validation node.
    #[must_use]
    pub fn schema_source(&self, index: usize) -> Option<&suspect_schema::ProgramSource> {
        self.program.nodes.get(index).map(|node| &node.source)
    }

    /// Render an installable gem, YARD sources, RBS, provenance and runnable examples.
    ///
    /// # Errors
    /// Invalid package identity fails before any artifact is returned.
    pub fn render(&self) -> Result<Vec<OutFile>, Vec<HttpDiagnostic>> {
        emit_sdk(self, &PackageConfig::default())
    }
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

fn needs_resource_profile(contract: &Contract, closure: &[SchemaId]) -> bool {
    closure.iter().any(|id| {
        if contract
            .schema(id)
            .is_some_and(|schema| schema.ignores_ref_siblings())
        {
            return false;
        }
        contract.dynamic_reference(id).is_some()
            || contract.resource_scope(id).is_some_and(|scope| {
                scope.base_source().is_some()
                    || contract.resource(scope.resource()).is_some_and(|resource| {
                        resource.anchors().iter().any(|anchor| {
                            anchor.kind() == suspect_ir::contract::AnchorKind::Dynamic
                                && anchor.target().document() == id.document()
                                && (anchor.target() == id
                                    || id
                                        .pointer()
                                        .strip_prefix(anchor.target().pointer())
                                        .is_some_and(|suffix| suffix.starts_with('/')))
                        })
                    })
            })
    })
}

/// Plan exactly the selected outgoing operations and their native model closure.
///
/// # Errors
/// Source-linked HTTP, validation, native representation and resource-policy
/// diagnostics block the entire artifact set. No preview/fallback mode exists.
pub fn plan_sdk(
    contract: Arc<Contract>,
    selected: &[SourceId],
    config: RubyConfig,
) -> Result<SdkPlan, Vec<HttpDiagnostic>> {
    let wire = http_protocol::plan(&contract, selected, protocol::capabilities(&config))
        .into_result()
        .map_err(|errors| {
            errors
                .into_iter()
                .map(|e| HttpDiagnostic {
                    source: e.source().source().clone(),
                    at: e.source().span(),
                    code: e.code(),
                    message: e.message().into(),
                })
                .collect::<Vec<_>>()
        })?;
    let credential_env = credential_env::plan(&contract, &wire, config.credential_env.as_ref())?;
    let entry = SourceId::new(contract.entry().clone(), Default::default());
    let mut errors = Vec::new();
    if selected.is_empty() {
        errors.push(diagnostic(
            &contract,
            entry.clone(),
            "ruby-no-operations",
            "select at least one outgoing operation",
        ));
    }
    if config.max_json_depth == 0
        || config.max_json_depth > 128
        || config.schema.max_depth == 0
        || config.schema.max_depth > 128
        || config.schema.max_number_bytes == 0
        || config.schema.max_number_bytes > 4096
        || config.schema.max_errors == 0
        || config.schema.max_errors > 1000
        || [
            config.max_request_bytes,
            config.max_response_bytes,
            config.max_capture_bytes,
            config.max_header_bytes,
            config.max_url_bytes,
            config.max_response_chunks,
            config.max_json_work,
            config.max_conversion_steps,
            config.max_part_bytes,
            config.max_parts,
            config.max_stream_item_bytes,
            config.max_stream_items,
            config.schema.max_evaluation_steps,
            config.schema.max_equality_steps,
        ]
        .iter()
        .any(|n| *n == 0 || *n > 128 * 1024 * 1024)
        || config.max_capture_bytes > config.max_response_bytes
    {
        errors.push(diagnostic(&contract, entry.clone(), "ruby-resource-policy", "positive bounded limits are required: depth <=128, numeric tokens <=4096 bytes, findings <=1000, byte/work ceilings <=128 MiB, capture <=response ceiling"));
    }
    // The protocol owns the effective candidate-aware schema closure. These are
    // compilation dependencies, not additional HTTP body inputs or dynamic
    // target selections.
    let reachable = wire.codec_schema_closure().to_vec();
    for id in &reachable {
        if contract
            .schema(id)
            .is_some_and(|schema| crate::schema_view::reference_only(schema))
        {
            continue;
        }
        if let Some(raw) = contract.source(id) {
            for keyword in ["readOnly", "writeOnly"] {
                if raw
                    .get(keyword)
                    .is_some_and(|v| v != &serde_json::Value::Bool(false))
                {
                    errors.push(diagnostic(&contract, id.child(keyword), "ruby-directional-codec-unsupported", "the neutral Ruby codec profile requires directional annotations to be absent or false"));
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let compiler = OwnedCompiler::new(config.schema.clone());
    let compiled = if needs_resource_profile(&contract, &reachable) {
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
                code: "ruby-schema-compilation",
                message: format!("{:?}: {}", e.kind, e.message),
            })
            .collect::<Vec<_>>()
    })?;
    let program = compiled.program();
    if !matches!(
        (program.version, program.profile),
        (OwnedProgram::V1_VERSION, OwnedProgram::V1_PROFILE)
            | (OwnedProgram::V2_VERSION, OwnedProgram::V2_PROFILE)
            | (OwnedProgram::V3_VERSION, OwnedProgram::V3_PROFILE)
    ) {
        return Err(vec![diagnostic(
            &contract,
            entry.clone(),
            "ruby-validation-profile",
            "Ruby admits only the exact checked v1, scoped-applicator v2 and resource/dynamic v3 profiles",
        )]);
    }
    program.check().map_err(|error| {
        vec![diagnostic(
            &contract,
            entry.clone(),
            "ruby-validation-program",
            error.to_string(),
        )]
    })?;
    if program.nodes.len() > 10_000
        || serde_json::to_vec(&program).expect("checked program").len() > 16 * 1024 * 1024
    {
        return Err(vec![diagnostic(
            &contract,
            entry,
            "ruby-program-size",
            "the Ruby profile supports at most 10000 compiled nodes and 16 MiB of validation metadata",
        )]);
    }
    // Shared compiler growth must not silently admit a new Ruby runtime opcode.
    // Numeric const/enum tokens also need to be representable while loading the
    // package's metadata, independently of the per-evaluation operand limit.
    for node in &program.nodes {
        for check in &node.checks {
            let serialized = serde_json::to_value(check).expect("checked instruction");
            let supported = [
                "always",
                "type",
                "ref",
                "dynamicRef",
                "allOf",
                "anyOf",
                "oneOf",
                "not",
                "properties",
                "additionalProperties",
                "items",
                "prefixItems",
                "required",
                "bound",
                "multipleOf",
                "count",
                "const",
                "enum",
                "uniqueItems",
                "pattern",
                "if",
                "dependentRequired",
                "dependentSchemas",
                "contains",
                "patternProperties",
                "additionalPropertiesWithPatterns",
                "propertyNames",
                "unevaluatedProperties",
                "unevaluatedItems",
            ];
            let op = serialized["op"].as_str().expect("instruction tag");
            let id = || {
                reachable
                    .iter()
                    .find(|id| {
                        id.document().as_str() == node.source.document
                            && id.pointer() == node.source.pointer
                    })
                    .expect("selected compiled source")
                    .clone()
            };
            if !supported.contains(&op) {
                errors.push(diagnostic(
                    &contract,
                    id(),
                    "ruby-validation-instruction",
                    format!("the Ruby runtime has not admitted portable instruction {op}"),
                ));
            }
            let mut values = match &check.instruction {
                suspect_schema::ProgramInstruction::Const { value } => vec![value],
                suspect_schema::ProgramInstruction::Enum { values } => values.iter().collect(),
                _ => Vec::new(),
            };
            while let Some(value) = values.pop() {
                match value {
                    serde_json::Value::Number(number) if number.to_string().len() > 4096 => {
                        errors.push(diagnostic(
                            &contract,
                            id().child(op),
                            "ruby-numeric-literal-limit",
                            "a compiled literal exceeds Ruby's 4096-byte exact JSON token ceiling",
                        ));
                        break;
                    }
                    serde_json::Value::Array(children) => values.extend(children),
                    serde_json::Value::Object(children) => values.extend(children.values()),
                    _ => {}
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let indices: BTreeMap<_, _> = program
        .roots
        .iter()
        .map(|r| {
            (
                (r.source.document.clone(), r.source.pointer.clone()),
                r.target,
            )
        })
        .collect();
    let indices = reachable
        .iter()
        .map(|id| {
            (
                id.clone(),
                indices[&(id.document().to_string(), id.pointer().to_owned())],
            )
        })
        .collect::<BTreeMap<_, _>>();
    let hints = protocol::hints(&wire, &indices);
    let roots = wire
        .codec_roots()
        .iter()
        .map(|id| indices[id])
        .collect::<Vec<_>>();
    let models = models::plan(&contract, &program, &roots, hints)?;
    let (operations, records) = protocol::lower(&wire, &indices, &models);
    let examples = if program.version == OwnedProgram::V3_VERSION {
        examples::plan_protocol_examples_v3(
            contract.clone(),
            &wire,
            examples::ExampleConfig::default(),
        )
    } else if program.version == OwnedProgram::V2_VERSION {
        examples::plan_protocol_examples_v2(
            contract.clone(),
            &wire,
            examples::ExampleConfig::default(),
        )
    } else {
        examples::plan_protocol_examples(
            contract.clone(),
            &wire,
            examples::ExampleConfig::default(),
        )
    };
    let native_examples = samples::plan(
        &models,
        &examples,
        program.version == OwnedProgram::V3_VERSION,
    );
    Ok(SdkPlan {
        contract,
        config,
        program,
        models,
        operations,
        examples,
        native_examples,
        protocol: wire,
        records,
        credential_env,
    })
}

/// Emit a complete package through the common artifact-writer interface.
///
/// # Errors
/// Unsafe or colliding gem/require/namespace identities block all output.
pub fn emit_sdk(
    plan: &SdkPlan,
    package: &PackageConfig,
) -> Result<Vec<OutFile>, Vec<HttpDiagnostic>> {
    let valid_name = !package.name.is_empty()
        && package.name.len() <= 64
        && package.name.starts_with(|c: char| c.is_ascii_lowercase())
        && package
            .name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c));
    let valid_require = !package.require_name.is_empty()
        && package.require_name.len() <= 64
        && package
            .require_name
            .starts_with(|c: char| c.is_ascii_lowercase())
        && package
            .require_name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        && ![
            "abbrev",
            "base64",
            "benchmark",
            "bigdecimal",
            "bundler",
            "cgi",
            "csv",
            "date",
            "delegate",
            "digest",
            "drb",
            "english",
            "erb",
            "etc",
            "fcntl",
            "fiddle",
            "fileutils",
            "find",
            "forwardable",
            "getoptlong",
            "ipaddr",
            "irb",
            "json",
            "logger",
            "monitor",
            "mutex_m",
            "net",
            "nkf",
            "observer",
            "openssl",
            "optparse",
            "ostruct",
            "pathname",
            "pp",
            "prettyprint",
            "pstore",
            "psych",
            "racc",
            "rdoc",
            "readline",
            "reline",
            "resolv",
            "rinda",
            "rubygems",
            "securerandom",
            "set",
            "shellwords",
            "singleton",
            "socket",
            "stringio",
            "strscan",
            "tempfile",
            "time",
            "timeout",
            "tmpdir",
            "tsort",
            "uri",
            "weakref",
            "yaml",
            "zlib",
        ]
        .contains(&package.require_name.as_str());
    let valid_namespace = !package.namespace.is_empty()
        && package.namespace.len() <= 64
        && package
            .namespace
            .starts_with(|c: char| c.is_ascii_uppercase())
        && package
            .namespace
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
        && !models::reserved_constants().contains(&package.namespace);
    let valid_version = semver::Version::parse(&package.version)
        .is_ok_and(|v| v.pre.is_empty() && v.build.is_empty());
    if !valid_name || !valid_require || !valid_namespace || !valid_version {
        return Err(vec![diagnostic(
            &plan.contract,
            SourceId::new(plan.contract.entry().clone(), Default::default()),
            "ruby-package-identity",
            "use an ASCII gem name, collision-safe snake_case require name, UpperCamel namespace and exact three-part release version",
        )]);
    }
    Ok(emit::package(plan, package))
}
