//! Native storage and names over the one shared HTTP protocol plan.
use super::{ModelPlan, models};
use crate::http_protocol as wire;
use std::collections::{BTreeMap, BTreeSet};
use suspect_ir::contract::{SchemaId, SourceId};

/// Ruby value carrier of a parameter, media payload, part or record field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeType {
    /// Generated model codec for this validation-program node.
    Codec(usize),
    /// Schema-free JSON value.
    Json,
    /// Schema-free text converted as this scalar.
    Scalar(wire::ScalarType),
    /// Binary `String` (ASCII-8BIT).
    Bytes,
    /// Generated form/multipart/header record class with this name.
    Record(String),
    /// Repeated values.
    Array(Box<NativeType>),
    /// A form/multipart part carrying this value plus optional part headers.
    Part(Box<NativeType>),
    /// Lazily decoded item stream.
    Stream(Box<NativeType>),
    /// Positional multipart alternatives, in prefix order then the item type.
    Union(Vec<NativeType>),
}
impl NativeType {
    /// Program node of a [`NativeType::Codec`]; `None` for every other carrier.
    pub fn schema_index(&self) -> Option<usize> {
        if let Self::Codec(i) = self {
            Some(*i)
        } else {
            None
        }
    }
}

/// A caller-supplied path, query, header or cookie parameter of one operation.
#[derive(Debug, Clone)]
pub struct PlannedParameter {
    /// Source parameter declaration.
    pub source: SourceId,
    /// Parameter name on the wire.
    pub wire_name: String,
    /// Ruby keyword argument, unique within the operation's keywords.
    pub keyword: String,
    /// Whether the source marks the parameter required.
    pub required: bool,
    /// Value carrier: the codec, or a record for form-content parameters.
    pub value_type: NativeType,
    /// Admitted wire serialization plan.
    pub wire: wire::ParameterPlan,
}
impl PlannedParameter {
    /// Codec root schema of the parameter value.
    pub fn schema(&self) -> &SchemaId {
        self.wire.codec().schema().id()
    }
    /// Program node of the value codec, when the value is codec-bound.
    pub fn schema_index(&self) -> Option<usize> {
        self.value_type.schema_index()
    }
    /// Where the parameter is sent.
    pub fn location(&self) -> wire::ParameterLocation {
        self.wire.location()
    }
    /// Declared style/explode serialization.
    pub fn serialization(&self) -> &wire::ParameterSerialization {
        self.wire.serialization()
    }
}
/// One admitted media type of a request body or response.
#[derive(Debug, Clone)]
pub struct PlannedMedia {
    /// Source media-type declaration.
    pub source: SourceId,
    /// Ruby carrier of the payload.
    pub value_type: NativeType,
    /// Admitted media plan.
    pub wire: wire::MediaPlan,
}
impl PlannedMedia {
    /// Program node of the payload codec, when the payload is codec-bound.
    pub fn schema_index(&self) -> Option<usize> {
        self.value_type.schema_index()
    }
}
/// The request body of one operation.
#[derive(Debug, Clone)]
pub struct PlannedBody {
    /// Source request-body declaration.
    pub source: SourceId,
    /// Whether the source marks the body required.
    pub required: bool,
    /// Admitted media types, in declaration order.
    pub media: Vec<PlannedMedia>,
}
/// One declared response header.
#[derive(Debug, Clone)]
pub struct PlannedHeader {
    /// Source header declaration.
    pub source: SourceId,
    /// Ruby member name, unique within the response.
    pub name: String,
    /// Header name on the wire.
    pub wire_name: String,
    /// Program node of the header value codec.
    pub schema_index: usize,
    /// Whether the source marks the header required.
    pub required: bool,
    /// Admitted header parsing plan.
    pub wire: wire::HeaderPlan,
}
/// One declared response status (exact, range or `default`) of an operation.
#[derive(Debug, Clone)]
pub struct PlannedResponse {
    /// Source response declaration.
    pub source: SourceId,
    /// Declared status selector.
    pub status: wire::ResponseStatus,
    /// Status key as written in the source (`200`, `4XX`, `default`).
    pub status_key: String,
    /// Admitted media types; empty when `body_forbidden`.
    pub media: Vec<PlannedMedia>,
    /// Collision-free Ruby response class name.
    pub class_name: String,
    /// Error class raised for this status; distinct from `class_name` only for `default`.
    pub error_class: String,
    /// Declared response headers.
    pub headers: Vec<PlannedHeader>,
    /// Generated header record class, when any header is declared.
    pub headers_class: Option<String>,
    /// Whether HTTP forbids a body (HEAD, 1xx, 204, 205, 304).
    pub body_forbidden: bool,
    /// Admitted response plan.
    pub wire: wire::ResponsePlan,
}
impl PlannedResponse {
    /// Whether this status can match a successful (2xx) response.
    pub fn can_succeed(&self) -> bool {
        matches!(
            self.status,
            wire::ResponseStatus::Exact(200..=299)
                | wire::ResponseStatus::Range(2)
                | wire::ResponseStatus::Default
        )
    }
    /// Whether this status can match a non-2xx response.
    pub fn can_fail(&self) -> bool {
        !matches!(
            self.status,
            wire::ResponseStatus::Exact(200..=299) | wire::ResponseStatus::Range(2)
        )
    }
}
/// One selected outgoing operation and its Ruby client method.
#[derive(Debug, Clone)]
pub struct PlannedOperation {
    /// Source operation declaration.
    pub source: SourceId,
    /// Source `operationId`, or a method+path name for unnamed operations.
    pub operation_id: String,
    /// Collision-free client method name.
    pub method_name: String,
    /// HTTP method token.
    pub method: String,
    /// Path template as declared.
    pub path: String,
    /// Source description, falling back to the summary; empty when neither exists.
    pub description: String,
    /// Caller parameters, in protocol order.
    pub parameters: Vec<PlannedParameter>,
    /// Request body, if declared.
    pub body: Option<PlannedBody>,
    /// Declared responses, in protocol order.
    pub responses: Vec<PlannedResponse>,
    /// Operation-wide API error class.
    pub error_class: String,
    /// Admitted operation plan.
    pub wire: wire::OperationPlan,
}
/// One member of a generated record class.
#[derive(Debug, Clone)]
pub struct RecordField {
    /// Source part or header declaration.
    pub source: SourceId,
    /// Part or header name on the wire.
    pub wire_name: String,
    /// Ruby member name, unique within the record.
    pub name: String,
    /// Whether the source marks the member required.
    pub required: bool,
    /// Ruby carrier of the member value.
    pub value_type: NativeType,
}
/// What a generated record class encodes or decodes.
#[derive(Debug, Clone)]
pub enum RecordBinding {
    /// Named form or multipart body fields.
    Media(Box<wire::MediaPlan>),
    /// Response or part headers.
    Headers(Vec<wire::HeaderPlan>),
}
/// A generated record class for form/multipart fields or declared headers.
#[derive(Debug, Clone)]
pub struct NativeRecord {
    /// Source media or response/part declaration the record belongs to.
    pub source: SourceId,
    /// Collision-free Ruby class name.
    pub name: String,
    /// Declared members.
    pub fields: Vec<RecordField>,
    /// Carrier for undeclared parts, when the body admits them.
    pub additional: Option<NativeType>,
    /// Wire declaration the record binds.
    pub binding: RecordBinding,
}

