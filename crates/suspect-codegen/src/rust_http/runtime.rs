//! Source-bound HTTP execution with finite request, response, part and stream budgets.
//!
//! Credentials and servers are caller-owned. Each operation owns its exchange;
//! dropping a pending call or an item stream drops the response body. The runtime
//! starts no tasks, performs no retries and infers no acquisition or paging loops.

#![forbid(unsafe_code)]

mod descriptors;
mod errors;
mod media;
mod parameters;
mod parts;
mod security;
mod servers;
mod stream;

pub use descriptors::*;
pub use errors::*;
pub use media::{Media, MediaKind, MediaRange, ParsedMedia, ResponseSpec, Status};
pub use parameters::{
    AdditionalScalars, Parameter, ParameterLocation, ParameterValue, PercentEncoding, ScalarType,
    Serialization, Shape, Style, decode_header, scalar_bytes, scalar_text, scalar_value,
    serialize_parameter,
};
pub use parts::{
    AggregateSpec, EncodedPart, ParsedParts, Part, PartKind, PartSpec, PartValue,
    check_cardinality, decode_part_value, encode_part, parse_parts, prepare_parts, validate_counts,
};
pub use security::{Credential, CredentialProvider, Credentials};
pub use stream::{Framing, ItemStream, encode_items};

use crate::{JsonLimits, JsonValue};
use std::{fmt, future::Future};

const MAX_EMPTY_CHUNKS: usize = 1_024;
const POLL_SLACK: usize = 4_096;
const DEFAULT_CAPTURE_BYTES: usize = 65_536;

/// Header list preserving duplicate fields and non-UTF-8 values.
pub type Headers = Vec<(String, Vec<u8>)>;
pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// An already validated single outgoing request. Debug omits URL/header/body
/// values because query and cookie credentials are also sensitive runtime data.
pub struct Request {
    pub method: &'static str,
    pub url: String,
    pub headers: Headers,
    pub body: Option<Vec<u8>>,
}
impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("url_bytes", &self.url.len())
            .field("headers", &HeaderSummary(&self.headers))
            .field("body_bytes", &self.body.as_ref().map(Vec::len))
            .finish()
    }
}

pub struct TransportResponse<B> {
    pub status: u16,
    pub headers: Headers,
    pub body: B,
}

/// Pull-based body. A transport must bound its own chunk allocation; the SDK
/// checks each returned chunk before copying or retaining it. Drop cancels it.
pub trait ResponseBody: Send {
    fn next_chunk(&mut self) -> impl Future<Output = Result<Option<Vec<u8>>, BoxError>> + Send;
}
/// One request and a pull-based response; adapters must not retry, redirect or
/// prebuffer an entire response. No executor is required by the core runtime.
pub trait Transport: Send + Sync {
    type Body: ResponseBody;
    fn send(
        &self,
        request: Request,
    ) -> impl Future<Output = Result<TransportResponse<Self::Body>, BoxError>> + Send;
}

/// Explicit runtime selection and lower resource limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientOptions {
    /// Absolute server override. HTTP overrides target loopback; declared HTTP
    /// candidates retain their declared origins. Cannot be combined with a
    /// candidate index or variable overrides.
    pub server_url: Option<String>,
    pub server_index: Option<usize>,
    pub server_variables: std::collections::BTreeMap<String, String>,
    /// URL serving a local-file description when its server is relative.
    pub document_url: Option<String>,
    pub max_response_bytes: Option<usize>,
    pub max_part_bytes: Option<usize>,
    pub max_stream_item_bytes: Option<usize>,
    pub max_chunk_bytes: Option<usize>,
    pub max_error_capture_bytes: usize,
    /// Full User-Agent override. `Some(empty)` suppresses the automatic
    /// attribution header entirely.
    pub user_agent: Option<String>,
    /// Replaces the SDK identity token in the automatic attribution header:
    /// `<name>` or `<name>/<version>` of RFC 9110 tokens.
    pub application_id: Option<String>,
}
impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            server_url: None,
            server_index: None,
            server_variables: Default::default(),
            document_url: None,
            max_response_bytes: None,
            max_part_bytes: None,
            max_stream_item_bytes: None,
            max_chunk_bytes: None,
            max_error_capture_bytes: DEFAULT_CAPTURE_BYTES,
            user_agent: None,
            application_id: None,
        }
    }
}

