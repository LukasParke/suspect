//! Admission and typed lifecycle bindings. All API shapes come from the Go plan.
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use suspect_ir::contract::{Contract, SourceId};

use crate::{
    go_http::{self, HttpPlan, PlannedOperation, PlannedResponse},
    http_protocol::{CredentialHook, ParameterLocation, Representation, ResponseStatus},
};

use super::{native, *};

#[derive(Debug)]
pub(super) struct BoundInput {
    pub mapping: Input,
    pub member: String,
    pub body: bool,
    pub source: SourceId,
    pub scalar: native::Scalar,
}

#[derive(Debug)]
pub(super) struct BoundOutput {
    pub attribute: String,
    pub field: native::Field,
}

#[derive(Debug)]
pub(super) struct BoundResponse {
    pub response: PlannedResponse,
    pub outputs: Vec<BoundOutput>,
}

#[derive(Debug)]
pub(super) struct BoundCall {
    pub operation: PlannedOperation,
    pub inputs: Vec<BoundInput>,
    pub success: BoundResponse,
    pub partial: Vec<BoundResponse>,
    pub missing: Vec<PlannedResponse>,
}

#[derive(Debug)]
pub(super) struct BoundResource {
    pub name: String,
    pub native_name: String,
    pub create: BoundCall,
    pub read: BoundCall,
    pub update: BoundCall,
    pub delete: BoundCall,
}

#[derive(Debug)]
pub(super) struct BoundDataSource {
    pub name: String,
    pub native_name: String,
    pub read: BoundCall,
}

type Attributes = std::collections::BTreeMap<String, Attribute>;

fn root(contract: &Contract) -> SourceId {
    SourceId::new(contract.entry().clone(), Default::default())
}

fn error(
    contract: &Contract,
    source: &SourceId,
    pointer: &str,
    code: &'static str,
    message: impl Into<String>,
) -> Diagnostic {
    Diagnostic {
        code,
        message: message.into(),
        mapping_pointer: pointer.into(),
        at: contract.source_span(source).unwrap_or(0..0),
        source: source.clone(),
    }
}

