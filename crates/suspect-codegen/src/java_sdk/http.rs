//! Native signatures over the immutable rich protocol plan.
use super::{
    PackageConfig,
    models::{JavaModelPlan, allocate, bounded, member, reserved_members},
    protocol::{self, JavaHeaders, JavaMedia, JavaValue, Names},
};
use crate::http_protocol as wire;
use std::collections::BTreeSet;
use suspect_ir::contract::{SchemaId, SourceId};

#[derive(Debug, Clone)]
pub struct JavaInputArgument {
    pub name: String,
    pub source: SourceId,
    pub ty: JavaValue,
}
#[derive(Debug, Clone)]
pub struct JavaInputConstructor {
    pub name: String,
    pub arguments: Vec<JavaInputArgument>,
}
#[derive(Debug, Clone)]
pub struct JavaOperation {
    pub source: SourceId,
    pub operation_id: String,
    pub method_name: String,
    pub async_method_name: String,
    pub input_type: String,
    pub constructor: JavaInputConstructor,
    pub success_type: String,
    pub http_method: String,
    pub path: String,
    pub parameters: Vec<JavaParameter>,
    pub body: Option<JavaBody>,
    pub responses: Vec<JavaResponse>,
    pub description: String,
    pub wire: wire::OperationPlan,
}
#[derive(Debug, Clone)]
pub struct JavaParameter {
    pub source: SourceId,
    pub native_name: String,
    pub native_type: String,
    pub schema: SchemaId,
    pub required: bool,
    pub wire: wire::ParameterPlan,
}
#[derive(Debug, Clone)]
pub struct JavaBody {
    pub source: SourceId,
    pub native_name: String,
    pub native_type: String,
    pub required: bool,
    pub choice_type: Option<String>,
    pub value: JavaValue,
    pub media: Vec<JavaMedia>,
    pub wire: wire::BodyPlan,
}
#[derive(Debug, Clone)]
pub struct JavaResponse {
    pub source: SourceId,
    pub variant_name: String,
    pub error_variant_name: Option<String>,
    pub native_type: String,
    pub value: JavaValue,
    pub choice_type: Option<String>,
    pub media: Vec<JavaMedia>,
    pub headers: Option<JavaHeaders>,
    pub wire: wire::ResponsePlan,
}
impl JavaResponse {
    #[must_use]
    pub fn can_succeed(&self) -> bool {
        protocol::can_succeed(self.wire.status())
    }
    #[must_use]
    pub fn can_fail(&self) -> bool {
        protocol::can_fail(self.wire.status())
    }
}

