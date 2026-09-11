use super::{HttpPlan, NativeType, PackageConfig, PlannedGroup, PlannedOperation};
use crate::OutFile;
use crate::http_protocol as p;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use suspect_ir::contract::SourceId;
fn q(value: &str) -> String {
    super::native_examples::quote(value)
}

/// Root imports are explicit, including type-checker-visible re-exports. The
/// documentation inventory consumes this same list, rather than promising
/// helpers that happen to leak through an implementation module's imports.
pub(super) const ROOT_EXPORTS: &[(&str, &str, &str)] = &[
    (
        "_client",
        "Client",
        "Reusable synchronous client with keyword-only operation inputs. Context-manager exit closes SDK-owned transports; injected transports remain caller-owned.",
    ),
    (
        "_client",
        "AsyncClient",
        "Reusable asynchronous client. Await operations on the caller task; async context-manager exit closes SDK-owned transports.",
    ),
    (
        "_runtime",
        "SdkError",
        "Transport, representation, resource and unexpected/invalid-response failure. kind and source classify the failure; status, headers, capture, truncated and cause are explicit diagnostic attributes. Ordinary formatting omits their sensitive values.",
    ),
    (
        "_runtime",
        "ApiError",
        "Base exception for validated declared non-2xx responses. Concrete classes in operations carry typed data, status, headers and source.",
    ),
    (
        "codec_runtime",
        "CodecError",
        "Source-located native conversion, schema or finite-budget failure. Input validation may raise this before any request is sent.",
    ),
    (
        "json_runtime",
        "JsonError",
        "Classified exact-JSON syntax, representation or resource failure. kind, offset and path are explicit diagnostics.",
    ),
    (
        "validation",
        "ValidationError",
        "Located schema invalidity or incomplete low-level evaluation. Model codecs expose these failures as CodecError.",
    ),
    (
        "json_runtime",
        "JsonNumber",
        "Exact JSON number token, constructed from a string. Decimal tokens never pass through float. token retains the spelling; to_int performs a bounded exact integer conversion.",
    ),
    (
        "json_runtime",
        "JsonValue",
        "Exact JSON values: None, bool, int, str, JsonNumber, lists and string-keyed dictionaries. Model codecs enforce the source schema over these values.",
    ),
    (
        "models",
        "Unset",
        "Distinct type for optional absence. None is a JSON null value only where the source schema permits it.",
    ),
    (
        "models",
        "UNSET",
        "Public absent-value sentinel. Omitted optional inputs and model fields are not sent; source defaults are not applied.",
    ),
    (
        "_types",
        "BasicAuth",
        "Explicit username/password credentials and a caller-selected UTF-8 or Latin-1 charset.",
    ),
    (
        "_types",
        "Authorization",
        "Complete Authorization header supplied by the caller for OAuth/OIDC. No token type is inferred.",
    ),
    (
        "_types",
        "CredentialRequest",
        "Located scheme, operation, permissions and OAuth/OIDC metadata passed to a caller-owned credential hook. effective_server_url supplies the resolved HTTP base for the retained literal endpoint URLs.",
    ),
    (
        "_types",
        "OAuthFlow",
        "Source OAuth flow metadata. Describing a flow does not execute it.",
    ),
    (
        "_types",
        "Credential",
        "An explicit native credential or caller-owned credential provider.",
    ),
    (
        "_types",
        "CredentialProvider",
        "A sync or async caller callback supplying a credential. Async callbacks execute on the operation's caller task.",
    ),
    (
        "_types",
        "AuthValue",
        "Bearer/API-key string, BasicAuth, or explicit OAuth/OIDC Authorization value.",
    ),
    (
        "_types",
        "Part",
        "Finite typed multipart/form part, optional concrete Content-Type and filename. Generated part subclasses expose typed declared headers.",
    ),
    (
        "_types",
        "Link",
        "Response link metadata and source identity. Links trigger no expression evaluation or operation calls.",
    ),
    (
        "_streams",
        "SyncStream",
        "Synchronous, pull-driven source-validated item iterator. Use a context manager or close when stopping early.",
    ),
    (
        "_streams",
        "AsyncStream",
        "Asynchronous source-validated item iterator on the caller task. Use async with or aclose when stopping early.",
    ),
];