pub(super) fn plan(
    contract: Arc<Contract>,
    profile: MappingProfile,
    config: TargetConfig,
) -> Result<ProviderPlan, Vec<Diagnostic>> {
    let mut errors = Vec::new();
    let at = root(&contract);
    if profile.format != PROFILE {
        errors.push(error(
            &contract,
            &at,
            "/format",
            "terraform-profile",
            format!("expected {PROFILE}"),
        ));
    }
    if profile.resources.len() + profile.data_sources.len() == 0
        || profile.resources.len() + profile.data_sources.len() > 64
    {
        errors.push(error(
            &contract,
            &at,
            "",
            "terraform-profile-size",
            "lifecycle v1 requires 1..64 resource/data-source mappings",
        ));
    }
    check_config(&contract, &config, &mut errors);
    // Selection is deduplicated by ID, but every lifecycle use has its own
    // mapping source. A shared miss/ambiguity must not erase those addresses.
    let mut ids: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (name, r) in &profile.resources {
        for (phase, id) in [
            ("create", r.create.operation_id.as_str()),
            ("read", r.read.operation_id.as_str()),
            ("update", r.update.operation_id.as_str()),
            ("delete", r.delete.operation_id.as_str()),
        ] {
            ids.entry(id)
                .or_default()
                .push(format!("/resources/{}/{phase}/operation_id", token(name)));
        }
    }
    for (name, d) in &profile.data_sources {
        ids.entry(d.read.operation_id.as_str())
            .or_default()
            .push(format!("/data_sources/{}/read/operation_id", token(name)));
    }
    let mut selected = Vec::new();
    for (id, pointers) in ids {
        let matches = contract
            .operations()
            .filter(|op| op.operation_id() == Some(id))
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            for pointer in pointers {
                errors.push(error(&contract, &at, &pointer, "terraform-operation", format!("operation ID {id:?} must identify exactly one outgoing Contract operation (found {})", matches.len())));
            }
        } else {
            selected.push(matches[0].source().clone());
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    selected.sort();
    let sdk = go_http::plan_http(contract.clone(), &selected, go_http::HttpConfig::default())
        .map_err(|findings| {
            findings
                .into_iter()
                .map(|d| Diagnostic {
                    code: d.code,
                    message: d.message,
                    mapping_pointer: String::new(),
                    source: d.source,
                    at: d.at,
                })
                .collect::<Vec<_>>()
        })?;
    let credential = check_auth(&sdk, &profile.authentication, &mut errors);
    let mut resources = Vec::new();
    for (index, (name, mapping)) in profile.resources.iter().enumerate() {
        let pointer = format!("/resources/{}", token(name));
        check_attributes(
            &sdk,
            name,
            &mapping.attributes,
            &pointer,
            false,
            &mut errors,
        );
        let identity = mapping.attributes.get(&mapping.identity.attribute);
        if !identity.is_some_and(|a| {
            a.r#type == AttributeType::String
                && a.mode == AttributeMode::Computed
                && !a.sensitive
                && !a.write_only
                && !a.state_only
        }) {
            errors.push(error(
                &contract,
                &at,
                &format!("{pointer}/identity"),
                "terraform-identity",
                "identity must name a non-sensitive computed string API attribute",
            ));
        }
        let before = errors.len();
        let create = bind_call(
            &sdk,
            &mapping.attributes,
            &mapping.create.operation_id,
            &mapping.create.inputs,
            &mapping.create.success,
            &mapping.create.partial,
            &[],
            &format!("{pointer}/create"),
            Phase::Create,
            &mut errors,
        );
        let read = bind_call(
            &sdk,
            &mapping.attributes,
            &mapping.read.operation_id,
            &mapping.read.inputs,
            &mapping.read.success,
            &[],
            &mapping.read.missing,
            &format!("{pointer}/read"),
            Phase::Read,
            &mut errors,
        );
        let update = bind_call(
            &sdk,
            &mapping.attributes,
            &mapping.update.operation_id,
            &mapping.update.inputs,
            &mapping.update.success,
            &mapping.update.partial,
            &[],
            &format!("{pointer}/update"),
            Phase::Update,
            &mut errors,
        );
        let delete = bind_call(
            &sdk,
            &mapping.attributes,
            &mapping.delete.operation_id,
            &mapping.delete.inputs,
            &Response {
                status: mapping.delete.success_status,
                state: Default::default(),
            },
            &[],
            &mapping.delete.missing,
            &format!("{pointer}/delete"),
            Phase::Delete,
            &mut errors,
        );
        if let (Some(create), Some(read), Some(update), Some(delete)) =
            (create, read, update, delete)
        {
            for call in [&create, &read, &update] {
                for response in std::iter::once(&call.success).chain(&call.partial) {
                    let id = response
                        .outputs
                        .iter()
                        .find(|o| o.attribute == mapping.identity.attribute);
                    if !id.is_some_and(|o| {
                        o.field.scalar.presence == native::Presence::Required
                            && o.field.scalar.kind == AttributeType::String
                    }) {
                        errors.push(error(&contract, &call.operation.source, &format!("{pointer}/identity"), "terraform-identity", "every state-bearing response must supply a required non-null string ID"));
                    }
                }
            }
            for call in [&read, &delete] {
                if call.inputs.len() != 1
                    || call.inputs.iter().any(|i| {
                        i.mapping.attribute != mapping.identity.attribute
                            || i.mapping.null != NullInput::Reject
                            || i.body
                    })
                {
                    errors.push(error(
                        &contract,
                        &call.operation.source,
                        &format!("{pointer}/identity"),
                        "terraform-import-input",
                        "opaque-string import/refresh/delete must need only the identity parameter",
                    ));
                }
            }
            check_mutable_coverage(&sdk, mapping, &create, &update, &pointer, &mut errors);
            if errors.len() == before {
                resources.push(BoundResource {
                    name: name.clone(),
                    native_name: format!("Resource{index}"),
                    create,
                    read,
                    update,
                    delete,
                });
            }
        }
    }
    let mut data_sources = Vec::new();
    for (index, (name, mapping)) in profile.data_sources.iter().enumerate() {
        let pointer = format!("/data_sources/{}", token(name));
        check_attributes(&sdk, name, &mapping.attributes, &pointer, true, &mut errors);
        if let Some(read) = bind_call(
            &sdk,
            &mapping.attributes,
            &mapping.read.operation_id,
            &mapping.read.inputs,
            &mapping.read.success,
            &[],
            &mapping.read.missing,
            &format!("{pointer}/read"),
            Phase::DataRead,
            &mut errors,
        ) {
            for (name, attr) in &mapping.attributes {
                let input = read.inputs.iter().any(|i| i.mapping.attribute == *name);
                if attr.mode.configured() != input {
                    errors.push(error(&contract, &read.operation.source, &format!("{pointer}/attributes/{}", token(name)), "terraform-data-input", "each required data-source attribute must bind a read input; computed attributes cannot be inputs"));
                }
            }
            data_sources.push(BoundDataSource {
                name: name.clone(),
                native_name: format!("DataSource{index}"),
                read,
            });
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let sdk_files = go_http::emit_http(
        &sdk,
        &go_http::PackageConfig {
            module_path: config.sdk.module_path.clone(),
            package_name: "sdk".into(),
            version: config.sdk.version.clone(),
        },
    )
    .map_err(|messages| {
        messages
            .into_iter()
            .map(|message| {
                error(
                    &contract,
                    &at,
                    "/config/sdk",
                    "terraform-sdk-package",
                    message,
                )
            })
            .collect::<Vec<_>>()
    })?;
    // Package identity is derived from the actual SDK artifact inventory, not
    // from operation names or parsed Go source. The SDK's native example is
    // also a Go package and must not overlap either generated provider package.
    let provider_packages = [
        config.module_path.clone(),
        format!("{}/provider", config.module_path),
    ];
    for file in &sdk_files {
        if let Some(path) = file
            .path
            .strip_prefix("go/")
            .filter(|path| path.ends_with(".go"))
        {
            let package = path.rsplit_once('/').map_or_else(
                || config.sdk.module_path.clone(),
                |(directory, _)| format!("{}/{directory}", config.sdk.module_path),
            );
            if provider_packages.contains(&package) {
                return Err(vec![error(
                    &contract,
                    &at,
                    "/config/sdk/module_path",
                    "terraform-package",
                    format!(
                        "generated SDK package {package} collides with a generated provider package import path"
                    ),
                )]);
            }
        }
    }
    Ok(ProviderPlan {
        sdk,
        sdk_files,
        profile,
        config,
        resources,
        data_sources,
        credential,
    })
}

fn identifier(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.as_bytes()[0].is_ascii_lowercase()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && !matches!(
            name,
            "count"
                | "for_each"
                | "depends_on"
                | "provider"
                | "lifecycle"
                | "connection"
                | "provisioner"
        )
}

fn token(name: &str) -> String {
    name.replace('~', "~0").replace('/', "~1")
}

fn version(value: &str) -> Option<semver::Version> {
    semver::Version::parse(value)
        .ok()
        .filter(|v| v.pre.is_empty() && v.build.is_empty())
}

fn module(path: &str, package_version: &str) -> bool {
    if path.len() > 256
        || path.contains("..")
        || !path.contains('/')
        || path
            .split('/')
            .next()
            .is_none_or(|host| !host.contains('.'))
        || path
            .split('/')
            .any(|part| part.is_empty() || part.starts_with('.') || part.ends_with('.'))
        || !path.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'_' | b'/')
        })
    {
        return false;
    }
    let Some(v) = version(package_version) else {
        return false;
    };
    let suffix = path.rsplit('/').next().unwrap_or("");
    let path_major = suffix.strip_prefix('v').and_then(|n| n.parse::<u64>().ok());
    if v.major >= 2 {
        suffix == format!("v{}", v.major)
    } else {
        path_major.is_none()
    }
}

