use super::*;
use crate::rust_models::{Decl, Init, Key, RepresentationRole, Type};

pub(super) fn generic_example(op: &PlannedOperation, crate_name: &str) -> String {
    format!(
        "use {crate_name}::{{Client,http::Transport}};\nuse {crate_name}::operations::{} as operation;\n\npub async fn call_operation<T:Transport>(client:&Client<T>,input:operation::{})->std::result::Result<operation::{},operation::{}>{}{{\n    client.{}(input).await\n}}",
        op.module_name,
        op.input_type,
        op.success_type,
        op.error_type,
        if operation::streams(op) {
            " where T::Body:'static "
        } else {
            " "
        },
        op.function_name
    )
}

/// A first-request callsite that constructs the native operation input. Native
/// model constructors come from the existing typed model declarations and a
/// validated example, never by parsing or rewriting generated Rust snippets.
pub(super) fn example(plan: &HttpPlan, op: &PlannedOperation, crate_name: &str) -> String {
    let mut args = Vec::new();
    let mut values = Vec::new();
    let mut imports = format!("use {crate_name}::{{Client,Credentials}};\n");
    if op.default_function_name.is_none() {
        imports.push_str(&format!(
            "use {crate_name}::operations::{} as operation;\n",
            op.module_name
        ));
    }
    let security = op.wire.security().alternatives();
    let credentials = if security.is_empty() || security[0].is_anonymous() {
        "Credentials::new()".into()
    } else if security[0].requirements().len() == 1 {
        let requirement = &security[0].requirements()[0];
        let credential = &plan.credentials[requirement.scheme().use_site().source()];
        match requirement.credential() {
            wire::CredentialHook::Bearer { .. } => {
                args.push("token: String".into());
                format!("Credentials::{}(token)", credential.constructor)
            }
            wire::CredentialHook::Basic => {
                args.push("username: String, password: String".into());
                format!("Credentials::{}(username,password)", credential.constructor)
            }
            wire::CredentialHook::ApiKey { .. } => {
                args.push("api_key: String".into());
                format!("Credentials::{}(api_key)", credential.constructor)
            }
            _ => {
                args.push("authorization: String".into());
                format!("Credentials::{}(authorization)", credential.constructor)
            }
        }
    } else {
        args.push("credentials: Credentials".into());
        "credentials".into()
    };
    let examples = plan
        .examples
        .operations()
        .iter()
        .find(|e| e.source == op.source);
    for p in op.parameters.iter().filter(|p| p.wire.required()) {
        let value = examples
            .and_then(|e| {
                e.entries
                    .iter()
                    .find(|e| e.schema == *p.wire.codec().schema().id())
            })
            .and_then(|e| {
                native_value(
                    plan,
                    &(e.schema.clone(), RepresentationRole::Model),
                    &e.value,
                    crate_name,
                    0,
                )
            });
        values.push(value.unwrap_or_else(|| {
            let name = format!("parameter_{}", p.name);
            args.push(format!("{name}: {crate_name}::models::{}", p.model));
            name
        }));
    }
    if let Some(b) = op.body.as_ref().filter(|b| b.wire.required()) {
        let model = if b.media.len() == 1 {
            match &b.media[0].payload {
                Payload::Model { schema, .. } => Some(schema),
                _ => None,
            }
        } else {
            None
        };
        let mut value = model
            .and_then(|schema| {
                examples.and_then(|e| e.entries.iter().find(|e| &e.schema == schema))
            })
            .and_then(|e| {
                native_value(
                    plan,
                    &(e.schema.clone(), RepresentationRole::Model),
                    &e.value,
                    crate_name,
                    0,
                )
            });
        if value.is_none()
            && b.media.len() == 1
            && let Payload::Parts(aggregate) = &b.media[0].payload
        {
            value = aggregate_recipe(
                plan,
                op,
                aggregate,
                b.media[0].wire.representation(),
                crate_name,
                &mut args,
            );
        }
        values.push(value.unwrap_or_else(|| {
            let ty = if b.media.len() == 1 {
                external_type(&b.media[0].payload, crate_name)
            } else {
                format!("operation::{}", b.type_name)
            };
            args.push(format!("body: {ty}"));
            "body".into()
        }));
    }
    let content_type = op.body.as_ref().filter(|b| {
        b.wire.required()
            && b.media.len() == 1
            && !matches!(
                b.media[0].wire.media_type().range(),
                wire::MediaRange::Concrete { .. }
            )
    });
    if content_type.is_some() {
        args.push("content_type: String".into());
    }
    imports.push_str(&format!("\npub async fn first_request({})->std::result::Result<(),Box<dyn std::error::Error>> {{\n    let client=Client::with_reqwest({credentials})?;\n",args.join(", ")));
    if let Some(default) = &op.default_function_name {
        imports.push_str(&format!("    let response=client.{default}().await?;\n"));
    } else {
        imports.push_str(&format!(
            "    let input=operation::{}::new({}){};\n    let response=client.{}(input).await?;\n",
            op.input_type,
            values.join(", "),
            if content_type.is_some() {
                ".with_content_type(content_type)"
            } else {
                ""
            },
            op.function_name
        ));
    }
    imports.push_str("    let _=response;\n    Ok(())\n}");
    imports
}

