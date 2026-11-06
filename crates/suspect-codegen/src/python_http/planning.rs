//! Python names and native carriers over the one admitted protocol plan.
use std::collections::{BTreeMap, BTreeSet};

use super::{allocate, python_name};
use crate::http_protocol as p;
use suspect_ir::contract::{SchemaId, SourceId};

/// Python type of one native value, rendered against the model symbol table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeType {
    /// A generated model class, `models.<Name>`.
    Model(SchemaId),
    /// A builtin or runtime type such as `bytes`, `str`, `None` or `JsonValue`.
    Builtin(&'static str),
    /// A generated form, multipart, part or headers class in `operations`.
    Group(String),
    /// `list[T]`, for repeated parts.
    List(Box<Self>),
    /// `Part[T]`, a part value without declared headers.
    Part(Box<Self>),
    /// A deduplicated `A | B` union.
    Union(Vec<Self>),
    /// A response stream: `SyncStream[T]` / `AsyncStream[T]` of the declared
    /// item schema, or of `JsonValue` when `None`.
    Stream(Option<SchemaId>),
    /// A request stream: `Iterable[T]` (plus `AsyncIterable[T]` for the async
    /// client) of the declared item schema, or of `JsonValue` when `None`.
    Items(Option<SchemaId>),
}

impl NativeType {
    pub(crate) fn render(
        &self,
        names: &BTreeMap<SchemaId, String>,
        qualified: bool,
        asynchronous: bool,
    ) -> String {
        match self {
            Self::Model(source) => format!("models.{}", names[source]),
            Self::Builtin(name) => (*name).into(),
            Self::Group(name) => format!("{}{name}", if qualified { "operations." } else { "" }),
            Self::List(inner) => format!("list[{}]", inner.render(names, qualified, asynchronous)),
            Self::Part(inner) => format!("Part[{}]", inner.render(names, qualified, asynchronous)),
            Self::Union(values) => values
                .iter()
                .map(|value| value.render(names, qualified, asynchronous))
                .collect::<Vec<_>>()
                .join(" | "),
            Self::Stream(source) => match source {
                Some(source) => format!(
                    "{}Stream[models.{}]",
                    if asynchronous { "Async" } else { "Sync" },
                    names[source]
                ),
                // A schemaless stream surfaces untyped parsed envelope values.
                None => format!(
                    "{}Stream[JsonValue]",
                    if asynchronous { "Async" } else { "Sync" }
                ),
            },
            Self::Items(source) => match source {
                Some(source) => {
                    if asynchronous {
                        format!(
                            "Iterable[models.{0}] | AsyncIterable[models.{0}]",
                            names[source]
                        )
                    } else {
                        format!("Iterable[models.{}]", names[source])
                    }
                }
                None => {
                    if asynchronous {
                        "Iterable[JsonValue] | AsyncIterable[JsonValue]".into()
                    } else {
                        "Iterable[JsonValue]".into()
                    }
                }
            },
        }
    }
    /// The model schema, when this is exactly one [`Self::Model`].
    pub fn schema(&self) -> Option<&SchemaId> {
        if let Self::Model(source) = self {
            Some(source)
        } else {
            None
        }
    }
    /// Whether this is, or is a union containing, a response stream.
    pub(crate) fn streaming(&self) -> bool {
        match self {
            Self::Stream(_) => true,
            Self::Union(values) => values.iter().any(Self::streaming),
            _ => false,
        }
    }
    pub(crate) fn union(values: impl IntoIterator<Item = Self>) -> Self {
        let mut result = Vec::new();
        for value in values {
            if !result.contains(&value) {
                result.push(value);
            }
        }
        if result.len() == 1 {
            result.remove(0)
        } else {
            Self::Union(result)
        }
    }
}

/// One source parameter's allocated keyword argument.
#[derive(Debug, Clone)]
pub struct PlannedParameter {
    /// Allocated snake-case Python argument name.
    pub name: String,
    pub(crate) wire: p::ParameterPlan,
}
impl PlannedParameter {
    /// Codec schema of the parameter value.
    pub fn schema(&self) -> &SchemaId {
        self.wire.codec().schema().id()
    }
    /// Shared wire plan of the source parameter.
    pub fn protocol(&self) -> &p::ParameterPlan {
        &self.wire
    }
    pub(crate) fn wire(&self) -> &p::ParameterPlan {
        &self.wire
    }
}

