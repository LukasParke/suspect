//! Java-native payload/header/part bindings for the shared HTTP protocol plan.
//! Binary and aggregate wire values never acquire invented JSON codec roots.

use super::{
    HttpDiagnostic, ProtocolConfig,
    models::{JavaModelPlan, allocate, bounded, member, reserved_members, reserved_types},
};
use crate::http_protocol::{
    self as wire, AdditionalParts, Capability, PartRepresentation, Representation,
};
use std::collections::{BTreeMap, BTreeSet};
use suspect_ir::contract::{Contract, SchemaId, SourceId};

/// An actual native wire value, separate from JSON model representations.
#[derive(Debug, Clone)]
pub enum JavaValue {
    Named(String),
    Model(SchemaId),
    Json,
    Text,
    Bytes,
    NoContent,
    ResponseBody(Box<JavaValue>),
    Aggregate(Box<JavaAggregate>),
    Stream { schema: SchemaId, request: bool },
}
impl JavaValue {
    #[must_use]
    pub fn native_type(&self, models: &JavaModelPlan) -> String {
        match self {
            Self::Named(name) => name.clone(),
            Self::Model(id) => models.native_type(id),
            Self::Json => "JsonValue".into(),
            Self::Text => "String".into(),
            Self::Bytes => "Bytes".into(),
            Self::NoContent => "NoContent".into(),
            Self::ResponseBody(inner) => format!("ResponseBody<{}>", inner.native_type(models)),
            Self::Aggregate(value) => value.name.clone(),
            Self::Stream { schema, request } => format!(
                "{}<{}>",
                if *request {
                    "java.util.List"
                } else {
                    "EventStream"
                },
                models.native_type(schema)
            ),
        }
    }
    #[must_use]
    pub fn schema(&self) -> Option<&SchemaId> {
        match self {
            Self::Model(id) | Self::Stream { schema: id, .. } => Some(id),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct JavaHeader {
    pub name: String,
    pub wire: wire::HeaderPlan,
    pub schema: SchemaId,
}
#[derive(Debug, Clone)]
pub struct JavaHeaders {
    pub name: String,
    pub source: SourceId,
    pub fields: Vec<JavaHeader>,
    pub descriptor: String,
}
#[derive(Debug, Clone)]
pub struct JavaPart {
    pub name: String,
    /// Multipart parts have native metadata-bearing wrapper classes.
    pub wrapper: Option<String>,
    pub value: JavaValue,
    pub headers: Option<JavaHeaders>,
    pub wire: wire::PartPlan,
    pub descriptor: String,
}
impl JavaPart {
    pub fn item_type(&self, models: &JavaModelPlan) -> String {
        self.wrapper
            .clone()
            .unwrap_or_else(|| self.value.native_type(models))
    }
    pub fn native_type(&self, models: &JavaModelPlan) -> String {
        let ty = self.item_type(models);
        if self.wire.multiplicity() == wire::PartMultiplicity::RepeatedArrayItems {
            format!("java.util.List<{ty}>")
        } else {
            ty
        }
    }
}
#[derive(Debug, Clone)]
pub enum AggregateRules {
    Named(wire::ObjectRules),
    Positional {
        schema: wire::SchemaUse,
        min_items: Option<wire::Located<u64>>,
        max_items: Option<wire::Located<u64>>,
    },
}
#[derive(Debug, Clone)]
pub struct JavaAggregate {
    pub name: String,
    pub source: SourceId,
    pub multipart: bool,
    pub rules: AggregateRules,
    pub parts: Vec<JavaPart>,
    pub additional: Option<JavaPart>,
    pub descriptor: String,
}
#[derive(Debug, Clone)]
pub struct JavaMedia {
    pub name: String,
    pub value: JavaValue,
    pub wire: wire::MediaPlan,
    pub descriptor: String,
}

#[must_use]
pub fn capabilities(config: &ProtocolConfig) -> wire::Capabilities {
    use Capability::*;
    // Exhaustive opt-in list, not Capability::ALL: a shared addition cannot widen Java silently.
    let mut value = wire::Capabilities::for_adapter(
        "java-http-protocol-v1",
        [
            UnnamedOperations,
            AdditionalMethods,
            CustomMethods,
            HttpServers,
            RelativeServers,
            DocumentRelativeServers,
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
    .with_limits(config.limits);
    for profile in &config.compatibility_profiles {
        value = value.with_profile(*profile);
    }
    value
}

pub(crate) fn plan(
    contract: &Contract,
    selected: &[SourceId],
    config: &ProtocolConfig,
) -> Result<wire::ProtocolPlan, Vec<HttpDiagnostic>> {
    admitted(contract, selected, capabilities(config))
}

/// Resource-aware capabilities for the separately verified V3 entrypoint.
#[must_use]
pub fn capabilities_v3(config: &ProtocolConfig) -> wire::Capabilities {
    capabilities(config)
        .with(Capability::SchemaResources)
        .with(Capability::DynamicSchemaReferences)
}
pub(crate) fn plan_v3(
    contract: &Contract,
    selected: &[SourceId],
    config: &ProtocolConfig,
) -> Result<wire::ProtocolPlan, Vec<HttpDiagnostic>> {
    admitted(contract, selected, capabilities_v3(config))
}
fn admitted(
    contract: &Contract,
    selected: &[SourceId],
    capabilities: wire::Capabilities,
) -> Result<wire::ProtocolPlan, Vec<HttpDiagnostic>> {
    let plan = wire::plan(contract, selected, capabilities)
        .into_result()
        .map_err(|errors| {
            errors
                .into_iter()
                .filter(|e| e.severity() == wire::Severity::Error)
                .map(|e| HttpDiagnostic {
                    source: e.source().source().clone(),
                    at: e.source().span(),
                    code: e.code(),
                    message: e.message().into(),
                })
                .collect::<Vec<_>>()
        })?;
    let mut errors = Vec::new();
    for op in plan.operations() {
        factory_limit(
            contract,
            op.source().terminal().source(),
            op.parameters().iter().filter(|p| p.required()).count()
                + usize::from(op.body().is_some_and(|b| b.required())),
            op.parameters().len() + usize::from(op.body().is_some()),
            &mut errors,
        );
        for response in op.responses() {
            factory_limit(
                contract,
                response.source().terminal().source(),
                response.headers().iter().filter(|h| h.required()).count(),
                response.headers().len(),
                &mut errors,
            );
        }
        if !valid_path(op.path()) {
            errors.push(super::diagnostic(contract,op.source().terminal().source().clone(),"java-path-unsupported","path literals require valid UTF-8 percent escapes, no controls/whitespace/backslashes, and no dot traversal segments"));
        }
        if op.method().as_str() == "CONNECT" {
            errors.push(super::diagnostic(
                contract,
                op.source().terminal().source().clone(),
                "java-connect-tunnel-unsupported",
                "CONNECT authority/tunnel exchanges are not ordinary path/response SDK operations",
            ));
        }
        for parameter in op.parameters() {
            if parameter.location() == wire::ParameterLocation::Header
                && [
                    "host",
                    "content-length",
                    "connection",
                    "expect",
                    "upgrade",
                    "transfer-encoding",
                ]
                .contains(&parameter.name().to_ascii_lowercase().as_str())
            {
                errors.push(super::diagnostic(contract,parameter.source().terminal().source().clone(),"java-transport-header-unsupported","this HTTP framing/authority header is managed by the JDK transport and cannot be emitted as an ordinary parameter"));
            }
        }
        for header in op.responses().iter().flat_map(|r| r.headers()) {
            if header.name().eq_ignore_ascii_case("set-cookie") {
                errors.push(super::diagnostic(contract,header.source().terminal().source().clone(),"java-set-cookie-header-unsupported","Set-Cookie has a repeat-field grammar; a generic comma-joined typed header would be lossy"));
            }
            if matches!(
                header.serialization(),
                wire::ParameterSerialization::Style {
                    shape: wire::WireShape::FlatObject {
                        additional: wire::AdditionalScalars::AnyScalar,
                        ..
                    },
                    ..
                }
            ) {
                errors.push(super::diagnostic(contract,header.source().terminal().source().clone(),"java-header-decoding-ambiguous","untyped additional scalar header values require an explicit decoding policy; lexical text cannot identify number, boolean and string domains"));
            }
        }
        for media in op.responses().iter().flat_map(|r| r.media()) {
            let (parts, additional) = match media.representation() {
                Representation::Form { form } => (form.fields(), Some(form.additional())),
                Representation::Multipart {
                    multipart:
                        wire::MultipartPlan::Named {
                            parts, additional, ..
                        },
                } => (parts.as_slice(), Some(additional)),
                Representation::Multipart {
                    multipart: wire::MultipartPlan::Positional { prefix, items, .. },
                } => (prefix.as_slice(), Some(items)),
                _ => ([].as_slice(), None),
            };
            let extra = additional.and_then(|v| match v {
                AdditionalParts::Allowed(p) => Some(p.as_ref()),
                AdditionalParts::Forbidden => None,
            });
            for part in parts.iter().chain(extra) {
                if matches!(part.representation(),PartRepresentation::Style{serialization,..} if ambiguous_scalars(serialization))
                {
                    errors.push(super::diagnostic(contract,part.source().terminal().source().clone(),"java-part-decoding-ambiguous","style-encoded part object extras require an explicit scalar type; lexical fields cannot identify arbitrary scalar domains"));
                }
                for header in part.headers() {
                    if header.name().eq_ignore_ascii_case("set-cookie")
                        || ambiguous_scalars(header.serialization())
                    {
                        errors.push(super::diagnostic(contract,header.source().terminal().source().clone(),"java-part-header-decoding-ambiguous","this part header needs an explicit repeat-field or additional scalar decoding policy"));
                    }
                }
            }
        }
        let mut medias = op
            .body()
            .into_iter()
            .flat_map(|b| b.media())
            .chain(op.responses().iter().flat_map(|r| r.media()));
        for media in medias.by_ref() {
            let (parts, additional) = match media.representation() {
                Representation::Form { form } => (form.fields(), Some(form.additional())),
                Representation::Multipart {
                    multipart:
                        wire::MultipartPlan::Named {
                            parts, additional, ..
                        },
                } => (parts.as_slice(), Some(additional)),
                Representation::Multipart {
                    multipart: wire::MultipartPlan::Positional { prefix, items, .. },
                } => (prefix.as_slice(), Some(items)),
                _ => ([].as_slice(), None),
            };
            factory_limit(
                contract,
                media.source().terminal().source(),
                parts.iter().filter(|p| p.required()).count(),
                parts.len(),
                &mut errors,
            );
            let extra = additional.and_then(|p| match p {
                AdditionalParts::Allowed(p) => Some(p.as_ref()),
                AdditionalParts::Forbidden => None,
            });
            for part in parts.iter().chain(extra) {
                factory_limit(
                    contract,
                    part.source().terminal().source(),
                    part.headers().iter().filter(|h| h.required()).count(),
                    part.headers().len(),
                    &mut errors,
                );
            }
            if let Representation::Multipart {
                multipart:
                    wire::MultipartPlan::Positional {
                        prefix, max_items, ..
                    },
            } = media.representation()
                && (prefix.len() > 512 || max_items.as_ref().is_none_or(|n| *n.value() > 4096))
            {
                errors.push(super::diagnostic(contract,media.source().terminal().source().clone(),"java-positional-multipart-limit","positional multipart requires a finite maxItems at most 4096 and at most 512 named prefix positions"));
            }
        }
    }
    if errors.is_empty() {
        Ok(plan)
    } else {
        Err(errors)
    }
}
fn factory_limit(
    contract: &Contract,
    source: &SourceId,
    required: usize,
    fields: usize,
    errors: &mut Vec<HttpDiagnostic>,
) {
    if required > 200 || fields > 512 {
        errors.push(super::diagnostic(contract,source.clone(),"java-constructor-resource-limit","native input/header/aggregate factories support at most 200 required arguments and 512 fields within JVM method limits"));
    }
}

fn ambiguous_scalars(serialization: &wire::ParameterSerialization) -> bool {
    matches!(
        serialization,
        wire::ParameterSerialization::Style {
            shape: wire::WireShape::FlatObject {
                additional: wire::AdditionalScalars::AnyScalar,
                ..
            },
            ..
        }
    )
}
fn valid_path(path: &str) -> bool {
    if path
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return false;
    }
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let digit = |b: u8| char::from(b).to_digit(16).map(|v| v as u8);
            let Some(high) = bytes.get(index + 1).and_then(|b| digit(*b)) else {
                return false;
            };
            let Some(low) = bytes.get(index + 2).and_then(|b| digit(*b)) else {
                return false;
            };
            decoded.push(high * 16 + low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    let Ok(value) = std::str::from_utf8(&decoded) else {
        return false;
    };
    !value.chars().any(char::is_control) && !value.split('/').any(|v| matches!(v, "." | ".."))
}

pub(crate) struct Names {
    pub used: BTreeSet<String>,
}
impl Names {
    pub(crate) fn new(models: &JavaModelPlan, client: &str) -> Self {
        let mut used = reserved_types();
        used.extend(models.names().values().cloned());
        used.insert(client.into());
        Self { used }
    }
    pub(crate) fn name(&mut self, base: &str) -> String {
        allocate(&bounded(base), &mut self.used)
    }
    pub(crate) fn headers(
        &mut self,
        base: &str,
        source: SourceId,
        headers: &[wire::HeaderPlan],
        descriptor: String,
    ) -> Option<JavaHeaders> {
        if headers.is_empty() {
            return None;
        }
        let name = self.name(base);
        let mut used = reserved_members();
        used.insert("values".into());
        Some(JavaHeaders {
            name,
            source,
            descriptor,
            fields: headers
                .iter()
                .map(|header| JavaHeader {
                    name: allocate(&member(header.name()), &mut used),
                    schema: header.codec().schema().id().clone(),
                    wire: header.clone(),
                })
                .collect(),
        })
    }
    pub(crate) fn media(
        &mut self,
        base: &str,
        media: &[wire::MediaPlan],
        request: bool,
        path: &str,
    ) -> Vec<JavaMedia> {
        let mut members = self.used.clone();
        media
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let descriptor = format!("{path}/{i}");
                let label = allocate(&media_name(m.media_type()), &mut members);
                let value = match m.representation() {
                    Representation::Json { codec } => codec.as_ref().map_or(JavaValue::Json, |c| {
                        JavaValue::Model(c.schema().id().clone())
                    }),
                    Representation::Text { codec, .. } => {
                        codec.as_ref().map_or(JavaValue::Text, |c| {
                            JavaValue::Model(c.schema().id().clone())
                        })
                    }
                    Representation::Binary { .. } => JavaValue::Bytes,
                    Representation::Stream { stream } => JavaValue::Stream {
                        schema: stream.item_codec().schema().id().clone(),
                        request,
                    },
                    Representation::Form { form } => self.aggregate(
                        &format!("{base}Form"),
                        m,
                        false,
                        AggregateRules::Named(form.rules().clone()),
                        (form.fields(), form.additional()),
                        &format!("{descriptor}/representation/form"),
                    ),
                    Representation::Multipart { multipart } => match multipart {
                        wire::MultipartPlan::Named {
                            rules,
                            parts,
                            additional,
                        } => self.aggregate(
                            &format!("{base}Multipart"),
                            m,
                            true,
                            AggregateRules::Named(rules.clone()),
                            (parts, additional),
                            &format!("{descriptor}/representation/multipart"),
                        ),
                        wire::MultipartPlan::Positional {
                            schema,
                            prefix,
                            items,
                            min_items,
                            max_items,
                        } => self.aggregate(
                            &format!("{base}Multipart"),
                            m,
                            true,
                            AggregateRules::Positional {
                                schema: schema.clone(),
                                min_items: min_items.clone(),
                                max_items: max_items.clone(),
                            },
                            (prefix, items),
                            &format!("{descriptor}/representation/multipart"),
                        ),
                    },
                };
                JavaMedia {
                    name: label,
                    value,
                    wire: m.clone(),
                    descriptor,
                }
            })
            .collect()
    }
    fn aggregate(
        &mut self,
        base: &str,
        media: &wire::MediaPlan,
        multipart: bool,
        rules: AggregateRules,
        (parts, additional): (&[wire::PartPlan], &AdditionalParts),
        descriptor: &str,
    ) -> JavaValue {
        let name = self.name(base);
        let mut used = reserved_members();
        used.extend([
            "parts".into(),
            "items".into(),
            "additional".into(),
            "additionalParts".into(),
            "addItem".into(),
            "putAdditionalPart".into(),
        ]);
        let parts = parts
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let member = allocate(
                    &member(
                        &p.name()
                            .map_or_else(|| format!("part{}", i + 1), str::to_owned),
                    ),
                    &mut used,
                );
                self.part(
                    &format!("{name}{}", crate::rust_models::pascal(&member)),
                    member,
                    p,
                    multipart,
                    &format!(
                        "{descriptor}/{}/{i}",
                        if matches!(&rules, AggregateRules::Positional { .. }) {
                            "prefix"
                        } else if multipart {
                            "parts"
                        } else {
                            "fields"
                        }
                    ),
                )
            })
            .collect();
        let additional = match additional {
            AdditionalParts::Forbidden => None,
            AdditionalParts::Allowed(p) => Some(self.part(
                &format!("{name}Additional"),
                "additional".into(),
                p,
                multipart,
                &format!(
                    "{descriptor}/{}/part",
                    if matches!(&rules, AggregateRules::Positional { .. }) {
                        "items"
                    } else {
                        "additional"
                    }
                ),
            )),
        };
        JavaValue::Aggregate(Box::new(JavaAggregate {
            name,
            source: media.source().use_site().source().clone(),
            multipart,
            rules,
            parts,
            additional,
            descriptor: descriptor.into(),
        }))
    }
    fn part(
        &mut self,
        base: &str,
        name: String,
        part: &wire::PartPlan,
        multipart: bool,
        descriptor: &str,
    ) -> JavaPart {
        let value = match part.representation() {
            PartRepresentation::Json { codec, .. }
            | PartRepresentation::Text { codec, .. }
            | PartRepresentation::Style { codec, .. } => {
                JavaValue::Model(codec.schema().id().clone())
            }
            PartRepresentation::Binary { .. } => JavaValue::Bytes,
        };
        let wrapper = multipart.then(|| self.name(&format!("{base}Part")));
        let headers = self.headers(
            &format!("{base}Headers"),
            part.source().use_site().source().clone(),
            part.headers(),
            format!("{descriptor}/headers"),
        );
        JavaPart {
            name,
            wrapper,
            value,
            headers,
            wire: part.clone(),
            descriptor: descriptor.into(),
        }
    }
}

pub(crate) fn media_name(media: &wire::MediaType) -> String {
    let name = match media.range() {
        wire::MediaRange::Any => "Binary".into(),
        wire::MediaRange::Type { type_name } => {
            format!("{}Range", crate::rust_models::pascal(type_name))
        }
        wire::MediaRange::Concrete { type_name, subtype } if media.is_json() => {
            if subtype == "json" {
                "Json".into()
            } else {
                crate::rust_models::pascal(subtype)
            }
        }
        wire::MediaRange::Concrete { type_name, subtype }
            if type_name == "text" && subtype == "plain" =>
        {
            "Text".into()
        }
        wire::MediaRange::Concrete { type_name, subtype }
            if type_name == "application" && subtype == "octet-stream" =>
        {
            "Binary".into()
        }
        wire::MediaRange::Concrete { type_name, subtype } => {
            crate::rust_models::pascal(&format!("{type_name}_{subtype}"))
        }
    };
    if media.parameters().is_empty() {
        name
    } else {
        format!(
            "{name}{}",
            crate::rust_models::pascal(
                &media
                    .parameters()
                    .values()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("_")
            )
        )
    }
}
pub(crate) fn needs_choice(media: &[JavaMedia]) -> bool {
    media.len() != 1
        || !matches!(
            media[0].wire.media_type().range(),
            wire::MediaRange::Concrete { .. }
        )
}
pub(crate) fn can_succeed(status: wire::ResponseStatus) -> bool {
    matches!(
        status,
        wire::ResponseStatus::Exact(200..=299)
            | wire::ResponseStatus::Range(2)
            | wire::ResponseStatus::Default
    )
}
pub(crate) fn can_fail(status: wire::ResponseStatus) -> bool {
    !matches!(
        status,
        wire::ResponseStatus::Exact(200..=299) | wire::ResponseStatus::Range(2)
    )
}
pub(crate) fn always_empty(method: &wire::Method, status: wire::ResponseStatus) -> bool {
    method == wire::Method::Head
        || matches!(
            status,
            wire::ResponseStatus::Exact(100..=199 | 204 | 205 | 304)
                | wire::ResponseStatus::Range(1)
        )
}
pub(crate) fn may_empty(method: &wire::Method, status: wire::ResponseStatus) -> bool {
    always_empty(method, status)
        || matches!(
            status,
            wire::ResponseStatus::Range(2) | wire::ResponseStatus::Default
        )
}

pub(crate) fn header_groups(
    operations: &[super::http::JavaOperation],
) -> BTreeMap<String, JavaHeaders> {
    let mut out = BTreeMap::new();
    fn visit(value: &JavaValue, out: &mut BTreeMap<String, JavaHeaders>) {
        if let JavaValue::Aggregate(a) = value {
            for p in a.parts.iter().chain(a.additional.iter()) {
                if let Some(h) = &p.headers {
                    out.insert(h.name.clone(), h.clone());
                }
            }
        }
    }
    for op in operations {
        for response in &op.responses {
            if let Some(h) = &response.headers {
                out.insert(h.name.clone(), h.clone());
            }
            for media in &response.media {
                visit(&media.value, &mut out);
            }
        }
        if let Some(body) = &op.body {
            for media in &body.media {
                visit(&media.value, &mut out);
            }
        }
    }
    out
}
