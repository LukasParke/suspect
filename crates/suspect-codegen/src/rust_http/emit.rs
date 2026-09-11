use super::*;
use crate::{OutFile, http_protocol as wire, rust_models::prose};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use suspect_ir::contract::SourceId;

mod bodies;
mod credential_env;
mod descriptors;
mod docs;
mod operation;

fn source_json(id: &SourceId) -> Value {
    json!({"document":id.document().as_str(),"pointer":id.pointer()})
}

pub(super) fn payload_type(payload: &Payload) -> String {
    match payload {
        Payload::Model { model, .. } => format!("crate::models::{model}"),
        Payload::Json => "crate::JsonValue".into(),
        Payload::Text => "std::string::String".into(),
        Payload::Bytes => "std::vec::Vec<u8>".into(),
        Payload::NoContent => "()".into(),
        Payload::Stream {
            model,
            request: true,
            ..
        } => format!("std::vec::Vec<crate::models::{model}>"),
        Payload::Stream {
            model,
            request: false,
            ..
        } => format!("crate::http::ItemStream<crate::models::{model}>"),
        Payload::Parts(aggregate) => aggregate.type_name.clone(),
    }
}
fn payload_descriptor(payload: &Payload) -> Value {
    match payload {
        Payload::Model { model, .. } => {
            json!({"kind":"model","model":model})
        }
        Payload::Stream { model, request, .. } => {
            json!({"kind":if *request {"finite-items"}else{"item-stream"},"model":model})
        }
        Payload::Parts(a) => {
            json!({"kind":if a.multipart {"multipart"}else{"form"},"type":a.type_name,"positional":a.positional,
            "parts":a.parts.iter().map(part_descriptor).collect::<Vec<_>>(),"additional":a.additional.as_deref().map(part_descriptor)})
        }
        _ => {
            json!({"kind":match payload {Payload::Json=>"json",Payload::Text=>"text",Payload::Bytes=>"bytes",_=>"no-content"}})
        }
    }
}
fn part_descriptor(p: &PlannedPart) -> Value {
    json!({"member":p.name,"wire":p.wire.name(),"model":p.model,"required":p.wire.required(),"multiplicity":p.wire.multiplicity(),"headersType":p.headers_type,"headers":p.headers.iter().map(header_descriptor).collect::<Vec<_>>()})
}
fn header_descriptor(h: &PlannedHeader) -> Value {
    json!({"member":h.name,"wire":h.wire.name(),"model":h.model,"required":h.wire.required()})
}

/// Public native API projection. Source addresses, spans, wire descriptions and
/// annotations remain in `wire()` / the manifest, not in source-API comparisons.
pub(super) fn interface(op: &PlannedOperation) -> Value {
    json!({
        "parameters":op.parameters.iter().map(|p|json!({"member":p.name,"wire":p.wire.name(),"model":p.model,"required":p.wire.required(),"location":p.wire.location()})).collect::<Vec<_>>(),
        "body":op.body.as_ref().map(|b|json!({"member":"body","required":b.wire.required(),"type":b.type_name,"model":if b.media.len()==1 {match &b.media[0].payload {Payload::Model{model,..}=>Some(model),_=>None}}else{None},"media":b.media.iter().map(|m|json!({"variant":m.variant,"mediaType":m.wire.media_type().declared(),"payload":payload_descriptor(&m.payload)})).collect::<Vec<_>>()})),
        "responses":op.responses.iter().flat_map(|r|r.variants.iter().map(|v|json!({"status":match r.wire.status(){wire::ResponseStatus::Exact(s)=>json!(s),_=>json!(r.wire.status_key())},"variant":v.name,"success":v.success,"error":v.error,"forbidden":v.forbidden,"mediaType":v.media_index.map(|i|r.wire.media()[i].media_type().declared()),"model":match &v.payload {Payload::Model{model,..}=>Some(model),_=>None},"payload":payload_descriptor(&v.payload),"headersType":r.headers_type,"headers":r.headers.iter().map(header_descriptor).collect::<Vec<_>>(),"linksType":"&'static [crate::http::Link]"}))).collect::<Vec<_>>(),
        "credential":{"constructors":op.credentials.iter().map(|c|json!({"name":c.constructor,"kind":match c.requirement.credential(){wire::CredentialHook::Bearer{..}=>"bearer",wire::CredentialHook::Basic=>"basic",wire::CredentialHook::ApiKey{..}=>"api-key",_=>"authorization"}})).collect::<Vec<_>>()},
        "constructor":{"input":op.input_type,"defaultFunction":op.default_function_name},
        "responseUnions":{"success":op.success_type,"apiError":op.api_error_type,"soleSuccess":op.responses.iter().flat_map(|r|&r.variants).filter(|v|v.success).count()==1},
    })
}

