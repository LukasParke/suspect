//! Admission: bind the closed mapping to actual allocated TypeScript SDK
//! names, models and codecs.
//!
//! Every refusal is located at both its mapping pointer and its contract
//! source. Nothing here inspects emitted TypeScript, reinterprets a source
//! schema, or invents a public name the mapping did not supply.
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use serde_json::Value;
use suspect_ir::contract::{Contract, SchemaId, SourceId};

use crate::{
    application::{Diagnostic, DocumentScope, select_operation},
    http_protocol as protocol,
    typescript::{
        ModelSymbol, ModelView,
        http::{self as ts_http, protocol_emit},
        package,
    },
};

use super::{
    BodyBinding, MappingProfile, McpTargetConfig, Representation, ServerPlan, ToolAdvisory,
    ToolMapping,
    schema::{self, Adapter},
};

/// Tool names reserved so a public tool can never be mistaken for an MCP
/// protocol method family in a client's own surface.
pub const RESERVED_TOOL_NAMES: &[&str] = &[
    "completion",
    "elicitation",
    "initialize",
    "logging",
    "mcp",
    "notifications",
    "ping",
    "prompts",
    "resources",
    "roots",
    "sampling",
    "tasks",
    "tools",
];

/// One admitted tool: public names on the left, allocated SDK names on the right.
#[derive(Debug)]
pub(super) struct BoundTool {
    pub name: String,
    pub title: String,
    pub description: String,
    pub annotations: ToolAdvisory,
    /// Index into [`ts_http::HttpPlan::operations`].
    pub operation: usize,
    /// Input properties in source order: parameters first, then the body.
    pub inputs: Vec<BoundInput>,
    pub responses: Vec<BoundResponse>,
    /// Allocated unique TypeScript identifier stem for this tool's constants.
    pub symbol: String,
}

#[derive(Debug)]
pub(super) struct BoundInput {
    /// Public tool input property name.
    pub property: String,
    /// Exact source parameter name, or `None` for the request body.
    pub parameter: Option<String>,
    /// Surface location: a source parameter location, or `"body"`.
    pub location: &'static str,
    pub required: bool,
    /// Native SDK input member name.
    pub member: String,
    /// Allocated SDK model name.
    pub model: String,
    /// Allocated SDK codec export name.
    pub codec: String,
    /// Declared media type, present exactly for the request body.
    pub media_type: Option<String>,
    pub representation: Representation,
    /// Projected MCP JSON Schema for this property.
    pub schema: Value,
    pub adapter: Adapter,
    pub schema_source: SchemaId,
}

/// One declared response, with the exact concrete statuses it selects in each
/// direction and the generated codec that decodes its body.
#[derive(Debug)]
pub(super) struct BoundResponse {
    pub status: Value,
    /// Success statuses that carry a declared body.
    pub success_document: Vec<u16>,
    /// Success statuses whose body HTTP itself suppresses.
    pub success_empty: Vec<u16>,
    /// Failure statuses that carry a declared body.
    pub failure_document: Vec<u16>,
    /// Failure statuses whose body HTTP itself suppresses.
    pub failure_empty: Vec<u16>,
    /// Allocated SDK codec export name, present when any status carries a body.
    pub codec: Option<String>,
    pub model: Option<String>,
    pub media_type: Option<String>,
    pub schema_source: Option<SchemaId>,
}

impl BoundResponse {
    pub(super) fn carries_document(&self) -> bool {
        !self.success_document.is_empty() || !self.failure_document.is_empty()
    }
    pub(super) fn success(&self) -> bool {
        !self.success_document.is_empty() || !self.success_empty.is_empty()
    }
    pub(super) fn failure(&self) -> bool {
        !self.failure_document.is_empty() || !self.failure_empty.is_empty()
    }
    pub(super) fn representation(&self) -> &'static str {
        if self.carries_document() {
            "exact_json_document"
        } else {
            "no_content"
        }
    }
}