/// One declared response or part header's allocated attribute.
#[derive(Debug, Clone)]
pub struct PlannedHeader {
    /// Allocated snake-case Python attribute name.
    pub name: String,
    pub(crate) wire: p::HeaderPlan,
}
impl PlannedHeader {
    /// Codec schema of the header value.
    pub fn schema(&self) -> &SchemaId {
        self.wire.codec().schema().id()
    }
    /// Shared wire plan of the source header.
    pub fn protocol(&self) -> &p::HeaderPlan {
        &self.wire
    }
}

/// One attribute of a generated form, multipart or headers class.
#[derive(Debug, Clone)]
pub struct PlannedField {
    /// Allocated snake-case Python attribute name.
    pub name: String,
    /// Exact field, part or header name on the wire.
    pub wire_name: String,
    /// Use-site source of the field declaration.
    pub source: SourceId,
    /// Python type of the attribute.
    pub ty: NativeType,
    /// Whether the source declares the field required.
    pub required: bool,
    pub(crate) part: Option<p::PartPlan>,
}

/// One generated class in `operations` for a form, multipart body, part with
/// headers, or header set.
#[derive(Debug, Clone)]
pub struct PlannedGroup {
    /// Allocated class name.
    pub name: String,
    /// Source the class is generated from.
    pub source: SourceId,
    /// `"form"`, `"multipart"`, `"part"` or `"headers"`.
    pub kind: &'static str,
    /// Declared attributes in source order; empty for `"part"`.
    pub fields: Vec<PlannedField>,
    /// Type of admitted additional fields or parts, when allowed.
    pub additional: Option<NativeType>,
    pub(crate) additional_part: Option<p::PartPlan>,
    /// Allocated headers of a `"headers"` group; empty otherwise.
    pub headers: Vec<PlannedHeader>,
    pub(crate) part_value: Option<NativeType>,
    pub(crate) header_group: Option<String>,
    pub(crate) required_headers: bool,
}

/// One declared media with its Python type.
#[derive(Debug, Clone)]
pub struct PlannedMedia {
    /// Python type of a value in this media.
    pub ty: NativeType,
    pub(crate) wire: p::MediaPlan,
}
impl PlannedMedia {
    /// Shared wire plan of the source media.
    pub fn protocol(&self) -> &p::MediaPlan {
        &self.wire
    }
}

/// One request body's `body` argument.
#[derive(Debug, Clone)]
pub struct PlannedBody {
    /// Use-site source of the request body declaration.
    pub source: SourceId,
    /// Whether the source declares the body required.
    pub required: bool,
    /// Declared media in source order.
    pub media: Vec<PlannedMedia>,
    /// Union of the media types.
    pub ty: NativeType,
    /// Whether the call takes a `content_type` argument: several media, or a
    /// media range.
    pub content_type_parameter: bool,
}
impl PlannedBody {
    /// The model schema, when the body is exactly one model.
    pub fn schema(&self) -> Option<&SchemaId> {
        self.ty.schema()
    }
}

/// One declared response with its allocated result classes.
#[derive(Debug, Clone)]
pub struct PlannedResponse {
    /// Allocated `<Stem>Status<Suffix>` result class.
    pub class_name: String,
    /// Async client result class; differs from `class_name` only for streams.
    pub async_class_name: String,
    /// API error class; a separate `<Stem>StatusDefaultApiError` for `default`.
    pub error_class_name: String,
    /// Async client API error class; differs only for streams.
    pub async_error_class_name: String,
    /// Python type of the decoded body, `None` when the method or status
    /// forbids one and `| None` when a range or default status permits none.
    pub ty: NativeType,
    /// Declared media in source order; empty when a body is forbidden.
    pub media: Vec<PlannedMedia>,
    /// Allocated headers class name, present when headers are declared.
    pub header_group: Option<String>,
    pub(crate) wire: p::ResponsePlan,
}
impl PlannedResponse {
    /// Source-declared status selector.
    pub fn status(&self) -> p::ResponseStatus {
        self.wire.status()
    }
    /// The declared status, when it is an exact code.
    pub fn exact_status(&self) -> Option<u16> {
        if let p::ResponseStatus::Exact(status) = self.status() {
            Some(status)
        } else {
            None
        }
    }
    /// The model schema, when the body is exactly one model.
    pub fn schema(&self) -> Option<&SchemaId> {
        self.ty.schema()
    }
    /// Shared wire plan of the source response.
    pub fn protocol(&self) -> &p::ResponsePlan {
        &self.wire
    }
    pub(crate) fn wire(&self) -> &p::ResponsePlan {
        &self.wire
    }
    /// Whether the status selector admits a success (2xx, `2XX`, `default`).
    pub(crate) fn succeeds(&self) -> bool {
        match self.status() {
            p::ResponseStatus::Exact(n) => (200..300).contains(&n),
            p::ResponseStatus::Range(n) => n == 2,
            p::ResponseStatus::Default => true,
        }
    }
    /// Whether the status selector admits a failure (non-2xx, `default`).
    pub(crate) fn fails(&self) -> bool {
        self.status() == p::ResponseStatus::Default || !self.succeeds()
    }
}

