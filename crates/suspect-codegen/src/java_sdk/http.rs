//! Native signatures over the immutable rich protocol plan.
use super::{
    PackageConfig,
    models::{JavaModelPlan, allocate, bounded, member, reserved_members},
    protocol::{self, JavaHeaders, JavaMedia, JavaValue, Names},
};
use crate::http_protocol as wire;
use std::collections::BTreeSet;
use suspect_ir::contract::{SchemaId, SourceId};

/// One positional argument of an input type's `builder(...)` factory.
#[derive(Debug, Clone)]
pub struct JavaInputArgument {
    /// Java parameter name.
    pub name: String,
    /// Source parameter or request body bound to the argument.
    pub source: SourceId,
    /// Native value carried by the argument.
    pub ty: JavaValue,
}
/// Static factory that constructs an operation input from its required values.
#[derive(Debug, Clone)]
pub struct JavaInputConstructor {
    /// Factory method name.
    pub name: String,
    /// Required parameters in source order, followed by a required body.
    pub arguments: Vec<JavaInputArgument>,
}
/// Generated client method pair, input type and status variants for one operation.
#[derive(Debug, Clone)]
pub struct JavaOperation {
    /// Terminal source operation.
    pub source: SourceId,
    /// Source `operationId`, or `"<METHOD> <path>"` when absent.
    pub operation_id: String,
    /// Synchronous client method.
    pub method_name: String,
    /// `CompletableFuture` client method (`<method>Async`).
    pub async_method_name: String,
    /// Generated immutable input class.
    pub input_type: String,
    /// Factory taking the input's required values.
    pub constructor: JavaInputConstructor,
    /// Success result type: the single success variant, a sealed interface over
    /// several, or `Never` when no response can succeed.
    pub success_type: String,
    /// Upper-case HTTP method.
    pub http_method: String,
    /// Source path template.
    pub path: String,
    /// Planned parameters in protocol order.
    pub parameters: Vec<JavaParameter>,
    /// Planned request body, when declared.
    pub body: Option<JavaBody>,
    /// Planned responses in protocol order.
    pub responses: Vec<JavaResponse>,
    /// Source description; empty when absent.
    pub description: String,
    /// Shared wire decisions for the operation.
    pub wire: wire::OperationPlan,
}
/// One operation parameter as an input-builder member.
#[derive(Debug, Clone)]
pub struct JavaParameter {
    /// Parameter use site.
    pub source: SourceId,
    /// Java member name, unique within the input type.
    pub native_name: String,
    /// Rendered Java type of the parameter codec schema.
    pub native_type: String,
    /// Parameter codec schema.
    pub schema: SchemaId,
    /// Whether the parameter is a `builder(...)` argument.
    pub required: bool,
    /// Shared wire decisions for the parameter.
    pub wire: wire::ParameterPlan,
}
/// Operation request body as the input's `body` member.
#[derive(Debug, Clone)]
pub struct JavaBody {
    /// Request body use site.
    pub source: SourceId,
    /// Java member name.
    pub native_name: String,
    /// Rendered Java type of `value`.
    pub native_type: String,
    /// Whether the body is a `builder(...)` argument.
    pub required: bool,
    /// Sealed choice type over several media; `None` for a single media.
    pub choice_type: Option<String>,
    /// Body value: the choice type, or the single media's value.
    pub value: JavaValue,
    /// Planned request media in protocol order.
    pub media: Vec<JavaMedia>,
    /// Shared wire decisions for the body.
    pub wire: wire::BodyPlan,
}
/// One declared response status as a native result or exception variant.
#[derive(Debug, Clone)]
pub struct JavaResponse {
    /// Response use site.
    pub source: SourceId,
    /// Generated status variant class.
    pub variant_name: String,
    /// Exception class thrown for this status, when it can fail; equals
    /// `variant_name` when the status can only fail.
    pub error_variant_name: Option<String>,
    /// Rendered Java type of `value`.
    pub native_type: String,
    /// Response data: [`JavaValue::NoContent`] when the body is forbidden, raw
    /// bytes when no media is declared, wrapped in
    /// [`JavaValue::ResponseBody`] when the body may be empty.
    pub value: JavaValue,
    /// Sealed choice type over several media; `None` otherwise.
    pub choice_type: Option<String>,
    /// Planned response media; empty when the body is forbidden.
    pub media: Vec<JavaMedia>,
    /// Typed response headers class, when headers are declared.
    pub headers: Option<JavaHeaders>,
    /// Shared wire decisions for the response.
    pub wire: wire::ResponsePlan,
}
impl JavaResponse {
    /// Whether this status can yield a success: 2xx, `2XX` or `default`.
    #[must_use]
    pub fn can_succeed(&self) -> bool {
        protocol::can_succeed(self.wire.status())
    }
    /// Whether this status can yield an error: anything but 2xx or `2XX`.
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