fn root(contract: &Contract) -> SourceId {
    SourceId::new(contract.entry().clone(), Default::default())
}

pub(super) fn plan(
    contract: Arc<Contract>,
    mapping: MappingProfile,
    config: McpTargetConfig,
) -> Result<ServerPlan, Vec<Diagnostic>> {
    if mapping.format != super::PROFILE {
        return Err(vec![Diagnostic::new(
            &contract,
            root(&contract),
            "/format",
            "mcp-mapping-version",
            format!(
                "unknown MCP mapping profile {:?}; this target implements {:?} only",
                mapping.format,
                super::PROFILE
            ),
        )]);
    }

    let mut errors = Vec::new();
    identity(&contract, &config, &mut errors);
    public_names(&contract, &mapping, &mut errors);
    let selected = selections(&contract, &mapping, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }

    // The SDK plans exactly the mapped operations, once each, in mapping order.
    let mut sources = Vec::new();
    for source in &selected {
        if !sources.contains(source) {
            sources.push(source.clone());
        }
    }
    // The expanded profile is the enumerated, natively tested protocol family
    // set. An application target must be able to expose ordinary API-key and
    // absolute-server contracts; every unsupported construct still refuses.
    let sdk = ts_http::plan_http(
        contract.clone(),
        &sources,
        ts_http::HttpConfig {
            credential_env: config.credential_env.clone(),
            ..ts_http::HttpConfig::expanded()
        },
    )
    .map_err(|findings| {
        findings
            .into_iter()
            .map(|finding| Diagnostic {
                code: finding.code,
                message: finding.message,
                mapping_pointer: "/tools".into(),
                source: finding.source,
                at: finding.at,
            })
            .collect::<Vec<_>>()
    })?;

    let identity = package::PackageConfig {
        name: config.package_name.clone(),
        version: config.version.clone(),
    };
    let sdk_files = match package::emit_http(&sdk, &identity) {
        Ok(files) => files,
        Err(error) => {
            return Err(vec![Diagnostic::new(
                &contract,
                root(&contract),
                "/package_name",
                "mcp-package-identity",
                error.to_string(),
            )]);
        }
    };

    // A tool input never carries a credential, so an operation whose source
    // security is mandatory can only be exposed with an explicit credential
    // environment policy behind it.
    if sdk
        .operations()
        .iter()
        .any(|operation| operation.interface()["client"]["optionsRequired"] == Value::Bool(true))
    {
        return Err(vec![Diagnostic::new(
            &contract,
            root(&contract),
            "/credential_env",
            "mcp-credentials-unconfigured",
            "the selected operations require credentials, which tool inputs never carry: configure credential_env so the generated SDK reads them from named environment variables",
        )]);
    }

    let native = Sdk {
        symbols: sdk
            .codecs()
            .models()
            .symbols()
            .iter()
            .map(|symbol| (symbol.name(), symbol))
            .collect(),
        plan: &sdk,
    };

    let mut tools = Vec::new();
    let mut used = BTreeSet::new();
    for (index, tool) in mapping.tools.iter().enumerate() {
        // The SDK was asked to plan exactly these sources, but a Path Item
        // reached through a `$ref` resolves to a source the plan does not
        // record, so this is a refusal rather than an assertion.
        let Some(operation) = sdk
            .operations()
            .iter()
            .position(|planned| planned.source == selected[index])
        else {
            errors.push(crate::application::unplanned_operation(
                &contract,
                format!("/tools/{index}/selector"),
                &tool.selector,
                &selected[index],
            ));
            continue;
        };
        if let Some(bound) = bind(
            &contract,
            &native,
            index,
            tool,
            operation,
            &mut used,
            &mut errors,
        ) {
            tools.push(bound);
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    // Every document the surface manifest will name must have a relative form,
    // so an out-of-tree document is refused here rather than leaking an
    // absolute generator path into the review artifact at emission.
    let scope = DocumentScope::new(&contract);
    manifest_documents(&contract, &sdk, &tools, &scope, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(ServerPlan {
        credential_bindings: sdk.credential_env().map(|policy| {
            policy
                .bindings()
                .iter()
                .map(|binding| (binding.name().to_owned(), binding.variable().to_owned()))
                .collect()
        }),
        sdk,
        sdk_files,
        mapping,
        config,
        tools,
        scope,
    })
}

/// Admit every contract document the surface manifest records: each tool's
/// selected operation, each input property's schema and each declared
/// response's body schema.
fn manifest_documents(
    contract: &Contract,
    sdk: &ts_http::HttpPlan,
    tools: &[BoundTool],
    scope: &DocumentScope,
    errors: &mut Vec<Diagnostic>,
) {
    for (index, tool) in tools.iter().enumerate() {
        let operation = &sdk.operations()[tool.operation];
        if let Err(error) = scope.admit(
            contract,
            format!("/tools/{index}/selector"),
            &operation.source,
        ) {
            errors.push(error);
        }
        for input in &tool.inputs {
            let pointer = match &input.parameter {
                Some(parameter) => format!("/tools/{index}/input/parameters/{parameter}"),
                None => format!("/tools/{index}/input/body"),
            };
            if let Err(error) = scope.admit(contract, pointer, &input.schema_source) {
                errors.push(error);
            }
        }
        for response in &tool.responses {
            if let Some(source) = &response.schema_source
                && let Err(error) = scope.admit(contract, format!("/tools/{index}"), source)
            {
                errors.push(error);
            }
        }
    }
}

/// Package, executable, server and toolchain identity the emitted package
/// cannot repair, plus the finite runtime bounds compiled into it.
fn identity(contract: &Contract, config: &McpTargetConfig, errors: &mut Vec<Diagnostic>) {
    let mut refuse = |pointer: &str, code: &'static str, message: String| {
        errors.push(Diagnostic::new(
            contract,
            root(contract),
            pointer,
            code,
            message,
        ));
    };
    let name = config.bin_name.as_str();
    let usable = !name.is_empty()
        && name.len() <= 64
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.ends_with('-')
        && !name.contains("--");
    if !usable {
        refuse(
            "/bin_name",
            "mcp-bin-name",
            format!(
                "bin_name {name:?} must be 1..=64 lower-case letters, digits and single interior dashes"
            ),
        );
    }
    let server = config.server_name.as_str();
    if server.is_empty() || server.len() > 128 || server.chars().any(char::is_control) {
        refuse(
            "/server_name",
            "mcp-server-identity",
            format!("server_name {server:?} must be 1..=128 bytes without control characters"),
        );
    }
    if let Some(variable) = &config.server_url_env
        && let Err(reason) = crate::credential_env::validate_variable_name(variable)
    {
        refuse(
            "/server_url_env",
            "mcp-server-identity",
            format!("server_url_env {variable:?} {reason}"),
        );
    }
    if !(1..=3_600_000).contains(&config.runtime.call_deadline_ms) {
        refuse(
            "/runtime/call_deadline_ms",
            "mcp-runtime-bounds",
            format!(
                "call_deadline_ms {} must be 1..=3600000",
                config.runtime.call_deadline_ms
            ),
        );
    }
    for (pointer, field, value) in [
        (
            "/runtime/max_input_bytes",
            "max_input_bytes",
            config.runtime.max_input_bytes,
        ),
        (
            "/runtime/max_result_bytes",
            "max_result_bytes",
            config.runtime.max_result_bytes,
        ),
    ] {
        if !(1..=8 * 1024 * 1024).contains(&value) {
            refuse(
                pointer,
                "mcp-runtime-bounds",
                format!("{field} {value} must be 1..=8388608"),
            );
        }
    }
    for (pointer, field, actual, verified) in [
        (
            "/mcp_server_version",
            "mcp_server_version",
            config.mcp_server_version.as_str(),
            super::MCP_SERVER_VERSION,
        ),
        (
            "/mcp_client_version",
            "mcp_client_version",
            config.mcp_client_version.as_str(),
            super::MCP_CLIENT_VERSION,
        ),
        (
            "/typescript_version",
            "typescript_version",
            config.typescript_version.as_str(),
            super::TYPESCRIPT_VERSION,
        ),
        (
            "/node_types_version",
            "node_types_version",
            config.node_types_version.as_str(),
            super::NODE_TYPES_VERSION,
        ),
    ] {
        if actual != verified {
            refuse(
                pointer,
                "mcp-toolchain-pin",
                format!(
                    "{field} {actual:?} is not the registry-verified pin this target implements ({verified:?})"
                ),
            );
        }
    }
    if config.node_minimum_major < super::NODE_MINIMUM_MAJOR {
        refuse(
            "/node_minimum_major",
            "mcp-toolchain-pin",
            format!(
                "node_minimum_major {} is below the floor the emitted application actually requires ({}): the application depends on the official MCP SDK and embeds the canonical TypeScript SDK as compiled-in source, so `engines.node` must not claim a looser runtime than either of them supports",
                config.node_minimum_major,
                super::NODE_MINIMUM_MAJOR
            ),
        );
    }
    if !exact_version(&config.node_version) {
        refuse(
            "/node_version",
            "mcp-toolchain-pin",
            format!(
                "node_version {:?} must be an exact `X.Y.Z` version",
                config.node_version
            ),
        );
    }
    if !exact_version(&config.npm_version) {
        refuse(
            "/npm_version",
            "mcp-toolchain-pin",
            format!(
                "npm_version {:?} must be an exact `X.Y.Z` version",
                config.npm_version
            ),
        );
    }
}

/// `X.Y.Z` of ASCII digits, as a reviewed toolchain release is spelled.
fn exact_version(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty() && part.len() <= 6 && part.bytes().all(|byte| byte.is_ascii_digit())
        })
}