/// Allocated native operation names shared by code, examples and documentation.
#[derive(Debug, Clone)]
pub struct PlannedOperation {
    /// Source location of the operation declaration.
    pub source: SourceId,
    /// Source operationId, or `<METHOD>_<path>` when none is declared.
    pub operation_id: String,
    /// Allocated snake-case client method name.
    pub snake_name: String,
    /// `<Stem>Success` result union name.
    pub success_type: String,
    /// Async client result union; differs from `success_type` only for streams.
    pub async_success_type: String,
    /// `<Stem>ApiError` error union name.
    pub error_type: String,
    /// Async client error union; differs from `error_type` only for streams.
    pub async_error_type: String,
    pub(crate) wire: p::OperationPlan,
    pub(crate) parameters: Vec<PlannedParameter>,
    pub(crate) body: Option<PlannedBody>,
    pub(crate) responses: Vec<PlannedResponse>,
}
impl PlannedOperation {
    /// Allocated keyword arguments in source parameter order.
    pub fn parameters(&self) -> &[PlannedParameter] {
        &self.parameters
    }
    /// Declared responses in source order.
    pub fn responses(&self) -> &[PlannedResponse] {
        &self.responses
    }
    /// The request body, when declared.
    pub fn body(&self) -> Option<&PlannedBody> {
        self.body.as_ref()
    }
    /// Shared wire plan of the source operation.
    pub fn protocol(&self) -> &p::OperationPlan {
        &self.wire
    }
    pub(crate) fn wire(&self) -> &p::OperationPlan {
        &self.wire
    }
    /// Source operation description, empty when none is declared.
    pub fn description(&self) -> &str {
        self.wire.description().map_or("", |text| text.value())
    }
    pub(crate) fn exports(&self) -> Vec<&str> {
        let mut names = vec![
            &*self.success_type,
            &*self.async_success_type,
            &*self.error_type,
            &*self.async_error_type,
        ];
        for response in &self.responses {
            if response.succeeds() {
                names.extend([&*response.class_name, &*response.async_class_name]);
            }
            if response.fails() {
                names.extend([
                    &*response.error_class_name,
                    &*response.async_error_class_name,
                ]);
            }
        }
        names.sort();
        names.dedup();
        names
    }
}