/// ua/v1 application identity: `<name>` or `<name>/<version>` of RFC 9110 tokens.
fn is_application_identity(value: &str) -> bool {
    const TOKEN_EXTRA: &[u8] = b"!#$%&'*+-.^_`|~";
    let token = |part: &str| {
        !part.is_empty()
            && part.len() <= 128
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || TOKEN_EXTRA.contains(&byte))
    };
    match value.split_once('/') {
        Some((name, version)) => token(name) && token(version),
        None => token(value),
    }
}

/// ua/v1 attribution: explicit override wins, `Some(empty)` suppresses, and the
/// default identifies suspect as the generator and the SDK package or a
/// caller-supplied application as the client.
fn resolve_user_agent(options: &ClientOptions) -> Option<String> {
    use crate::attribution::{
        ATTRIBUTION_SDK_NAME, ATTRIBUTION_SDK_VERSION, ATTRIBUTION_SPEC_VERSION,
        ATTRIBUTION_SUSPECT_VERSION,
    };
    if let Some(user_agent) = &options.user_agent {
        return if user_agent.is_empty() {
            None
        } else {
            Some(user_agent.clone())
        };
    }
    if ATTRIBUTION_SUSPECT_VERSION.is_empty() {
        return None;
    }
    let identity = match &options.application_id {
        Some(application_id) if !application_id.is_empty() => {
            if application_id.len() > 128 || !is_application_identity(application_id) {
                return None;
            }
            application_id.clone()
        }
        _ => format!("{ATTRIBUTION_SDK_NAME}/{ATTRIBUTION_SDK_VERSION}"),
    };
    Some(format!(
        "suspect/{ATTRIBUTION_SUSPECT_VERSION} {identity} (rust/unknown; openapi/{ATTRIBUTION_SPEC_VERSION})"
    ))
}

pub struct Client<T> {
    transport: T,
    credentials: Credentials,
    options: ClientOptions,
}
impl<T> Client<T> {
    #[must_use]
    pub fn with_transport(transport: T, credentials: Credentials) -> Self {
        Self {
            transport,
            credentials,
            options: Default::default(),
        }
    }
    #[must_use]
    pub fn with_options(mut self, options: ClientOptions) -> Self {
        self.options = options;
        self
    }
    /// Check caller budgets before encoding or copying a body/part.
    pub fn limits(&self, op: &Operation) -> Result<Limits, SdkError> {
        let lower = |requested: Option<usize>, ceiling: usize| {
            if requested.is_some_and(|v| v > ceiling) {
                Err(representation_error(
                    op.source,
                    op.source,
                    "caller byte limit exceeds the generated ceiling",
                ))
            } else {
                Ok(requested.unwrap_or(ceiling))
            }
        };
        Ok(Limits {
            response: lower(self.options.max_response_bytes, op.limits.response)?,
            part: lower(self.options.max_part_bytes, op.limits.part)?,
            item: lower(self.options.max_stream_item_bytes, op.limits.item)?,
            chunk: lower(self.options.max_chunk_bytes, op.limits.chunk)?,
            ..op.limits
        })
    }
}