// The target deliberately accepts canonical lowercase ASCII source addresses.
// Within that subset, mirror Terraform's tfaddr.ParseProviderPart and
// svchost.ForComparison/IDNA lookup rules. Namespace/type parts are single
// labels with no consecutive dashes; hostnames have distinct label rules.
fn provider_part(part: &str) -> bool {
    !part.is_empty()
        && !part.starts_with('-')
        && !part.ends_with('-')
        && !part.contains("--")
        && part
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn provider_hostname(host: &str) -> bool {
    host.contains('.')
        && host.split('.').all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                // IDNA reserves the third/fourth-character double dash. This
                // also rejects direct xn-- input, forbidden by svchost.
                && label.as_bytes().get(2..4) != Some(b"--")
                && label.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

fn provider_address(address: &str, name: &str) -> Result<(), &'static str> {
    let parts = address.split('/').collect::<Vec<_>>();
    let [hostname, namespace, provider_type] = parts.as_slice() else {
        return Err("provider address must contain exactly hostname/namespace/type");
    };
    if !provider_hostname(hostname) {
        return Err(
            "provider hostname must be lowercase ASCII dotted labels without empty labels, edge dashes, or IDNA-reserved third/fourth-character double dashes (including punycode)",
        );
    }
    if !provider_part(namespace) {
        return Err(
            "provider namespace must contain lowercase ASCII letters/digits and single internal dashes; dots, underscores, edge dashes and consecutive dashes are not allowed",
        );
    }
    if !provider_part(provider_type) || provider_type.starts_with("terraform-") {
        return Err(
            "provider type must contain lowercase ASCII letters/digits and single internal dashes, without a terraform- prefix",
        );
    }
    if *provider_type != name {
        return Err("provider source-address type must equal provider_name");
    }
    Ok(())
}

fn check_config(contract: &Contract, config: &TargetConfig, errors: &mut Vec<Diagnostic>) {
    let at = root(contract);
    let mut check = |ok: bool, pointer: &str, message: &str| {
        if !ok {
            errors.push(error(contract, &at, pointer, "terraform-package", message));
        }
    };
    check(
        identifier(&config.provider_name) && !config.provider_name.contains('_'),
        "/config/provider_name",
        "provider name must be a lowercase Terraform provider identifier",
    );
    if let Err(message) = provider_address(&config.provider_address, &config.provider_name) {
        check(false, "/config/provider_address", message);
    }
    check(
        module(&config.module_path, &config.version)
            && !super::package::reserved_module(&config.module_path),
        "/config/module_path",
        "provider module path and exact stable SemVer must have compatible major versions",
    );
    check(
        module(&config.sdk.module_path, &config.sdk.version)
            && !super::package::reserved_module(&config.sdk.module_path)
            && config.sdk.module_path != config.module_path,
        "/config/sdk",
        "SDK must be a distinct Go module pinned to an exact compatible stable SemVer",
    );
    check(
        config.sdk.module_path != format!("{}/provider", config.module_path),
        "/config/sdk/module_path",
        "SDK module path collides with the generated provider subpackage import path",
    );
    let floor = version(&config.go_version);
    let tool = config.go_toolchain.strip_prefix("go").and_then(version);
    check(
        floor
            .as_ref()
            .is_some_and(|v| *v >= semver::Version::new(1, 23, 0))
            && tool
                .as_ref()
                .zip(floor.as_ref())
                .is_some_and(|(t, f)| t >= f),
        "/config/go_toolchain",
        "Go language/toolchain pins must be exact versions with toolchain >= language >= 1.23.0",
    );
    check(
        version(&config.terraform_version).is_some_and(|v| v >= semver::Version::new(1, 11, 0)),
        "/config/terraform_version",
        "Terraform must be pinned to an exact stable version >= 1.11.0 for write-only support",
    );
    check(
        config.framework_version == "1.15.1",
        "/config/framework_version",
        "lifecycle v1 currently admits the witnessed Plugin Framework 1.15.1 dependency profile",
    );
}

fn check_auth(
    sdk: &HttpPlan,
    auth: &Authentication,
    errors: &mut Vec<Diagnostic>,
) -> Option<String> {
    for op in sdk.operations() {
        let alternatives = op.wire().security().alternatives();
        let valid = match auth {
            Authentication::None => {
                alternatives.is_empty()
                    || alternatives.len() == 1 && alternatives[0].requirements().is_empty()
            }
            Authentication::Bearer { scheme } => {
                alternatives.len() == 1 && alternatives[0].requirements().len() == 1 && {
                    let req = &alternatives[0].requirements()[0];
                    req.name() == scheme
                        && matches!(req.credential(), CredentialHook::Bearer { .. })
                }
            }
        };
        if !valid {
            errors.push(error(sdk.contract(), &op.source, "/authentication", "terraform-authentication", "mapping must exactly match anonymous security or a single declared bearer scheme; other alternatives/hooks need a new mapping profile"));
        }
    }
    match auth {
        Authentication::None => None,
        Authentication::Bearer { scheme } => sdk.credentials().get(scheme).cloned(),
    }
}

fn check_attributes(
    sdk: &HttpPlan,
    name: &str,
    attrs: &Attributes,
    pointer: &str,
    data: bool,
    errors: &mut Vec<Diagnostic>,
) {
    let at = root(sdk.contract());
    if !identifier(name) || attrs.is_empty() || attrs.len() > 256 {
        errors.push(error(
            sdk.contract(),
            &at,
            pointer,
            "terraform-schema",
            "type names must be Terraform identifiers and types must contain 1..256 attributes",
        ));
    }
    for (name, a) in attrs {
        if !identifier(name)
            || a.write_only
                && (a.mode.computed() || !a.sensitive || a.state_only || a.requires_replace)
            || a.state_only && (a.mode.computed() || a.sensitive || a.requires_replace)
            || a.requires_replace && !a.mode.configured()
            || data
                && (a.write_only
                    || a.state_only
                    || a.requires_replace
                    || !matches!(a.mode, AttributeMode::Required | AttributeMode::Computed))
        {
            errors.push(error(sdk.contract(), &at, &format!("{pointer}/attributes/{}", token(name)), "terraform-attribute", "invalid name/mode combination: write-only must be configured+sensitive, triggers configured, replacement configurable; data sources support required/computed only"));
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Create,
    Read,
    Update,
    Delete,
    DataRead,
}

#[allow(clippy::too_many_arguments)]
fn bind_call(
    sdk: &HttpPlan,
    attrs: &Attributes,
    id: &str,
    inputs: &[Input],
    success: &Response,
    partial: &[Response],
    missing: &[u16],
    pointer: &str,
    phase: Phase,
    errors: &mut Vec<Diagnostic>,
) -> Option<BoundCall> {
    let op = sdk.operations().iter().find(|op| op.operation_id == id)?;
    let before = errors.len();
    if inputs.len() > 512 || partial.len() + missing.len() > 16 {
        errors.push(error(
            sdk.contract(),
            &op.source,
            pointer,
            "terraform-binding-size",
            "operation mapping exceeds lifecycle v1 finite limits",
        ));
        return None;
    }
    if !matches!(phase, Phase::Create | Phase::Update) && op.body().is_some() {
        errors.push(error(
            sdk.contract(),
            &op.source,
            pointer,
            "terraform-lifecycle-input",
            "read/delete v1 mappings support scalar parameters without a request body",
        ));
    }
    if op.responses().iter().filter(|r| r.can_succeed()).count() != 1 {
        errors.push(error(
            sdk.contract(),
            &op.source,
            pointer,
            "terraform-response-choice",
            "v1 requires one exact successful SDK status/representation",
        ));
    }
    if let Some(body) = op.body() {
        if !body.wire().required()
            || body.media().len() != 1
            || body.schema().is_none()
            || !matches!(
                body.media()[0].wire().representation(),
                Representation::Json { .. }
            )
            || body.media()[0].requires_content_type()
        {
            errors.push(error(
                sdk.contract(),
                &op.source,
                pointer,
                "terraform-body",
                "v1 mutation bodies must be required, single concrete JSON native records",
            ));
        } else if let Err(message) =
            native::record(native::table(sdk), body.schema().expect("checked schema"))
        {
            errors.push(error(
                sdk.contract(),
                body.schema().unwrap(),
                pointer,
                "terraform-native-record",
                message,
            ));
        }
    }
    let mut bound_inputs = Vec::new();
    let mut used_members = BTreeSet::new();
    for (index, input) in inputs.iter().enumerate() {
        let p = format!("{pointer}/inputs/{index}");
        let Some(attr) = attrs.get(&input.attribute) else {
            errors.push(error(
                sdk.contract(),
                &op.source,
                &p,
                "terraform-input-attribute",
                "input references an undeclared Terraform attribute",
            ));
            continue;
        };
        let binding = match &input.target {
            InputTarget::Parameter { location, name } => {
                let location = match location {
                    Location::Path => ParameterLocation::Path,
                    Location::Query => ParameterLocation::Query,
                    Location::Header => ParameterLocation::Header,
                    Location::Cookie => ParameterLocation::Cookie,
                };
                op.parameters()
                    .iter()
                    .find(|p| p.wire().name() == name && p.wire().location() == location)
                    .map(|p| {
                        native::parameter(native::table(sdk), p.schema(), p.wire().required()).map(
                            |scalar| BoundInput {
                                mapping: input.clone(),
                                member: p.field_name.clone(),
                                body: false,
                                source: p.schema().clone(),
                                scalar,
                            },
                        )
                    })
                    .unwrap_or_else(|| {
                        Err("no allocated SDK parameter has this location and wire name".into())
                    })
            }
            InputTarget::Body { path } => op
                .body()
                .and_then(|b| b.schema())
                .ok_or_else(|| "operation has no unambiguous JSON body model".into())
                .and_then(|schema| {
                    native::field(native::table(sdk), schema, path).map(|field| BoundInput {
                        mapping: input.clone(),
                        member: field.name,
                        body: true,
                        source: field.source,
                        scalar: field.scalar,
                    })
                }),
        };
        let bound = match binding {
            Ok(b) => b,
            Err(message) => {
                errors.push(error(
                    sdk.contract(),
                    &op.source,
                    &p,
                    "terraform-native-input",
                    message,
                ));
                continue;
            }
        };
        let presence = bound.scalar.presence;
        let optional = matches!(
            presence,
            native::Presence::Optional | native::Presence::OptionalNullable
        );
        let nullable = matches!(
            presence,
            native::Presence::Nullable | native::Presence::OptionalNullable
        );
        if !used_members.insert((bound.body, bound.member.clone()))
            || attr.r#type != bound.scalar.kind
            || attr.state_only
            || input.null == NullInput::Omit && !optional
            || input.null == NullInput::SendNull && !nullable
            || attr.mode == AttributeMode::OptionalComputed
                && (!optional || input.null != NullInput::Omit)
            || matches!(phase, Phase::Create | Phase::DataRead) && !attr.mode.configured()
        {
            errors.push(error(sdk.contract(), &bound.source, &p, "terraform-input-shape", "input must have a unique matching scalar binding and a representable explicit null policy; computed/defaulted API values cannot be invented"));
        }
        if let InputWhen::TriggerChanged { attribute } = &input.when {
            if phase != Phase::Update
                || !attr.write_only
                || !optional
                || input.null != NullInput::Reject
                || !attrs
                    .get(attribute)
                    .is_some_and(|a| a.state_only && a.mode.configured())
            {
                errors.push(error(sdk.contract(), &bound.source, &p, "terraform-write-only-trigger", "trigger_changed requires an optional SDK write-only update field, reject-null and a configured state-only trigger"));
            }
        } else if phase == Phase::Update && attr.write_only {
            errors.push(error(
                sdk.contract(),
                &bound.source,
                &p,
                "terraform-write-only-trigger",
                "write-only updates require an explicit trigger_changed binding",
            ));
        }
        bound_inputs.push(bound);
    }
    for p in op.parameters().iter().filter(|p| p.wire().required()) {
        if !used_members.contains(&(false, p.field_name.clone())) {
            errors.push(error(
                sdk.contract(),
                p.schema(),
                pointer,
                "terraform-input-coverage",
                "a required native SDK parameter has no lifecycle input mapping",
            ));
        }
    }
    if let Some(schema) = op.body().and_then(|b| b.schema())
        && let Ok(fields) = native::record(native::table(sdk), schema)
    {
        for f in fields.iter().filter(|f| f.init.is_none()) {
            if !used_members.contains(&(true, f.name.clone())) {
                errors.push(error(
                    sdk.contract(),
                    &f.source,
                    pointer,
                    "terraform-input-coverage",
                    "a required native SDK body field has no lifecycle input mapping",
                ));
            }
        }
    }
    let success = bind_response(
        sdk,
        op,
        attrs,
        success,
        true,
        &format!(
            "{pointer}/{}",
            if phase == Phase::Delete {
                "success_status"
            } else {
                "success"
            }
        ),
        phase == Phase::Delete,
        errors,
    );
    let mut seen = BTreeSet::new();
    let partial = partial
        .iter()
        .enumerate()
        .filter_map(|(index, r)| {
            let pointer = format!("{pointer}/partial/{index}");
            if !seen.insert(r.status) {
                errors.push(error(
                    sdk.contract(),
                    &op.source,
                    &pointer,
                    "terraform-status-duplicate",
                    "partial/missing statuses must be distinct",
                ));
            }
            bind_response(sdk, op, attrs, r, false, &pointer, false, errors)
        })
        .collect();
    let missing = missing
        .iter()
        .enumerate()
        .filter_map(|(index, status)| {
            let pointer = format!("{pointer}/missing/{index}");
            if !seen.insert(*status) {
                errors.push(error(
                    sdk.contract(),
                    &op.source,
                    &pointer,
                    "terraform-status-duplicate",
                    "partial/missing statuses must be distinct",
                ));
            }
            let response = exact_response(sdk, op, *status, false, &pointer, errors)?;
            if let Some(media) = response.media()
                && matches!(media.wire().representation(), Representation::Stream { .. })
            {
                errors.push(error(
                    sdk.contract(),
                    media.wire().source().use_site().source(),
                    &pointer,
                    "terraform-missing-response-deferred",
                    "missing-resource status requires completed SDK response validation; deferred SDK stream representations are outside lifecycle v1 missing mappings",
                ));
                return None;
            }
            Some(response.clone())
        })
        .collect();
    if errors.len() != before {
        return None;
    }
    Some(BoundCall {
        operation: op.clone(),
        inputs: bound_inputs,
        success: success?,
        partial,
        missing,
    })
}

fn exact_response<'a>(
    sdk: &HttpPlan,
    op: &'a PlannedOperation,
    status: u16,
    success: bool,
    pointer: &str,
    errors: &mut Vec<Diagnostic>,
) -> Option<&'a PlannedResponse> {
    let matches = op
        .responses()
        .iter()
        .filter(|r| r.status() == ResponseStatus::Exact(status))
        .collect::<Vec<_>>();
    if matches.len() != 1 || success != (200..300).contains(&status) {
        errors.push(error(sdk.contract(), &op.source, pointer, "terraform-response-status", "status must select exactly one source-declared SDK response of the correct success/error kind"));
        None
    } else {
        Some(matches[0])
    }
}

#[allow(clippy::too_many_arguments)]
fn bind_response(
    sdk: &HttpPlan,
    op: &PlannedOperation,
    attrs: &Attributes,
    mapping: &Response,
    success: bool,
    pointer: &str,
    delete: bool,
    errors: &mut Vec<Diagnostic>,
) -> Option<BoundResponse> {
    let status_pointer = if delete {
        pointer.to_owned()
    } else {
        format!("{pointer}/status")
    };
    let response = exact_response(sdk, op, mapping.status, success, &status_pointer, errors)?;
    if delete {
        if !response.forbidden_body || !mapping.state.is_empty() {
            errors.push(error(
                sdk.contract(),
                &op.source,
                pointer,
                "terraform-delete-response",
                "v1 delete success must be an SDK no-content response",
            ));
        }
        return Some(BoundResponse {
            response: response.clone(),
            outputs: vec![],
        });
    }
    let Some(schema) = response.schema().filter(|_| {
        response.media().is_some_and(|m| {
            matches!(m.wire().representation(), Representation::Json { .. })
                && !m.requires_content_type()
        })
    }) else {
        errors.push(error(
            sdk.contract(),
            &op.source,
            pointer,
            "terraform-response-model",
            "state-bearing responses require concrete JSON native records",
        ));
        return None;
    };
    let mut outputs = Vec::new();
    for (name, path) in &mapping.state {
        let p = format!("{pointer}/state/{}", token(name));
        match (attrs.get(name), native::field(native::table(sdk), schema, path)) {
            (Some(attr), Ok(field)) if !attr.write_only && !attr.state_only && attr.r#type == field.scalar.kind => {
                outputs.push(BoundOutput { attribute: name.clone(), field });
            }
            (_, Err(message)) => errors.push(error(sdk.contract(), schema, &p, "terraform-native-output", message)),
            _ => errors.push(error(sdk.contract(), schema, &p, "terraform-state-shape", "state path must bind a matching non-write-only, non-trigger Terraform scalar attribute")),
        }
    }
    for (name, attr) in attrs {
        if !attr.write_only && !attr.state_only && !mapping.state.contains_key(name) {
            errors.push(error(sdk.contract(), schema, pointer, "terraform-state-coverage", format!("state response does not map attribute {name:?}; refresh cannot preserve an invented value")));
        }
    }
    Some(BoundResponse {
        response: response.clone(),
        outputs,
    })
}