/// Every public tool and property name, before any contract binding.
/// Duplicate and reserved names are refused here so no two tools, and no two
/// properties of one tool, can ever claim the same public name.
fn public_names(contract: &Contract, mapping: &MappingProfile, errors: &mut Vec<Diagnostic>) {
    let mut refuse = |pointer: String, code: &'static str, message: String| {
        errors.push(Diagnostic::new(
            contract,
            root(contract),
            pointer,
            code,
            message,
        ));
    };
    let mut names: BTreeSet<&str> = BTreeSet::new();
    for (index, tool) in mapping.tools.iter().enumerate() {
        let pointer = format!("/tools/{index}/name");
        if !tool_name(&tool.name) {
            refuse(
                pointer,
                "mcp-tool-name",
                format!(
                    "tool name {:?} must be 1..=64 characters of lower-case letters, digits, underscores and single interior dashes, starting with a letter",
                    tool.name
                ),
            );
        } else if RESERVED_TOOL_NAMES.contains(&tool.name.as_str()) {
            refuse(
                pointer,
                "mcp-tool-reserved",
                format!(
                    "tool name {:?} names an MCP protocol method family and is reserved",
                    tool.name
                ),
            );
        } else if !names.insert(tool.name.as_str()) {
            refuse(
                pointer,
                "mcp-tool-collision",
                format!("tool name {:?} is already mapped", tool.name),
            );
        }
    }
    for (index, tool) in mapping.tools.iter().enumerate() {
        let mut properties: BTreeSet<&str> = BTreeSet::new();
        let mut declared: Vec<(String, &str)> = tool
            .input
            .parameters
            .iter()
            .map(|(parameter, entry)| {
                (
                    format!("/tools/{index}/input/parameters/{parameter}/property"),
                    entry.property.as_str(),
                )
            })
            .collect();
        if let BodyBinding::JsonValue { property, .. } = &tool.input.body {
            declared.push((
                format!("/tools/{index}/input/body/property"),
                property.as_str(),
            ));
        }
        for (pointer, property) in declared {
            if !property_name(property) {
                refuse(
                    pointer,
                    "mcp-property-name",
                    format!(
                        "input property {property:?} must be 1..=64 characters of letters, digits and underscores, starting with a lower-case letter"
                    ),
                );
            } else if !properties.insert(property) {
                refuse(
                    pointer,
                    "mcp-property-collision",
                    format!("input property {property:?} is already mapped on this tool"),
                );
            }
        }
    }
}

