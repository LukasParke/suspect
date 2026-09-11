//! Source-bound external Go documentation, native package comments and runnable
//! examples. Signatures come from `go doc` at documentation-build time; native
//! names and all wire/example bindings come from the retained plans.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use suspect_ir::contract::SourceId;

use super::{HttpPlan, PackageConfig, PlannedOperation};
use crate::{
    OutFile,
    go_models::{GoDecl, GoType},
};

struct Symbol {
    name: String,
    /// A registry field is documented by its owning anonymous struct. All other
    /// entries are native go-doc queries, including methods and struct fields.
    lookup: String,
    kind: &'static str,
    file: &'static str,
    source: Option<SourceId>,
    description: String,
    related: Vec<String>,
}

fn q(text: &str) -> String {
    serde_json::to_string(text).expect("Go-compatible string literal")
}

fn location(source: &SourceId) -> Value {
    json!({"document":source.document().as_str(), "pointer":source.pointer()})
}

fn source_text(source: &SourceId) -> String {
    format!("Source: {}#{}", source.document(), source.pointer())
}

fn comment(text: &str) -> String {
    text.replace(['\r', '\n', '\u{2028}', '\u{2029}'], " ")
}

fn literal(text: &str, language: &str) -> String {
    let mut out = format!(".. code-block:: {language}\n\n");
    for line in text
        .replace(
            [
                '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}',
                '\u{2029}',
            ],
            "\n",
        )
        .lines()
    {
        out.push_str("   ");
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');
    out
}

fn heading(text: &str, level: char) -> String {
    format!("{text}\n{}\n\n", level.to_string().repeat(text.len()))
}

fn type_links(ty: &GoType, plan: &HttpPlan, links: &mut Vec<String>) {
    match ty {
        GoType::Named(key) => links.push(plan.codecs().models().descriptors().names[key].clone()),
        GoType::Nullable(inner) | GoType::Optional(inner) | GoType::Presence(inner) => {
            links.push(
                match ty {
                    GoType::Nullable(_) => "Nullable",
                    GoType::Optional(_) => "Optional",
                    _ => "Presence",
                }
                .into(),
            );
            type_links(inner, plan, links);
        }
        GoType::Slice(inner) | GoType::Map(inner) | GoType::Pointer(inner) => {
            type_links(inner, plan, links)
        }
        GoType::Primitive(name @ ("Number" | "Integer" | "Value")) => links.push((*name).into()),
        _ => {}
    }
}

fn add(
    symbols: &mut Vec<Symbol>,
    name: String,
    kind: &'static str,
    file: &'static str,
    source: Option<&SourceId>,
    description: impl Into<String>,
    related: Vec<String>,
) {
    symbols.push(Symbol {
        lookup: name.clone(),
        name,
        kind,
        file,
        source: source.cloned(),
        description: description.into(),
        related,
    });
}

fn symbols(plan: &HttpPlan) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    for (file, names, description) in [
        (
            "http_runtime.go",
            &[
                "Client",
                "NewClient",
                "Client.CloseIdleConnections",
                "ClientOptions",
                "Credentials",
                "Credentials.WithBearer",
                "Doer",
                "Doer.Do",
                "HTTPSource",
                "SDKError",
                "SDKError.Error",
                "SDKError.Unwrap",
                "SDKError.Format",
                "SDKError.ResourceLimited",
                "APIResponse",
                "APIResponse.Close",
            ][..],
            "Reusable HTTP runtime. Use context-owned cancellation, explicit source-scheme credentials and ClientOptions for an injected Doer, server override and finite response/capture policy.",
        ),
        (
            "codec_runtime.go",
            &[
                "Codec",
                "Codec.Decode",
                "Codec.DecodeValue",
                "Codec.Encode",
                "Codec.EncodeValue",
                "CodecError",
                "CodecError.Error",
                "CodecError.Unwrap",
                "CodecError.ResourceLimited",
            ][..],
            "Typed source codec. Decoding validates exact wire values; encoding revalidates mutable native models. Incomplete evaluation or exhausted resource budgets are errors.",
        ),
        (
            "codecs.go",
            &["Codecs"][..],
            "The public registry contains one typed Codec per native model, including non-null representations. Each registry entry below links to its canonical schema source.",
        ),
        (
            "models.go",
            &[
                "Nullable",
                "NullableNull",
                "NullableValue",
                "Nullable.MarshalJSON",
                "Nullable.UnmarshalJSON",
                "Optional",
                "OptionalAbsent",
                "OptionalSome",
                "Optional.MarshalJSON",
                "Optional.UnmarshalJSON",
                "Presence",
                "PresenceMissing",
                "PresenceNull",
                "PresenceSome",
                "Presence.MarshalJSON",
                "Presence.UnmarshalJSON",
            ][..],
            "Presence wrappers distinguish null, absence and a constructed value. Encode through the owning model's source codec; standalone wrapper encoding/json methods intentionally reject serialization without schema context.",
        ),
        (
            "json.go",
            &[
                "Value",
                "Number",
                "Number.String",
                "Number.IsInteger",
                "Integer",
                "Integer.String",
                "ParseNumber",
                "ParseInteger",
                "Parse",
                "Encode",
                "Limits",
                "DefaultLimits",
                "JSONError",
                "JSONError.Error",
                "JSONErrorKind",
                "JSONSyntax",
                "JSONDuplicateName",
                "JSONBadUnicode",
                "JSONLimit",
                "JSONCycle",
                "JSONType",
            ][..],
            "Exact JSON support. Number/Integer retain validated tokens; zero-value numbers are invalid. Parsing and encoding have per-call finite budgets and never use float64 for native numbers.",
        ),
        (
            "validation.go",
            &[
                "ValidationSource",
                "ValidationError",
                "ValidationError.Error",
                "ValidationError.ResourceLimited",
                "ValidationFinding",
                "ValidationFinding.Source",
                "ValidationFinding.InstancePath",
                "ValidationFinding.Message",
                "ValidationError.Findings",
                "Validate",
            ][..],
            "Low-level checked schema program. Prefer a model's typed Codec for ordinary use. Incomplete schema evaluation never becomes validation success.",
        ),
    ] {
        for name in names {
            add(
                &mut symbols,
                (*name).into(),
                "runtime",
                file,
                None,
                description,
                Vec::new(),
            );
        }
    }
    for (owner, fields) in [
        ("HTTPSource", &["Document", "Pointer"][..]),
        (
            "SDKError",
            &[
                "Kind",
                "Operation",
                "Source",
                "Status",
                "Headers",
                "RawCapture",
                "Truncated",
                "Cause",
            ][..],
        ),
        (
            "ClientOptions",
            &[
                "Transport",
                "ServerURL",
                "MaxResponseBytes",
                "MaxCaptureBytes",
                "Timeout",
            ][..],
        ),
        ("APIResponse", &["Status", "Headers", "Data"][..]),
    ] {
        for field in fields {
            add(
                &mut symbols,
                format!("{owner}.{field}"),
                "runtime-field",
                "http_runtime.go",
                None,
                "Explicit HTTP configuration or diagnostic metadata. Error display omits captures and causes; fields preserve them for deliberate inspection.",
                vec![owner.into()],
            );
        }
    }
    for (file, owner, fields) in [
        ("models.go", "Nullable", &["IsValue", "Value"][..]),
        ("models.go", "Optional", &["IsSet", "Value"][..]),
        ("models.go", "Presence", &["IsSet", "Null", "Value"][..]),
        (
            "codec_runtime.go",
            "CodecError",
            &["Kind", "Source", "Path", "Message", "Cause"][..],
        ),
        ("json.go", "JSONError", &["Kind", "Offset", "Message"][..]),
        (
            "json.go",
            "Limits",
            &[
                "MaxBytes",
                "MaxOutputBytes",
                "MaxDepth",
                "MaxNodes",
                "MaxNumberLength",
            ][..],
        ),
        (
            "validation.go",
            "ValidationSource",
            &["Document", "Pointer"][..],
        ),
        (
            "validation.go",
            "ValidationError",
            &["Kind", "Source", "InstancePath", "Message"][..],
        ),
    ] {
        for field in fields {
            add(
                &mut symbols,
                format!("{owner}.{field}"),
                "runtime-field",
                file,
                None,
                "Public runtime field. The owning native declaration documents its value, presence, resource or diagnostic policy.",
                vec![owner.into()],
            );
        }
    }
    for model in plan.codecs().models().symbols() {
        let name = model.name();
        let source = model.source();
        let declaration =
            &plan.codecs().models().descriptors().declarations[&(source.clone(), model.role())];
        let description = plan
            .contract()
            .source(source)
            .and_then(|raw| raw.get("description"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let mut related = vec![format!("Codecs.{name}")];
        for ty in declaration.types() {
            type_links(ty, plan, &mut related);
        }
        if let GoDecl::Struct {
            extras: Some(extra),
            ..
        } = declaration
        {
            type_links(extra, plan, &mut related);
        }
        related.sort();
        related.dedup();
        related.retain(|target| target != name);
        add(
            &mut symbols,
            name.into(),
            "model",
            "models.go",
            Some(source),
            description,
            related,
        );
        symbols.push(Symbol {
            name: format!("Codecs.{name}"), lookup: "Codecs".into(), kind: "codec", file: "codecs.go",
            source: Some(source.clone()), description: "Typed registry entry. Its actual Codec type is shown in the linked native Codecs registry declaration.".into(),
            related: vec![name.into(), "Codecs".into(), "Codec".into()],
        });
        match declaration {
            GoDecl::Struct { fields, extras } => {
                add(
                    &mut symbols,
                    format!("New{name}"),
                    "model-constructor",
                    "models.go",
                    Some(source),
                    "Construct the source-required fields; optional fields start absent and source defaults are not applied.",
                    vec![name.into()],
                );
                for field in fields {
                    let mut related = vec![name.into()];
                    type_links(&field.ty, plan, &mut related);
                    related.sort();
                    related.dedup();
                    add(
                        &mut symbols,
                        format!("{name}.{}", field.name),
                        "model-field",
                        "models.go",
                        Some(&field.source),
                        format!(
                            "Wire property: {}. Required: {}.\n{}",
                            field.wire,
                            field.init.is_none(),
                            field.description
                        ),
                        related,
                    );
                }
                if extras.is_some() {
                    for member in ["SetExtra", "Extra"] {
                        add(
                            &mut symbols,
                            format!("{name}.{member}"),
                            "extra-properties",
                            "models.go",
                            Some(source),
                            "Access undeclared wire properties according to the source additionalProperties policy. Declared wire properties cannot be shadowed.",
                            vec![name.into()],
                        );
                    }
                }
                for member in ["MarshalJSON", "UnmarshalJSON"] {
                    add(
                        &mut symbols,
                        format!("{name}.{member}"),
                        "model-json",
                        "models.go",
                        Some(source),
                        "encoding/json adapter using this model's source-aware codec.",
                        vec![format!("Codecs.{name}")],
                    );
                }
            }
            GoDecl::Literals { values, .. } => {
                for value in values {
                    add(
                        &mut symbols,
                        format!("{name}{}", value.name),
                        "literal",
                        "models.go",
                        Some(source),
                        format!("Source literal: {}", value.token),
                        vec![name.into()],
                    );
                }
                for member in ["MarshalJSON", "UnmarshalJSON"] {
                    add(
                        &mut symbols,
                        format!("{name}.{member}"),
                        "model-json",
                        "models.go",
                        Some(source),
                        "encoding/json adapter validates source literal membership through the codec.",
                        vec![format!("Codecs.{name}")],
                    );
                }
            }
            GoDecl::Union(variants) => {
                for variant in variants {
                    let concrete = format!("{name}{}", variant.name);
                    add(
                        &mut symbols,
                        concrete.clone(),
                        "union-alternative",
                        "models.go",
                        Some(&variant.source),
                        "One source union alternative. The enclosing codec validates branch membership and oneOf exclusivity.",
                        vec![name.into(), format!("Codecs.{name}")],
                    );
                    add(
                        &mut symbols,
                        format!("New{concrete}"),
                        "union-constructor",
                        "models.go",
                        Some(&variant.source),
                        "Construct this source alternative of the closed union.",
                        vec![concrete.clone()],
                    );
                    add(
                        &mut symbols,
                        format!("{concrete}.Value"),
                        "union-value",
                        "models.go",
                        Some(&variant.source),
                        "The alternative's native model value.",
                        vec![concrete.clone()],
                    );
                    add(
                        &mut symbols,
                        format!("{concrete}.MarshalJSON"),
                        "model-json",
                        "models.go",
                        Some(&variant.source),
                        "Encode through the enclosing union's source codec.",
                        vec![format!("Codecs.{name}")],
                    );
                }
            }
            GoDecl::Alias(_) => {}
        }
    }
    for (scheme, constructor) in plan.credentials() {
        let requirement = plan
            .protocol()
            .operations()
            .iter()
            .flat_map(|op| op.security().alternatives())
            .flat_map(|a| a.requirements())
            .find(|r| r.name() == scheme)
            .expect("planned credential has a requirement");
        add(
            &mut symbols,
            constructor.clone(),
            "credential",
            "operations.go",
            Some(requirement.scheme().use_site().source()),
            format!("Supply caller-owned credentials for the exact source scheme {scheme}."),
            vec!["Credentials".into()],
        );
    }
    for op in plan.operations() {
        add(
            &mut symbols,
            format!("Client.{}", op.method_name),
            "operation",
            "operations.go",
            Some(&op.source),
            super::emit::description(op),
            vec![
                op.input_type.clone(),
                op.success_type.clone(),
                op.error_variant.clone(),
            ],
        );
        add(
            &mut symbols,
            op.input_type.clone(),
            "operation-input",
            "operations.go",
            Some(&op.source),
            "Required constructor arguments and source-defined optional input members.",
            vec![
                op.input_constructor.clone(),
                format!("Client.{}", op.method_name),
            ],
        );
        add(
            &mut symbols,
            op.input_constructor.clone(),
            "input-constructor",
            "operations.go",
            Some(&op.source),
            "Construct the required inputs in their planned order. Optional inputs start absent.",
            vec![op.input_type.clone()],
        );
        for parameter in op.parameters() {
            let wire = parameter.wire();
            let description = format!(
                "{:?} wire parameter: {}. Required: {}. Serialization: {:?}.\n{}",
                wire.location(),
                wire.name(),
                wire.required(),
                wire.serialization(),
                wire.description().map_or("", |v| v.value())
            );
            let related = vec![
                op.input_type.clone(),
                plan.symbols()[parameter.schema()].clone(),
            ];
            add(
                &mut symbols,
                format!("{}.{}", op.input_type, parameter.field_name),
                "parameter",
                "operations.go",
                Some(wire.source().use_site().source()),
                description.clone(),
                related.clone(),
            );
            if let Some(setter) = &parameter.setter_name {
                add(
                    &mut symbols,
                    format!("{}.{setter}", op.input_type),
                    "input-setter",
                    "operations.go",
                    Some(wire.source().use_site().source()),
                    description,
                    related,
                );
            }
        }
        if let Some(body) = op.body() {
            let wire = body.wire();
            let mut related = vec![op.input_type.clone()];
            if let Some(schema) = body.schema() {
                related.push(plan.symbols()[schema].clone());
            }
            add(
                &mut symbols,
                format!("{}.{}", op.input_type, body.field_name),
                "request-body",
                "operations.go",
                Some(wire.source().use_site().source()),
                format!(
                    "{} request body. Required: {}.",
                    body.native_type,
                    wire.required()
                ),
                related.clone(),
            );
            if let Some(setter) = &body.setter_name {
                add(
                    &mut symbols,
                    format!("{}.{setter}", op.input_type),
                    "input-setter",
                    "operations.go",
                    Some(wire.source().use_site().source()),
                    "Supply the optional source-defined request body.",
                    related,
                );
            }
        }
        for (name, success) in [(&op.success_type, true), (&op.error_variant, false)] {
            add(
                &mut symbols,
                name.clone(),
                "response-union",
                "operations.go",
                Some(&op.source),
                if success {
                    "Closed source-declared success union. Type-switch on its exact status wrappers."
                } else {
                    "Closed declared API-error union. errors.As can recover a concrete status wrapper. Transport, resource and malformed responses are SDKError failures."
                },
                op.responses()
                    .iter()
                    .filter(|r| {
                        if success {
                            r.can_succeed()
                        } else {
                            r.can_fail()
                        }
                    })
                    .map(|r| r.type_name.clone())
                    .collect(),
            );
        }
        for response in op.responses() {
            let wire = response.wire();
            let related = response
                .schema()
                .map(|s| {
                    let model = &plan.symbols()[s];
                    vec![
                        model.clone(),
                        format!("Codecs.{model}"),
                        "APIResponse".into(),
                    ]
                })
                .unwrap_or_else(|| vec!["APIResponse".into()]);
            add(
                &mut symbols,
                response.type_name.clone(),
                "response",
                "operations.go",
                Some(wire.source().use_site().source()),
                format!(
                    "HTTP {} {}. Embeds APIResponse with source-validated Data, Status and Headers.\n{}",
                    wire.status_key(),
                    response.native_type,
                    wire.description().value()
                ),
                related,
            );
            if response.can_fail() {
                for member in ["Error", "Format"] {
                    add(
                        &mut symbols,
                        format!("{}.{member}", response.type_name),
                        "api-error",
                        "operations.go",
                        Some(wire.source().use_site().source()),
                        "Redacted formatting for this declared API error; response data is explicitly accessible.",
                        vec![response.type_name.clone()],
                    );
                }
            }
        }
    }
    protocol_symbols(plan, &mut symbols);
    symbols.sort_by(|a, b| a.name.cmp(&b.name));
    symbols.dedup_by(|a, b| a.name == b.name);
    symbols
}

fn protocol_symbols(plan: &HttpPlan, symbols: &mut Vec<Symbol>) {
    if let Some(factory) = plan.credential_env_factory() {
        add(
            symbols,
            factory.into(),
            "credential-env-factory",
            "http_environment.go",
            None,
            "Snapshot the explicitly configured environment variables once at client creation. Zero or one ClientOptions. Missing/empty/unusable values remain missing; protected operations resolve source auth and fail before HTTP. NewClient remains wholly explicit.",
            vec![
                "NewClient".into(),
                "Credentials".into(),
                "ClientOptions".into(),
            ],
        );
    }
    for (file, names) in [
        (
            "http_runtime.go",
            &[
                "HTTPProvenance",
                "HTTPProvenance.UseSite",
                "HTTPProvenance.Definition",
                "HTTPProvenance.References",
                "NoContent",
                "Content",
                "Content.ContentType",
                "Content.Data",
                "Part",
                "NewPart",
                "Part.Data",
                "Part.ContentType",
                "Part.Filename",
                "Part.Headers",
                "Part.WithContentType",
                "Part.WithFilename",
                "Part.WithHeader",
                "LocatedValue",
                "LocatedValue.Source",
                "LocatedValue.JSON",
                "Link",
                "Link.Name",
                "Link.Source",
                "Link.OperationID",
                "Link.OperationRef",
                "Link.Target",
                "Link.Parameters",
                "Link.RequestBody",
                "Link.Description",
                "Link.Server",
                "APIResponse.ContentType",
                "APIResponse.Links",
                "ClientOptions.ServerIndex",
                "ClientOptions.ServerVariables",
                "ClientOptions.DocumentURL",
                "ClientOptions.SecurityAlternative",
                "ClientOptions.MaxPartBytes",
                "ClientOptions.MaxStreamItemBytes",
            ][..],
        ),
        (
            "http_security.go",
            &[
                "Server",
                "Server.URL",
                "Server.Name",
                "Server.Description",
                "Server.Source",
                "Server.DefaultFrom",
                "Server.DocumentBase",
                "Server.Variables",
                "ServerVariable",
                "ServerVariable.Name",
                "ServerVariable.Default",
                "ServerVariable.Values",
                "ServerVariable.Source",
                "ServerVariable.Description",
                "OAuthFlow",
                "OAuthFlow.Source",
                "OAuthFlow.Kind",
                "OAuthFlow.URLBase",
                "OAuthFlow.AuthorizationURL",
                "OAuthFlow.TokenURL",
                "OAuthFlow.RefreshURL",
                "OAuthFlow.DeviceAuthorizationURL",
                "OAuthFlow.Scopes",
                "SecurityRequirement",
                "SecurityRequirement.Name",
                "SecurityRequirement.Kind",
                "SecurityRequirement.URLBase",
                "SecurityRequirement.Location",
                "SecurityRequirement.WireName",
                "SecurityRequirement.Source",
                "SecurityRequirement.Scheme",
                "SecurityRequirement.Scopes",
                "SecurityRequirement.Roles",
                "SecurityRequirement.Flows",
                "SecurityRequirement.MetadataURL",
                "SecurityRequirement.DiscoveryURL",
                "SecurityRequirement.Description",
                "SecurityAlternative",
                "SecurityAlternative.Source",
                "SecurityAlternative.Requirements",
                "CredentialRequest",
                "CredentialRequest.Operation",
                "CredentialRequest.ServerURL",
                "CredentialRequest.Requirement",
                "CredentialHook",
                "Authorization",
                "Authorization.Scheme",
                "Authorization.Value",
                "Credentials.WithBasic",
                "Credentials.WithAPIKey",
                "Credentials.WithAuthorization",
                "Credentials.WithHook",
            ][..],
        ),
        (
            "http_stream.go",
            &[
                "Stream",
                "Stream.Next",
                "Stream.Value",
                "Stream.Err",
                "Stream.Close",
            ][..],
        ),
    ] {
        for name in names {
            add(
                symbols,
                (*name).into(),
                "protocol-runtime",
                file,
                None,
                "Source-backed HTTP protocol API. Streams and parts are finite; credential and link metadata do not activate workflows.",
                vec![],
            );
        }
    }
    for aggregate in super::emit::aggregates(plan) {
        add(
            symbols,
            aggregate.type_name.clone(),
            "aggregate",
            "operations.go",
            None,
            "Typed form/multipart fields with structural required/extras/cardinality rules and per-part codecs.",
            vec![aggregate.constructor.clone()],
        );
        add(
            symbols,
            aggregate.constructor.clone(),
            "aggregate-constructor",
            "operations.go",
            None,
            "Construct required fields using ordinary native values. File data is in-memory bytes.",
            vec![aggregate.type_name.clone()],
        );
        for part in aggregate.parts.iter().chain(aggregate.additional.iter()) {
            add(
                symbols,
                format!("{}.{}", aggregate.type_name, part.field_name),
                "part",
                "operations.go",
                Some(part.wire.source().use_site().source()),
                format!("Native part type: {}.", part.native_type),
                vec![aggregate.type_name.clone()],
            );
            if let Some(setter) = &part.setter_name {
                add(
                    symbols,
                    format!("{}.{setter}", aggregate.type_name),
                    "part-setter",
                    "operations.go",
                    Some(part.wire.source().use_site().source()),
                    "Supply an optional part.",
                    vec![aggregate.type_name.clone()],
                );
            }
        }
    }
    for op in plan.operations() {
        if let Some(method) = &op.data_method {
            add(
                symbols,
                format!("Client.{method}"),
                "data-operation",
                "operations.go",
                Some(&op.source),
                "Return the single successful data representation directly.",
                vec![format!("Client.{}", op.method_name)],
            );
        }
        if let Some(body) = op.body().filter(|b| b.is_choice()) {
            add(
                symbols,
                body.native_type.clone(),
                "body-choice",
                "operations.go",
                Some(body.wire().source().use_site().source()),
                "Closed source-declared request-media choice.",
                vec![],
            );
            for m in body.media() {
                add(
                    symbols,
                    m.choice_type.clone(),
                    "body-media",
                    "operations.go",
                    Some(m.wire().source().use_site().source()),
                    format!(
                        "Typed {} body; concrete Content-Type selection is checked before encoding.",
                        m.wire().media_type().declared()
                    ),
                    vec![body.native_type.clone(), m.constructor.clone()],
                );
                add(
                    symbols,
                    m.constructor.clone(),
                    "body-constructor",
                    "operations.go",
                    Some(m.wire().source().use_site().source()),
                    "Construct a typed request-media alternative.",
                    vec![m.choice_type.clone()],
                );
            }
        }
        for r in op.responses() {
            add(
                symbols,
                r.headers_type.clone(),
                "response-headers",
                "operations.go",
                Some(r.wire().source().use_site().source()),
                "Required and optional headers decoded through their exact source codecs.",
                vec![],
            );
            add(
                symbols,
                format!("{}.DecodedHeaders", r.type_name),
                "response-headers-field",
                "operations.go",
                Some(r.wire().source().use_site().source()),
                "Source-validated typed headers, separate from the raw HTTP header map.",
                vec![r.headers_type.clone()],
            );
            for h in &r.headers {
                add(
                    symbols,
                    format!("{}.{}", r.headers_type, h.field_name),
                    "response-header",
                    "operations.go",
                    Some(h.wire.source().use_site().source()),
                    format!("Header {}. Required: {}.", h.wire.name(), h.wire.required()),
                    vec![plan.symbols()[h.wire.codec().schema().id()].clone()],
                );
            }
        }
    }
}

/// Full generation-root-relative paths; package configuration never changes a
/// source binding or an allocated native operation/model name.
pub(super) fn artifacts(plan: &HttpPlan, package: &PackageConfig) -> Vec<OutFile> {
    let symbols = symbols(plan);
    let labels: BTreeMap<_, _> = symbols
        .iter()
        .enumerate()
        .map(|(i, symbol)| (symbol.name.clone(), format!("go-symbol-{i}")))
        .collect();
    let mut api = heading("Public Go symbols", '=');
    api.push_str("Native declarations below are obtained from go doc in this exact module. Descriptions and native comments render literally. Every model, field, codec entry, operation, input and concrete response retains its canonical source binding.\n\n");
    for symbol in &symbols {
        api.push_str(&format!(".. _{}:\n\n", labels[&symbol.name]));
        api.push_str(&heading(&format!("``{}``", symbol.name), '-'));
        if let Some(source) = &symbol.source {
            api.push_str(&literal(&source_text(source), "text"));
        }
        if !symbol.description.is_empty() {
            api.push_str(&literal(&symbol.description, "text"));
        }
        if matches!(symbol.kind, "model" | "model-field" | "union-alternative")
            && let Some(raw) = symbol
                .source
                .as_ref()
                .and_then(|source| plan.contract().source(source))
        {
            api.push_str("Original schema (annotations are not inferred behavior):\n\n");
            api.push_str(&literal(
                &serde_json::to_string_pretty(raw).unwrap(),
                "json",
            ));
        }
        api.push_str(&format!(
            ".. native-go:: {}\n{}\n",
            symbol.name,
            if symbol.lookup == symbol.name {
                String::new()
            } else {
                format!("   :lookup: {}\n", symbol.lookup)
            }
        ));
        if !symbol.related.is_empty() {
            api.push_str("Related: ");
            api.push_str(
                &symbol
                    .related
                    .iter()
                    .map(|name| format!(":ref:`{}`", labels[name]))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            api.push_str(".\n\n");
        }
    }
    let mut operations = heading("HTTP source bindings", '=');
    for operation in plan.operations() {
        operations.push_str(&heading(&format!("``{}``", operation.method_name), '-'));
        operations.push_str(&format!(
            "Native call: :ref:`{}`.\n\n",
            labels[&format!("Client.{}", operation.method_name)]
        ));
        operations.push_str(&literal(
            &serde_json::to_string_pretty(&operation_binding(plan, operation)).unwrap(),
            "json",
        ));
    }
    let examples = validated_examples(plan, package);
    let mut bindings = json!({
        "format":"suspect-go-docs-v1", "module":package.module_path, "package":package.package_name, "version":package.version,
        "signatureAuthority":"go doc from the generated module",
        "exampleAvailability":"source/synthesized codec values and explicitly labeled native byte/schema-free recipes; native representation, transport and API failures remain runtime outcomes",
        "symbols":symbols.iter().map(|symbol|json!({
            "name":symbol.name,"lookup":symbol.lookup,"kind":symbol.kind,"file":symbol.file,
            "source":symbol.source.as_ref().map(location),"description":symbol.description,"related":symbol.related,
            "documentation":{"file":"docs/api.rst","anchor":labels[&symbol.name]},
        })).collect::<Vec<_>>(),
        "operations":plan.operations().iter().map(|op|operation_binding(plan,op)).collect::<Vec<_>>(),
        "examples":examples.1,
    });
    if let Some(policy) = plan.credential_env() {
        bindings["credentialEnv"] = json!(policy);
        bindings["credentialEnvFactory"] = json!(plan.credential_env_factory());
    }
    let mut index = format!(
        "{}Source-selected standard-library HTTP clients and exact native model codecs.\n\n.. toctree::\n   :maxdepth: 2\n\n   api\n   operations\n   examples\n   native\n\nImport and documentation\n------------------------\n\nUse the configured module path in your consumer (for local development, a go.mod replace directive can point at this generated module)::\n\n   import sdk {}\n\nThe declared floor is Go 1.23. Native package comments are available through ``go doc -all .``. Build browsable documentation with Sphinx 8.2.3; every declaration is obtained from the selected Go toolchain, and missing native symbols or crosslinks fail the build::\n\n   python -m sphinx -W --keep-going -b html docs docs/_build/html\n\nRuntime policy\n--------------\n\n``NewClient`` accepts explicit source-scheme ``Credentials`` and ``ClientOptions``. Each operation takes ``context.Context`` plus a typed input. The context owns request and body cancellation. ``ServerURL`` overrides the source server including its prefix. Default transport ignores ambient proxies, disables decompression, follows no redirects and adds no SDK retry policy. Standard net/http connection recovery remains transport policy; request bodies have no replay factory. An injected ``Doer`` supplies its own transport policy.\n\nInputs validate through their source codecs before transport, including mutable model constraints. Exact numbers and presence/null wrappers preserve wire distinctions. Declared successful statuses return a closed result union; declared non-2xx statuses return concrete errors with decoded ``APIResponse`` data. Other failures are ``SDKError``. Normal error formatting omits captures and causes; explicit fields and Unwrap retain diagnostics.\n\nGenerated request/response ceilings are finite ({} / {} bytes); callers may lower the response ceiling. Zero ``MaxCaptureBytes`` selects the runtime default, not capture disabling. Wrapper zero values are absent/null, and zero exact-number values are invalid. No defaults, pagination or authentication flows are inferred. Documentation describes this admitted slice and does not certify a complete release.\n\n:download:`Machine-readable source bindings <source-bindings.json>`\n",
        heading(&package.module_path, '='),
        q(&package.module_path),
        plan.config.max_request_bytes,
        plan.config.max_response_bytes
    );
    if let Some(policy) = plan.credential_env() {
        let factory = plan
            .credential_env_factory()
            .expect("bound environment factory");
        index.push_str(&heading("Explicit environment credentials", '-'));
        index.push_str(&format!("Use :ref:`{}` with zero or one ``ClientOptions`` to snapshot the configured variables at client creation. Missing, empty or unusable values remain missing; source security resolution happens at the operation before HTTP. ``NewClient`` is always explicit and never fills empty/null/missing members from environment values.\n\n",labels[factory]));
        index.push_str(&literal(
            &serde_json::to_string_pretty(&policy.semantic_descriptor()).unwrap(),
            "json",
        ));
        index.push_str(":download:`Source-bound environment policy <../credential-env.json>`\n\n");
    }
    let example_docs = format!(
        "{}The shared ExamplePlan validates declared examples before native lowering and labels synthesized values explicitly. Inputs bind by canonical parameter/media container. If a required value is unavailable the callsite is omitted and its reason remains in source-bindings.json; optional missing inputs stay absent.\n\nThe packaged program decodes, encodes and decodes every available example through the native codec registry. Its operation calls use the allocated input constructors and optional setters, so go test/go run compile the real public API::\n\n   go test ./...\n   go run ./examples/validated\n\nBy default only codecs execute. To run all available operation calls against an explicit fixture, supply its URL and a fixture token. The token is supplied explicitly to every source bearer scheme in this sample::\n\n   go run ./examples/validated -server-url http://127.0.0.1:8080/api/v1 -token fixture-token\n\nThe sample owns a 30-second context deadline. HTTP execution expects declared successful responses; API failures are returned normally.\n\n.. literalinclude:: ../examples/validated/main.go\n   :language: go\n\n{}:download:`ExamplePlan manifest <../examples.json>`\n\n.. literalinclude:: ../examples.json\n   :language: json\n",
        heading("Executable examples", '='),
        heading("Values, origins and located findings", '-')
    );
    vec![
        OutFile {
            path: "go/doc.go".into(),
            content: package_comment(plan, package),
        },
        OutFile {
            path: "go/docs/conf.py".into(),
            content: format!(
                "PROJECT = {}\nVERSION = {}\n{SPHINX}\n",
                q(&package.module_path),
                q(&package.version)
            ),
        },
        OutFile {
            path: "go/docs/requirements.txt".into(),
            content: "Sphinx==8.2.3\n".into(),
        },
        OutFile {
            path: "go/docs/index.rst".into(),
            content: index,
        },
        OutFile {
            path: "go/docs/api.rst".into(),
            content: api,
        },
        OutFile {
            path: "go/docs/operations.rst".into(),
            content: operations,
        },
        OutFile {
            path: "go/docs/examples.rst".into(),
            content: example_docs,
        },
        OutFile {
            path: "go/docs/native.rst".into(),
            content: format!(
                "{}The native tool's complete public package output includes exported runtime declarations and helpers as well as source-bound symbols.\n\n.. native-go:: .\n",
                heading("Complete native package reference", '=')
            ),
        },
        OutFile {
            path: "go/docs/source-bindings.json".into(),
            content: format!("{}\n", serde_json::to_string_pretty(&bindings).unwrap()),
        },
        OutFile {
            path: "go/examples/validated/main.go".into(),
            content: examples.0,
        },
    ]
}

fn operation_binding(plan: &HttpPlan, op: &PlannedOperation) -> Value {
    super::emit::binding(plan, op)
}

fn package_comment(plan: &HttpPlan, package: &PackageConfig) -> String {
    let mut code = format!(
        "// Package {} provides source-selected HTTP operations with exact native models\n// and source-aware [Codecs]. Configure [NewClient] with explicit [Credentials]\n// and [ClientOptions]; every operation accepts a caller-owned context.\n//\n// # Selected operations\n//\n",
        package.package_name
    );
    for op in plan.operations() {
        code.push_str(&format!(
            "// [Client.{}] uses [{}] and returns [{}]; declared failures implement [{}].\n//\n",
            op.method_name, op.input_type, op.success_type, op.error_variant
        ));
        code.push_str(&operation_comment(plan, op));
        code.push_str("//\n");
    }
    code.push_str("// # Documentation and examples\n//\n// docs/index.rst builds with Sphinx 8.2.3 using go doc for actual declarations.\n// docs/source-bindings.json binds public symbols to original source locations.\n// Run go run ./examples/validated for source-example codec checks; explicit\n// -server-url and -token flags enable fixture HTTP calls.\n//\n// Optional absence and source-admitted null remain distinct. Mutable models\n// validate again on encoding. Credentials and captures are not included in\n// ordinary HTTP error formatting; explicit fields retain diagnostic metadata.\n");
    code.push_str(&format!("package {}\n", package.package_name));
    code
}

/// Optional small parent-emitter integration: use this directly before the
/// native Client method. It adds source prose and real go-doc symbol crosslinks
/// without maintaining another signature or native-name allocation.
pub(super) fn operation_comment(_plan: &HttpPlan, op: &PlannedOperation) -> String {
    let mut text = format!(
        "// {} calls the source operation once with caller-owned context cancellation.\n// Use [{}] (constructed by [{}]); success implements [{}],\n// declared API failures implement [{}], and other failures are [SDKError].\n//\n//\t{}\n//\t{}\n//\n",
        op.method_name,
        op.input_type,
        op.input_constructor,
        op.success_type,
        op.error_variant,
        comment(super::emit::description(op)),
        comment(&source_text(&op.source))
    );
    for response in op.responses() {
        text.push_str(&format!(
            "// HTTP {}: [{}] contains {}.\n",
            response.wire().status_key(),
            response.type_name,
            response.native_type
        ));
    }
    text
}

fn validated_examples(plan: &HttpPlan, package: &PackageConfig) -> (String, Vec<Value>) {
    super::native_examples::render(plan, package)
}

pub(super) fn quickstart(plan: &HttpPlan, package: &PackageConfig) -> String {
    super::native_examples::quickstart(plan, package)
}

// go doc owns native declaration extraction. Its output is a literal node,
// never reinterpreted as reStructuredText or scanned for generator semantics.
const SPHINX: &str = r#"import json
import os
from pathlib import Path
import subprocess

from docutils import nodes
from sphinx.errors import SphinxError
from sphinx.util.docutils import SphinxDirective

project = PROJECT
release = VERSION
extensions = []
master_doc = "index"
html_theme = "alabaster"
exclude_patterns = ["_build"]
nitpicky = True
_native = {}


def _documentation(symbol):
    if symbol not in _native:
        command = [os.environ.get("SUSPECT_GO_BIN", "go"), "doc", "-all", "."]
        if symbol != ".":
            command.append(symbol)
        environment = dict(os.environ, GOWORK="off")
        if "SUSPECT_GO_TOOLCHAIN" in environment:
            environment["GOTOOLCHAIN"] = environment["SUSPECT_GO_TOOLCHAIN"]
        result = subprocess.run(command, cwd=Path(__file__).resolve().parent.parent,
                                env=environment, text=True, capture_output=True, timeout=90)
        if result.returncode:
            raise SphinxError("Native Go documentation failed for " + symbol + ": " + result.stderr)
        _native[symbol] = result.stdout
    return _native[symbol]


class NativeGo(SphinxDirective):
    required_arguments = 1
    option_spec = {"lookup": str}

    def run(self):
        name = self.arguments[0]
        lookup = self.options.get("lookup", name)
        documentation = _documentation(lookup)
        inventory = getattr(self.env, "suspect_go_symbols", {})
        inventory.setdefault(self.env.docname, {})[name] = lookup
        self.env.suspect_go_symbols = inventory
        root = Path(__file__).resolve().parent.parent
        for source in [root / "go.mod", root / "docs/source-bindings.json", *root.glob("*.go")]:
            self.env.note_dependency(str(source))
        if lookup != name:
            text = name + " — declaration in " + lookup + " (see the linked native registry)."
            return [nodes.literal_block(text, text, language="text")]
        # go doc emits declarations interleaved with ordinary prose, not a
        # compilable Go file. Preserve its complete native output as literal text.
        return [nodes.literal_block(documentation, documentation, language="text")]


def _finished(app, exception):
    if exception is not None:
        return
    bindings = json.loads((Path(__file__).parent / "source-bindings.json").read_text())
    seen = {}
    for entries in getattr(app.env, "suspect_go_symbols", {}).values():
        seen.update(entries)
    missing = [s["name"] for s in bindings["symbols"] if s["name"] not in seen]
    if missing:
        raise SphinxError("Undocumented planned Go symbols: " + ", ".join(missing))
    coverage = {"format": "suspect-go-native-docs-v1", "module": PROJECT,
                "documented": sorted(seen), "queries": sorted(set(seen.values())),
                "plannedSymbols": len(bindings["symbols"])}
    (Path(app.outdir) / "coverage.json").write_text(json.dumps(coverage, indent=2) + "\n")


def _purge(app, env, docname):
    getattr(env, "suspect_go_symbols", {}).pop(docname, None)


def setup(app):
    app.add_directive("native-go", NativeGo)
    app.connect("env-purge-doc", _purge)
    app.connect("build-finished", _finished)
    return {"version": "1", "parallel_read_safe": False}
"#;