pub(crate) fn plan_operations(
    protocol: &wire::ProtocolPlan,
    models: &JavaModelPlan,
    config: &PackageConfig,
    credential_env: bool,
) -> Vec<JavaOperation> {
    let mut methods: BTreeSet<String> = [
        "close",
        "wait",
        "notify",
        "notifyAll",
        "getClass",
        "hashCode",
        "equals",
        "toString",
        "options",
        "operationMetadata",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    if credential_env {
        methods.extend(
            ["fromEnv", "_credentialEnvOptions", "_readCredentialEnv"]
                .into_iter()
                .map(str::to_owned),
        );
    }
    let mut names = Names::new(models, &config.api_name);
    protocol
        .operations()
        .iter()
        .enumerate()
        .map(|(op_index, op)| {
            let operation_id = op
                .operation_id()
                .map(|v| v.value().clone())
                .unwrap_or_else(|| format!("{} {}", op.method().as_str(), op.path()));
            let stem = bounded(&crate::rust_models::pascal(&operation_id));
            let base = member(&operation_id);
            let mut method_name = allocate(&base, &mut methods);
            while methods
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&format!("{method_name}Async")))
            {
                method_name = allocate(&base, &mut methods);
            }
            let async_method_name = format!("{method_name}Async");
            methods.insert(async_method_name.clone());
            let input_type = names.name(&format!("{stem}Input"));
            let mut members = reserved_members();
            members.extend(["body".into(), "requestOptions".into()]);
            let parameters: Vec<_> = op
                .parameters()
                .iter()
                .map(|p| JavaParameter {
                    source: p.source().use_site().source().clone(),
                    native_name: allocate(&member(p.name()), &mut members),
                    native_type: models.native_type(p.codec().schema().id()),
                    schema: p.codec().schema().id().clone(),
                    required: p.required(),
                    wire: p.clone(),
                })
                .collect();
            let body = op.body().map(|b| {
                let media = names.media(
                    &stem,
                    b.media(),
                    true,
                    &format!("/operations/{op_index}/body/media"),
                );
                let choice_type =
                    protocol::needs_choice(&media).then(|| names.name(&format!("{stem}Body")));
                let value = choice_type.as_ref().map_or_else(
                    || media[0].value.clone(),
                    |name| JavaValue::Named(name.clone()),
                );
                JavaBody {
                    source: b.source().use_site().source().clone(),
                    native_name: "body".into(),
                    native_type: value.native_type(models),
                    required: b.required(),
                    choice_type,
                    value,
                    media,
                    wire: b.clone(),
                }
            });
            let responses: Vec<_> = op
                .responses()
                .iter()
                .enumerate()
                .map(|(response_index, r)| {
                    let suffix = match r.status() {
                        wire::ResponseStatus::Exact(n) => n.to_string(),
                        wire::ResponseStatus::Range(n) => format!("{n}XX"),
                        wire::ResponseStatus::Default => "Default".into(),
                    };
                    let base = format!("{stem}Status{suffix}");
                    let variant_name = names.name(&base);
                    let error_variant_name = if protocol::can_fail(r.status()) {
                        Some(if protocol::can_succeed(r.status()) {
                            names.name(&format!("{base}Error"))
                        } else {
                            variant_name.clone()
                        })
                    } else {
                        None
                    };
                    let empty = protocol::always_empty(op.method(), r.status());
                    // A body-forbidden response retains its metadata without binding any
                    // phantom body model. Headers still carry their actual codecs.
                    let media = if empty {
                        vec![]
                    } else {
                        names.media(
                            &format!("{stem}Response{suffix}"),
                            r.media(),
                            false,
                            &format!("/operations/{op_index}/responses/{response_index}/media"),
                        )
                    };
                    let choice_type =
                        (!empty && !media.is_empty() && protocol::needs_choice(&media))
                            .then(|| names.name(&format!("{stem}Response{suffix}Body")));
                    let value = if empty {
                        JavaValue::NoContent
                    } else if media.is_empty() {
                        JavaValue::Bytes
                    } else {
                        choice_type.as_ref().map_or_else(
                            || media[0].value.clone(),
                            |name| JavaValue::Named(name.clone()),
                        )
                    };
                    let value = if !empty && protocol::may_empty(op.method(), r.status()) {
                        JavaValue::ResponseBody(Box::new(value))
                    } else {
                        value
                    };
                    let headers = names.headers(
                        &format!("{stem}Response{suffix}Headers"),
                        r.source().use_site().source().clone(),
                        r.headers(),
                        format!("/operations/{op_index}/responses/{response_index}/headers"),
                    );
                    JavaResponse {
                        source: r.source().use_site().source().clone(),
                        variant_name,
                        error_variant_name,
                        native_type: value.native_type(models),
                        value,
                        choice_type,
                        media,
                        headers,
                        wire: r.clone(),
                    }
                })
                .collect();
            let successes: Vec<_> = responses.iter().filter(|r| r.can_succeed()).collect();
            let success_type = match successes.as_slice() {
                [] => "Never".into(),
                [one] => one.variant_name.clone(),
                _ => names.name(&format!("{stem}Success")),
            };
            let mut arguments = parameters
                .iter()
                .filter(|p| p.required)
                .map(|p| JavaInputArgument {
                    name: p.native_name.clone(),
                    source: p.source.clone(),
                    ty: JavaValue::Model(p.schema.clone()),
                })
                .collect::<Vec<_>>();
            if let Some(b) = &body
                && b.required
            {
                arguments.push(JavaInputArgument {
                    name: b.native_name.clone(),
                    source: b.source.clone(),
                    ty: b.value.clone(),
                });
            }
            JavaOperation {
                source: op.source().terminal().source().clone(),
                operation_id,
                method_name,
                async_method_name,
                input_type,
                constructor: JavaInputConstructor {
                    name: "builder".into(),
                    arguments,
                },
                success_type,
                http_method: op.method().as_str().into(),
                path: op.path().into(),
                parameters,
                body,
                responses,
                description: op
                    .description()
                    .map(|v| v.value().clone())
                    .unwrap_or_default(),
                wire: op.clone(),
            }
        })
        .collect()
}

pub(crate) use super::http_emit::client;