pub(super) fn capabilities(config: &super::RubyConfig) -> wire::Capabilities {
    use wire::Capability::*;
    let mut caps = wire::Capabilities::for_adapter(
        "ruby-http-protocol-v1",
        [
            UnnamedOperations,
            AdditionalMethods,
            CustomMethods,
            HttpServers,
            RelativeServers,
            MultipleServers,
            ServerVariables,
            AnonymousSecurity,
            SecurityAlternatives,
            ConjunctiveSecurity,
            HttpBasic,
            ApiKeys,
            OAuth2,
            OpenIdConnect,
            SecurityRoles,
            ParameterStyles,
            HeaderParameters,
            CookieParameters,
            ReservedParameters,
            ContentParameters,
            QuerystringParameters,
            QuerystringForm,
            RangeResponses,
            DefaultResponses,
            UndeclaredResponses,
            MultipleMediaTypes,
            MediaRanges,
            MediaTypeParameters,
            StructuredJsonMedia,
            SchemaFreeJson,
            TextBodies,
            BinaryBodies,
            UndeclaredResponseBody,
            ResponseHeaders,
            ResponseLinks,
            FormBodies,
            MultipartBodies,
            PartEncodings,
            PositionalMultipart,
            ServerSentEvents,
            JsonLines,
            OpenApi30,
            OpenApi32,
        ],
    )
    .with_limits(wire::ByteLimits::new(
        config.max_response_bytes.max(config.max_request_bytes) as u64,
        config.max_part_bytes as u64,
        config.max_stream_item_bytes as u64,
    ));
    if config.document_relative_servers {
        caps = caps.with(DocumentRelativeServers);
    }
    if config.schema_resources {
        caps = caps.with(SchemaResources);
    }
    if config.dynamic_schema_references {
        caps = caps.with(DynamicSchemaReferences);
    }
    if config.legacy_binary_strings {
        caps.with_profile(wire::CompatibilityProfile::LegacyBinaryStringV1)
    } else {
        caps
    }
}