/// Prepared body selected through its declared representation. The constructor
/// checks concrete media precedence, including wildcard schema-bypass attempts.
pub struct PreparedBody {
    pub bytes: Vec<u8>,
    pub content_type: String,
}
impl PreparedBody {
    pub fn new(
        op: &Operation,
        index: usize,
        content_type: String,
        bytes: Vec<u8>,
    ) -> Result<Self, SdkError> {
        let declared = op.request_media.get(index).ok_or_else(|| {
            representation_error(op.source, op.source, "request media index is not declared")
        })?;
        let selected = media::select(&op.request_media, &content_type).map_err(|_| {
            representation_error(
                op.source,
                op.source,
                "request Content-Type is invalid or has no declared representation",
            )
        })?;
        if selected != index {
            return Err(representation_error(
                op.source,
                declared.source,
                "request media selects a different typed body representation",
            ));
        }
        if bytes.len() > op.limits.request {
            return Err(resource_error(
                op.source,
                declared.source,
                "request body exceeds its byte ceiling",
            ));
        }
        Ok(Self {
            bytes,
            content_type,
        })
    }
}

impl<T: Transport> Client<T> {
    /// Open the single exchange. Streaming operations retain this owned body;
    /// finite operations consume it under the same limits.
    pub async fn open(
        &self,
        op: &'static Operation,
        parameters: &[ParameterValue],
        body: Option<PreparedBody>,
    ) -> Result<Exchange<T::Body>, SdkError> {
        let limits = self.limits(op)?;
        let base = servers::resolve(op, &self.options)?;
        let mut path = op.path_template.to_string();
        let mut query = Vec::new();
        let mut cookies = Vec::new();
        let mut headers = Vec::new();
        let mut serialized_bytes = 0usize;
        for entry in parameters {
            let parameter = &entry.parameter;
            if !parameter.required
                && parameters::empty_composite(&entry.value)
                && matches!(parameter.serialization, Serialization::Style { .. })
            {
                continue;
            }
            let value = if let Serialization::Form { spec } = parameter.serialization {
                parts::query_form(
                    op.source,
                    spec,
                    &entry.value,
                    limits.request,
                    limits.part,
                    op.json_limits,
                )?
            } else {
                serialize_parameter(
                    op.source,
                    parameter,
                    &entry.value,
                    limits.request,
                    op.json_limits,
                )?
            };
            serialized_bytes = serialized_bytes
                .saturating_add(value.len())
                .saturating_add(parameter.name.len());
            if serialized_bytes > op.limits.request {
                return Err(resource_error(
                    op.source,
                    parameter.source,
                    "serialized parameters exceed the request byte ceiling",
                ));
            }
            match parameter.location {
                ParameterLocation::Path => {
                    let marker = format!("{{{}}}", parameter.name);
                    if !path.contains(&marker) {
                        return Err(representation_error(
                            op.source,
                            parameter.source,
                            "path parameter is not present in the template",
                        ));
                    }
                    let count = path.matches(&marker).count();
                    if path.len().saturating_add(value.len().saturating_mul(count))
                        > op.limits.request
                    {
                        return Err(resource_error(
                            op.source,
                            parameter.source,
                            "expanded path exceeds the request byte ceiling",
                        ));
                    }
                    path = path.replace(&marker, &value);
                }
                ParameterLocation::Query | ParameterLocation::Querystring => query.push(value),
                ParameterLocation::Header => {
                    headers.push((parameter.name.into(), value.into_bytes()))
                }
                ParameterLocation::Cookie => cookies.push(value),
            }
        }
        self.credentials
            .attach(op, &mut query, &mut cookies, &mut headers)?;
        if !cookies.is_empty() {
            headers.push(("cookie".into(), cookies.join("; ").into_bytes()));
        }
        if !op.accept.is_empty() {
            headers.push(("accept".into(), op.accept.as_bytes().to_vec()));
        }
        if let Some(body) = &body {
            headers.push(("content-type".into(), body.content_type.as_bytes().to_vec()));
        }
        // ua/v1 attribution is applied after declared parameters so an explicit
        // caller-supplied User-Agent header keeps precedence over the default.
        if !headers.iter().any(|(name, _)| name.eq_ignore_ascii_case("user-agent")) {
            if let Some(user_agent) = resolve_user_agent(&self.options) {
                headers.push(("user-agent".into(), user_agent.into_bytes()));
            }
        }
        check_headers(op.source, op.source, &headers, op.limits.header, false)?;
        let url = servers::assemble(op, &base, &path, &query)?;
        let response = self
            .transport
            .send(Request {
                method: op.method,
                url,
                headers,
                body: body.map(|v| v.bytes),
            })
            .await
            .map_err(|cause| {
                SdkError::new(
                    SdkErrorKind::Transport,
                    op.source,
                    op.source,
                    "request transport failed",
                )
                .with_cause(cause)
            })?;
        check_headers(
            op.source,
            op.source,
            &response.headers,
            op.limits.header,
            true,
        )
        .map_err(|mut error| {
            error.status = Some(response.status);
            error.truncated = true;
            error
        })?;
        let content_type = media::content_type(&response.headers);
        Ok(Exchange {
            op,
            status: response.status,
            headers: response.headers,
            content_type,
            body: Some(response.body),
            limits,
            capture_limit: self.options.max_error_capture_bytes.min(limits.response),
        })
    }
    /// Consume a finite response. Generated streaming operations call `open`.
    pub async fn send(
        &self,
        op: &'static Operation,
        parameters: &[ParameterValue],
        body: Option<PreparedBody>,
    ) -> Result<RawResponse, SdkError> {
        self.open(op, parameters, body).await?.read().await
    }
}

