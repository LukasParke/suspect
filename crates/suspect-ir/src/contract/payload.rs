//! Request and response representations, borrowing the same source documents.

use serde_json::Value;

use super::http::Object;
use super::{Contract, Operation, Parameter, ParameterStyle, Schema, SchemaId, Server, SourceId};

/// A response's exact HTTP status, status-class range, or default fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResponseStatus {
    /// An exact status from 100 through 599.
    Exact(u16),
    /// A class from 1 through 5, written `1XX` through `5XX` in OpenAPI.
    Range(u8),
    /// The response for statuses not covered by an exact code or range.
    Default,
}

impl ResponseStatus {
    pub(super) fn parse(value: &str) -> Option<Self> {
        if value == "default" {
            return Some(Self::Default);
        }
        let bytes = value.as_bytes();
        if bytes.len() != 3 || !(b'1'..=b'5').contains(&bytes[0]) {
            return None;
        }
        if &bytes[1..] == b"XX" {
            return Some(Self::Range(bytes[0] - b'0'));
        }
        if bytes.iter().all(u8::is_ascii_digit) {
            return value.parse().ok().map(Self::Exact);
        }
        None
    }
}

impl<'a> Operation<'a> {
    /// Declared request body, including references and every media type.
    pub fn request_body(&self) -> Option<RequestBody<'a>> {
        let object = self.object();
        Some(RequestBody {
            object: Object::new(object.contract, object.field("requestBody")?.0),
        })
    }

    /// Responses in stable key order. Invalid status keys remain visible with
    /// `status() == None` and a contract error. Specification extensions are raw.
    pub fn responses(&self) -> Vec<Response<'a>> {
        self.object()
            .named("responses")
            .into_iter()
            .filter(|(key, _)| !key.starts_with("x-"))
            .map(|(key, object)| Response { key, object })
            .collect()
    }
}

impl super::Contract {
    /// Named Example Objects belonging to an indexed HTTP container. References
    /// retain definition/use-site identity; instance data is never scanned.
    pub fn examples_at(&self, source: &SourceId) -> Vec<Example<'_>> {
        examples(&Object::new(self, source.clone()))
    }
}

impl<'a> Parameter<'a> {
    /// All content entries; valid content-based parameters declare exactly one.
    pub fn content(&self) -> Vec<MediaType<'a>> {
        content(&self.object)
    }
    /// Inline example, including explicit null and arbitrary-precision numbers.
    pub fn example(&self) -> Option<&'a Value> {
        self.object.field("example").map(|(_, v)| v)
    }
    /// Actual value location, including a referenced parameter definition.
    pub fn example_source(&self) -> Option<SourceId> {
        self.object.field("example").map(|(source, _)| source)
    }
    /// Named Example Objects, with references interpreted only at this level.
    pub fn examples(&self) -> Vec<Example<'a>> {
        examples(&self.object)
    }
}

/// An OpenAPI Request Body Object or reference to one.
#[derive(Debug, Clone)]
pub struct RequestBody<'a> {
    object: Object<'a>,
}
impl<'a> RequestBody<'a> {
    pub fn source(&self) -> &SourceId {
        &self.object.source
    }
    pub fn raw(&self) -> &'a Value {
        self.object.raw()
    }
    /// Terminal reference target; the declaration remains available via source/raw.
    pub fn resolved_source(&self) -> Option<SourceId> {
        self.object.resolved_source()
    }
    pub fn description(&self) -> Option<&'a str> {
        self.object.string("description")
    }
    pub fn required(&self) -> Option<bool> {
        self.object.boolean("required")
    }
    pub fn content(&self) -> Vec<MediaType<'a>> {
        content(&self.object)
    }
}

/// A response declaration retaining its original status key and source.
#[derive(Debug, Clone)]
pub struct Response<'a> {
    key: &'a str,
    object: Object<'a>,
}
impl<'a> Response<'a> {
    pub fn source(&self) -> &SourceId {
        &self.object.source
    }
    pub fn raw(&self) -> &'a Value {
        self.object.raw()
    }
    /// Terminal reference target; the response map entry remains the declaration source.
    pub fn resolved_source(&self) -> Option<SourceId> {
        self.object.resolved_source()
    }
    pub fn status_key(&self) -> &'a str {
        self.key
    }
    pub fn status(&self) -> Option<ResponseStatus> {
        ResponseStatus::parse(self.key)
    }
    pub fn description(&self) -> Option<&'a str> {
        self.object.string("description")
    }
    /// Response summary, introduced by OpenAPI 3.2.
    pub fn summary(&self) -> Option<&'a str> {
        self.object.field_32("summary")?.1.as_str()
    }
    pub fn content(&self) -> Vec<MediaType<'a>> {
        content(&self.object)
    }
    pub fn headers(&self) -> Vec<Header<'a>> {
        headers(&self.object)
    }
    /// Links remain source-addressed and preserve runtime expressions as text.
    pub fn links(&self) -> Vec<Link<'a>> {
        self.object
            .named("links")
            .into_iter()
            .map(|(name, object)| Link { name, object })
            .collect()
    }
}