fn check_mutable_coverage(
    sdk: &HttpPlan,
    mapping: &ResourceMapping,
    create: &BoundCall,
    update: &BoundCall,
    pointer: &str,
    errors: &mut Vec<Diagnostic>,
) {
    for input in &update.inputs {
        if mapping.attributes[&input.mapping.attribute].mode == AttributeMode::Computed
            && input.mapping.attribute != mapping.identity.attribute
        {
            errors.push(error(
                sdk.contract(),
                &input.source,
                pointer,
                "terraform-input-shape",
                "only the explicit stable identity may supply a computed update input",
            ));
        }
    }
    for (name, attr) in &mapping.attributes {
        let created = create.inputs.iter().any(|i| i.mapping.attribute == *name);
        let updated = update.inputs.iter().any(|i| i.mapping.attribute == *name);
        let trigger = update.inputs.iter().any(|i| matches!(&i.mapping.when, InputWhen::TriggerChanged { attribute } if attribute == name));
        if attr.state_only && !trigger
            || attr.mode.configured()
                && !attr.state_only
                && (!created || !attr.requires_replace && !updated)
        {
            errors.push(error(sdk.contract(), &update.operation.source, &format!("{pointer}/attributes/{}", token(name)), "terraform-lifecycle-coverage", "configured API attributes need create and update bindings (or replacement); Terraform-only attributes must be write-only update triggers"));
        }
    }
}

pub(super) fn attribute_field(attrs: &Attributes, name: &str) -> String {
    format!(
        "Value{}",
        attrs
            .keys()
            .position(|key| key == name)
            .expect("admitted attribute")
    )
}