/// An owned response before finite consumption or typed item streaming.
pub struct Exchange<B> {
    op: &'static Operation,
    pub status: u16,
    pub headers: Headers,
    pub content_type: Option<String>,
    body: Option<B>,
    limits: Limits,
    capture_limit: usize,
}
impl<B: ResponseBody> Exchange<B> {
    pub fn header_error(&self, _source: Source, error: SdkError) -> SdkError {
        SdkError::new(
            if error.kind == SdkErrorKind::ResourceLimit {
                SdkErrorKind::ResourceLimit
            } else {
                SdkErrorKind::ResponseDecoding
            },
            self.op.source,
            error.source,
            "declared response header is invalid",
        )
        .with_response(self.status, &self.headers, &[], self.capture_limit, false)
        .with_cause(Box::new(error))
    }
    pub fn selection(&self) -> Result<Selection, SdkError> {
        media::match_response(self.op, self.status, self.content_type.as_deref()).map_err(
            |source| {
                SdkError::new(
                    SdkErrorKind::UnexpectedResponse,
                    self.op.source,
                    source,
                    "response status or media type is not declared by the source",
                )
                .with_response(
                    self.status,
                    &self.headers,
                    &[],
                    self.capture_limit,
                    false,
                )
            },
        )
    }
    /// Finite response consumption. HEAD/1xx/204/205/304 drop the body without a poll.
    pub async fn read(mut self) -> Result<RawResponse, SdkError> {
        let mut buffer = Vec::new();
        if !media::forbidden(self.op.method, self.status) {
            let mut polls = self.limits.response.saturating_add(POLL_SLACK);
            let mut empty = 0usize;
            let body = self.body.as_mut().expect("owned response body");
            loop {
                if polls == 0 {
                    return Err(self.failure(
                        SdkErrorKind::ResourceLimit,
                        "response polling budget exceeded",
                        &buffer,
                        &[],
                        None,
                    ));
                }
                polls -= 1;
                let next = body.next_chunk().await;
                match next {
                    Ok(None) => break,
                    Ok(Some(chunk)) => {
                        if chunk.is_empty() {
                            empty += 1;
                            if empty > MAX_EMPTY_CHUNKS {
                                return Err(self.failure(
                                    SdkErrorKind::ResourceLimit,
                                    "too many empty response chunks",
                                    &buffer,
                                    &[],
                                    None,
                                ));
                            }
                            continue;
                        }
                        empty = 0;
                        if chunk.len() > self.limits.chunk
                            || buffer.len().saturating_add(chunk.len()) > self.limits.response
                        {
                            return Err(self.failure(
                                SdkErrorKind::ResourceLimit,
                                "response chunk or total byte ceiling exceeded",
                                &buffer,
                                &chunk,
                                None,
                            ));
                        }
                        buffer.extend_from_slice(&chunk);
                    }
                    Err(cause) => {
                        return Err(self.failure(
                            SdkErrorKind::Transport,
                            "response body stream failed",
                            &buffer,
                            &[],
                            Some(cause),
                        ));
                    }
                }
            }
        }
        Ok(RawResponse {
            status: self.status,
            content_type: self.content_type,
            headers: self.headers,
            body: buffer,
            capture_limit: self.capture_limit,
            limits: self.limits,
        })
    }
    fn failure(
        &self,
        kind: SdkErrorKind,
        message: &str,
        bytes: &[u8],
        next: &[u8],
        cause: Option<BoxError>,
    ) -> SdkError {
        let capture = bytes
            .iter()
            .chain(next)
            .copied()
            .take(self.capture_limit)
            .collect::<Vec<_>>();
        let mut error = SdkError::new(kind, self.op.source, self.op.source, message).with_response(
            self.status,
            &self.headers,
            &capture,
            self.capture_limit,
            true,
        );
        error.cause = cause;
        error
    }
    pub fn into_item_response<M, H>(
        self,
        data: fn(JsonValue) -> Result<M, crate::codecs::CodecError>,
        framing: Framing,
        source: Source,
        max_item: usize,
        media: &'static str,
        typed_headers: H,
        links: &'static [Link],
    ) -> ApiResponse<ItemStream<M>, H>
    where
        B: 'static,
    {
        let status = self.status;
        let headers = self.headers.clone();
        let actual_content_type = self.content_type.clone();
        ApiResponse {
            status,
            content_type: media,
            actual_content_type,
            headers,
            typed_headers,
            links,
            data: ItemStream::new(self, data, framing, source, max_item),
        }
    }
}