/// Tool name token: lower-case letters, digits, underscores and single
/// interior dashes, within the MCP tool-name character set and warning-free.
fn tool_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.starts_with(|c: char| c.is_ascii_lowercase())
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Input property token: a plain JSON object member a client can type.
fn property_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.starts_with(|c: char| c.is_ascii_lowercase())
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Resolve every selector through the shared application selector, keeping a
/// placeholder for refused entries so later indexes stay aligned.
fn selections(
    contract: &Contract,
    mapping: &MappingProfile,
    errors: &mut Vec<Diagnostic>,
) -> Vec<SourceId> {
    let mut selected = Vec::with_capacity(mapping.tools.len());
    for (index, tool) in mapping.tools.iter().enumerate() {
        let pointer = format!("/tools/{index}/selector");
        match select_operation(contract, &pointer, &tool.selector) {
            Ok(operation) => selected.push(operation.source().clone()),
            Err(error) => {
                errors.push(error);
                selected.push(root(contract));
            }
        }
    }
    selected
}

/// The allocated model symbol for one schema in one direction. A plan without
/// directional annotations carries neutral symbols for both directions.
fn symbol<'a>(
    symbols: &BTreeMap<&str, &'a ModelSymbol>,
    schema: &SchemaId,
    view: ModelView,
) -> Option<&'a ModelSymbol> {
    let mut neutral = None;
    for candidate in symbols.values() {
        if candidate.source() != schema {
            continue;
        }
        if candidate.view() == view {
            return Some(candidate);
        }
        if candidate.view() == ModelView::Neutral {
            neutral = Some(*candidate);
        }
    }
    neutral
}