/// One content-map entry. Media names, schemas, examples, and encoding remain
/// distinct; no preferred media type is selected by the contract layer.
#[derive(Debug, Clone)]
pub struct MediaType<'a> {
    name: &'a str,
    object: Object<'a>,
}
impl<'a> MediaType<'a> {
    pub fn name(&self) -> &'a str {
        self.name
    }
    pub fn source(&self) -> &SourceId {
        &self.object.source
    }
    pub fn raw(&self) -> &'a Value {
        self.object.raw()
    }
    /// Terminal Media Type Object, preserving the content-map reference mount
    /// separately in `source()`. Media references require OpenAPI 3.2.
    pub fn resolved_source(&self) -> Option<SourceId> {
        self.object.resolved_source()
    }
    pub fn schema(&self) -> Option<Schema<'a>> {
        self.object.schema()
    }
    /// Per-item schema for sequential content. It is independent of `schema`,
    /// which describes the complete content; neither is substituted for the other.
    pub fn item_schema(&self) -> Option<Schema<'a>> {
        self.object
            .contract
            .schema(&self.object.field_32("itemSchema")?.0)
    }
    /// Declared complete-content and per-item roots, ready for `reachable_from`.
    pub fn schema_roots(&self) -> Vec<SchemaId> {
        [self.schema(), self.item_schema()]
            .into_iter()
            .flatten()
            .map(|schema| schema.id().clone())
            .collect()
    }
    pub fn example(&self) -> Option<&'a Value> {
        self.object.field("example").map(|(_, v)| v)
    }
    pub fn examples(&self) -> Vec<Example<'a>> {
        examples(&self.object)
    }
    pub fn encoding(&self) -> Vec<Encoding<'a>> {
        encodings(&self.object)
    }
    /// Positional multipart encodings, in their original array order (3.2).
    pub fn prefix_encoding(&self) -> Vec<EncodingObject<'a>> {
        prefix_encodings(&self.object)
    }
    /// Encoding for remaining multipart items (3.2).
    pub fn item_encoding(&self) -> Option<EncodingObject<'a>> {
        item_encoding(&self.object)
    }
}

impl Contract {
    /// Reusable Media Type Objects in the entry's `components.mediaTypes` (3.2).
    /// Here `name()` is the component key, not a wire media type. Content-map
    /// views retain their own media type names when referencing these definitions.
    pub fn media_types(&self) -> Vec<MediaType<'_>> {
        if !self.openapi_version().starts_with("3.2.") {
            return Vec::new();
        }
        Object::new(
            self,
            SourceId::new(self.entry().clone(), suspect_low::Pointer::root()).child("components"),
        )
        .named("mediaTypes")
        .into_iter()
        .map(|(name, object)| MediaType { name, object })
        .collect()
    }
}

/// A response or encoding header, whose map key supplies its name.
#[derive(Debug, Clone)]
pub struct Header<'a> {
    name: &'a str,
    object: Object<'a>,
}
impl<'a> Header<'a> {
    pub fn name(&self) -> &'a str {
        self.name
    }
    pub fn source(&self) -> &SourceId {
        &self.object.source
    }
    pub fn raw(&self) -> &'a Value {
        self.object.raw()
    }
    pub fn resolved_source(&self) -> Option<SourceId> {
        self.object.resolved_source()
    }
    pub fn description(&self) -> Option<&'a str> {
        self.object.string("description")
    }
    pub fn required(&self) -> Option<bool> {
        self.object.boolean("required")
    }
    pub fn deprecated(&self) -> Option<bool> {
        self.object.boolean("deprecated")
    }
    pub fn style(&self) -> Option<ParameterStyle> {
        self.object.style()
    }
    pub fn explode(&self) -> Option<bool> {
        self.object.boolean("explode")
    }
    pub fn effective_style(&self) -> Option<ParameterStyle> {
        if self.object.field("content").is_some() {
            return None;
        }
        if self.object.field("style").is_some() {
            self.style()
        } else {
            Some(ParameterStyle::Simple)
        }
    }
    pub fn effective_explode(&self) -> Option<bool> {
        if self.object.field("content").is_some() {
            return None;
        }
        if self.object.field("explode").is_some() {
            self.explode()
        } else {
            self.effective_style().map(|_| false)
        }
    }
    pub fn schema(&self) -> Option<Schema<'a>> {
        self.object.schema()
    }
    pub fn content(&self) -> Vec<MediaType<'a>> {
        content(&self.object)
    }
    pub fn example(&self) -> Option<&'a Value> {
        self.object.field("example").map(|(_, v)| v)
    }
    pub fn examples(&self) -> Vec<Example<'a>> {
        examples(&self.object)
    }
}

