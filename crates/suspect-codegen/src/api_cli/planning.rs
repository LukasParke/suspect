//! Admission: bind the closed mapping to actual allocated Go SDK names.
//!
//! Every refusal is located at both its mapping pointer and its contract
//! source. Nothing here inspects emitted Go source, reinterprets a schema, or
//! invents a public name the mapping did not supply.
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use suspect_ir::contract::{Contract, SchemaId, SourceId};

use crate::{
    application::{Diagnostic, DocumentScope, select_operation},
    go_http,
    go_models::{GoDecl, GoDescriptors, GoType},
    http_protocol as protocol,
    rust_models::RepresentationRole,
};

use super::{
    BodyPolicy, CliPlan, CliTargetConfig, CommandMapping, ConfirmationPolicy, MappingProfile,
    Representation,
};

/// Command names Cobra itself owns; a mapping may never claim them.
const RESERVED_COMMANDS: &[&str] = &["help", "completion", "__complete", "__completeNoDesc"];

/// Flag names the generated runtime owns on every command or on the root.
const RESERVED_FLAGS: &[&str] = &[
    "help",
    "body",
    "body-file",
    "confirm",
    "server-url",
    "version",
];

/// The generated flag that supplies a request body document.
pub(super) const BODY_FLAG: &str = "body-file";

/// The generated flag that supplies a confirmation token.
pub(super) const CONFIRM_FLAG: &str = "confirm";

/// One admitted command: public names on the left, allocated SDK names on the right.
#[derive(Debug)]
pub(super) struct BoundCommand {
    pub path: Vec<String>,
    pub summary: String,
    pub description: String,
    /// Index into [`go_http::HttpPlan::operations`].
    pub operation: usize,
    /// Flags in source parameter order.
    pub flags: Vec<BoundFlag>,
    pub body: Option<BoundBody>,
    pub confirmation: ConfirmationPolicy,
    pub responses: Vec<BoundResponse>,
    /// Allocated unique Go identifier stem for this command's generated
    /// constructor and failure reporter.
    pub symbol: String,
}

#[derive(Debug)]
pub(super) struct BoundFlag {
    pub flag: String,
    pub parameter: String,
    pub location: &'static str,
    pub required: bool,
    pub representation: Representation,
    pub description: String,
    /// Allocated codec/model name in the generated SDK.
    pub model: String,
    pub setter_name: Option<String>,
    pub schema: SchemaId,
}

#[derive(Debug)]
pub(super) struct BoundBody {
    pub required: bool,
    pub media_type: String,
    pub model: String,
    pub setter_name: Option<String>,
    pub schema: SchemaId,
}

/// The exact JSON surface of one declared response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Payload {
    /// The response carries no body at all; the command writes no document.
    NoContent,
    /// The response body decodes through one named generated codec.
    ExactJson,
}

impl Payload {
    pub(super) const fn surface(self) -> &'static str {
        match self {
            Self::NoContent => "no_content",
            Self::ExactJson => "exact_json_document",
        }
    }
}

#[derive(Debug)]
pub(super) struct BoundResponse {
    pub status: serde_json::Value,
    pub type_name: String,
    pub payload: Payload,
    /// Allocated codec/model name, present exactly for [`Payload::ExactJson`].
    pub model: Option<String>,
    pub success: bool,
    pub failure: bool,
}

/// Root of the admitted command tree, used only for emission.
#[derive(Debug, Default)]
pub(super) struct Group {
    pub children: BTreeMap<String, Node>,
}

#[derive(Debug)]
pub(super) enum Node {
    Leaf(usize),
    Group(Group),
}

impl Group {
    fn insert(&mut self, path: &[String], command: usize) {
        match path {
            [name] => {
                self.children.insert(name.clone(), Node::Leaf(command));
            }
            [name, rest @ ..] => match self
                .children
                .entry(name.clone())
                .or_insert_with(|| Node::Group(Group::default()))
            {
                Node::Group(group) => group.insert(rest, command),
                Node::Leaf(_) => unreachable!("prefix collisions are refused during admission"),
            },
            [] => unreachable!("empty command paths are refused during admission"),
        }
    }
}

