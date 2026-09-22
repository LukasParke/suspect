//! Native storage and names over the one shared HTTP protocol plan.
use super::{ModelPlan, models};
use crate::http_protocol as wire;
use std::collections::{BTreeMap, BTreeSet};
use suspect_ir::contract::{SchemaId, SourceId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeType {
    Codec(usize),
    Json,
    Scalar(wire::ScalarType),
    Bytes,
    NoContent,
    Record(String),
    Array(Box<NativeType>),
    Part(Box<NativeType>),
    Stream(Box<NativeType>),
    Union(Vec<NativeType>),
}
impl NativeType {
    pub fn schema_index(&self) -> Option<usize> {
        if let Self::Codec(i) = self {
            Some(*i)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlannedParameter {
    pub source: SourceId,
    pub wire_name: String,
    pub keyword: String,
    pub required: bool,
    pub value_type: NativeType,
    pub wire: wire::ParameterPlan,
}
impl PlannedParameter {
    pub fn schema(&self) -> &SchemaId {
        self.wire.codec().schema().id()
    }
    pub fn schema_index(&self) -> Option<usize> {
        self.value_type.schema_index()
    }
    pub fn location(&self) -> wire::ParameterLocation {
        self.wire.location()
    }
    pub fn serialization(&self) -> &wire::ParameterSerialization {
        self.wire.serialization()
    }
}
#[derive(Debug, Clone)]
pub struct PlannedMedia {
    pub source: SourceId,
    pub value_type: NativeType,
    pub wire: wire::MediaPlan,
}
impl PlannedMedia {
    pub fn schema_index(&self) -> Option<usize> {
        self.value_type.schema_index()
    }
}
#[derive(Debug, Clone)]
pub struct PlannedBody {
    pub source: SourceId,
    pub required: bool,
    pub media: Vec<PlannedMedia>,
}
#[derive(Debug, Clone)]
pub struct PlannedHeader {
    pub source: SourceId,
    pub name: String,
    pub wire_name: String,
    pub schema_index: usize,
    pub required: bool,
    pub wire: wire::HeaderPlan,
}
#[derive(Debug, Clone)]
pub struct PlannedResponse {
    pub source: SourceId,
    pub status: wire::ResponseStatus,
    pub status_key: String,
    pub media: Vec<PlannedMedia>,
    pub class_name: String,
    pub error_class: String,
    pub headers: Vec<PlannedHeader>,
    pub headers_class: Option<String>,
    pub body_forbidden: bool,
    pub wire: wire::ResponsePlan,
}
impl PlannedResponse {
    pub fn can_succeed(&self) -> bool {
        matches!(
            self.status,
            wire::ResponseStatus::Exact(200..=299)
                | wire::ResponseStatus::Range(2)
                | wire::ResponseStatus::Default
        )
    }
    pub fn can_fail(&self) -> bool {
        !matches!(
            self.status,
            wire::ResponseStatus::Exact(200..=299) | wire::ResponseStatus::Range(2)
        )
    }
}
#[derive(Debug, Clone)]
pub struct PlannedOperation {
    pub source: SourceId,
    pub operation_id: String,
    pub method_name: String,
    pub method: String,
    pub path: String,
    pub description: String,
    pub parameters: Vec<PlannedParameter>,
    pub body: Option<PlannedBody>,
    pub responses: Vec<PlannedResponse>,
    pub error_class: String,
    pub wire: wire::OperationPlan,
}
#[derive(Debug, Clone)]
pub struct RecordField {
    pub source: SourceId,
    pub wire_name: String,
    pub name: String,
    pub required: bool,
    pub value_type: NativeType,
}
#[derive(Debug, Clone)]
pub enum RecordBinding {
    Media(Box<wire::MediaPlan>),
    Headers(Vec<wire::HeaderPlan>),
}
#[derive(Debug, Clone)]
pub struct NativeRecord {
    pub source: SourceId,
    pub name: String,
    pub fields: Vec<RecordField>,
    pub additional: Option<NativeType>,
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