/// The actual generated SDK a tool binds to: the plan and its allocated model
/// symbols, which are only ever consulted together.
struct Sdk<'a> {
    plan: &'a ts_http::HttpPlan,
    symbols: BTreeMap<&'a str, &'a ModelSymbol>,
}

impl Sdk<'_> {
    /// The allocated codec export name of one model, read from the codec plan's
    /// own interface rather than re-derived from the model name.
    fn codec(&self, model: &str) -> String {
        self.plan.codecs().interfaces()[model]["name"]
            .as_str()
            .expect("allocated codec export name")
            .to_owned()
    }
}

/// Exactly one concrete JSON media carrying a generated codec, or nothing.
fn exact_json(media: &[protocol::MediaPlan]) -> Option<(&protocol::CodecRef, String)> {
    let [single] = media else { return None };
    if !matches!(
        single.media_type().range(),
        protocol::MediaRange::Concrete { .. }
    ) {
        return None;
    }
    match single.representation() {
        protocol::Representation::Json { codec: Some(codec) } => {
            Some((codec, single.media_type().declared().to_owned()))
        }
        _ => None,
    }
}

fn location(value: protocol::ParameterLocation) -> &'static str {
    match value {
        protocol::ParameterLocation::Path => "path",
        protocol::ParameterLocation::Query => "query",
        protocol::ParameterLocation::Querystring => "querystring",
        protocol::ParameterLocation::Header => "header",
        protocol::ParameterLocation::Cookie => "cookie",
    }
}

/// Combine the mapping's own prose with the projection's representation note,
/// so a client reads both what the property means and how to spell its value.
fn description(mapped: &str, schema: &mut Value) {
    let note = schema
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let text = match note {
        Some(note) if mapped.is_empty() => note,
        Some(note) => format!("{mapped} {note}"),
        None => mapped.to_owned(),
    };
    schema["description"] = Value::from(text);
}