/// The admitted command tree in stable, name-sorted order.
pub(super) fn tree(commands: &[BoundCommand]) -> Group {
    let mut root = Group::default();
    for (index, command) in commands.iter().enumerate() {
        root.insert(&command.path, index);
    }
    root
}

fn root(contract: &Contract) -> SourceId {
    SourceId::new(contract.entry().clone(), Default::default())
}

pub(super) fn plan(
    contract: Arc<Contract>,
    mapping: MappingProfile,
    config: CliTargetConfig,
) -> Result<CliPlan, Vec<Diagnostic>> {
    if mapping.format != super::PROFILE {
        return Err(vec![Diagnostic::new(
            &contract,
            root(&contract),
            "/format",
            "cli-mapping-version",
            format!(
                "unknown CLI mapping profile {:?}; this target implements {:?} only",
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
    let sdk = go_http::plan_http(
        contract.clone(),
        &sources,
        go_http::HttpConfig {
            credential_env: config.credential_env.clone(),
            ..Default::default()
        },
    )
    .map_err(|findings| {
        findings
            .into_iter()
            .map(|finding| Diagnostic {
                code: finding.code,
                message: finding.message,
                mapping_pointer: "/commands".into(),
                source: finding.source,
                at: finding.at,
            })
            .collect::<Vec<_>>()
    })?;

    let package = go_http::PackageConfig {
        module_path: format!("{}/internal/sdk", config.module_path),
        package_name: "sdk".into(),
        version: config.version.clone(),
    };
    let sdk_files = match go_http::emit_http(&sdk, &package) {
        Ok(files) => files,
        Err(messages) => {
            return Err(messages
                .into_iter()
                .map(|message| {
                    Diagnostic::new(
                        &contract,
                        root(&contract),
                        "/module_path",
                        "cli-package-identity",
                        message,
                    )
                })
                .collect());
        }
    };

    let mut commands = Vec::new();
    let mut used_constructors = BTreeSet::new();
    for (index, command) in mapping.commands.iter().enumerate() {
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
                format!("/commands/{index}/selector"),
                &command.selector,
                &selected[index],
            ));
            continue;
        };
        if let Some(bound) = bind(
            &contract,
            &sdk,
            index,
            command,
            operation,
            &mut used_constructors,
            &mut errors,
        ) {
            commands.push(bound);
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    // Every document the surface manifest will name must have a relative form,
    // so an out-of-tree document is refused here rather than leaking an
    // absolute generator path into the review artifact at emission.
    let scope = DocumentScope::new(&contract);
    manifest_documents(&contract, &sdk, &commands, &scope, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(CliPlan {
        credential_factory: sdk.credential_env_factory().map(str::to_owned),
        sdk,
        sdk_files,
        mapping,
        config,
        commands,
        scope,
    })
}

/// Admit every contract document the surface manifest records: each command's
/// selected operation, each flag's parameter schema and each request body
/// schema. Responses contribute no document to the manifest.
fn manifest_documents(
    contract: &Contract,
    sdk: &go_http::HttpPlan,
    commands: &[BoundCommand],
    scope: &DocumentScope,
    errors: &mut Vec<Diagnostic>,
) {
    for (index, command) in commands.iter().enumerate() {
        let operation = &sdk.operations()[command.operation];
        if let Err(error) = scope.admit(
            contract,
            format!("/commands/{index}/selector"),
            &operation.source,
        ) {
            errors.push(error);
        }
        for flag in &command.flags {
            if let Err(error) = scope.admit(
                contract,
                format!("/commands/{index}/parameters/{}", flag.parameter),
                &flag.schema,
            ) {
                errors.push(error);
            }
        }
        if let Some(body) = &command.body
            && let Err(error) =
                scope.admit(contract, format!("/commands/{index}/body"), &body.schema)
        {
            errors.push(error);
        }
    }
}

/// Package, binary and bound identity that the emitted module cannot repair.
fn identity(contract: &Contract, config: &CliTargetConfig, errors: &mut Vec<Diagnostic>) {
    let mut refuse = |pointer: &str, code: &'static str, message: String| {
        errors.push(Diagnostic::new(
            contract,
            root(contract),
            pointer,
            code,
            message,
        ));
    };
    let name = config.binary_name.as_str();
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
            "/binary_name",
            "cli-binary-name",
            format!(
                "binary_name {name:?} must be 1..=64 lower-case letters, digits and single interior dashes"
            ),
        );
    }
    if !(1..=3_600_000).contains(&config.runtime.request_deadline_ms) {
        refuse(
            "/runtime/request_deadline_ms",
            "cli-runtime-bounds",
            format!(
                "request_deadline_ms {} must be 1..=3600000",
                config.runtime.request_deadline_ms
            ),
        );
    }
    if !(1..=8 * 1024 * 1024).contains(&config.runtime.max_input_bytes) {
        refuse(
            "/runtime/max_input_bytes",
            "cli-runtime-bounds",
            format!(
                "max_input_bytes {} must be 1..=8388608",
                config.runtime.max_input_bytes
            ),
        );
    }
    if !go_directive(&config.go_version) {
        refuse(
            "/go_version",
            "cli-package-identity",
            format!(
                "go_version {:?} must be an exact Go language version",
                config.go_version
            ),
        );
    }
    if !config
        .go_toolchain
        .strip_prefix("go")
        .is_some_and(go_directive)
    {
        refuse(
            "/go_toolchain",
            "cli-package-identity",
            format!(
                "go_toolchain {:?} must be an exact `goX.Y[.Z]` toolchain name",
                config.go_toolchain
            ),
        );
    }
}

/// `X.Y` or `X.Y.Z` of ASCII digits, as `go mod` writes them.
fn go_directive(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    (2..=3).contains(&parts.len())
        && parts.iter().all(|part| {
            !part.is_empty() && part.len() <= 4 && part.bytes().all(|byte| byte.is_ascii_digit())
        })
}

/// Every public command and flag name, before any contract binding. Duplicate
/// and reserved names are refused here so no two commands can ever race for
/// the same invocation.
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
    let mut paths: BTreeMap<&[String], usize> = BTreeMap::new();
    for (index, command) in mapping.commands.iter().enumerate() {
        let pointer = format!("/commands/{index}/path");
        if command.path.is_empty() || command.path.len() > 4 {
            refuse(
                pointer.clone(),
                "cli-command-name",
                "a command path needs 1..=4 segments".into(),
            );
            continue;
        }
        if let Some(bad) = command.path.iter().find(|segment| !token(segment)) {
            refuse(
                pointer.clone(),
                "cli-command-name",
                format!(
                    "command segment {bad:?} must be 1..=32 lower-case letters, digits and single interior dashes"
                ),
            );
            continue;
        }
        if let Some(bad) = command
            .path
            .iter()
            .find(|segment| RESERVED_COMMANDS.contains(&segment.as_str()))
        {
            refuse(
                pointer.clone(),
                "cli-command-reserved",
                format!("command segment {bad:?} is owned by the generated command runtime"),
            );
            continue;
        }
        if paths.insert(command.path.as_slice(), index).is_some() {
            refuse(
                pointer,
                "cli-command-collision",
                format!(
                    "command path {:?} is already mapped",
                    command.path.join(" ")
                ),
            );
        }
    }
    // A command that runs an operation can never also be a group of commands.
    for (index, command) in mapping.commands.iter().enumerate() {
        let shadowed = mapping
            .commands
            .iter()
            .enumerate()
            .any(|(other, candidate)| {
                other != index
                    && candidate.path.len() > command.path.len()
                    && candidate.path.starts_with(&command.path)
            });
        if shadowed {
            refuse(
                format!("/commands/{index}/path"),
                "cli-command-collision",
                format!(
                    "command path {:?} also prefixes another mapped command, so it cannot run an operation",
                    command.path.join(" ")
                ),
            );
        }
    }
    for (index, command) in mapping.commands.iter().enumerate() {
        let mut flags: BTreeSet<&str> = BTreeSet::new();
        for (parameter, entry) in &command.parameters {
            let pointer = format!("/commands/{index}/parameters/{parameter}/flag");
            if !token(&entry.flag) {
                refuse(
                    pointer,
                    "cli-flag-name",
                    format!(
                        "flag {:?} must be 1..=32 lower-case letters, digits and single interior dashes",
                        entry.flag
                    ),
                );
            } else if RESERVED_FLAGS.contains(&entry.flag.as_str()) {
                refuse(
                    pointer,
                    "cli-flag-reserved",
                    format!(
                        "flag {:?} is owned by the generated command runtime",
                        entry.flag
                    ),
                );
            } else if !flags.insert(entry.flag.as_str()) {
                refuse(
                    pointer,
                    "cli-flag-collision",
                    format!("flag {:?} is already mapped on this command", entry.flag),
                );
            }
        }
    }
}

/// Lower-case command/flag token: no leading digit rule is imposed, but no
/// upper case, no underscores, and no leading, trailing or doubled dash.
fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Resolve every selector through the shared application selector, keeping a
/// placeholder for refused entries so later indexes stay aligned.
fn selections(
    contract: &Contract,
    mapping: &MappingProfile,
    errors: &mut Vec<Diagnostic>,
) -> Vec<SourceId> {
    let mut selected = Vec::with_capacity(mapping.commands.len());
    for (index, command) in mapping.commands.iter().enumerate() {
        let pointer = format!("/commands/{index}/selector");
        match select_operation(contract, &pointer, &command.selector) {
            Ok(operation) => selected.push(operation.source().clone()),
            Err(error) => {
                errors.push(error);
                selected.push(root(contract));
            }
        }
    }
    selected
}

#[allow(clippy::too_many_lines)]
fn bind(
    contract: &Contract,
    sdk: &go_http::HttpPlan,
    index: usize,
    command: &CommandMapping,
    operation: usize,
    used: &mut BTreeSet<String>,
    errors: &mut Vec<Diagnostic>,
) -> Option<BoundCommand> {
    let planned = &sdk.operations()[operation];
    let table = sdk.codecs().models().descriptors();
    let before = errors.len();
    let mut refuse = |pointer: String, source: SourceId, code: &'static str, message: String| {
        errors.push(Diagnostic::new(contract, source, pointer, code, message));
    };

    let mut flags = Vec::new();
    for parameter in planned.parameters() {
        let name = parameter.wire().name();
        let use_site = parameter.wire().source().use_site().source().clone();
        let Some(entry) = command.parameters.get(name) else {
            refuse(
                format!("/commands/{index}/parameters"),
                use_site,
                "cli-parameter-unmapped",
                format!(
                    "source parameter {name:?} has no mapped flag; every parameter of an exposed operation needs one"
                ),
            );
            continue;
        };
        match representation(table, parameter.schema()) {
            Ok(representation) => flags.push(BoundFlag {
                flag: entry.flag.clone(),
                parameter: name.to_owned(),
                location: location(parameter.wire().location()),
                required: parameter.wire().required(),
                representation,
                description: entry.description.clone(),
                model: sdk.symbols()[parameter.schema()].clone(),
                setter_name: parameter.setter_name.clone(),
                schema: parameter.schema().clone(),
            }),
            Err(reason) => refuse(
                format!("/commands/{index}/parameters/{name}"),
                use_site,
                "cli-parameter-unrepresentable",
                format!("source parameter {name:?} cannot be carried by one exact flag: {reason}"),
            ),
        }
    }
    let known: BTreeSet<&str> = planned
        .parameters()
        .iter()
        .map(|parameter| parameter.wire().name())
        .collect();
    for parameter in command.parameters.keys() {
        if !known.contains(parameter.as_str()) {
            refuse(
                format!("/commands/{index}/parameters/{parameter}"),
                planned.source.clone(),
                "cli-parameter-unknown",
                format!("{parameter:?} is not a parameter of the selected operation"),
            );
        }
    }

    let body = match (planned.body(), command.body) {
        (None, BodyPolicy::None) => None,
        (None, BodyPolicy::JsonDocument) => {
            refuse(
                format!("/commands/{index}/body"),
                planned.source.clone(),
                "cli-body-policy",
                "the selected operation declares no request body".into(),
            );
            None
        }
        (Some(declared), BodyPolicy::None) => {
            refuse(
                format!("/commands/{index}/body"),
                declared.wire().source().use_site().source().clone(),
                "cli-body-policy",
                "the selected operation declares a request body, which the mapping must expose"
                    .into(),
            );
            None
        }
        (Some(declared), BodyPolicy::JsonDocument) => {
            let use_site = declared.wire().source().use_site().source().clone();
            match exact_json(declared.media(), declared.schema()) {
                Some(schema) => Some(BoundBody {
                    required: declared.wire().required(),
                    media_type: declared.media()[0]
                        .wire()
                        .media_type()
                        .declared()
                        .to_owned(),
                    model: sdk.symbols()[schema].clone(),
                    setter_name: declared.setter_name.clone(),
                    schema: schema.clone(),
                }),
                None => {
                    refuse(
                        format!("/commands/{index}/body"),
                        use_site,
                        "cli-unsupported-media",
                        "a request body needs exactly one concrete JSON media with a generated codec".into(),
                    );
                    None
                }
            }
        }
    };

    let mut responses = Vec::new();
    for response in planned.responses() {
        let use_site = response.wire().source().use_site().source().clone();
        let (payload, model) = if response.forbidden_body {
            (Payload::NoContent, None)
        } else {
            match exact_json(
                response.media().map(std::slice::from_ref).unwrap_or(&[]),
                response.schema(),
            ) {
                Some(schema) => (Payload::ExactJson, Some(sdk.symbols()[schema].clone())),
                None => {
                    refuse(
                        format!("/commands/{index}"),
                        use_site,
                        "cli-unsupported-media",
                        format!(
                            "response {:?} needs either no content or exactly one concrete JSON media with a generated codec",
                            response.wire().status_key()
                        ),
                    );
                    continue;
                }
            }
        };
        responses.push(BoundResponse {
            status: match response.status() {
                protocol::ResponseStatus::Exact(code) => serde_json::Value::from(code),
                _ => serde_json::Value::from(response.wire().status_key()),
            },
            type_name: response.type_name.clone(),
            payload,
            model,
            success: response.can_succeed(),
            failure: response.can_fail(),
        });
    }

    if errors.len() != before {
        return None;
    }
    let stem: String = command
        .path
        .iter()
        .map(|segment| exported(segment))
        .collect();
    let mut symbol = stem.clone();
    let mut suffix = 2;
    while !used.insert(symbol.clone()) {
        symbol = format!("{stem}{suffix}");
        suffix += 1;
    }
    Some(BoundCommand {
        path: command.path.clone(),
        summary: command.summary.clone(),
        description: command.description.clone(),
        operation,
        flags,
        body,
        confirmation: command.confirmation.clone(),
        responses,
        symbol,
    })
}

