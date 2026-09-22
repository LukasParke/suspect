//! Native presentation of the verified protocol. Schema identities remain bound
//! to the existing model plan; no schema or emitted-code re-parsing takes place.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use serde_json::{Value, json};
use suspect_ir::contract::SchemaId;

use super::{HttpConfig, HttpSymbols, PlannedOperation, source_json, src, upper};
use crate::http_protocol::*;

pub(super) fn canonical_media(media: &MediaType) -> String {
    let essence = match media.range() {
        MediaRange::Any => "*/*".to_owned(),
        MediaRange::Type { type_name } => format!("{type_name}/*"),
        MediaRange::Concrete { type_name, subtype } => format!("{type_name}/{subtype}"),
    };
    let mut value = essence;
    for (key, parameter) in media.parameters() {
        let parameter = if key == "charset" {
            parameter.to_ascii_lowercase()
        } else {
            parameter.clone()
        };
        let token = !parameter.is_empty()
            && parameter
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b));
        write!(
            value,
            ";{key}={}",
            if token {
                parameter
            } else {
                format!(
                    "\"{}\"",
                    parameter.replace('\\', "\\\\").replace('"', "\\\"")
                )
            }
        )
        .unwrap();
    }
    value
}

/// JSON-like JavaScript literals preserve exact numeric metadata and inert keys.
pub(super) fn literal(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => {
            if value
                .as_i64()
                .is_some_and(|v| (-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&v))
                || value.as_u64().is_some_and(|v| v <= 9_007_199_254_740_991)
            {
                value.to_string()
            } else {
                format!(
                    "/* @__PURE__ */ JsonNumber.parse({})",
                    q(&value.to_string())
                )
            }
        }
        Value::String(value) => q(value),
        Value::Array(values) => format!(
            "[{}]",
            values.iter().map(literal).collect::<Vec<_>>().join(",")
        ),
        Value::Object(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!(
                    "{}:{}",
                    if key == "__proto__" {
                        format!("[{}]", q(key))
                    } else {
                        q(key)
                    },
                    literal(value)
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}
fn q(value: &str) -> String {
    serde_json::to_string(value).unwrap()
}
pub(super) fn schema_key(schema: &SchemaId) -> String {
    serde_json::to_string(&[schema.document().as_str(), schema.pointer()]).unwrap()
}
fn model(codec: &CodecRef, names: &BTreeMap<SchemaId, String>) -> String {
    format!("Models.{}", names[codec.schema().id()])
}
pub(super) fn media_type(
    media: &MediaPlan,
    names: &BTreeMap<SchemaId, String>,
    response: bool,
) -> String {
    match media.representation() {
        Representation::Json { codec } => codec
            .as_ref()
            .map_or_else(|| "Models.JsonValue".into(), |codec| model(codec, names)),
        Representation::Text { codec, .. } => codec
            .as_ref()
            .map_or_else(|| "string".into(), |codec| model(codec, names)),
        Representation::Binary { .. } => "Uint8Array".into(),
        Representation::Stream { stream } => {
            // A schemaless stream surfaces untyped parsed envelope values.
            let item = stream
                .item_codec()
                .map_or_else(|| "Models.JsonValue".into(), |codec| model(codec, names));
            if response {
                format!("AsyncIterable<{item}>")
            } else {
                format!("Iterable<{item}> | AsyncIterable<{item}>")
            }
        }
        Representation::Form { form } => object_type(
            form.rules(),
            form.fields(),
            form.additional(),
            names,
            response,
            false,
        ),
        Representation::Multipart {
            multipart:
                MultipartPlan::Named {
                    rules,
                    parts,
                    additional,
                },
        } => object_type(rules, parts, additional, names, response, true),
        Representation::Multipart {
            multipart:
                MultipartPlan::Positional {
                    prefix,
                    items,
                    min_items,
                    max_items,
                    ..
                },
        } => {
            let mut members = prefix
                .iter()
                .enumerate()
                .filter(|(index, _)| {
                    max_items
                        .as_ref()
                        .is_none_or(|max| (*index as u64) < *max.value())
                })
                .map(|(index, part)| {
                    let ty = part_item_type(part, names, response, true);
                    let required = min_items
                        .as_ref()
                        .is_some_and(|min| (index as u64) < *min.value());
                    format!("part{index}{}: {ty}", if required { "" } else { "?" })
                })
                .collect::<Vec<_>>();
            if let AdditionalParts::Allowed(part) = items {
                members.push(format!(
                    "...items: ({})[]",
                    part_item_type(part, names, response, true)
                ));
            }
            format!("readonly [{}]", members.join(", "))
        }
    }
}
pub(super) fn extra_member(parts: &[PartPlan]) -> String {
    let mut name = "additionalFields".to_owned();
    while parts.iter().any(|part| part.name() == Some(name.as_str())) {
        name.insert(0, '_');
    }
    name
}
fn object_type(
    rules: &ObjectRules,
    parts: &[PartPlan],
    additional: &AdditionalParts,
    names: &BTreeMap<SchemaId, String>,
    response: bool,
    multipart: bool,
) -> String {
    let mut fields = parts
        .iter()
        .map(|part| {
            let item = part_item_type(part, names, response, multipart);
            let ty = if part.multiplicity() == PartMultiplicity::RepeatedArrayItems {
                format!("ReadonlyArray<{item}>")
            } else {
                item
            };
            format!(
                "readonly {}{}: {ty}",
                q(part.name().unwrap()),
                if part.required() { "" } else { "?" }
            )
        })
        .collect::<Vec<_>>();
    if let AdditionalParts::Allowed(part) = additional {
        let item = part_item_type(part, names, response, multipart);
        let ty = if part.multiplicity() == PartMultiplicity::RepeatedArrayItems {
            format!("ReadonlyArray<{item}>")
        } else {
            item
        };
        let required = rules
            .required()
            .iter()
            .filter(|name| {
                !parts
                    .iter()
                    .any(|part| part.name() == Some(name.value().as_str()))
            })
            .map(|name| format!("readonly {}: {ty}", q(name.value())))
            .collect::<Vec<_>>();
        fields.push(format!(
            "readonly {}{}: Readonly<Record<string, {ty}>>{}",
            q(&extra_member(parts)),
            if required.is_empty() { "?" } else { "" },
            if required.is_empty() {
                String::new()
            } else {
                format!(" & {{ {} }}", required.join("; "))
            }
        ));
    }
    if fields.is_empty() {
        "Readonly<Record<string, never>>".into()
    } else {
        format!("{{ {} }}", fields.join("; "))
    }
}
pub(super) fn header_type(headers: &[HeaderPlan], names: &BTreeMap<SchemaId, String>) -> String {
    if headers.is_empty() {
        return "Record<string, never>".into();
    }
    format!(
        "{{ {} }}",
        headers
            .iter()
            .map(|header| format!(
                "readonly {}{}: {}",
                q(header.name()),
                if header.required() { "" } else { "?" },
                model(header.codec(), names)
            ))
            .collect::<Vec<_>>()
            .join("; ")
    )
}
pub(super) fn part_item_type(
    part: &PartPlan,
    names: &BTreeMap<SchemaId, String>,
    response: bool,
    multipart: bool,
) -> String {
    let native = match part.representation() {
        PartRepresentation::Json { codec, .. }
        | PartRepresentation::Text { codec, .. }
        | PartRepresentation::Style { codec, .. } => model(codec, names),
        PartRepresentation::Binary { .. } => "Uint8Array".into(),
    };
    if !multipart {
        return native;
    }
    let wrapped = !part.headers().is_empty()
        || part.content_types().len() > 1
        || part
            .content_types()
            .iter()
            .any(|m| !matches!(m.range(), MediaRange::Concrete { .. }));
    let binary = matches!(part.representation(), PartRepresentation::Binary { .. });
    if !wrapped && !binary {
        return native;
    }
    let media = if part
        .content_types()
        .iter()
        .any(|m| !matches!(m.range(), MediaRange::Concrete { .. }))
    {
        "string".into()
    } else {
        let values = part
            .content_types()
            .iter()
            .map(|m| q(&canonical_media(m)))
            .collect::<Vec<_>>();
        if values.is_empty() {
            "never".into()
        } else {
            values.join(" | ")
        }
    };
    let headers = header_type(part.headers(), names);
    let mut result = format!("Part<{native}, {headers}, {media}>");
    if part.headers().iter().any(HeaderPlan::required) {
        write!(result, " & {{ readonly headers: {headers} }}").unwrap();
    }
    if part.content_types().len() > 1
        || part
            .content_types()
            .iter()
            .any(|m| !matches!(m.range(), MediaRange::Concrete { .. }))
    {
        write!(result, " & {{ readonly contentType: {media} }}").unwrap();
    }
    if binary && !wrapped && !response {
        format!("Uint8Array | {result}")
    } else {
        result
    }
}
pub(super) fn tagged_body(body: &BodyPlan) -> bool {
    body.media().len() != 1
        || !matches!(
            body.media()[0].media_type().range(),
            MediaRange::Concrete { .. }
        )
}
pub(super) fn body_type(body: &BodyPlan, names: &BTreeMap<SchemaId, String>) -> String {
    if !tagged_body(body) {
        return media_type(&body.media()[0], names, false);
    }
    body.media()
        .iter()
        .map(|media| {
            if matches!(media.media_type().range(), MediaRange::Concrete { .. }) {
                format!(
                    "MediaBody<{}, {}>",
                    q(&canonical_media(media.media_type())),
                    media_type(media, names, false)
                )
            } else {
                format!(
                    "(MediaBody<string, {}> & {{ readonly mediaType: {} }})",
                    media_type(media, names, false),
                    q(&canonical_media(media.media_type()))
                )
            }
        })
        .collect::<Vec<_>>()
        .join(" | ")
}
fn statuses(
    operation: &OperationPlan,
    response: &ResponsePlan,
    success: bool,
    suppressed: bool,
) -> Vec<u16> {
    (100..600)
        .filter(|status| {
            (200..300).contains(status) == success
                && (operation.method() == Method::Head
                    || *status < 200
                    || matches!(status, 204 | 205 | 304))
                    == suppressed
        })
        .filter(|status| {
            operation
                .responses()
                .iter()
                .filter_map(|candidate| {
                    let rank = match candidate.status() {
                        ResponseStatus::Exact(value) if value == *status => 3,
                        ResponseStatus::Range(class) if u16::from(class) == status / 100 => 2,
                        ResponseStatus::Default => 1,
                        _ => return None,
                    };
                    Some((rank, candidate))
                })
                .max_by_key(|(rank, _)| *rank)
                .is_some_and(|(_, selected)| selected.source() == response.source())
        })
        .collect()
}
fn status_type(statuses: &[u16]) -> String {
    let mut terms = Vec::new();
    for class in 1..6 {
        let selected = statuses
            .iter()
            .filter(|status| **status / 100 == class)
            .copied()
            .collect::<BTreeSet<_>>();
        if selected.len() >= 80 {
            let omitted = (class * 100..(class + 1) * 100)
                .filter(|status| !selected.contains(status))
                .map(|status| status.to_string())
                .collect::<Vec<_>>();
            terms.push(if omitted.is_empty() {
                format!("StatusClass<{class}>")
            } else {
                format!("Exclude<StatusClass<{class}>, {}>", omitted.join(" | "))
            });
        } else {
            terms.extend(selected.iter().map(ToString::to_string));
        }
    }
    terms.join(" | ")
}
fn response_stem(op: &PlannedOperation, response: &ResponsePlan) -> String {
    format!(
        "{}Response{}",
        upper(&op.function_name),
        upper(response.status_key())
    )
}
pub(super) fn response_union(
    op: &PlannedOperation,
    names: &BTreeMap<SchemaId, String>,
    success: bool,
) -> String {
    let mut variants = Vec::new();
    for response in op.protocol().responses() {
        let stem = response_stem(op, response);
        let generic = if success {
            "ApiResponse"
        } else {
            "DeclaredApiError"
        };
        let extra = if response.headers().is_empty() && response.links().is_empty() {
            String::new()
        } else {
            format!(
                ", {}, {}",
                if response.headers().is_empty() {
                    "Record<string, never>".to_owned()
                } else {
                    format!("{stem}Headers")
                },
                if response.links().is_empty() {
                    "Record<string, never>".to_owned()
                } else {
                    format!("{stem}Links")
                }
            )
        };
        let suppressed = statuses(op.protocol(), response, success, true);
        let suppressed_types = if op.protocol().method() == Method::Head {
            (!suppressed.is_empty())
                .then(|| status_type(&suppressed))
                .into_iter()
                .collect::<Vec<_>>()
        } else {
            // A grouped `204 | 205` discriminant remains in TypeScript's object
            // union after sequential status exclusions. Emit each fixed bodyless
            // status separately; the uniform informational class stays compact.
            let informational = suppressed
                .iter()
                .copied()
                .filter(|status| *status < 200)
                .collect::<Vec<_>>();
            (!informational.is_empty())
                .then(|| status_type(&informational))
                .into_iter()
                .chain(
                    suppressed
                        .iter()
                        .filter(|status| **status >= 200)
                        .map(ToString::to_string),
                )
                .collect::<Vec<_>>()
        };
        for status in suppressed_types {
            variants.push(format!("{generic}<undefined, {status}, null{extra}>"));
        }
        let statuses = statuses(op.protocol(), response, success, false);
        if statuses.is_empty() {
            continue;
        }
        let status = status_type(&statuses);
        if response.media().is_empty() {
            variants.push(format!("{generic}<Uint8Array, {status}, null{extra}>"));
        }
        for media in response.media() {
            let wildcard = !matches!(media.media_type().range(), MediaRange::Concrete { .. });
            let extra = if wildcard {
                format!(
                    "{}, {}",
                    if extra.is_empty() {
                        ", Record<string, never>, Record<string, never>"
                    } else {
                        &extra
                    },
                    q(&canonical_media(media.media_type()))
                )
            } else {
                extra.clone()
            };
            variants.push(format!(
                "{generic}<{}, {status}, {}{extra}>",
                media_type(media, names, true),
                if wildcard {
                    "string".into()
                } else {
                    q(&canonical_media(media.media_type()))
                }
            ));
        }
    }
    if variants.is_empty() {
        "never".into()
    } else {
        variants.join(" | ")
    }
}
pub(super) fn credentials_type(requirement: &CredentialRequirement) -> &'static str {
    match requirement.credential() {
        CredentialHook::Basic => "Credential<BasicCredential>",
        CredentialHook::OAuth2 { .. } | CredentialHook::OpenIdConnect { .. } => {
            "Credential<AuthorizationCredential>"
        }
        _ => "Credential<string>",
    }
}

pub(super) fn client_requirements(
    ops: &[PlannedOperation],
) -> (BTreeMap<&str, BTreeSet<&'static str>>, BTreeSet<String>) {
    let mut credentials: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut requirements = BTreeSet::new();
    for op in ops {
        let alternatives = op.protocol().security().alternatives();
        for alternative in alternatives {
            for requirement in alternative.requirements() {
                credentials
                    .entry(requirement.name())
                    .or_default()
                    .insert(credentials_type(requirement));
            }
        }
        if !alternatives.is_empty() && !alternatives.iter().any(SecurityAlternative::is_anonymous) {
            requirements.insert(format!(
                "({})",
                alternatives
                    .iter()
                    .map(|alternative| format!(
                        "Pick<Credentials, {}>",
                        alternative
                            .requirements()
                            .iter()
                            .map(|requirement| q(requirement.name()))
                            .collect::<Vec<_>>()
                            .join(" | ")
                    ))
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
        }
    }
    (credentials, requirements)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit(
    ops: &[PlannedOperation],
    symbols: &HttpSymbols,
    config: &HttpConfig,
    credential_env: Option<&crate::credential_env::CredentialEnvPlan>,
    attribution: Option<&crate::attribution::AttributionDescriptor>,
    pagination: &[super::pagination_emit::PaginatedOperation],
    oauth: &[&OAuthSchemePlan],
    streams: &[super::stream_emit::StreamEventsOperation],
) -> String {
    let mut code = String::from(
        "// Generated from the verified HTTP protocol and native model plans.\nimport type * as Models from './models.js';\nimport * as Codecs from './model-codecs.js';\nimport { JsonNumber } from './json.js';\nimport { executeOperation, createRuntimeClient, isDeclaredApiError, freezeMetadata, type ClientOptions as RuntimeClientOptions, type CallOptions, type Credential, type BasicCredential, type AuthorizationCredential, type ApiResponse, type DeclaredApiError, type MediaBody, type Part, type LinkMetadata } from './runtime.js';\nexport { isSdkError } from './runtime.js';\nexport type { ClientOptions as RuntimeClientOptions, CallOptions, CredentialContext, CredentialProvider, Credential, BasicCredential, AuthorizationCredential, ApiResponse, DeclaredApiError, ReadonlyResponseData, ReadonlyBytes, SdkError, SdkFailureKind, Fetch, MediaBody, Part, BinaryPart, LinkMetadata } from './runtime.js';\n/** One actual HTTP status class; response selection excludes more-specific declarations. */\ntype StatusClass<C extends 1 | 2 | 3 | 4 | 5> = `${C}${Digit}${Digit}` extends `${infer S extends number}` ? S : never;\n/** A decimal status digit. */\ntype Digit = 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9;\n\n",
    );
    if !streams.is_empty() {
        // The typed events decode reads framed envelopes leniently and brands
        // its per-kind decode failures exactly like the runtime does.
        code.push_str("import { parseJson, stringifyJson } from './json.js';\nimport { responseFailure } from './http/common.js';\n");
    }
    if !pagination.is_empty() {
        code.push_str("import { paginationDescriptors, walkPages, walkItems, nextPageInput, type PageBody, type PageProperty, type PageItem } from './pagination.js';\n");
    }
    if !oauth.is_empty() {
        code.push_str(&super::oauth_emit::reexports(oauth));
    }
    if let Some(attribution) = attribution {
        writeln!(code, "/** ua/v1 attribution: every request identifies suspect as the generator and the SDK or a caller-supplied application as the client. */\nconst attribution = {} as const;", literal(&serde_json::to_value(attribution).unwrap())).unwrap();
    } else {
        code.push_str(
            "const attribution: import('./http/types.js').AttributionPlan | null = null;\n",
        );
    }
    let (credentials, requirements) = client_requirements(ops);
    if let Some(helper) = &symbols.credential_env_helper {
        writeln!(
            code,
            "import {{ withCredentialEnv as {helper} }} from './http/credential-env.js';"
        )
        .unwrap();
    }
    code.push_str("export type { ServerChoice, ServerPlan, CredentialRequirement, CredentialValue, OAuthFlow, Located, Provenance, ResourceContext, SourceLocation, Location } from './runtime.js';\nexport type { StatusClass, Digit };\n");
    code.push_str("/** Credentials keyed by the exact source scheme names. OAuth/OIDC uses a complete caller-chosen Authorization value. */\nexport interface Credentials {\n");
    for (name, types) in &credentials {
        writeln!(
            code,
            "  readonly {}: {};",
            q(name),
            types.iter().copied().collect::<Vec<_>>().join(" | ")
        )
        .unwrap();
    }
    code.push_str("}\n/** Complete source-declared alternatives required by the selected operations. */\nexport type ClientCredentials = Partial<Credentials>");
    for requirement in &requirements {
        write!(code, " & {requirement}").unwrap();
    }
    code.push_str(";\n");
    let scheme_names = if credentials.is_empty() {
        "never".into()
    } else {
        credentials
            .keys()
            .map(|name| q(name))
            .collect::<Vec<_>>()
            .join(" | ")
    };
    writeln!(code,"/** Reusable client policy with source-named credentials and explicit transport controls. */\nexport interface ClientOptions extends RuntimeClientOptions<{scheme_names}, ClientCredentials> {{ {} }}\n", if requirements.is_empty() || credential_env.is_some() { "" } else { "readonly auth: ClientCredentials;" }).unwrap();
    let limits = json!({"response":config.max_response_bytes,"request":config.max_request_bytes,"part":config.max_part_bytes,"streamItem":config.max_stream_item_bytes,"streamBuffer":config.max_stream_buffer_bytes,"streamItems":config.max_stream_items});
    for op in ops {
        let input = &op.input_type;
        let wire = op.protocol();
        code.push_str(&crate::typescript::declaration_comment(
            &format!("Input for {}.", op.operation_id),
            &src(&op.source),
            "Required members and codec bindings come from the source contract.",
        ));
        writeln!(code, "export interface {input} {{").unwrap();
        for (parameter, native) in wire.parameters().iter().zip(&op.parameters) {
            let (style, explode) = match parameter.serialization() {
                ParameterSerialization::Style { style, explode, .. } => (
                    serde_json::to_value(style)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_owned(),
                    *explode,
                ),
                ParameterSerialization::Content { .. } => ("content".into(), false),
            };
            let location = serde_json::to_value(parameter.location())
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned();
            code.push_str(&crate::typescript::declaration_comment(&format!("{location} parameter `{}` (style={style}, explode={explode}).",parameter.name()),&src(&native.source),"Source-bound codec validation precedes wire serialization; defaults are never injected."));
            writeln!(
                code,
                "  {}{}: {};",
                native.native_name,
                if parameter.required() { "" } else { "?" },
                model(parameter.codec(), &symbols.request)
            )
            .unwrap();
        }
        if let Some(body) = wire.body() {
            writeln!(code,"  /** Source request body. Media choices are explicit when required. */\n  body{}: {};",if body.required() { "" } else { "?" },body_type(body,&symbols.request)).unwrap();
        }
        code.push_str("}\n");
        for response in wire.responses() {
            let stem = response_stem(op, response);
            if !response.headers().is_empty() {
                writeln!(code,"/** Source-declared typed response headers, in addition to raw Headers. */\nexport type {stem}Headers = {};",header_type(response.headers(),&symbols.response)).unwrap();
            }
            if !response.links().is_empty() {
                writeln!(code,"/** Source Link metadata; reading a Link invokes no operation. */\nexport type {stem}Links = {{ {} }};",response.links().iter().map(|link|format!("readonly {}: LinkMetadata",q(link.name()))).collect::<Vec<_>>().join("; ")).unwrap();
            }
        }
        writeln!(code,"/** Matched successful statuses, classified from the actual HTTP status. */\nexport type {} = {};\n/** Branded declared non-success responses. Streaming data validates per item. */\nexport type {} = {};",op.success_type,response_union(op,&symbols.response,true),op.error_type,response_union(op,&symbols.response,false)).unwrap();
        writeln!(
            code,
            "const {}Source = {} as const;\nconst {}Wire = {} as const;",
            op.function_name,
            literal(&source_json(&op.source)),
            op.function_name,
            literal(&serde_json::to_value(wire).unwrap())
        )
        .unwrap();
        let names = op
            .parameters
            .iter()
            .map(|p| p.native_name.as_str())
            .collect::<Vec<_>>();
        let inputs = names
            .iter()
            .copied()
            .chain(wire.body().map(|_| "body"))
            .collect::<Vec<_>>();
        let binding = |map: &BTreeMap<SchemaId, String>| {
            map.iter()
                .map(|(schema, name)| format!("{}:Codecs.{name}Codec", q(&schema_key(schema))))
                .collect::<Vec<_>>()
                .join(",")
        };
        // Per-operation codec tables allow a named-operation-only bundle to shed
        // other operations and their codec closures.
        let roots = operation_roots(wire);
        let request = symbols
            .request
            .iter()
            .filter(|(schema, _)| roots.contains(*schema))
            .map(|(id, name)| (id.clone(), name.clone()))
            .collect();
        let response = symbols
            .response
            .iter()
            .filter(|(schema, _)| roots.contains(*schema))
            .map(|(id, name)| (id.clone(), name.clone()))
            .collect();
        let extras = object_extras(wire);
        writeln!(code,"const {}Descriptor = {{ operationId: {}, source: {}Source, wire: {}Wire, limits: {}, inputMembers: {}, parameterMembers: {}, requestCodecs: {{{}}}, responseCodecs: {{{}}}, objectExtras: {}, taggedBody: {}, attribution }} as const;",op.function_name,q(&op.operation_id),op.function_name,op.function_name,literal(&limits),serde_json::to_string(&inputs).unwrap(),serde_json::to_string(&names).unwrap(),binding(&request),binding(&response),serde_json::to_string(&extras).unwrap(),wire.body().is_some_and(tagged_body)).unwrap();
        if let Some(stream) = streams
            .iter()
            .find(|stream| stream.function_name == op.function_name)
        {
            // The typed events descriptor re-emits the full descriptor as one
            // plain literal whose every value is a literal, an identifier or a
            // namespace codec read: no spreads and no member reads, which
            // bundlers cannot prove side-effect free (a getter could hide
            // behind `descriptor.responseCodecs`). A pure-annotated call is
            // therefore not enough — the clone is only sheddable as a literal.
            // The replaced stream item codec is omitted from the re-emitted
            // table (not overridden), so the literal carries no duplicate key.
            let lenient_response = response
                .iter()
                .filter(|(schema, _)| schema_key(schema) != stream.item_codec_key)
                .map(|(id, name)| (id.clone(), name.clone()))
                .collect();
            let mut response_codecs = binding(&lenient_response);
            if !response_codecs.is_empty() {
                response_codecs.push(',');
            }
            response_codecs.push_str(&format!(
                "[{}]: {{ decode: (text: string) => text, encode: (value: unknown) => stringifyJson(value) }}",
                q(&stream.item_codec_key)
            ));
            writeln!(code,"{}const {}EventsDescriptor = {{ operationId: {}, source: {}Source, wire: {}Wire, limits: {}, inputMembers: {}, parameterMembers: {}, requestCodecs: {{{}}}, responseCodecs: {{{}}}, objectExtras: {}, taggedBody: {}, attribution }} as const;",crate::typescript::declaration_comment(&format!("The {} descriptor with its stream item codec replaced by a lenient frame reader: the existing stream item iteration, limits and cancellation are unchanged while the framed envelope arrives as raw text, so the typed decode below owns per-kind validation and undeclared event kinds stay representable. Every other codec and wire policy is identical. The clone re-emits the full descriptor as one plain literal, so a bundler sheds it — without retaining this operation's Source/Wire data — when the consumer references no typed events iterator.",op.function_name),&src(&op.source),"The direct operation function and its untyped stream iterator are unchanged."),op.function_name,q(&op.operation_id),op.function_name,op.function_name,literal(&limits),serde_json::to_string(&inputs).unwrap(),serde_json::to_string(&names).unwrap(),binding(&request),response_codecs,serde_json::to_string(&extras).unwrap(),wire.body().is_some_and(tagged_body)).unwrap();
        }
        writeln!(code,"/** Source description: {}\n * @param client Explicit credentials, transport, server choice and limits.\n * @param input Native operation input; omit when every member is optional.\n * @param call Per-call cancellation and server/security selection.\n * @returns The declared success union; streams expose native AsyncIterable items.\n * @throws A branded declared API error or source-linked SDK failure.\n * @remarks OpenAPI source: {}\n */\nexport function {}(client: ClientOptions{}, input: {input}{}, call?: CallOptions): Promise<{}> {{ return executeOperation({}Descriptor, input, client, call); }}\n/** Tests a branded error for this exact operation.\n * @param error Unknown caught value.\n * @returns Whether the value is a declared error of this operation.\n */\nexport function {}(error: unknown): error is {} {{ return isDeclaredApiError(error, {}Source); }}\n",crate::typescript::escape_prose(&op.description),crate::typescript::escape_prose(&src(&op.source)),op.function_name,if requirements.is_empty() && op.input_optional() {" = {}"} else {""},if op.input_optional() {" = {}"} else {""},op.success_type,op.function_name,op.error_guard,op.error_type,op.function_name).unwrap();
    }
    if !pagination.is_empty() {
        code.push_str("/** Compiled pagination: each descriptor records the source-selected pattern, the exact request members and response pointers, and the generic walker applies the documented stop rules. The first page of every walk is the direct call's result (supplying the descriptor's documented fallback page size when the caller omitted the limit control) and later pages rebuild only the pagination controls, keeping the limit the walk last used. */\n");
        for page in pagination {
            let item = format!("{}Item", upper(&page.function_name));
            code.push_str(&crate::typescript::declaration_comment(
                &format!("One item across every page of {}.", page.operation_id),
                &page.source,
                "Generated pagination: the walker reads the compiled descriptor's items pointer with plain property access and never searches schemas.",
            ));
            writeln!(code, "export type {item} = {};", page.item_type).unwrap();
            code.push_str(&crate::typescript::declaration_comment(
                &format!("Every page result of {}, starting with the direct call's result.", page.operation_id),
                &page.source,
                "Later pages rebuild only the pagination controls and preserve every other input member exactly. Early break never starts a not-yet-started page request and call cancellation propagates.",
            ));
            writeln!(
                code,
                "export function {}Pages(client: ClientOptions, input: {}, call?: CallOptions): AsyncIterable<{}> {{\n  return walkPages<{}, {}>(paginationDescriptors.{}, input, (page_input) => {}(client, page_input, call));\n}}\n",
                page.function_name, page.input_type, page.success_type, page.input_type, page.success_type, page.function_name, page.function_name,
            )
            .unwrap();
            code.push_str(&crate::typescript::declaration_comment(
                &format!("Every item across all pages of {}.", page.operation_id),
                &page.source,
                "Items are read from the compiled items pointer page by page. Early break never starts a not-yet-started page request and call cancellation propagates.",
            ));
            writeln!(
                code,
                "export function {}Items(client: ClientOptions, input: {}, call?: CallOptions): AsyncIterable<{}> {{\n  return walkItems<{}, {}, {}>(paginationDescriptors.{}, input, (page_input) => {}(client, page_input, call));\n}}\n",
                page.function_name, page.input_type, item, page.input_type, page.success_type, item, page.function_name, page.function_name,
            )
            .unwrap();
            code.push_str(&crate::typescript::declaration_comment(
                &format!("The input record for the page after {}, or null when the walk stops.", page.operation_id),
                &page.source,
                "Fetches the page described by input to compute the continuation, so callers can drive pages manually.",
            ));
            writeln!(
                code,
                "export function {}NextPage(client: ClientOptions, input: {}, call?: CallOptions): Promise<{} | null> {{\n  return nextPageInput<{}, {}>(paginationDescriptors.{}, (page_input) => {}(client, page_input, call), input);\n}}\n",
                page.function_name, page.input_type, page.input_type, page.input_type, page.success_type, page.function_name, page.function_name,
            )
            .unwrap();
        }
    }
    if !streams.is_empty() {
        code.push_str("/** Compiled typed stream events: each descriptor clone keeps the exact wire and transport policy while the stream item codec reads framed envelopes leniently, and the typed generator applies the compiled per-kind decode, sentinel and completion semantics. The direct operation function and its untyped stream iterator are unchanged. */\n");
        code.push_str(&super::stream_emit::emit(streams));
    }
    writeln!(code,"/** Source-backed metadata for server choices, credentials, responses and links. */\nexport const operationMetadata = /* @__PURE__ */ freezeMetadata({{ {} }});",ops.iter().map(|op|format!("{}: {}Wire",q(&op.function_name),op.function_name)).collect::<Vec<_>>().join(",")).unwrap();
    if let Some(policy) = credential_env {
        let bindings = policy
            .bindings()
            .iter()
            .map(|binding| json!({"name":binding.name(),"variable":binding.variable()}))
            .collect::<Vec<_>>();
        let helper = symbols
            .credential_env_helper
            .as_ref()
            .expect("allocated environment helper");
        writeln!(code,"/** Creates reusable bound operation methods, snapshotting mapped environment defaults only when auth is omitted.\n * @param options Explicit auth (including undefined/null/empty in JavaScript) is authoritative and is never supplemented. Other options control transport policy.\n * @returns A client with native operation methods; missing environment credentials fail at protected calls before HTTP.\n */\nexport function createClient(options: ClientOptions = {{}}) {{ const client = createRuntimeClient({helper}(options, {})); return {{",serde_json::to_string(&bindings).unwrap()).unwrap();
    } else {
        writeln!(code,"/** Creates reusable bound operation methods.\n * @param options Explicit credentials and transport policy.\n * @returns A client with native operation methods.\n */\nexport function createClient(options: ClientOptions{}) {{ const client = createRuntimeClient(options); return {{",if requirements.is_empty() {" = {}"} else {""}).unwrap();
    }
    for op in ops {
        writeln!(
            code,
            "  {}: (input: {}{}, call?: CallOptions) => {}(client, input, call),",
            op.function_name,
            op.input_type,
            if op.input_optional() { " = {}" } else { "" },
            op.function_name
        )
        .unwrap();
    }
    code.push_str("}; }\n");
    code
}

fn representation_roots(representation: &Representation, roots: &mut BTreeSet<SchemaId>) {
    match representation {
        Representation::Json { codec } | Representation::Text { codec, .. } => {
            if let Some(codec) = codec {
                roots.insert(codec.schema().id().clone());
            }
        }
        Representation::Binary { .. } => {}
        Representation::Stream { stream } => {
            if let Some(codec) = stream.item_codec() {
                roots.insert(codec.schema().id().clone());
            }
        }
        Representation::Form { form } => part_roots(form.fields(), form.additional(), roots),
        Representation::Multipart { multipart } => match multipart {
            MultipartPlan::Named {
                parts, additional, ..
            } => part_roots(parts, additional, roots),
            MultipartPlan::Positional { prefix, items, .. } => part_roots(prefix, items, roots),
        },
    }
}
fn part_roots(parts: &[PartPlan], additional: &AdditionalParts, roots: &mut BTreeSet<SchemaId>) {
    for part in parts.iter().chain(match additional {
        AdditionalParts::Allowed(part) => Some(part.as_ref()),
        _ => None,
    }) {
        match part.representation() {
            PartRepresentation::Json { codec, .. }
            | PartRepresentation::Text { codec, .. }
            | PartRepresentation::Style { codec, .. } => {
                roots.insert(codec.schema().id().clone());
            }
            PartRepresentation::Binary { .. } => {}
        }
        roots.extend(
            part.headers()
                .iter()
                .map(|header| header.codec().schema().id().clone()),
        );
    }
}
fn operation_roots(operation: &OperationPlan) -> BTreeSet<SchemaId> {
    let mut roots = operation
        .parameters()
        .iter()
        .map(|parameter| parameter.codec().schema().id().clone())
        .collect::<BTreeSet<_>>();
    for parameter in operation.parameters() {
        if let Some(media) = parameter.content_media() {
            representation_roots(media.representation(), &mut roots);
        }
    }
    if let Some(body) = operation.body() {
        for media in body.media() {
            representation_roots(media.representation(), &mut roots);
        }
    }
    for response in operation.responses() {
        roots.extend(
            response
                .headers()
                .iter()
                .map(|header| header.codec().schema().id().clone()),
        );
        if operation.method() == Method::Head
            || matches!(
                response.status(),
                ResponseStatus::Exact(100..=199 | 204 | 205 | 304)
            )
        {
            continue;
        }
        for media in response.media() {
            representation_roots(media.representation(), &mut roots);
        }
    }
    roots
}
fn object_extras(operation: &OperationPlan) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    let media = operation
        .body()
        .into_iter()
        .flat_map(BodyPlan::media)
        .chain(operation.responses().iter().flat_map(ResponsePlan::media));
    for media in media {
        let object = match media.representation() {
            Representation::Form { form } => Some((form.rules(), form.fields(), form.additional())),
            Representation::Multipart {
                multipart:
                    MultipartPlan::Named {
                        rules,
                        parts,
                        additional,
                    },
            } => Some((rules, parts.as_slice(), additional)),
            _ => None,
        };
        if let Some((rules, parts, AdditionalParts::Allowed(_))) = object {
            result.insert(schema_key(rules.schema().id()), extra_member(parts));
        }
    }
    result
}

pub(super) fn manifest(
    ops: &[PlannedOperation],
    symbols: &HttpSymbols,
    config: &HttpConfig,
    protocol: &ProtocolPlan,
    directional: bool,
    credential_env: Option<&crate::credential_env::CredentialEnvPlan>,
) -> Value {
    let bindings = |codec: Option<&CodecRef>, names: &BTreeMap<SchemaId, String>| {
        codec.and_then(|codec| names.get(codec.schema().id()).map(|name| (codec,name)))
            .map_or_else(|| json!({}), |(codec,name)| json!({"model":name,"codec":format!("{name}Codec"),"schemaSource":source_json(codec.schema().id())}))
    };
    let operations=ops.iter().map(|op| {
        let wire=op.protocol();
        let parameters=wire.parameters().iter().zip(&op.parameters).map(|(parameter,native)|{
            let (style,explode,reserved,array)=match parameter.serialization() {
                ParameterSerialization::Style {style,explode,shape,percent_encoding}=>(serde_json::to_value(style).unwrap(),*explode,*percent_encoding==PercentEncoding::ReservedExpansion,matches!(shape,WireShape::Array{..})),
                ParameterSerialization::Content {..}=>(json!("content"),false,false,false),
            };
            let mut value=json!({"name":parameter.name(),"member":native.native_name,"location":parameter.location(),"required":parameter.required(),"style":style,"explode":explode,"allowReserved":reserved,"array":array,"source":source_json(&native.source),"provenance":parameter.source(),"serialization":parameter.serialization()});
            value.as_object_mut().unwrap().extend(bindings(Some(parameter.codec()),&symbols.request).as_object().unwrap().clone());
            value
        }).collect::<Vec<_>>();
        let body=wire.body().map(|body|{
            let mut value=json!({"member":"body","required":body.required(),"source":source_json(body.source().use_site().source()),"provenance":body.source(),"nativeType":body_type(body,&symbols.request),"taggedMedia":tagged_body(body),"media":body.media()});
            if body.media().len()==1 {
                let codec=match body.media()[0].representation() {Representation::Json {codec}|Representation::Text {codec,..}=>codec.as_ref(),_=>None};
                value.as_object_mut().unwrap().extend(bindings(codec,&symbols.request).as_object().unwrap().clone());
            }
            value
        });
        let responses=wire.responses().iter().flat_map(|response| {
            let suppressed=wire.method()==Method::Head || matches!(response.status(),ResponseStatus::Exact(100..=199|204|205|304)|ResponseStatus::Range(1));
            let media:Vec<Option<&MediaPlan>>=if suppressed || response.media().is_empty(){vec![None]}else{response.media().iter().map(Some).collect()};
            media.into_iter().map(move |media|(response,media,suppressed))
        }).map(|(response,media,suppressed)|{
            let mut value=json!({"status":match response.status(){ResponseStatus::Exact(status)=>json!(status),_=>json!(response.status_key())},"statusKey":response.status_key(),"mediaType":media.map(|media|canonical_media(media.media_type())),"source":source_json(response.source().use_site().source()),"provenance":response.source(),"nativeType":if suppressed{"undefined".into()}else{media.map_or_else(||"Uint8Array".into(),|media|media_type(media,&symbols.response,true))},"headers":response.headers(),"links":response.links()});
            let codec=media.and_then(|media| match media.representation(){Representation::Json{codec}|Representation::Text{codec,..}=>codec.as_ref(),_=>None});
            value.as_object_mut().unwrap().extend(bindings(codec,&symbols.response).as_object().unwrap().clone());
            value
        }).collect::<Vec<_>>();
        json!({"operationId":op.operation_id,"sourceOperationId":wire.operation_id().map(Located::value),"export":op.function_name,"inputType":op.input_type,"inputOptional":op.input_optional(),"successType":op.success_type,"errorType":op.error_type,"errorGuard":op.error_guard,"descriptionText":op.description,"hasSourceDescription":!op.description.trim().is_empty(),"method":op.method,"path":op.path,"server":op.server,"servers":wire.servers(),"source":source_json(&op.source),"provenance":wire.source(),"security":{"schemeName":op.security_scheme_name,"source":source_json(&op.security_source),"useSource":source_json(&op.security_use_source),"definitionSource":source_json(&op.security_definition_source),"plan":wire.security()},"parameters":parameters,"body":body,"responses":responses})
    }).collect::<Vec<_>>();
    let mut manifest = json!({"format":"suspect-typescript-http-v1","releaseReady":false,"protocolPlanVersion":protocol.version(),"capabilities":protocol.capabilities(),"protocol":protocol,"maxResponseBytes":config.max_response_bytes,"maxRequestBytes":config.max_request_bytes,"maxPartBytes":config.max_part_bytes,"maxStreamItemBytes":config.max_stream_item_bytes,"maxStreamBufferBytes":config.max_stream_buffer_bytes,"maxStreamItems":config.max_stream_items,"directionPolicy":if directional{"oas31-required-applicability-v1"}else{"neutral-equivalent"},"operations":operations});
    if let Some(policy) = credential_env {
        manifest["credentialEnv"] = serde_json::to_value(policy).unwrap();
    }
    manifest
}

pub(super) fn credential_env_docs(policy: &crate::credential_env::CredentialEnvPlan) -> String {
    let bindings = policy
        .bindings()
        .iter()
        .map(|binding| {
            format!(
                "- Source scheme `{}` reads variable `{}` at client creation.\n",
                crate::typescript::escape_prose(binding.name()),
                crate::typescript::escape_prose(binding.variable())
            )
        })
        .collect::<String>();
    format!(
        "\n## Environment credentials (explicit v1 policy)\n\n{bindings}\n`createClient()` and `createClient({{}})` snapshot these variables only when the auth property is absent. An explicit auth argument is authoritative for the whole credentials object. JavaScript auth: undefined, null, an empty object, empty credential values or missing members never trigger environment fallback. Null/non-object auth retains the explicit constructor's TypeError; missing credentials fail with a secret-free request-validation error before a protected operation sends HTTP. Anonymous operations remain usable without environment access.\n\nImport and generation do not read environment values. Changes to the environment affect new clients, not existing ones. Browser hosts without process environment access keep credentials unavailable; explicit credentials and anonymous operations still work. Standalone operation functions use their explicit client options and do not read environment variables. Declared server selection and caller overrides keep their existing behavior.\n\n```ts\nconst client = createClient(); // source-bound defaults are snapshotted here\n```\n"
    )
}

pub(super) fn docs(
    ops: &[PlannedOperation],
    config: &HttpConfig,
    directional: bool,
    first: Option<&super::FirstRequest>,
    streams: &[super::stream_emit::StreamEventsOperation],
) -> String {
    let mut text = format!(
        "# TypeScript HTTP client\n\nThe `{}` profile implements exactly the admitted source operations in [http-manifest.json](http-manifest.json). Requests use native Fetch or an explicit Fetch-compatible transport. The package has no runtime dependencies.\n\n",
        config.capabilities().adapter()
    );
    if let Some(first) = first {
        write!(text,"## First request\n\nPass explicit client policy and native values. This recipe is also compiled as `examples/first-request.ts`.\n\n```ts\n{}```\n\n",first.documentation_source()).unwrap();
    }
    text.push_str("## Values and media\n\nSingle-media requests take `body` directly. Multiple-media requests use `{body: {contentType, data}}`. Wildcard bodies also name the source range, for example `{body: {mediaType: 'application/*', contentType: 'application/pdf', data: bytes}}`; runtime matching still selects the most-specific declaration and prevents bypassing its codec. Strings, booleans, bigint integers, and JsonNumber decimals keep their native model representations. Defaults are never injected.\n\nResponses select exact status before range before default, then media specificity and matching parameters. Success depends on the actual 200–299 status. `contentType` identifies concrete received media (canonical declaration media for concrete matches); `mediaType` is the literal matched declaration, useful for narrowing wildcard unions; `rawContentType` retains the entire field. Unspecified response content is bounded Uint8Array data. HEAD and body-forbidden statuses return undefined data. Required headers are validated in `typedHeaders`, alongside raw `headers`. `links` and `operationMetadata` are source-backed metadata and trigger no calls. Multiple Set-Cookie fields cannot be folded into a declared scalar; unavailable browser headers fail if required.\n\n## Credentials and servers\n\n`auth` uses exact source scheme names. Bearer and API keys accept strings; Basic accepts `{username, password}` with ASCII by default or explicit `encoding: 'utf-8' | 'latin1'`. OAuth2/OIDC accepts `{authorization: 'Scheme credential'}` or a caller callback returning that object. Callbacks receive source, scope/role and flow/discovery metadata plus AbortSignal. The SDK performs no discovery, acquisition, refresh or retries. Security alternatives are OR; each alternative's requirements are AND. The first complete source alternative is used unless `call.securityAlternative` selects one explicitly. An anonymous alternative sends no credentials.\n\nChoose declared servers with `server: {index}` or `{name}`, plus `variables` when desired. Defaults and enums are source-validated. Relative servers use the URL serving their source document; local-file descriptions need an explicit HTTP `documentURL` or `serverURL`. A per-call `server` overrides the client selection.\n\n## Parameters and finite parts\n\nParameters retain scalar, scalar-array and flat-object serialization, source styles, UTF-8 percent-encoding and allowReserved hazards. Headers are passed without URI encoding or invented quoting. Cookies use the declared form/cookie strategy. Codecs validate first; nested values, ambiguous delimiters, controls and required empty composites fail before transport. Optional empty form-query arrays retain the baseline omission convenience.\n\nForm and multipart inputs have native fields and per-part codecs. Repeated fields use arrays. Mixed binary aggregates are structurally checked, never converted to placeholder JSON nulls. Typed additional fields use an `additionalFields` bag (qualified if that name is already declared). Binary bodies and parts use finite in-memory Uint8Array values; filenames never cause filesystem access. Multipart metadata uses `{data, headers, contentType, filename}` when required by the part plan. Multiple part media choices require contentType; raw byte parts use the filename `blob` unless explicitly supplied. Framing collisions, invalid headers, missing fields, extras, cardinality and size failures are explicit. Positional multipart preserves prefix/item order and codecs.\n\n## Streaming and lifetime\n\nOAS 3.2 itemSchema drives native AsyncIterable response data. SSE yields the standard parsed field envelope with string data/id/event and integer retry. JSON-looking data and [DONE] stay ordinary strings. JSON Lines requires one JSON value per line and permits an unterminated final record. Request item sequences accept Iterable or AsyncIterable and are validated and buffered within finite limits before sending. No streamed multipart, nested transfer encoding, vendor sentinel or JSON-in-data convention is activated.\n\nReading is pull-driven. Consume the iterable, break/return its iterator, or abort the call. Abort, early return, EOF, codec failure and resource failure cancel/unlock readers and remove caller listeners. Returned-but-unused streams can be cancelled with the call's AbortSignal. Errors have operation-specific guards; non-API failures use isSdkError and the stable kind categories. Byte error bodies use detached snapshots.\n\n## Fetch platform boundaries\n\nNative Fetch forbids TRACE and some request headers. The adapter checks native Request representability; TRACE requires a capable explicit transport, and browsers cannot directly attach Cookie credentials. Redirects are errors and ambient credentials are omitted. Browser response-header visibility follows Fetch/CORS. These platform limits are covered by explicit refusals rather than silently dropping source-declared data.\n\n");
    text.push_str("## Whole queries, method tokens and multipart styles\n\nOAS 3.2 querystring parameters use complete JSON, text or form content without a parameter-name prefix. JSON/text are component-encoded once; form output has no second encoding pass. They cannot be combined with ordinary query parameters or query API keys. Custom HTTP methods retain exact case and spelling. Native Fetch normalization (for example get → GET) is refused; a capable explicit transport receives the source token.\n\nNamed multipart RFC6570 styles expand physical MIME fields: arrays can repeat names, objects can use property names, deepObject uses bracketed names, and joined styles use literal delimiters. URI encoding is not applied to part names/payloads. Part codecs and required headers are validated; conflicting names, ambiguous delimiters, inconsistent grouped metadata and framing injection fail. OAS 3.2 encodings apply per outer-array item.\n\n## Explicit profile boundaries\n\nExpansion of positional values or repeated composite items without recoverable physical item-group boundaries is refused (`http-typescript-multipart-style-grouping`). Legacy OAS 3.1+ string/binary markers require the explicit LegacyBinaryStringV1 profile. No complete OpenAPI protocol coverage is claimed by enabling this adapter.\n\n");
    writeln!(text,"## Resource policy\n\nGenerated ceilings: request {} bytes; response {} bytes; part {} bytes; stream item {} bytes; retained stream chunk {} bytes; {} items. Client limits may only reduce these. Multipart additionally bounds part count (10,000), header count (100 per part), and header bytes (64 KiB).\n",config.max_request_bytes,config.max_response_bytes,config.max_part_bytes,config.max_stream_item_bytes,config.max_stream_buffer_bytes,config.max_stream_items).unwrap();
    if directional {
        text.push_str("## Directional validation\n\nThe explicit `oas31-required-applicability-v1` model policy relaxes only proven directional requiredness. Supplied properties stay present and are validated; no read-only or write-only values are stripped. Unsupported applicability remains a source-linked planning failure.\n\n");
    }
    for op in ops {
        writeln!(text,"## `{}`\n\n`{} {}` — source `{}`.\n\nCall `client.{}({})`. Declared response keys: {}.\n\n{}\n",crate::typescript::escape_prose(&op.operation_id),op.method,crate::typescript::escape_prose(&op.path),crate::typescript::escape_prose(&src(&op.source)),op.function_name,if op.input_optional(){""}else{"input"},op.protocol().responses().iter().map(|response|response.status_key()).collect::<Vec<_>>().join(", "),crate::typescript::escape_prose(&op.description)).unwrap();
    }
    if !streams.is_empty() {
        text.push_str("## Typed stream events\n\nOperations whose source stream schema declares a discriminated SSE event set expose `<operation>Events(client, input, call?)`: an async generator over the same stream item iteration, yielding one typed event per frame. A declared event kind decodes the framed envelope through the operation's stream item codec into its declared model type; an event kind the source never declared surfaces through the typed `{ kind: 'unknown', event, data }` alternative without failing the stream; an invalid payload for a recognized kind remains a decoding error. When the source declares a terminal sentinel, the raw token completes the stream before any payload decoding, the last data frame before the sentinel or end of body is preserved as the completion's terminal `usage` instead of being yielded, and no further reads are issued. Per-item metadata (the event name as `kind`, plus `id`/`retry` when the item schema declares them) and the `{ reason, usage }` completion value — the generator's documented return value, read with a manual `next()` after the final event — are explicit. Early break or return cancels the response body and issues no further reads. The direct operation function and its untyped AsyncIterable stay exported and unchanged.\n\n");
        for stream in streams {
            writeln!(
                text,
                "- `{}`: declared kinds {}; terminal sentinel {}.\n",
                crate::typescript::escape_prose(&stream.function_name),
                stream
                    .events
                    .iter()
                    .map(|kind| format!("`{}`", crate::typescript::escape_prose(kind)))
                    .collect::<Vec<_>>()
                    .join(", "),
                match &stream.sentinel {
                    Some(token) => format!(
                        "`{}` (completes the stream before decoding)",
                        crate::typescript::escape_prose(token)
                    ),
                    None => "none (the end of the body completes the stream)".to_owned(),
                }
            )
            .unwrap();
        }
        text.push('\n');
    }
    text
}