#[allow(clippy::too_many_lines)]
fn bind(
    contract: &Contract,
    sdk: &Sdk<'_>,
    index: usize,
    tool: &ToolMapping,
    operation: usize,
    used: &mut BTreeSet<String>,
    errors: &mut Vec<Diagnostic>,
) -> Option<BoundTool> {
    let symbols = &sdk.symbols;
    let planned = &sdk.plan.operations()[operation];
    let wire = planned.protocol();
    let before = errors.len();
    let mut refuse = |pointer: String, source: SourceId, code: &'static str, message: String| {
        errors.push(Diagnostic::new(contract, source, pointer, code, message));
    };

    let mut inputs = Vec::new();
    for (parameter, native) in wire.parameters().iter().zip(&planned.parameters) {
        let name = parameter.name();
        let use_site = parameter.source().use_site().source().clone();
        let Some(entry) = tool.input.parameters.get(name) else {
            refuse(
                format!("/tools/{index}/input/parameters"),
                use_site,
                "mcp-parameter-unmapped",
                format!(
                    "source parameter {name:?} has no mapped tool input property; every parameter of an exposed operation needs one"
                ),
            );
            continue;
        };
        let schema = parameter.codec().schema().id();
        let Some(model) = symbol(symbols, schema, ModelView::Request) else {
            refuse(
                format!("/tools/{index}/input/parameters/{name}"),
                use_site,
                "mcp-unsupported-schema",
                format!("source parameter {name:?} has no generated model"),
            );
            continue;
        };
        match schema::project(symbols, model.expression()) {
            Ok(projection) => {
                let mut property = projection.schema;
                description(&entry.description, &mut property);
                inputs.push(BoundInput {
                    property: entry.property.clone(),
                    parameter: Some(name.to_owned()),
                    location: location(parameter.location()),
                    required: parameter.required(),
                    member: native.native_name.clone(),
                    model: model.name().to_owned(),
                    codec: sdk.codec(model.name()),
                    media_type: None,
                    representation: projection.representation,
                    schema: property,
                    adapter: projection.adapter,
                    schema_source: model.source().clone(),
                });
            }
            Err(refusal) => refuse(
                format!("/tools/{index}/input/parameters/{name}"),
                refusal.source.unwrap_or(use_site),
                "mcp-unsupported-schema",
                format!(
                    "source parameter {name:?} has no exact tool input projection: {}",
                    refusal.message
                ),
            ),
        }
    }
    let known: BTreeSet<&str> = wire
        .parameters()
        .iter()
        .map(protocol::ParameterPlan::name)
        .collect();
    for parameter in tool.input.parameters.keys() {
        if !known.contains(parameter.as_str()) {
            refuse(
                format!("/tools/{index}/input/parameters/{parameter}"),
                planned.source.clone(),
                "mcp-parameter-unknown",
                format!("{parameter:?} is not a parameter of the selected operation"),
            );
        }
    }

    match (wire.body(), &tool.input.body) {
        (None, BodyBinding::None) => {}
        (None, BodyBinding::JsonValue { .. }) => refuse(
            format!("/tools/{index}/input/body"),
            planned.source.clone(),
            "mcp-body-policy",
            "the selected operation declares no request body".into(),
        ),
        (Some(declared), BodyBinding::None) => refuse(
            format!("/tools/{index}/input/body"),
            declared.source().use_site().source().clone(),
            "mcp-body-policy",
            "the selected operation declares a request body, which the mapping must expose".into(),
        ),
        (
            Some(declared),
            BodyBinding::JsonValue {
                property,
                description: mapped,
            },
        ) => {
            let use_site = declared.source().use_site().source().clone();
            match exact_json(declared.media()) {
                None => refuse(
                    format!("/tools/{index}/input/body"),
                    use_site,
                    "mcp-unsupported-media",
                    "a request body needs exactly one concrete JSON media with a generated codec"
                        .into(),
                ),
                Some((codec, media_type)) => {
                    match symbol(symbols, codec.schema().id(), ModelView::Request) {
                        None => refuse(
                            format!("/tools/{index}/input/body"),
                            use_site,
                            "mcp-unsupported-schema",
                            "the declared request body has no generated model".into(),
                        ),
                        Some(model) => match schema::project(symbols, model.expression()) {
                            Ok(projection) => {
                                let mut schema = projection.schema;
                                description(mapped, &mut schema);
                                inputs.push(BoundInput {
                                    property: property.clone(),
                                    parameter: None,
                                    location: "body",
                                    required: declared.required(),
                                    member: "body".into(),
                                    model: model.name().to_owned(),
                                    codec: sdk.codec(model.name()),
                                    media_type: Some(media_type),
                                    representation: Representation::BodyValue,
                                    schema,
                                    adapter: projection.adapter,
                                    schema_source: model.source().clone(),
                                });
                            }
                            Err(refusal) => refuse(
                                format!("/tools/{index}/input/body"),
                                refusal.source.unwrap_or(use_site),
                                "mcp-unsupported-schema",
                                format!(
                                    "the declared request body has no exact tool input projection: {}",
                                    refusal.message
                                ),
                            ),
                        },
                    }
                }
            }
        }
    }

    let mut responses = Vec::new();
    for response in wire.responses() {
        let use_site = response.source().use_site().source().clone();
        let success_document = protocol_emit::statuses(wire, response, true, false);
        let success_empty = protocol_emit::statuses(wire, response, true, true);
        let failure_document = protocol_emit::statuses(wire, response, false, false);
        let failure_empty = protocol_emit::statuses(wire, response, false, true);
        let carries = !success_document.is_empty() || !failure_document.is_empty();
        let mut bound = BoundResponse {
            status: match response.status() {
                protocol::ResponseStatus::Exact(code) => Value::from(code),
                _ => Value::from(response.status_key()),
            },
            success_document,
            success_empty,
            failure_document,
            failure_empty,
            codec: None,
            model: None,
            media_type: None,
            schema_source: None,
        };
        if carries {
            match exact_json(response.media()) {
                Some((codec, media_type)) => {
                    match symbol(symbols, codec.schema().id(), ModelView::Response) {
                        Some(model) => {
                            bound.codec = Some(sdk.codec(model.name()));
                            bound.model = Some(model.name().to_owned());
                            bound.media_type = Some(media_type);
                            bound.schema_source = Some(model.source().clone());
                        }
                        None => {
                            refuse(
                                format!("/tools/{index}"),
                                use_site,
                                "mcp-unsupported-schema",
                                format!(
                                    "response {:?} has no generated model",
                                    response.status_key()
                                ),
                            );
                            continue;
                        }
                    }
                }
                None => {
                    refuse(
                        format!("/tools/{index}"),
                        use_site,
                        "mcp-unsupported-media",
                        format!(
                            "response {:?} needs either no content or exactly one concrete JSON media with a generated codec",
                            response.status_key()
                        ),
                    );
                    continue;
                }
            }
        }
        responses.push(bound);
    }

    if errors.len() != before {
        return None;
    }
    let stem = identifier(&tool.name);
    let mut symbol_name = stem.clone();
    let mut suffix = 2;
    while !used.insert(symbol_name.clone()) {
        symbol_name = format!("{stem}{suffix}");
        suffix += 1;
    }
    Some(BoundTool {
        name: tool.name.clone(),
        title: tool.title.clone(),
        description: tool.description.clone(),
        annotations: tool.annotations,
        operation,
        inputs,
        responses,
        symbol: symbol_name,
    })
}

/// A TypeScript identifier fragment from a public tool name. Public names are
/// already restricted to lower-case letters, digits, underscores and dashes.
fn identifier(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for character in name.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            out.push(character);
        } else {
            out.push('_');
        }
    }
    out
}