/// One multipart/form encoding entry, keyed by schema property name.
#[derive(Debug, Clone)]
pub struct Encoding<'a> {
    name: &'a str,
    object: Object<'a>,
}
impl<'a> Encoding<'a> {
    pub fn name(&self) -> &'a str {
        self.name
    }
    pub fn source(&self) -> &SourceId {
        &self.object.source
    }
    pub fn raw(&self) -> &'a Value {
        self.object.raw()
    }
    pub fn content_type(&self) -> Option<&'a str> {
        self.object.string("contentType")
    }
    pub fn headers(&self) -> Vec<Header<'a>> {
        headers(&self.object)
    }
    pub fn style(&self) -> Option<ParameterStyle> {
        self.object.style()
    }
    pub fn explode(&self) -> Option<bool> {
        self.object.boolean("explode")
    }
    pub fn allow_reserved(&self) -> Option<bool> {
        self.object.boolean("allowReserved")
    }
    pub fn effective_style(&self) -> Option<ParameterStyle> {
        self.object.encoding_style()
    }
    pub fn effective_explode(&self) -> Option<bool> {
        if self.object.field("explode").is_some() {
            self.explode()
        } else {
            self.effective_style()
                .map(|style| style == ParameterStyle::Form)
        }
    }
    /// Nested named encodings (3.2).
    pub fn encoding(&self) -> Vec<Encoding<'a>> {
        if self.object.field_32("encoding").is_none() {
            return Vec::new();
        }
        encodings(&self.object)
    }
    pub fn prefix_encoding(&self) -> Vec<EncodingObject<'a>> {
        prefix_encodings(&self.object)
    }
    pub fn item_encoding(&self) -> Option<EncodingObject<'a>> {
        item_encoding(&self.object)
    }
}

/// An Encoding Object in a positional field. Its source and array position
/// identify it; no synthetic property name is invented for an unnamed part.
#[derive(Debug, Clone)]
pub struct EncodingObject<'a> {
    object: Object<'a>,
}

impl<'a> EncodingObject<'a> {
    pub fn source(&self) -> &SourceId {
        &self.object.source
    }
    pub fn raw(&self) -> &'a Value {
        self.object.raw()
    }
    pub fn content_type(&self) -> Option<&'a str> {
        self.object.string("contentType")
    }
    pub fn headers(&self) -> Vec<Header<'a>> {
        headers(&self.object)
    }
    pub fn style(&self) -> Option<ParameterStyle> {
        self.object.style()
    }
    pub fn explode(&self) -> Option<bool> {
        self.object.boolean("explode")
    }
    pub fn allow_reserved(&self) -> Option<bool> {
        self.object.boolean("allowReserved")
    }
    pub fn effective_style(&self) -> Option<ParameterStyle> {
        self.object.encoding_style()
    }
    pub fn effective_explode(&self) -> Option<bool> {
        if self.object.field("explode").is_some() {
            self.explode()
        } else {
            self.effective_style()
                .map(|style| style == ParameterStyle::Form)
        }
    }
    pub fn encoding(&self) -> Vec<Encoding<'a>> {
        if self.object.field_32("encoding").is_none() {
            return Vec::new();
        }
        encodings(&self.object)
    }
    pub fn prefix_encoding(&self) -> Vec<EncodingObject<'a>> {
        prefix_encodings(&self.object)
    }
    pub fn item_encoding(&self) -> Option<EncodingObject<'a>> {
        item_encoding(&self.object)
    }
}