fn init() -> String {
    let mut code = String::from(
        "\"\"\"Native sync/async clients, source-bound models and public operation results.\"\"\"\n",
    );
    for (module, name, _) in ROOT_EXPORTS {
        code.push_str(&format!("from .{module} import {name} as {name}\n"));
    }
    code.push_str(
        "from . import models as models, model_codecs as codecs, operations as operations\n\n",
    );
    let exports: Vec<_> = ROOT_EXPORTS
        .iter()
        .map(|(_, name, _)| *name)
        .chain(["models", "codecs", "operations"])
        .collect();
    code.push_str(&format!(
        "__all__ = {}\n",
        serde_json::to_string(&exports).unwrap()
    ));
    code
}

pub(super) fn operation_exports(plan: &HttpPlan) -> Vec<&str> {
    plan.operations
        .iter()
        .flat_map(PlannedOperation::exports)
        .chain(plan.groups.iter().map(|group| group.name.as_str()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
pub(super) fn package(plan: &HttpPlan, package: &PackageConfig) -> Vec<OutFile> {
    let prefix = format!("python/src/{}/", package.import_name);
    let mut files: BTreeMap<_, _> = plan
        .codecs
        .render()
        .into_iter()
        .map(|file| {
            (
                format!(
                    "{prefix}{}",
                    file.path
                        .strip_prefix("python/")
                        .expect("Python artifact root")
                ),
                file.content,
            )
        })
        .collect();
    files.insert(format!("{prefix}__init__.py"), init());
    files.insert(format!("{prefix}py.typed"), String::new());
    files.insert(
        format!("{prefix}_runtime.py"),
        include_str!("runtime.py").into(),
    );
    for (name, content) in [
        ("_types.py", include_str!("types.py")),
        ("_wire.py", include_str!("wire.py")),
        ("_media.py", include_str!("media.py")),
        ("_auth.py", include_str!("auth.py")),
        ("_registry.py", include_str!("registry.py")),
        ("_parts.py", include_str!("parts.py")),
        ("_streams.py", include_str!("streams.py")),
        ("_urls.py", include_str!("urls.py")),
    ] {
        files.insert(format!("{prefix}{name}"), content.into());
    }
    files.insert(
        format!("{prefix}protocol-plan.json"),
        serde_json::to_string(&protocol_binding(plan, package)).unwrap(),
    );
    files.insert(format!("{prefix}_client.py"), client(plan));
    if let Some(policy) = plan.credential_env() {
        files.insert(
            format!("{prefix}_credential_env.py"),
            super::credential_env::runtime(policy),
        );
    }
    files.insert(format!("{prefix}operations.py"), operations(plan));
    files.insert("python/pyproject.toml".into(),format!("[build-system]\nrequires=[\"hatchling==1.29.0\"]\nbuild-backend=\"hatchling.build\"\n[project]\nname={}\nversion={}\nrequires-python=\">=3.11\"\ndependencies=[\"httpx==0.28.1\"]\nreadme=\"README.md\"\n[tool.hatch.build.targets.wheel]\npackages=[{}]\n",q(&package.name),q(&package.version),q(&format!("src/{}",package.import_name))));
    files.insert(
        format!("{prefix}http-manifest.json"),
        http_manifest(plan, package),
    );
    files.insert(
        format!("{prefix}examples.json"),
        crate::http_examples::manifest(&plan.examples),
    );
    for file in super::docs::artifacts(plan, package) {
        files.insert(file.path, file.content);
    }
    files
        .into_iter()
        .map(|(path, content)| OutFile { path, content })
        .collect()
}

fn http_manifest(plan: &HttpPlan, package: &PackageConfig) -> String {
    let mut metadata = protocol_binding(plan, package);
    metadata["format"] = json!("suspect-python-http-v2");
    metadata["releaseReady"] = json!(false);
    metadata["operations"] = metadata["bindings"].take();
    serde_json::to_string_pretty(&metadata).unwrap()
}

fn source(id: &SourceId) -> Value {
    json!({"document":id.document().as_str(),"pointer":id.pointer()})
}
fn status(value: p::ResponseStatus) -> Value {
    match value {
        p::ResponseStatus::Exact(value) => json!(value),
        p::ResponseStatus::Range(value) => json!(format!("{value}XX")),
        p::ResponseStatus::Default => json!("default"),
    }
}
fn group(ty: &NativeType) -> Option<&str> {
    if let NativeType::Group(name) = ty {
        Some(name)
    } else {
        None
    }
}
fn part_class(ty: &NativeType) -> Option<&str> {
    match ty {
        NativeType::Group(name) => Some(name),
        NativeType::List(inner) => part_class(inner),
        NativeType::Union(values) => values.iter().find_map(part_class),
        _ => None,
    }
}

pub(super) fn operation_binding(
    plan: &HttpPlan,
    package: &PackageConfig,
    op: &PlannedOperation,
) -> Value {
    let module = format!("{}.operations", package.import_name);
    json!({
        "operationId":op.operation_id,"method":op.snake_name,"httpMethod":op.wire.method().as_str(),"path":op.wire.path(),
        "source":source(&op.source),"provenance":op.wire.source(),"description":op.description(),
        "success":op.success_type,"asyncSuccess":op.async_success_type,"apiError":op.error_type,"asyncApiError":op.async_error_type,"resultModule":module,
        "servers":op.wire.servers(),"security":op.wire.security(),
        "parameters":op.parameters.iter().map(|p|json!({"member":p.name,"wire":p.wire.name(),"location":format!("{:?}",p.wire.location()),"required":p.wire.required(),"schema":source(p.schema()),"source":source(p.wire.source().use_site().source()),"model":plan.symbols[p.schema()],"serialization":p.wire.serialization(),"contentMedia":p.wire.content_media()})).collect::<Vec<_>>(),
        "body":op.body.as_ref().map(|body|json!({"source":source(&body.source),"member":"body","required":body.required,"model":body.schema().map(|id|&plan.symbols[id]),"schema":body.schema().map(source),"type":body.ty.render(&plan.symbols,true,false),"asyncType":body.ty.render(&plan.symbols,true,true),"contentTypeParameter":body.content_type_parameter,"media":body.media.iter().map(|m|json!({"source":source(m.wire.source().use_site().source()),"mediaType":m.wire.media_type().declared(),"type":m.ty.render(&plan.symbols,true,false),"group":group(&m.ty)})).collect::<Vec<_>>()})),
        "responses":op.responses.iter().map(|r|json!({"class":r.class_name,"asyncClass":r.async_class_name,"errorClass":r.error_class_name,"asyncErrorClass":r.async_error_class_name,"module":module,"__module__":module,"status":status(r.status()),"statusSelector":r.status(),"model":r.schema().map(|id|&plan.symbols[id]),"schema":r.schema().map(source),"source":source(r.wire.source().use_site().source()),"dataType":r.ty.render(&plan.symbols,true,false),"asyncDataType":r.ty.render(&plan.symbols,true,true),"headerGroup":r.header_group,"media":r.media.iter().map(|m|json!({"mediaType":m.wire.media_type().declared(),"group":group(&m.ty),"type":m.ty.render(&plan.symbols,true,false)})).collect::<Vec<_>>(),"headers":r.wire.headers(),"links":r.wire.links()})).collect::<Vec<_>>(),
    })
}

fn header_scalar(plan: &HttpPlan, codec: &p::CodecRef) -> String {
    let raw = plan
        .contract
        .schema(codec.schema().source().terminal().source())
        .map(|schema| crate::schema_view::raw(schema));
    raw.as_ref()
        .and_then(|raw| raw.get("type"))
        .and_then(|ty| {
            ty.as_str().or_else(|| {
                ty.as_array()
                    .and_then(|items| items.first())
                    .and_then(Value::as_str)
            })
        })
        .unwrap_or("string")
        .into()
}

fn group_binding(plan: &HttpPlan, group: &PlannedGroup) -> Value {
    json!({"name":group.name,"source":source(&group.source),"kind":group.kind,
        "fields":group.fields.iter().map(|field|json!({"member":field.name,"wire":field.wire_name,"required":field.required,"source":source(&field.source),"type":field.ty.render(&plan.symbols,true,false),"partClass":part_class(&field.ty),"part":field.part})).collect::<Vec<_>>(),
        "additional":group.additional.as_ref().map(|ty|ty.render(&plan.symbols,true,false)),"additionalPartClass":group.additional.as_ref().and_then(part_class),"additionalPart":group.additional_part,
        "headers":group.headers.iter().map(|header|json!({"member":header.name,"protocol":header.wire,"textScalar":header_scalar(plan,header.wire.codec())})).collect::<Vec<_>>(),"headerGroup":group.header_group,"requiredHeaders":group.required_headers,
    })
}

fn protocol_binding(plan: &HttpPlan, package: &PackageConfig) -> Value {
    let mut metadata = json!({"format":"suspect-python-protocol-v1","package":package.name,"import":package.import_name,"operationsModule":format!("{}.operations",package.import_name),
        "protocol":plan.protocol,"bindings":plan.operations.iter().map(|op|operation_binding(plan,package,op)).collect::<Vec<_>>(),
        "models":plan.symbols.iter().map(|(id,name)|json!({"source":source(id),"name":name})).collect::<Vec<_>>(),
        "groups":plan.groups.iter().map(|group|(group.name.clone(),group_binding(plan,group))).collect::<BTreeMap<_,_>>(),
        "limits":{"request":plan.config.max_request_bytes,"response":plan.config.max_response_bytes,"part":plan.config.max_part_bytes,"parts":plan.config.max_parts},
        "publicExports":{"root":ROOT_EXPORTS.iter().map(|(_,name,_)|*name).chain(["models","codecs","operations"]).collect::<Vec<_>>(),"operations":operation_exports(plan)},
    });
    let validation = plan.codecs.validation_program();
    if validation.version != suspect_schema::OwnedProgram::V1_VERSION {
        metadata["validation"] = json!({"version":validation.version,"profile":validation.profile,"scopedAnnotations":true});
    }
    if validation.version == suspect_schema::OwnedProgram::V3_VERSION {
        metadata["validation"]["dynamicResources"] = json!(true);
    }
    if let Some(policy) = plan.credential_env() {
        metadata["credentialEnv"] = json!(policy);
    }
    metadata
}

fn group_code(plan: &HttpPlan, group: &PlannedGroup) -> String {
    let name = &group.name;
    if let Some(ty) = &group.part_value {
        let ty = ty.render(&plan.symbols, false, false);
        let headers = group.header_group.as_ref().unwrap();
        return format!(
            "@dataclasses.dataclass(frozen=True, kw_only=True)\nclass {name}(Part[{ty}]):\n    {}\n    value: {ty}\n    headers: {headers}{}\n\n",
            q(&format!(
                "Source-bound part and declared headers. {}#{}",
                group.source.document(),
                group.source.pointer()
            )),
            if group.required_headers {
                " = dataclasses.field()"
            } else {
                " | None = None"
            }
        );
    }
    let mut code = format!(
        "@dataclasses.dataclass(kw_only=True{})\nclass {name}:\n    {}\n",
        if group.kind == "headers" {
            ", frozen=True"
        } else {
            ""
        },
        q(&format!(
            "Source-bound {} values. {}#{}",
            group.kind,
            group.source.document(),
            group.source.pointer()
        ))
    );
    for field in &group.fields {
        code.push_str(&format!(
            "    {}: {}{}\n",
            field.name,
            field.ty.render(&plan.symbols, false, false),
            if field.required {
                ""
            } else {
                " | Unset = UNSET"
            }
        ));
    }
    if let Some(extra) = &group.additional {
        let ty = extra.render(&plan.symbols, false, false);
        code.push_str(&format!("    _extra_fields: dict[str, {ty}] = dataclasses.field(default_factory=dict, init=False, repr=False)\n    def set_extra(self, key: str, value: {ty}) -> None:\n        if type(key) is not str or key in {}:\n            raise ValueError('extra key must be an undeclared wire name')\n        self._extra_fields[key] = value\n    @property\n    def extra_fields(self) -> Mapping[str, {ty}]:\n        return types.MappingProxyType(self._extra_fields)\n",serde_json::to_string(&group.fields.iter().map(|f|&f.wire_name).collect::<Vec<_>>()).unwrap()));
    }
    code.push('\n');
    code
}

fn operations(plan: &HttpPlan) -> String {
    let mut code = String::from(
        "\"\"\"Public operation results, declared API errors, headers and finite part bodies.\"\"\"\nfrom __future__ import annotations\nimport dataclasses\nimport types\nfrom collections.abc import Mapping, Iterable, AsyncIterable\nfrom typing import Literal, TypeAlias, Never\nfrom . import models\nfrom .models import UNSET, Unset\nfrom .json_runtime import JsonNumber, JsonValue\nfrom ._types import ApiError, Source, Part, Link\nfrom ._streams import SyncStream, AsyncStream\n\n",
    );
    for group in &plan.groups {
        code.push_str(&group_code(plan, group));
    }
    let mut emitted = BTreeSet::new();
    for op in &plan.operations {
        for asynchronous in [false, true] {
            let mut success = Vec::new();
            let mut failures = Vec::new();
            for response in &op.responses {
                let ty = response.ty.render(&plan.symbols, false, asynchronous);
                if response.succeeds() {
                    let name = if asynchronous {
                        &response.async_class_name
                    } else {
                        &response.class_name
                    };
                    success.push(name.as_str());
                    if emitted.insert(name.clone()) {
                        code.push_str(&format!("@dataclasses.dataclass(frozen=True, kw_only=True)\nclass {name}:\n    data: {ty}\n    headers: tuple[tuple[str, str], ...]\n"));
                        if let Some(status) = response.exact_status() {
                            code.push_str(&format!("    status: Literal[{status}] = dataclasses.field(default={status}, init=False)\n"));
                        } else {
                            code.push_str("    status: int\n");
                        }
                        if let Some(group) = &response.header_group {
                            code.push_str(&format!("    typed_headers: {group}\n"));
                        }
                        code.push_str("    content_type: str | None = None\n    links: tuple[Link, ...] = ()\n\n");
                    }
                }
                if response.fails() {
                    let name = if asynchronous {
                        &response.async_error_class_name
                    } else {
                        &response.error_class_name
                    };
                    failures.push(name.as_str());
                    if emitted.insert(name.clone()) {
                        code.push_str(&format!(
                            "class {name}(ApiError[{ty}]):\n    {}\n",
                            q(&format!(
                                "Declared HTTP {} failure; actual status is retained.",
                                response.wire.status_key()
                            ))
                        ));
                        if let Some(group) = &response.header_group {
                            code.push_str(&format!("    typed_headers: {group}\n    def __init__(self, *, status: int, data: {ty}, headers: tuple[tuple[str, str], ...], source: Source, typed_headers: {group}, content_type: str | None = None, links: tuple[Link, ...] = ()) -> None:\n        super().__init__(status=status, data=data, headers=headers, source=source, typed_headers=typed_headers, content_type=content_type, links=links)\n"));
                        }
                        code.push('\n');
                    }
                }
            }
            for (name, values) in [
                (
                    if asynchronous {
                        &op.async_success_type
                    } else {
                        &op.success_type
                    },
                    success,
                ),
                (
                    if asynchronous {
                        &op.async_error_type
                    } else {
                        &op.error_type
                    },
                    failures,
                ),
            ] {
                if emitted.insert(name.clone()) {
                    code.push_str(&format!(
                        "{name}: TypeAlias = {}\n",
                        if values.is_empty() {
                            "Never".into()
                        } else {
                            values.join(" | ")
                        }
                    ));
                }
            }
        }
        code.push('\n');
    }
    code.push_str(&format!(
        "__all__ = {}\n",
        serde_json::to_string(&operation_exports(plan)).unwrap()
    ));
    code
}

fn client(plan: &HttpPlan) -> String {
    let mut code = String::from(
        "\"\"\"Source-selected native HTTP clients over one admitted protocol plan.\"\"\"\nfrom __future__ import annotations\nfrom collections.abc import Iterable, AsyncIterable\nfrom typing import cast\nfrom . import models, operations\nfrom .models import UNSET, Unset\nfrom .json_runtime import JsonNumber, JsonValue\nfrom ._types import Part\nfrom ._runtime import SyncClient, AsyncClientBase\nfrom ._registry import operation as _operation\n",
    );
    if plan.credential_env().is_some() {
        code.push_str(super::credential_env::IMPORTS);
    }
    for name in operation_exports(plan) {
        code.push_str(&format!("from .operations import {name} as {name}\n"));
    }
    for asynchronous in [false, true] {
        code.push_str(if asynchronous {"\n\nclass AsyncClient(AsyncClientBase):\n    \"\"\"Context-managed async client. Streaming and credential hooks stay on the caller task.\"\"\"\n"}else{"\n\nclass Client(SyncClient):\n    \"\"\"Context-managed synchronous client with explicit credentials and transport ownership.\"\"\"\n"});
        if plan.credential_env().is_some() {
            code.push_str(&super::credential_env::constructor(asynchronous));
        }
        for (index, op) in plan.operations.iter().enumerate() {
            code.push_str(&method(plan, op, index, asynchronous));
        }
    }
    code
}

fn method(plan: &HttpPlan, op: &PlannedOperation, index: usize, asynchronous: bool) -> String {
    let mut args: Vec<_> = op
        .parameters
        .iter()
        .map(|p| {
            format!(
                "{}: models.{}{}",
                p.name,
                plan.symbols[p.schema()],
                if p.wire.required() {
                    ""
                } else {
                    " | Unset = UNSET"
                }
            )
        })
        .collect();
    if let Some(body) = &op.body {
        args.push(format!(
            "body: {}{}",
            body.ty.render(&plan.symbols, true, asynchronous),
            if body.required {
                ""
            } else {
                " | Unset = UNSET"
            }
        ));
        if body.content_type_parameter {
            args.push("content_type: str | Unset = UNSET".into());
        }
    }
    let signature = if args.is_empty() {
        "self".into()
    } else {
        format!("self, *, {}", args.join(", "))
    };
    let result = if asynchronous {
        &op.async_success_type
    } else {
        &op.success_type
    };
    let error = if asynchronous {
        &op.async_error_type
    } else {
        &op.error_type
    };
    let mut code = format!(
        "\n    {}def {}({signature}) -> operations.{result}:\n        {}\n",
        if asynchronous { "async " } else { "" },
        op.snake_name,
        q(&format!(
            "{}\n\nReturns operations.{result}. Declared API exceptions: operations.{error}.\nInput codec and SDK errors propagate separately. Close streaming data when stopping early.\nSource: {}#{}",
            op.description(),
            op.source.document(),
            op.source.pointer()
        ))
    );
    let arguments = format!(
        "{{{}}}",
        op.parameters
            .iter()
            .map(|p| format!("{}: {}", q(&p.name), p.name))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let call = format!(
        "{}self._call(_operation({index}), {arguments}{}{})",
        if asynchronous { "await " } else { "" },
        if op.body.is_some() { ", body" } else { "" },
        if op
            .body
            .as_ref()
            .is_some_and(|body| body.content_type_parameter)
        {
            ", content_type"
        } else {
            ""
        }
    );
    if op.responses.iter().any(|response| response.succeeds()) {
        code.push_str(&format!(
            "        return cast(operations.{result}, {call})\n"
        ));
    } else {
        code.push_str(&format!("        {call}\n        raise AssertionError('a declared error-only operation returned')\n"));
    }
    code
}