pub struct RawResponse {
    pub status: u16,
    /// Complete, syntactically valid concrete Content-Type, including parameters.
    pub content_type: Option<String>,
    pub headers: Headers,
    pub body: Vec<u8>,
    capture_limit: usize,
    pub limits: Limits,
}
impl RawResponse {
    pub fn selection(&self, op: &Operation) -> Result<Selection, SdkError> {
        media::match_response(op, self.status, self.content_type.as_deref()).map_err(|source| {
            self.response_error(
                op.source,
                source,
                SdkErrorKind::UnexpectedResponse,
                "response status or media type is not declared by the source",
            )
        })
    }
    #[must_use]
    pub fn into_api_response<T>(self, data: T, media: &'static str) -> ApiResponse<T> {
        self.into_typed_response(data, media, (), &[])
    }
    #[must_use]
    pub fn into_typed_response<T, H>(
        self,
        data: T,
        media: &'static str,
        typed_headers: H,
        links: &'static [Link],
    ) -> ApiResponse<T, H> {
        ApiResponse {
            status: self.status,
            content_type: media,
            actual_content_type: self.content_type,
            headers: self.headers,
            data,
            typed_headers,
            links,
        }
    }
    pub fn decoding_error(
        &self,
        operation: Source,
        source: Source,
        error: crate::codecs::CodecError,
    ) -> SdkError {
        self.response_error(
            operation,
            source,
            if codec_resource_limit(&error) {
                SdkErrorKind::ResourceLimit
            } else {
                SdkErrorKind::ResponseDecoding
            },
            "declared response could not be decoded against its schema",
        )
        .with_cause(Box::new(error))
    }
    pub fn wire_error(&self, operation: Source, _source: Source, error: SdkError) -> SdkError {
        self.response_error(
            operation,
            error.source,
            if error.kind == SdkErrorKind::ResourceLimit {
                SdkErrorKind::ResourceLimit
            } else {
                SdkErrorKind::ResponseDecoding
            },
            "declared response representation is invalid",
        )
        .with_cause(Box::new(error))
    }
    pub fn response_error(
        &self,
        operation: Source,
        source: Source,
        kind: SdkErrorKind,
        message: &str,
    ) -> SdkError {
        SdkError::new(kind, operation, source, message).with_response(
            self.status,
            &self.headers,
            &self.body,
            self.capture_limit,
            false,
        )
    }
    pub fn unexpected_error(self, operation: Source, source: Source) -> SdkError {
        self.response_error(
            operation,
            source,
            SdkErrorKind::UnexpectedResponse,
            "response status or media type is not declared by the source",
        )
    }
}