struct Lower {
    names: BTreeSet<String>,
    groups: Vec<PlannedGroup>,
}
impl Lower {
    fn name(&mut self, name: &str) -> String {
        allocate(name, &mut self.names)
    }
    fn headers(
        &mut self,
        source: &SourceId,
        stem: &str,
        headers: &[p::HeaderPlan],
    ) -> Option<String> {
        if headers.is_empty() {
            return None;
        }
        let name = self.name(&format!("{stem}Headers"));
        let mut used = BTreeSet::new();
        let headers: Vec<_> = headers
            .iter()
            .map(|wire| PlannedHeader {
                name: allocate(&python_name(wire.name()), &mut used),
                wire: wire.clone(),
            })
            .collect();
        let fields = headers
            .iter()
            .map(|header| PlannedField {
                name: header.name.clone(),
                wire_name: header.wire.name().into(),
                source: header.wire.source().use_site().source().clone(),
                ty: NativeType::Model(header.schema().clone()),
                required: header.wire.required(),
                part: None,
            })
            .collect();
        self.groups.push(PlannedGroup {
            name: name.clone(),
            source: source.clone(),
            kind: "headers",
            fields,
            additional: None,
            additional_part: None,
            headers,
            part_value: None,
            header_group: None,
            required_headers: false,
        });
        Some(name)
    }
    fn part(&mut self, stem: &str, part: &p::PartPlan) -> NativeType {
        let value = match part.representation() {
            p::PartRepresentation::Json { codec, .. }
            | p::PartRepresentation::Text { codec, .. }
            | p::PartRepresentation::Style { codec, .. } => {
                NativeType::Model(codec.schema().id().clone())
            }
            p::PartRepresentation::Binary { .. } => NativeType::Builtin("bytes"),
        };
        let header_group = self.headers(part.source().use_site().source(), stem, part.headers());
        let required_headers = part.headers().iter().any(|header| header.required());
        let wrapper = if let Some(header_group) = header_group {
            let name = self.name(&format!("{stem}Part"));
            self.groups.push(PlannedGroup {
                name: name.clone(),
                source: part.source().use_site().source().clone(),
                kind: "part",
                fields: Vec::new(),
                additional: None,
                additional_part: None,
                headers: Vec::new(),
                part_value: Some(value.clone()),
                header_group: Some(header_group),
                required_headers,
            });
            NativeType::Group(name)
        } else {
            NativeType::Part(Box::new(value.clone()))
        };
        let ty = if required_headers {
            wrapper
        } else {
            NativeType::union([value, wrapper])
        };
        if part.multiplicity() == p::PartMultiplicity::RepeatedArrayItems {
            NativeType::List(Box::new(ty))
        } else {
            ty
        }
    }
    fn media(&mut self, stem: &str, wire: &p::MediaPlan, request: bool) -> PlannedMedia {
        let ty = match wire.representation() {
            p::Representation::Json { codec } => codec
                .as_ref()
                .map_or(NativeType::Builtin("JsonValue"), |codec| {
                    NativeType::Model(codec.schema().id().clone())
                }),
            p::Representation::Text { codec, scalar, .. } => codec.as_ref().map_or(
                NativeType::Builtin(match scalar {
                    p::ScalarType::String => "str",
                    p::ScalarType::Boolean => "bool",
                    p::ScalarType::Integer => "int",
                    p::ScalarType::Number => "JsonNumber",
                }),
                |codec| NativeType::Model(codec.schema().id().clone()),
            ),
            p::Representation::Binary { .. } => NativeType::Builtin("bytes"),
            p::Representation::Stream { stream } => {
                let source = stream.item_codec().map(|codec| codec.schema().id().clone());
                if request {
                    NativeType::Items(source)
                } else {
                    NativeType::Stream(source)
                }
            }
            p::Representation::Form { .. } | p::Representation::Multipart { .. } => {
                let (rules, parts, additional, kind) = match wire.representation() {
                    p::Representation::Form { form } => {
                        (form.rules(), form.fields(), form.additional(), "form")
                    }
                    p::Representation::Multipart {
                        multipart:
                            p::MultipartPlan::Named {
                                rules,
                                parts,
                                additional,
                            },
                    } => (rules, parts.as_slice(), additional, "multipart"),
                    _ => unreachable!("unadmitted positional multipart"),
                };
                let name = self.name(&format!(
                    "{stem}{}",
                    if kind == "form" { "Form" } else { "Multipart" }
                ));
                let mut used = BTreeSet::from([
                    "_extra_fields".into(),
                    "extra_fields".into(),
                    "set_extra".into(),
                ]);
                let mut fields = Vec::new();
                for part in parts {
                    let wire_name = part.name().expect("named part");
                    let ty = self.part(
                        &format!("{name}{}", crate::rust_models::pascal(wire_name)),
                        part,
                    );
                    fields.push(PlannedField {
                        name: allocate(&python_name(wire_name), &mut used),
                        wire_name: wire_name.into(),
                        source: part.source().use_site().source().clone(),
                        ty,
                        required: part.required(),
                        part: Some(part.clone()),
                    });
                }
                let (extra_ty, extra_part) = match additional {
                    p::AdditionalParts::Forbidden => (None, None),
                    p::AdditionalParts::Allowed(part) => (
                        Some(self.part(&format!("{name}Extra"), part)),
                        Some(part.as_ref().clone()),
                    ),
                };
                self.groups.push(PlannedGroup {
                    name: name.clone(),
                    source: rules.schema().id().clone(),
                    kind,
                    fields,
                    additional: extra_ty,
                    additional_part: extra_part,
                    headers: Vec::new(),
                    part_value: None,
                    header_group: None,
                    required_headers: false,
                });
                NativeType::Group(name)
            }
        };
        PlannedMedia {
            ty,
            wire: wire.clone(),
        }
    }
}