pub(super) fn package(plan: &HttpPlan, package: &PackageConfig, crate_name: &str) -> Vec<OutFile> {
    let mut files: BTreeMap<_, _> = plan
        .codecs
        .render_for_crate(crate_name)
        .into_iter()
        .map(|f| (f.path, f.content))
        .collect();
    let models = files.remove("rust/README.md").expect("codec guide");
    files.insert(
        "rust/models.md".into(),
        format!("See [the package guide](README.md) for native HTTP use.\n\n{models}"),
    );
    files.insert(
        "rust/Cargo.toml".into(),
        crate::rust_codecs::cargo_manifest(&package.name, &package.version, true),
    );
    for (name, text) in [
        ("http.rs", include_str!("runtime.rs")),
        (
            "http/descriptors.rs",
            include_str!("runtime/descriptors.rs"),
        ),
        ("http/errors.rs", include_str!("runtime/errors.rs")),
        ("http/media.rs", include_str!("runtime/media.rs")),
        ("http/parameters.rs", include_str!("runtime/parameters.rs")),
        ("http/security.rs", include_str!("runtime/security.rs")),
        ("http/servers.rs", include_str!("runtime/servers.rs")),
        ("http/parts.rs", include_str!("runtime/parts.rs")),
        ("http/stream.rs", include_str!("runtime/stream.rs")),
        ("reqwest_transport.rs", include_str!("reqwest.rs")),
    ] {
        files.insert(format!("rust/src/{name}"), text.into());
    }
    files.insert("rust/src/lib.rs".into(), root(plan, crate_name));
    if let Some(policy) = plan.credential_env() {
        files.insert(
            "rust/src/credential_env.rs".into(),
            credential_env::emit(policy, crate_name),
        );
    }
    files.insert(
        "rust/src/operations/mod.rs".into(),
        plan.operations
            .iter()
            .map(|op| {
                format!(
                    "/// {} {}. Source operation: {}.\npub mod {};\n",
                    prose(op.wire.method().as_str()),
                    prose(op.wire.path()),
                    prose(&op.operation_id),
                    op.module_name
                )
            })
            .collect(),
    );
    for op in &plan.operations {
        files.insert(
            format!("rust/src/operations/{}.rs", op.module_name),
            operation::emit(plan, op, crate_name),
        );
    }
    files.insert(
        "rust/README.md".into(),
        docs::readme(plan, package, crate_name),
    );
    files.insert(
        "rust/examples.json".into(),
        crate::http_examples::manifest(&plan.examples),
    );
    files.insert(
        "rust/examples.md".into(),
        crate::http_examples::markdown(
            &plan.examples,
            "cargo run --example validated --features http",
        ),
    );
    files.insert(
        "rust/examples/validated.rs".into(),
        docs::validated_examples(plan, crate_name),
    );
    let operations = plan
        .operations
        .iter()
        .map(|op| {
            let mut record = interface(op);
            for (value, parameter) in record["parameters"]
                .as_array_mut()
                .expect("parameter bindings")
                .iter_mut()
                .zip(&op.parameters)
            {
                value["name"] = json!(parameter.wire.name());
                value["source"] = source_json(parameter.wire.source().use_site().source());
                value["schemaSource"] = source_json(parameter.wire.codec().schema().id());
                value["serialization"] = json!(parameter.wire.serialization());
            }
            for (value, response) in record["responses"]
                .as_array_mut()
                .expect("response bindings")
                .iter_mut()
                .zip(
                    op.responses
                        .iter()
                        .flat_map(|response| response.variants.iter().map(move |_| response)),
                )
            {
                value["source"] = source_json(response.wire.source().use_site().source());
                value["links"] = json!(response.wire.links());
            }
            if let Some(body) = &op.body {
                for (value, media) in record["body"]["media"]
                    .as_array_mut()
                    .expect("body media bindings")
                    .iter_mut()
                    .zip(&body.media)
                {
                    value["source"] = source_json(media.wire.source().use_site().source());
                }
            }
            let record = record.as_object_mut().expect("native operation descriptor");
            for (key, value) in [
                ("operationId", json!(op.operation_id)),
                ("source", source_json(&op.source)),
                (
                    "descriptionText",
                    json!(
                        op.wire
                            .description()
                            .map(|d| d.value().as_str())
                            .unwrap_or("")
                    ),
                ),
                ("module", json!(op.module_name)),
                ("function", json!(op.function_name)),
                ("inputType", json!(op.input_type)),
                ("successType", json!(op.success_type)),
                ("errorType", json!(op.error_type)),
                ("apiErrorType", json!(op.api_error_type)),
                ("method", json!(op.wire.method().as_str())),
                ("path", json!(op.wire.path())),
                ("protocol", json!(op.wire)),
            ] {
                record.insert(key.into(), value);
            }
            Value::Object(record.clone())
        })
        .collect::<Vec<_>>();
    let mut manifest = json!({
        "format":"suspect-rust-http-v1","protocolPlanVersion":plan.protocol.version(),"releaseReady":false,"capabilities":plan.protocol.capabilities(),"diagnostics":plan.protocol.diagnostics(),
        "package":{"name":package.name,"version":package.version,"crate":crate_name},"features":{"default":[],"customTransport":"http","recommendedTransport":"reqwest-rustls"},
        "maxRequestBytes":plan.config.max_request_bytes,"maxResponseBytes":plan.config.max_response_bytes,"maxPartBytes":plan.config.max_part_bytes,"maxStreamItemBytes":plan.config.max_stream_item_bytes,"maxChunkBytes":plan.config.max_chunk_bytes,"maxHeaderBytes":plan.config.max_header_bytes,
        "operations":operations,"credentials":plan.credentials.iter().map(|(source,c)|json!({"source":source_json(source),"constructor":c.constructor,"requirement":c.requirement})).collect::<Vec<_>>(),
        "models":plan.codecs.models().symbols().iter().map(|s|json!({"name":s.name(),"codec":format!("{}Codec",s.name()),"source":source_json(s.source()),"role":format!("{:?}",s.role())})).collect::<Vec<_>>()
    });
    if let Some(policy) = plan.credential_env() {
        manifest["credentialEnv"] = json!(policy);
    }
    files.insert(
        "rust/http-manifest.json".into(),
        serde_json::to_string_pretty(&manifest).expect("native manifest"),
    );
    files
        .into_iter()
        .map(|(path, content)| OutFile { path, content })
        .collect()
}