fn operation_name(op: &wire::OperationPlan) -> String {
    op.operation_id()
        .map(|v| v.value().clone())
        .unwrap_or_else(|| {
            format!(
                "{}{}",
                models::constant(op.method().as_str()),
                models::constant(op.path())
            )
        })
}
pub(super) fn hints(
    protocol: &wire::ProtocolPlan,
    indices: &BTreeMap<SchemaId, usize>,
) -> BTreeMap<usize, String> {
    let mut hints = BTreeMap::new();
    fn media(
        m: &wire::MediaPlan,
        stem: &str,
        indices: &BTreeMap<SchemaId, usize>,
        out: &mut BTreeMap<usize, String>,
    ) {
        use wire::Representation::*;
        match m.representation() {
            Json { codec: Some(c) } | Text { codec: Some(c), .. } => {
                if let Some(i) = indices.get(c.schema().id()) {
                    out.insert(*i, stem.into());
                }
            }
            Stream { stream } => {
                if let Some(codec) = stream.item_codec()
                    && let Some(i) = indices.get(codec.schema().id())
                {
                    out.insert(*i, format!("{stem}Item"));
                }
            }
            Form { form } => {
                for p in form.fields() {
                    part(p, stem, indices, out)
                }
            }
            Multipart {
                multipart: wire::MultipartPlan::Named { parts, .. },
            } => {
                for p in parts {
                    part(p, stem, indices, out)
                }
            }
            Multipart {
                multipart: wire::MultipartPlan::Positional { prefix, items, .. },
            } => {
                for p in prefix {
                    part(p, stem, indices, out)
                }
                if let wire::AdditionalParts::Allowed(p) = items {
                    part(p, stem, indices, out)
                }
            }
            _ => {}
        }
    }
    fn part(
        p: &wire::PartPlan,
        stem: &str,
        indices: &BTreeMap<SchemaId, usize>,
        out: &mut BTreeMap<usize, String>,
    ) {
        let codec = match p.representation() {
            wire::PartRepresentation::Json { codec, .. }
            | wire::PartRepresentation::Text { codec, .. }
            | wire::PartRepresentation::Style { codec, .. } => Some(codec),
            _ => None,
        };
        if let Some(i) = codec.and_then(|c| indices.get(c.schema().id())) {
            out.insert(
                *i,
                format!("{stem}{}", models::constant(p.name().unwrap_or("Item"))),
            );
        }
    }
    for op in protocol.operations() {
        let stem = models::constant(&operation_name(op));
        if let Some(b) = op.body() {
            for m in b.media() {
                media(m, &format!("{stem}Request"), indices, &mut hints)
            }
        }
        for r in op.responses() {
            for m in r.media() {
                media(
                    m,
                    &format!("{stem}Response{}", r.status_key()),
                    indices,
                    &mut hints,
                )
            }
        }
        for p in op.parameters() {
            if let Some(i) = indices.get(p.codec().schema().id()) {
                hints.insert(*i, format!("{stem}{}Parameter", models::constant(p.name())));
            }
            if let Some(m) = p.content_media() {
                media(
                    m,
                    &format!("{stem}{}Parameter", models::constant(p.name())),
                    indices,
                    &mut hints,
                )
            }
        }
    }
    hints
}