/// Decoded native payload, actual status, raw headers, typed declared headers
/// and links as source metadata. Link expressions never trigger another call.
pub struct ApiResponse<T, H = ()> {
    pub status: u16,
    pub content_type: &'static str,
    pub actual_content_type: Option<String>,
    pub headers: Headers,
    pub typed_headers: H,
    pub links: &'static [Link],
    pub data: T,
}
impl<T: fmt::Debug, H: fmt::Debug> fmt::Debug for ApiResponse<T, H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiResponse")
            .field("status", &self.status)
            .field("content_type", &self.content_type)
            .field("headers", &HeaderSummary(&self.headers))
            .field("data", &self.data)
            .finish()
    }
}

/// JSON without an asserted schema still uses the emitted exact parser limits.
pub fn json_bytes(op: &Operation, source: Source, value: &JsonValue) -> Result<Vec<u8>, SdkError> {
    let mut json = op.json_limits;
    json.max_output_bytes = json.max_output_bytes.min(op.limits.request);
    crate::stringify_json(value, json)
        .map(String::into_bytes)
        .map_err(|e| request_codec_error(op.source, source, e.into()))
}
pub fn parse_json_body(
    op: &Operation,
    source: Source,
    bytes: &[u8],
) -> Result<JsonValue, SdkError> {
    crate::parse_json_bytes(bytes, op.json_limits)
        .map_err(|e| request_codec_error(op.source, source, e.into()))
}
pub fn checked_bytes(
    operation: Source,
    source: Source,
    bytes: &[u8],
    limit: usize,
) -> Result<Vec<u8>, SdkError> {
    if bytes.len() > limit {
        return Err(resource_error(
            operation,
            source,
            "byte input exceeds its declared or native ceiling",
        ));
    }
    Ok(bytes.to_vec())
}
pub fn check_headers(
    operation: Source,
    source: Source,
    headers: &Headers,
    limit: usize,
    response: bool,
) -> Result<(), SdkError> {
    let mut size = 0usize;
    for (name, value) in headers {
        size = size.saturating_add(name.len()).saturating_add(value.len());
        if size > limit {
            return Err(resource_error(
                operation,
                source,
                "header byte ceiling exceeded",
            ));
        }
        // Malformed Content-Type remains an unmatched representation, with the
        // same bounded body capture as any other media mismatch.
        if !media::is_token(name)
            || !(response && name.eq_ignore_ascii_case("content-type"))
                && value.iter().any(|b| (*b < 32 && *b != b'\t') || *b == 127)
        {
            return Err(SdkError::new(
                if response {
                    SdkErrorKind::ResponseDecoding
                } else {
                    SdkErrorKind::RequestRepresentation
                },
                operation,
                source,
                "invalid header name or control character",
            ));
        }
    }
    Ok(())
}
struct HeaderSummary<'a>(&'a Headers);
impl fmt::Debug for HeaderSummary<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.0.iter().map(|(name, value)| (name, value.len())))
            .finish()
    }
}