fn root(plan: &HttpPlan, crate_name: &str) -> String {
    let mut code = String::from(
        "//! Exact OpenAPI models and codecs with source-selected native async HTTP operations.\n//!\n//! Enable `http` for a custom transport or `reqwest-rustls` for the recommended adapter.\n//! The default model-only package has no required dependencies.\n#![forbid(unsafe_code)]\npub mod models;\npub mod codecs;\npub mod validation;\nmod support;\nmod json;\npub use support::{ExtraFieldError,JsonInteger,JsonNonNullValue,JsonNumber,JsonValue,Never,Nullable,NumberError,Presence};\npub use json::{JsonError,JsonErrorKind,JsonLimits,parse_json,parse_json_bytes,stringify_json};\n#[cfg(feature=\"http\")]\npub mod http;\n#[cfg(feature=\"http\")]\npub mod operations;\n#[cfg(feature=\"http\")]\npub use http::{ApiResponse,Client,ClientOptions,Credentials};\n#[cfg(feature=\"reqwest-rustls\")]\npub mod reqwest_transport;\n\n#[cfg(feature=\"http\")]\nimpl Credentials {\n",
    );
    for (source, credential) in &plan.credentials {
        let name = &credential.constructor;
        let source = descriptors::source(source);
        let (args, attachment): (String, String) = match credential.requirement.credential() {
            wire::CredentialHook::Bearer { .. } => (
                "token: impl Into<String>".into(),
                format!("with_source_bearer({source},token)"),
            ),
            wire::CredentialHook::Basic => (
                "username: impl Into<String>, password: impl Into<String>".into(),
                format!("with_source_basic({source},username,password)"),
            ),
            wire::CredentialHook::ApiKey { .. } => (
                "value: impl Into<String>".into(),
                format!("with_source_api_key({source},value)"),
            ),
            wire::CredentialHook::OAuth2 { .. } | wire::CredentialHook::OpenIdConnect { .. } => (
                "authorization: impl Into<String>".into(),
                format!("with_source_authorization({source},authorization)"),
            ),
        };
        code.push_str(&format!("    /// Supply the source scheme {}. Values are caller-owned.\n    #[must_use]\n    pub fn {name}({args}) -> Self {{ Self::new().{attachment} }}\n",prose(credential.requirement.name())));
    }
    code.push_str("}\n#[cfg(feature=\"http\")]\nimpl<T:http::Transport> Client<T> {\n");
    for op in &plan.operations {
        let bound = if operation::streams(op) {
            " where T::Body: 'static"
        } else {
            ""
        };
        code.push_str(&format!("    /// {} {}. See [`operations::{}::{}`].\n    pub async fn {}(&self,input:operations::{}::{}) -> std::result::Result<operations::{}::{},operations::{}::{}>{bound} {{ operations::{}::{}(self,input).await }}\n",prose(op.wire.method().as_str()),prose(op.wire.path()),op.module_name,op.function_name,op.function_name,op.module_name,op.input_type,op.module_name,op.success_type,op.module_name,op.error_type,op.module_name,op.function_name));
        if let Some(name) = &op.default_function_name {
            code.push_str(&format!("    /// Call with every optional input absent.\n    pub async fn {name}(&self) -> std::result::Result<operations::{}::{},operations::{}::{}>{bound} {{ self.{}(operations::{}::{}::new()).await }}\n",op.module_name,op.success_type,op.module_name,op.error_type,op.function_name,op.module_name,op.input_type));
        }
    }
    code.push_str("}\n#[cfg(feature=\"reqwest-rustls\")]\nimpl Client<reqwest_transport::ReqwestTransport> {\n    /// Construct a caller-owned no-redirect, no-retry reqwest/rustls client.\n    pub fn with_reqwest(credentials:Credentials)->std::result::Result<Self,reqwest::Error>{Ok(Self::with_transport(reqwest_transport::ReqwestTransport::new()?,credentials))}\n}\n");
    if plan.credential_env().is_some() {
        code.push_str("#[cfg(feature=\"http\")]\nmod credential_env;\n");
    }
    if let Some(op) = plan.operations.first() {
        let snippet = format!(
            "# First request\n\n```no_run\n{}\n```",
            docs::example(plan, op, crate_name)
        );
        code.push_str(&format!(
            "#[cfg(feature=\"reqwest-rustls\")]\n#[doc={snippet:?}]\npub mod guide {{}}\n"
        ));
    }
    code
}