/// A named Example Object, preserving inline instance data without traversal.
#[derive(Debug, Clone)]
pub struct Example<'a> {
    name: &'a str,
    object: Object<'a>,
}
impl<'a> Example<'a> {
    pub fn name(&self) -> &'a str {
        self.name
    }
    pub fn source(&self) -> &SourceId {
        &self.object.source
    }
    pub fn raw(&self) -> &'a Value {
        self.object.raw()
    }
    pub fn summary(&self) -> Option<&'a str> {
        self.object.string("summary")
    }
    pub fn description(&self) -> Option<&'a str> {
        self.object.string("description")
    }
    pub fn value(&self) -> Option<&'a Value> {
        self.object.field("value").map(|(_, v)| v)
    }
    /// Actual value location, including a referenced Example Object.
    pub fn value_source(&self) -> Option<SourceId> {
        self.object.field("value").map(|(source, _)| source)
    }
    /// Terminal Example Object location, without reinterpreting instance data.
    pub fn resolved_source(&self) -> Option<SourceId> {
        self.object.resolved_source()
    }
    pub fn external_value(&self) -> Option<&'a str> {
        self.object.string("externalValue")
    }
    /// Schema-ready instance data (3.2), kept distinct from serialized examples.
    pub fn data_value(&self) -> Option<&'a Value> {
        self.object.field_32("dataValue").map(|(_, value)| value)
    }
    pub fn data_value_source(&self) -> Option<SourceId> {
        self.object.field_32("dataValue").map(|(source, _)| source)
    }
    pub fn serialized_value(&self) -> Option<&'a str> {
        self.object.field_32("serializedValue")?.1.as_str()
    }
    pub fn serialized_value_source(&self) -> Option<SourceId> {
        self.object
            .field_32("serializedValue")
            .map(|(source, _)| source)
    }
}

/// A Link Object connecting response data to another operation.
#[derive(Debug, Clone)]
pub struct Link<'a> {
    name: &'a str,
    object: Object<'a>,
}
impl<'a> Link<'a> {
    pub fn name(&self) -> &'a str {
        self.name
    }
    pub fn source(&self) -> &SourceId {
        &self.object.source
    }
    pub fn raw(&self) -> &'a Value {
        self.object.raw()
    }
    pub fn description(&self) -> Option<&'a str> {
        self.object.string("description")
    }
    pub fn operation_id(&self) -> Option<&'a str> {
        self.object.string("operationId")
    }
    pub fn operation_ref(&self) -> Option<&'a str> {
        self.object.string("operationRef")
    }
    pub fn parameters(&self) -> Option<&'a serde_json::Map<String, Value>> {
        self.object.field("parameters")?.1.as_object()
    }
    pub fn request_body(&self) -> Option<&'a Value> {
        self.object.field("requestBody").map(|(_, v)| v)
    }
    pub fn server(&self) -> Option<Server<'a>> {
        Some(Server {
            object: Some(Object::new(
                self.object.contract,
                self.object.field("server")?.0,
            )),
        })
    }
}

fn content<'a>(object: &Object<'a>) -> Vec<MediaType<'a>> {
    object
        .named("content")
        .into_iter()
        .map(|(name, object)| MediaType { name, object })
        .collect()
}
fn encodings<'a>(object: &Object<'a>) -> Vec<Encoding<'a>> {
    object
        .named("encoding")
        .into_iter()
        .map(|(name, object)| Encoding { name, object })
        .collect()
}
fn prefix_encodings<'a>(object: &Object<'a>) -> Vec<EncodingObject<'a>> {
    let Some((source, value)) = object.field_32("prefixEncoding") else {
        return Vec::new();
    };
    value
        .as_array()
        .into_iter()
        .flat_map(|array| 0..array.len())
        .map(|index| EncodingObject {
            object: Object::new(object.contract, source.child(&index.to_string())),
        })
        .collect()
}
fn item_encoding<'a>(object: &Object<'a>) -> Option<EncodingObject<'a>> {
    Some(EncodingObject {
        object: Object::new(object.contract, object.field_32("itemEncoding")?.0),
    })
}
fn headers<'a>(object: &Object<'a>) -> Vec<Header<'a>> {
    object
        .named("headers")
        .into_iter()
        .map(|(name, object)| Header { name, object })
        .collect()
}
fn examples<'a>(object: &Object<'a>) -> Vec<Example<'a>> {
    object
        .named("examples")
        .into_iter()
        .map(|(name, object)| Example { name, object })
        .collect()
}