pub(super) fn lower(protocol: &p::ProtocolPlan) -> (Vec<PlannedOperation>, Vec<PlannedGroup>) {
    let mut lower = Lower {
        names: [
            "Client",
            "AsyncClient",
            "Literal",
            "TypeAlias",
            "Never",
            "UNSET",
            "Unset",
            "ApiError",
            "Source",
            "Part",
            "JsonNumber",
            "JsonValue",
            "SyncStream",
            "AsyncStream",
            "Iterable",
            "AsyncIterable",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        groups: Vec::new(),
    };
    let mut methods = [
        "close",
        "aclose",
        "_call",
        "_prepare",
        "_exchange",
        "_configure",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let mut operations = Vec::new();
    for wire in protocol.operations() {
        let operation_id = wire
            .operation_id()
            .map(|id| id.value().clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("{}_{}", wire.method().as_str(), wire.path()));
        let source = wire.source().terminal().source().clone();
        let stem = crate::rust_models::pascal(&operation_id);
        let snake_name = allocate(&python_name(&operation_id), &mut methods);
        let success_type = lower.name(&format!("{stem}Success"));
        let error_type = lower.name(&format!("{stem}ApiError"));
        let mut members = [
            "body",
            "content_type",
            "self",
            "parameters",
            "body_bytes",
            "raw",
            "codecs",
            "models",
            "operations",
            "isinstance",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        let parameters = wire
            .parameters()
            .iter()
            .map(|parameter| PlannedParameter {
                name: allocate(&python_name(parameter.name()), &mut members),
                wire: parameter.clone(),
            })
            .collect();
        let body = wire.body().map(|body| {
            let media: Vec<_> = body
                .media()
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    lower.media(
                        &format!(
                            "{stem}Request{}",
                            if body.media().len() == 1 {
                                String::new()
                            } else {
                                format!("Media{}", i + 1)
                            }
                        ),
                        m,
                        true,
                    )
                })
                .collect();
            PlannedBody {
                source: body.source().use_site().source().clone(),
                required: body.required(),
                ty: NativeType::union(media.iter().map(|media| media.ty.clone())),
                content_type_parameter: media.len() > 1
                    || media.iter().any(|media| {
                        !matches!(
                            media.wire.media_type().range(),
                            p::MediaRange::Concrete { .. }
                        )
                    }),
                media,
            }
        });
        let mut responses = Vec::new();
        for response in wire.responses() {
            let suffix = match response.status() {
                p::ResponseStatus::Exact(value) => value.to_string(),
                p::ResponseStatus::Range(value) => format!("{value}XX"),
                p::ResponseStatus::Default => "Default".into(),
            };
            let class_name = lower.name(&format!("{stem}Status{suffix}"));
            let forbidden = wire.method().as_str() == "HEAD"
                || matches!(
                    response.status(),
                    p::ResponseStatus::Exact(100..=199 | 204 | 205 | 304)
                        | p::ResponseStatus::Range(1)
                );
            let media: Vec<_> = if forbidden {
                Vec::new()
            } else {
                response
                    .media()
                    .iter()
                    .enumerate()
                    .map(|(i, media)| {
                        lower.media(&format!("{class_name}Media{}", i + 1), media, false)
                    })
                    .collect()
            };
            let mut ty = if forbidden {
                NativeType::Builtin("None")
            } else if media.is_empty() {
                NativeType::Builtin("bytes")
            } else {
                NativeType::union(media.iter().map(|media| media.ty.clone()))
            };
            if matches!(
                response.status(),
                p::ResponseStatus::Range(2 | 3) | p::ResponseStatus::Default
            ) && !forbidden
            {
                ty = NativeType::union([ty, NativeType::Builtin("None")]);
            }
            let async_class_name = if ty.streaming() {
                lower.name(&format!("{stem}AsyncStatus{suffix}"))
            } else {
                class_name.clone()
            };
            let error_class_name = if response.status() == p::ResponseStatus::Default {
                lower.name(&format!("{stem}StatusDefaultApiError"))
            } else {
                class_name.clone()
            };
            let async_error_class_name =
                if response.status() == p::ResponseStatus::Default && ty.streaming() {
                    lower.name(&format!("{stem}AsyncStatusDefaultApiError"))
                } else if ty.streaming() {
                    async_class_name.clone()
                } else {
                    error_class_name.clone()
                };
            let header_group = lower.headers(
                response.source().use_site().source(),
                &class_name,
                response.headers(),
            );
            responses.push(PlannedResponse {
                class_name,
                async_class_name,
                error_class_name,
                async_error_class_name,
                ty,
                media,
                header_group,
                wire: response.clone(),
            });
        }
        let streaming = responses.iter().any(|response| response.ty.streaming());
        let async_success_type = if streaming {
            lower.name(&format!("{stem}AsyncSuccess"))
        } else {
            success_type.clone()
        };
        let async_error_type = if streaming {
            lower.name(&format!("{stem}AsyncApiError"))
        } else {
            error_type.clone()
        };
        operations.push(PlannedOperation {
            source,
            operation_id,
            snake_name,
            success_type,
            async_success_type,
            error_type,
            async_error_type,
            wire: wire.clone(),
            parameters,
            body,
            responses,
        });
    }
    (operations, lower.groups)
}