pub(super) fn lower(
    protocol: &wire::ProtocolPlan,
    indices: &BTreeMap<SchemaId, usize>,
    models: &ModelPlan,
) -> (Vec<PlannedOperation>, Vec<NativeRecord>) {
    let mut lower = Lower {
        indices,
        used: models::reserved_constants(),
        records: Vec::new(),
    };
    lower.used.extend(models.symbols().map(|s| s.name.clone()));
    let mut methods = models::reserved_members();
    let mut classes = models::reserved_constants();
    let mut operations = Vec::new();
    for op in protocol.operations() {
        let operation_id = operation_name(op);
        let stem = models::constant(&operation_id);
        let method_name = models::allocate(&models::member(&operation_id), &mut methods);
        let error_class = models::allocate(&format!("{stem}ApiError"), &mut classes);
        let mut keywords = models::reserved_members();
        keywords.extend(
            [
                "body",
                "timeout",
                "cancellation",
                "max_response_bytes",
                "max_capture_bytes",
                "content_type",
                "accept",
                "security",
                "server",
                "server_variables",
                "document_url",
            ]
            .map(str::to_owned),
        );
        let parameters = op
            .parameters()
            .iter()
            .map(|p| {
                let value_type = if let Some(m) = p
                    .content_media()
                    .filter(|m| matches!(m.representation(), wire::Representation::Form { .. }))
                {
                    lower
                        .media(m, &format!("{stem}{}Parameter", models::constant(p.name())))
                        .value_type
                } else {
                    NativeType::Codec(indices[p.codec().schema().id()])
                };
                PlannedParameter {
                    source: p.source().use_site().source().clone(),
                    wire_name: p.name().into(),
                    keyword: models::allocate(&models::member(p.name()), &mut keywords),
                    required: p.required(),
                    value_type,
                    wire: p.clone(),
                }
            })
            .collect();
        let body = op.body().map(|b| PlannedBody {
            source: b.source().use_site().source().clone(),
            required: b.required(),
            media: b
                .media()
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    lower.media(
                        m,
                        &format!(
                            "{stem}Request{}",
                            if i == 0 {
                                String::new()
                            } else {
                                models::constant(m.media_type().declared())
                            }
                        ),
                    )
                })
                .collect(),
        });
        let responses = op
            .responses()
            .iter()
            .map(|r| {
                let name =
                    models::allocate(&format!("{stem}Status{}", r.status_key()), &mut classes);
                let error = if matches!(r.status(), wire::ResponseStatus::Default) {
                    models::allocate(&format!("{name}Error"), &mut classes)
                } else {
                    name.clone()
                };
                let headers = lower.headers(r.headers());
                let headers_class = lower.header_record(
                    r.source().use_site().source(),
                    &format!("{name}Headers"),
                    r.headers(),
                );
                let forbidden = op.method() == wire::Method::Head
                    || matches!(
                        r.status(),
                        wire::ResponseStatus::Exact(100..=199 | 204 | 205 | 304)
                            | wire::ResponseStatus::Range(1)
                    );
                let media = if forbidden {
                    Vec::new()
                } else {
                    r.media()
                        .iter()
                        .map(|m| lower.media(m, &format!("{stem}Response{}", r.status_key())))
                        .collect()
                };
                PlannedResponse {
                    source: r.source().use_site().source().clone(),
                    status: r.status(),
                    status_key: r.status_key().into(),
                    class_name: name,
                    error_class: error,
                    headers,
                    headers_class,
                    media,
                    body_forbidden: forbidden,
                    wire: r.clone(),
                }
            })
            .collect();
        operations.push(PlannedOperation {
            source: op.source().use_site().source().clone(),
            operation_id,
            method_name,
            method: op.method().as_str().into(),
            path: op.path().into(),
            description: op
                .description()
                .or(op.summary())
                .map(|d| d.value().clone())
                .unwrap_or_default(),
            parameters,
            body,
            responses,
            error_class,
            wire: op.clone(),
        });
    }
    (operations, lower.records)
}
struct Lower<'a> {
    indices: &'a BTreeMap<SchemaId, usize>,
    used: BTreeSet<String>,
    records: Vec<NativeRecord>,
}
impl Lower<'_> {
    fn codec(&self, c: &wire::CodecRef) -> NativeType {
        NativeType::Codec(self.indices[c.schema().id()])
    }
    fn part_type(&mut self, p: &wire::PartPlan, stem: &str) -> NativeType {
        let value = match p.representation() {
            wire::PartRepresentation::Binary { .. } => NativeType::Bytes,
            wire::PartRepresentation::Json { codec, .. }
            | wire::PartRepresentation::Text { codec, .. }
            | wire::PartRepresentation::Style { codec, .. } => self.codec(codec),
        };
        self.header_record(
            p.source().use_site().source(),
            &format!("{stem}Headers"),
            p.headers(),
        );
        let value = NativeType::Part(Box::new(value));
        if p.multiplicity() == wire::PartMultiplicity::RepeatedArrayItems {
            NativeType::Array(Box::new(value))
        } else {
            value
        }
    }
    fn headers(&self, headers: &[wire::HeaderPlan]) -> Vec<PlannedHeader> {
        let mut names = models::reserved_members();
        headers
            .iter()
            .map(|h| PlannedHeader {
                source: h.source().use_site().source().clone(),
                name: models::allocate(&models::member(h.name()), &mut names),
                wire_name: h.name().into(),
                schema_index: self.indices[h.codec().schema().id()],
                required: h.required(),
                wire: h.clone(),
            })
            .collect()
    }
    fn header_record(
        &mut self,
        source: &SourceId,
        stem: &str,
        headers: &[wire::HeaderPlan],
    ) -> Option<String> {
        if headers.is_empty() {
            return None;
        }
        if let Some(record) = self
            .records
            .iter()
            .find(|r| &r.source == source && matches!(r.binding, RecordBinding::Headers(_)))
        {
            return Some(record.name.clone());
        }
        let name = models::allocate(stem, &mut self.used);
        let fields = self
            .headers(headers)
            .into_iter()
            .map(|h| RecordField {
                source: h.source,
                wire_name: h.wire_name,
                name: h.name,
                required: h.required,
                value_type: NativeType::Codec(h.schema_index),
            })
            .collect();
        self.records.push(NativeRecord {
            source: source.clone(),
            name: name.clone(),
            fields,
            additional: None,
            binding: RecordBinding::Headers(headers.to_vec()),
        });
        Some(name)
    }
    fn media(&mut self, m: &wire::MediaPlan, stem: &str) -> PlannedMedia {
        use wire::Representation::*;
        let value_type = match m.representation() {
            Json { codec } => codec.as_ref().map_or(NativeType::Json, |c| self.codec(c)),
            Text { codec, scalar, .. } => codec
                .as_ref()
                .map_or(NativeType::Scalar(*scalar), |c| self.codec(c)),
            Binary { .. } => NativeType::Bytes,
            Stream { stream } => {
                NativeType::Stream(Box::new(
                    stream.item_codec().map_or(NativeType::Json, |codec| self.codec(codec)),
                ))
            }
            Form { form } => self.named(m, stem, form.fields(), form.additional()),
            Multipart {
                multipart:
                    wire::MultipartPlan::Named {
                        parts, additional, ..
                    },
            } => self.named(m, stem, parts, additional),
            Multipart {
                multipart: wire::MultipartPlan::Positional { prefix, items, .. },
            } => {
                let mut types = prefix
                    .iter()
                    .enumerate()
                    .map(|(i, p)| self.part_type(p, &format!("{stem}Part{i}")))
                    .collect::<Vec<_>>();
                if let wire::AdditionalParts::Allowed(p) = items {
                    types.push(self.part_type(p, &format!("{stem}Item")));
                }
                NativeType::Array(Box::new(NativeType::Union(types)))
            }
        };
        PlannedMedia {
            source: m.source().use_site().source().clone(),
            value_type,
            wire: m.clone(),
        }
    }
    fn named(
        &mut self,
        m: &wire::MediaPlan,
        stem: &str,
        parts: &[wire::PartPlan],
        additional: &wire::AdditionalParts,
    ) -> NativeType {
        let source = m.source().use_site().source();
        if let Some(r) = self
            .records
            .iter()
            .find(|r| &r.source == source && matches!(r.binding, RecordBinding::Media(_)))
        {
            return NativeType::Record(r.name.clone());
        }
        let name = models::allocate(stem, &mut self.used);
        let mut names = models::reserved_members();
        let fields = parts
            .iter()
            .map(|p| {
                let wire_name = p.name().expect("named part").to_owned();
                RecordField {
                    source: p.source().use_site().source().clone(),
                    name: models::allocate(&models::member(&wire_name), &mut names),
                    required: p.required(),
                    value_type: self
                        .part_type(p, &format!("{name}{}", models::constant(&wire_name))),
                    wire_name,
                }
            })
            .collect();
        let additional = match additional {
            wire::AdditionalParts::Forbidden => None,
            wire::AdditionalParts::Allowed(p) => Some(self.part_type(p, &format!("{name}Extra"))),
        };
        self.records.push(NativeRecord {
            source: source.clone(),
            name: name.clone(),
            fields,
            additional,
            binding: RecordBinding::Media(Box::new(m.clone())),
        });
        NativeType::Record(name)
    }
}