/// Exactly one concrete JSON media carrying a generated codec, or nothing.
fn exact_json<'a>(
    media: &[go_http::PlannedMedia],
    schema: Option<&'a SchemaId>,
) -> Option<&'a SchemaId> {
    let [single] = media else { return None };
    if single.requires_content_type()
        || !matches!(
            single.wire().representation(),
            protocol::Representation::Json { codec: Some(_) }
        )
    {
        return None;
    }
    schema
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

/// Resolve the native Go carrier of one parameter model down to a scalar a
/// single flag can hold exactly. Wrappers, containers, records and unions are
/// refused rather than flattened into a lossy flag.
fn representation(table: &GoDescriptors, schema: &SchemaId) -> Result<Representation, String> {
    let mut key = (schema.clone(), RepresentationRole::Model);
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(key.clone()) {
            return Err("its native alias chain is cyclic".into());
        }
        match table.declarations.get(&key) {
            Some(GoDecl::Alias(GoType::Named(next))) => key = next.clone(),
            Some(GoDecl::Alias(GoType::Primitive(name))) => return scalar(name),
            Some(GoDecl::Literals { underlying, .. }) => return scalar(underlying),
            _ => {
                return Err(
                    "a flag carries only a native string, boolean, integer or number scalar".into(),
                );
            }
        }
    }
}

fn scalar(name: &str) -> Result<Representation, String> {
    match name {
        "string" => Ok(Representation::String),
        "bool" => Ok(Representation::Boolean),
        "Integer" => Ok(Representation::Integer),
        "Number" => Ok(Representation::Number),
        other => Err(format!(
            "native carrier {other} is not an exact flag scalar"
        )),
    }
}

/// Exported Go identifier fragment from a public command segment.
fn exported(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    let mut upper = true;
    for c in segment.chars() {
        if c.is_ascii_alphanumeric() {
            if upper {
                out.extend(c.to_uppercase());
                upper = false;
            } else {
                out.push(c);
            }
        } else {
            upper = true;
        }
    }
    out
}