fn external_type(payload: &Payload, crate_name: &str) -> String {
    match payload {
        Payload::Model { model, .. } => format!("{crate_name}::models::{model}"),
        Payload::Json => format!("{crate_name}::JsonValue"),
        Payload::Text => "String".into(),
        Payload::Bytes => "Vec<u8>".into(),
        Payload::NoContent => "()".into(),
        Payload::Stream { model, request, .. } => match (model, request) {
            (Some(model), true) => format!("Vec<{crate_name}::models::{model}>"),
            (Some(model), false) => {
                format!("{crate_name}::http::ItemStream<{crate_name}::models::{model}>")
            }
            // A schemaless stream surfaces untyped parsed envelope values.
            (None, true) => format!("Vec<{crate_name}::JsonValue>"),
            (None, false) => format!("{crate_name}::http::ItemStream<{crate_name}::JsonValue>"),
        },
        Payload::Parts(a) => format!("operation::{}", a.type_name),
    }
}

fn aggregate_recipe(
    plan: &HttpPlan,
    op: &PlannedOperation,
    a: &PlannedAggregate,
    representation: &wire::Representation,
    crate_name: &str,
    args: &mut Vec<String>,
) -> Option<String> {
    let rules = match representation {
        wire::Representation::Form { form } => Some(form.rules()),
        wire::Representation::Multipart {
            multipart: wire::MultipartPlan::Named { rules, .. },
        } => Some(rules),
        _ => None,
    };
    let required = a
        .parts
        .iter()
        .filter(|p| p.wire.required())
        .collect::<Vec<_>>();
    if let wire::Representation::Multipart {
        multipart: wire::MultipartPlan::Positional { min_items, .. },
    } = representation
        && min_items
            .as_ref()
            .is_some_and(|m| *m.value() > u64::try_from(required.len()).unwrap_or(u64::MAX))
    {
        return None;
    }
    if rules.is_some_and(|r| {
        r.required().iter().any(|name| {
            !a.parts
                .iter()
                .any(|p| p.wire.name() == Some(name.value().as_str()))
        }) || r
            .min_properties()
            .is_some_and(|m| *m.value() > u64::try_from(required.len()).unwrap_or(u64::MAX))
    }) {
        return None;
    }
    let examples = plan
        .examples
        .operations()
        .iter()
        .find(|e| e.source == op.source);
    let example = |schema: &SourceId| {
        examples
            .and_then(|op| {
                op.entries
                    .iter()
                    .find(|e| e.schema == *schema && !e.role.is_response())
            })
            .and_then(|e| {
                native_value(
                    plan,
                    &(schema.clone(), RepresentationRole::Model),
                    &e.value,
                    crate_name,
                    0,
                )
            })
    };
    let mut added = Vec::new();
    let mut values = Vec::new();
    for p in required {
        let data = match p.wire.representation() {
            wire::PartRepresentation::Json { codec, .. }
            | wire::PartRepresentation::Text { codec, .. }
            | wire::PartRepresentation::Style { codec, .. } => example(codec.schema().id())
                .unwrap_or_else(|| {
                    let name = format!("part_{}_data", p.name);
                    added.push(format!(
                        "{name}: {crate_name}::models::{}",
                        p.model.as_ref().expect("part model")
                    ));
                    name
                }),
            wire::PartRepresentation::Binary { .. } => {
                let name = format!("part_{}_bytes", p.name);
                added.push(format!("{name}: Vec<u8>"));
                name
            }
        };
        let value = if a.multipart {
            let mut value = if let Some(headers) = &p.headers_type {
                let fields = p
                    .headers
                    .iter()
                    .filter(|h| h.wire.required())
                    .map(|h| {
                        example(h.wire.codec().schema().id()).unwrap_or_else(|| {
                            let name = format!("part_{}_header_{}", p.name, h.name);
                            added.push(format!("{name}: {crate_name}::models::{}", h.model));
                            name
                        })
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    "{crate_name}::http::Part::with_headers({data},operation::{headers}::new({fields}))"
                )
            } else {
                format!("{crate_name}::http::Part::new({data})")
            };
            if p.wire
                .content_types()
                .first()
                .is_some_and(|m| !matches!(m.range(), wire::MediaRange::Concrete { .. }))
            {
                let name = format!("part_{}_content_type", p.name);
                added.push(format!("{name}: String"));
                value.push_str(&format!(".with_content_type({name})"));
            }
            value
        } else {
            data
        };
        if p.wire.multiplicity() == wire::PartMultiplicity::RepeatedArrayItems {
            let count = p.wire.min_items().map_or(1, |v| (*v.value()).max(1));
            if count > 8 || p.wire.max_items().is_some_and(|m| *m.value() < count) {
                return None;
            }
            values.push(format!("vec![{value};{count}]"));
        } else {
            values.push(value);
        }
    }
    args.extend(added);
    Some(format!(
        "operation::{}::new({})",
        a.type_name,
        values.join(",")
    ))
}

fn native_value(
    plan: &HttpPlan,
    key: &Key,
    value: &Value,
    crate_name: &str,
    depth: usize,
) -> Option<String> {
    if depth > 48 {
        return None;
    }
    let name = plan
        .codecs
        .models()
        .symbols()
        .iter()
        .find(|s| s.source() == &key.0 && s.role() == key.1)?
        .name();
    match plan.codecs.models().declarations.get(key)? {
        Decl::Alias(ty) => native_type(plan, ty, value, crate_name, depth + 1),
        Decl::Literals(values) => values
            .iter()
            .find(|v| v.value == *value)
            .map(|v| format!("{crate_name}::models::{name}::{}", v.name)),
        Decl::Enum(_) => None,
        Decl::Struct { fields, extras } => {
            let object = value.as_object()?;
            let mut args = Vec::new();
            let mut assignments = String::new();
            for field in fields {
                let value = object.get(&field.wire);
                if field.init.is_none() {
                    args.push(native_type(plan, &field.ty, value?, crate_name, depth + 1)?);
                } else if let Some(value) = value
                    && !matches!(field.init, Some(Init::Literal(..)))
                {
                    assignments.push_str(&format!(
                        " value.{}={};",
                        field.name,
                        native_type(plan, &field.ty, value, crate_name, depth + 1)?
                    ));
                }
            }
            if let Some((_, ty)) = extras {
                for (key, value) in object
                    .iter()
                    .filter(|(key, _)| !fields.iter().any(|f| &f.wire == *key))
                {
                    assignments.push_str(&format!(
                        " value.insert_extra({key:?}.into(),{})?;",
                        native_type(plan, ty, value, crate_name, depth + 1)?
                    ));
                }
            }
            let constructor = format!("{crate_name}::models::{name}::new({})", args.join(","));
            Some(if assignments.is_empty() {
                constructor
            } else {
                format!("{{let mut value={constructor};{assignments} value}}")
            })
        }
    }
}
fn native_type(
    plan: &HttpPlan,
    ty: &Type,
    value: &Value,
    crate_name: &str,
    depth: usize,
) -> Option<String> {
    if depth > 48 {
        return None;
    }
    Some(match ty {
        Type::Named(key) => native_value(plan, key, value, crate_name, depth + 1)?,
        Type::Primitive("std::string::String") => format!("{:?}.into()", value.as_str()?),
        Type::Primitive("bool") => value.as_bool()?.to_string(),
        Type::Primitive("crate::JsonInteger") => format!(
            "{:?}.parse::<{crate_name}::JsonInteger>()?",
            value.as_number()?.as_str()
        ),
        Type::Primitive("crate::JsonNumber") => format!(
            "{:?}.parse::<{crate_name}::JsonNumber>()?",
            value.as_number()?.as_str()
        ),
        Type::Primitive("i8" | "i16" | "i32" | "i64" | "i128") => value
            .as_number()?
            .as_str()
            .parse::<crate::rust_models::runtime::JsonInteger>()
            .ok()?
            .to_i128()?
            .to_string(),
        Type::Primitive("u8" | "u16" | "u32" | "u64" | "u128") => value
            .as_number()?
            .as_str()
            .parse::<crate::rust_models::runtime::JsonInteger>()
            .ok()?
            .to_u128()?
            .to_string(),
        Type::Primitive(_) => return None,
        Type::Nullable(inner) => {
            if value.is_null() {
                format!("{crate_name}::Nullable::Null")
            } else {
                format!(
                    "{crate_name}::Nullable::Value({})",
                    native_type(plan, inner, value, crate_name, depth + 1)?
                )
            }
        }
        Type::Optional(inner) => format!(
            "Some({})",
            native_type(plan, inner, value, crate_name, depth + 1)?
        ),
        Type::Presence(inner) => {
            if value.is_null() {
                format!("{crate_name}::Presence::Null")
            } else {
                format!(
                    "{crate_name}::Presence::Value({})",
                    native_type(plan, inner, value, crate_name, depth + 1)?
                )
            }
        }
        Type::Boxed(inner) => format!(
            "Box::new({})",
            native_type(plan, inner, value, crate_name, depth + 1)?
        ),
        Type::Vec(inner) => format!(
            "vec![{}]",
            value
                .as_array()?
                .iter()
                .map(|v| native_type(plan, inner, v, crate_name, depth + 1))
                .collect::<Option<Vec<_>>>()?
                .join(",")
        ),
        Type::Map(inner) => format!(
            "std::collections::BTreeMap::from([{}])",
            value
                .as_object()?
                .iter()
                .map(|(k, v)| native_type(plan, inner, v, crate_name, depth + 1)
                    .map(|v| format!("({k:?}.into(),{v})")))
                .collect::<Option<Vec<_>>>()?
                .join(",")
        ),
    })
}

pub(super) fn validated_examples(plan: &HttpPlan, crate_name: &str) -> String {
    let mut code = format!(
        "//! Executable source-bound codec examples.\nfn main()->std::result::Result<(),{crate_name}::codecs::CodecError>{{\n"
    );
    let mut count = 0;
    let bindings = crate::http_examples::bindings(&plan.examples);
    for (oi, op) in plan.operations.iter().enumerate() {
        let Some(binding) = bindings.get(&op.source) else {
            continue;
        };
        for (i, entry) in binding.operation.entries.iter().enumerate() {
            let Some(name) = plan.symbols.get(&entry.schema) else {
                continue;
            };
            let text = serde_json::to_string(&entry.value).expect("example JSON");
            code.push_str(&format!("    // {} example at {}\n    let value_{oi}_{i}={crate_name}::codecs::{name}Codec::decode({text:?})?;\n    let _={crate_name}::codecs::{name}Codec::decode(&{crate_name}::codecs::{name}Codec::encode(&value_{oi}_{i})?)?;\n",crate::http_examples::origin(&entry.origin),prose(entry.schema.pointer())));
            count += 1;
        }
        let parameters = op
            .parameters
            .iter()
            .map(|p| crate::http_examples::InputSlot {
                name: &p.name,
                required: p.wire.required(),
                container: p.wire.source().use_site().source().clone(),
            });
        let body = op
            .body
            .iter()
            .filter(|b| b.media.len() == 1 && matches!(b.media[0].payload, Payload::Model { .. }))
            .map(|b| crate::http_examples::InputSlot {
                name: "body",
                required: b.wire.required(),
                container: b.media[0].wire.source().use_site().source().clone(),
            });
        if op.body.as_ref().is_some_and(|b| {
            b.wire.required()
                && (b.media.len() != 1 || !matches!(b.media[0].payload, Payload::Model { .. }))
        }) {
            continue;
        }
        if let Some(bound) = binding.bind(parameters.chain(body)) {
            if bound.iter().any(|b| {
                !plan
                    .symbols
                    .contains_key(&binding.operation.entries[b.example_index].schema)
            }) {
                continue;
            }
            let mut args = Vec::new();
            let mut builders = String::new();
            for slot in bound {
                let value = format!("value_{oi}_{}", slot.example_index);
                if slot.required {
                    args.push(value);
                } else {
                    builders.push_str(&format!(".with_{}({value})", slot.name));
                }
            }
            code.push_str(&format!(
                "    let _input_{oi}={crate_name}::operations::{}::{}::new({}){builders};\n",
                op.module_name,
                op.input_type,
                args.join(",")
            ));
        }
    }
    code.push_str(&format!(
        "    println!(\"validated-examples {count}\");\n    Ok(())\n}}\n"
    ));
    code
}

pub(super) fn readme(plan: &HttpPlan, package: &PackageConfig, crate_name: &str) -> String {
    let mut docs = format!(
        "# {}\n\nSource-selected OpenAPI Rust SDK. This private package contains exactly the operations below. Rust 1.88 / edition 2024.\n\n## Install\n\n```toml\n[dependencies]\n{} = {{ path = \"./rust\", features = [\"reqwest-rustls\"] }}\ntokio = {{ version = \"1\", features = [\"macros\", \"rt-multi-thread\", \"time\"] }}\n```\n\nDefault features are empty: exact JSON, native models, validation and codecs have no required dependencies. `http` enables the generic async transport and pinned URL parser. `reqwest-rustls` adds the pinned reqwest/rustls adapter. `serde-json` optionally enables source-bound serde_json codec adapters.\n\n## First request\n\nRun the async call inside a caller-owned Tokio runtime. Supply credentials at runtime:\n\n",
        package.name, package.name
    );
    if let Some(op) = plan.operations.first() {
        docs.push_str(&format!(
            "```rust\n{}\n```\n\n",
            example(plan, op, crate_name)
        ));
    }
    docs.push_str("Required native inputs are constructor arguments. Optional input builders preserve omission; schema defaults are never inserted. No-input `_default` helpers omit every optional input. A sole-success enum dereferences to its `ApiResponse` and provides `into_response()` and `into_data()`, so consumers can read `.data` directly while existing status matches remain usable.\n\n## Execution contract\n\n`Client::with_transport` accepts a pull-based custom transport. `ClientOptions` selects a declared server index and variable overrides, or an explicit absolute server replacement. Relative servers resolve against the serving document URL; local descriptions require `document_url` or an explicit server. Source path prefixes are preserved. Absolute HTTP replacements are loopback-only; declared HTTP candidates retain their source origins.\n\nCredentials implement anonymous, disabled, OR alternatives and AND requirements. Generated scheme constructors are source-addressed, avoiding collisions between documents. Named runtime setters are explicit caller policy. The first fully supplied alternative is used unless `select_alternative(index)` selects one. Bearer, basic and API-key header/query/cookie attachment are implemented. OAuth/OIDC take a complete caller-supplied Authorization value or a `CredentialProvider` receiving located flow/scope/role metadata. The SDK performs no token acquisition, discovery or refresh. Conflicting attachments fail before transport.\n\nParameters use their declared simple/label/matrix/form/spaceDelimited/pipeDelimited/deepObject/cookie styles, JSON/text content and percent-encoding rules. Header values are unquoted; ambiguous composite delimiters are rejected. `allowReserved` callers pre-escape query hazards (`&`, `=`, `+`, `#`) and active delimiters. Optional empty style composites omit; required empty composites fail. Null never becomes a string or an absent required field.\n\nStatus selection is exact > class range > default, before media selection. Actual 2xx status determines success, including default responses. JSON/+json, UTF-8 text, native bytes, wildcards and media parameters have separate typed bindings. Missing or invalid Content-Type cannot select declared content. HEAD/1xx/204/304 drop bodies without decoding. Undeclared response content is bounded bytes. `ApiResponse` retains actual status, complete Content-Type, raw duplicate headers, typed declared headers and source-backed links. Required headers are enforced. Repeated scalar headers, including Set-Cookie, fail typed decoding; raw values remain in errors. Array/object list headers support explicit simple serialization. Links are immutable metadata and never trigger calls.\n\nForms and multipart use native aggregate structs and `Part<T, H>` values. File data is `Vec<u8>`, filename is metadata, and each text/JSON part has its own codec. Required fields, extras policy, property counts, array cardinalities and positional order are checked structurally. No mixed aggregate is converted to JSON with null placeholders. Multipart assembly checks part and body bounds before the final allocation, validates headers/disposition, and chooses a collision-free boundary. OAS 3.2 named per-item and positional encodings retain their source bindings.\n\nOAS 3.2 `itemSchema` supplies SSE or JSON-lines item codecs. Responses return `ItemStream<Model>`: use `while let Some(item) = response.data.next().await { let item = item?; }`. SSE exposes the parsed envelope (`data`, `event`, `id`, integer `retry`), combines multiline data and ignores comments/unknown fields/invalid retry fields. JSON inside data and `[DONE]` have no inferred special meaning. Finite request streams use `Vec<Model>` and the same item codecs. Stream parsing keeps one bounded transport chunk and one bounded item, honors pull backpressure, and enforces total byte/poll budgets. Dropping or closing a stream cancels its body; no task is detached.\n\nThe recommended adapter disables redirects, retries, proxy discovery, referer generation and automatic decompression. Configure pooling, TLS or timeouts with `ReqwestTransport::from_builder`. Wrap calls/stream pulls in the caller executor's timeout or cancellation selection.\n\nByte ceilings (request, response, part, item, chunk and headers) are finite; caller options may only lower generated limits. JSON parsing, schema evaluation and native conversion retain their independent finite budgets and exact number tokens. Error enums distinguish validated API errors from request/transport/resource/decoding failures. Debug and display omit credentials and captures. Inspect bounded `raw_capture` explicitly.\n\n## Operations\n\n");
    for op in &plan.operations {
        docs.push_str(&format!("### `{}`\n\n`{} {}`. Module `operations::{}`; method `{}`.\n\n{}\n\nSource: `{}#{}`. Input `{}`; success `{}`; errors `{}` / `{}`.\n\n",prose(&op.operation_id),prose(op.wire.method().as_str()),prose(op.wire.path()),op.module_name,op.function_name,prose(op.wire.description().map(|v|v.value().as_str()).unwrap_or("")),prose(op.source.document().as_str()),prose(op.source.pointer()),op.input_type,op.success_type,op.error_type,op.api_error_type));
        for r in &op.responses {
            for v in &r.variants {
                docs.push_str(&format!(
                    "- `{}` `{}` → `{}` (`{}`).\n",
                    r.wire.status_key(),
                    prose(
                        v.media_index
                            .map(|i| r.wire.media()[i].media_type().declared())
                            .unwrap_or("no declared media")
                    ),
                    v.name,
                    payload_type(&v.payload)
                ));
            }
        }
        docs.push('\n');
    }
    docs.push_str("## Physical document bases and logical metadata\n\nRelative server URLs use `Server::document_base`, the effective physical retrieval document selected by the shared plan. An absent entry-level server uses the entry document even when an operation is referenced from another document. An explicit empty server override uses its declaring document. `ClientOptions::document_url` overrides that physical base. Logical `$self`/`$id` values never become the API server base. Local-file descriptions require an explicit HTTP document URL or absolute server override.\n\nThe pinned native URL transport preserves encoded slashes and ordinary relative dot navigation. Encoded dot path segments (`%2e`, `%2e%2e` and mixed encoded/literal forms) are refused before URL parsing/joining because that transport would normalize them. Refusals retain physical source identity.\n\n`Operation::provenance` and other provenance descriptors expose `use_site_resource`, `terminal_resource`, and aligned `reference_resources`. Their `ResourceContext` values retain canonical/base URI names, aliases, resource boundaries and schema roots separately from physical `Source` fields. Server URLs expose `ApiUrlBase::ServerDocument`; OAuth/OIDC metadata exposes `EffectiveServer` without rewriting URLs or acquiring tokens/documents.\n\n");
    if plan.codecs.validation_version() == suspect_schema::OwnedProgram::V3_VERSION {
        docs.push_str("## Checked resource/dynamic validation\n\nThis package executes `suspect.validation.experimental.v3` / `oas31-jsonschema202012-resources-dynamic`. Each node enters its indexed resource, including nested entry without evaluating its parent. Dynamic references select the outermost actually entered matching binding; unentered candidates do not participate. Pointer/empty/static-anchor fallbacks remain static. Exact ordered resource context participates in cycle detection; returns and branch trials restore scope. Target annotations start fresh and evaluation failures remain distinct from invalid values.\n\n`validation::resources()` and `validation::node_scopes()` expose immutable indexed metadata. Dynamic values and context-dependent unions use exact JSON carriers, preserving null and numeric tokens until whole-root codec validation. Mutable encoding revalidates the actual root context. Source-backed examples use the same explicit v3 compiler; fallback annotations are not guessed.\n\n");
    }
    docs.push_str("## Source policy and verification\n\nThe HTTP source manifest records the exact capability list, byte policies, located annotations, typed protocol descriptors and native symbols. Unknown extensions remain uninterpreted. Legacy OAS 3.1+ `string/binary` byte markers require explicit `LegacyBinaryStringV1` generation configuration. Vendor stream conventions, automatic retries/paging, nested/streamed multipart and transfer encodings have no inferred behavior. Neutral Rust codecs still refuse active readOnly/writeOnly projections and unsupported native schema representations. Rust strings cannot represent lone UTF-16 surrogate code points.\n\n[Models and codecs](models.md), [source manifest](http-manifest.json), and [validated example provenance](examples.md) describe the same plan. Examples for actual codec roots are executed by the packaged sample; opaque byte schemas and mixed aggregate schemas are not JSON codec inputs.\n\n```sh\ncargo check --no-default-features\ncargo test --all-features\ncargo test --doc --all-features\ncargo doc --no-deps --all-features\ncargo run --example validated --features http\ncargo package --allow-dirty --no-verify\n```\n\nGeneration invokes no package manager and never publishes. Native installed-consumer, wire, negative-type, cancellation, ownership and Rust 1.88/current-toolchain gates determine acceptance for this selected package.\n");
    if let Some(policy) = plan.credential_env() {
        docs.push_str("\n## Runtime environment credentials\n\nWith `reqwest-rustls`, `Client::from_env() -> Result<Client<ReqwestTransport>, reqwest::Error>` snapshots the configured variables once at client creation. For a custom transport use `Client::with_transport_from_env(transport)`. `Credentials::from_env()` exposes the same snapshot for explicit alternative selection.\n\n");
        for binding in policy.bindings() {
            docs.push_str(&format!(
                "- Source scheme `{}` reads the variable named `{}`.\n",
                prose(binding.name()),
                prose(binding.variable())
            ));
        }
        docs.push_str("\nMissing, empty and non-Unicode/unavailable values stay absent. Protected calls use existing source-linked, secret-free errors before HTTP; anonymous calls can still run. Each later client reads a new snapshot. Existing `with_reqwest(credentials)` and `with_transport(transport, credentials)` use only their whole explicit argument, including empty credentials or missing members; they never fill from the environment. Rust's credential argument is non-nullable. The SDK uses the source server unless the caller explicitly overrides it. This policy reads no `.env` files and performs no token acquisition.\n");
    }
    docs
}
