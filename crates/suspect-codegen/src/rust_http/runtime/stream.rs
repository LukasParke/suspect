use super::{
    BoxError, Exchange, Headers, JsonLimits, MAX_EMPTY_CHUNKS, Operation, POLL_SLACK, ResponseBody,
    SdkError, SdkErrorKind, Source, codec_resource_limit, representation_error,
    request_codec_error, resource_error,
};
use crate::{JsonNonNullValue as Value, JsonValue, Nullable, codecs::CodecError};
use std::{collections::BTreeMap, fmt, future::Future, pin::Pin};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    ServerSentEvents,
    JsonLines,
}

trait ErasedBody: Send {
    fn next_chunk(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Vec<u8>>, BoxError>> + Send + '_>>;
}
impl<B: ResponseBody> ErasedBody for B {
    fn next_chunk(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Vec<u8>>, BoxError>> + Send + '_>> {
        Box::pin(ResponseBody::next_chunk(self))
    }
}

/// Native asynchronous pull stream. `next().await` returns at most one validated
/// source item; it reads no later transport chunk until needed. Drop or `close`
/// releases the body immediately, including when a next-item future is pending.
pub struct ItemStream<M> {
    body: Option<Box<dyn ErasedBody>>,
    decode: fn(JsonValue) -> Result<M, CodecError>,
    framing: Framing,
    operation: Source,
    source: Source,
    status: u16,
    headers: Headers,
    json: JsonLimits,
    item_limit: usize,
    chunk_limit: usize,
    total_limit: usize,
    total: usize,
    polls: usize,
    empty_chunks: usize,
    chunk: Vec<u8>,
    offset: usize,
    line: Vec<u8>,
    skip_lf: bool,
    first_line: bool,
    item_bytes: usize,
    envelope: BTreeMap<String, JsonValue>,
    data: String,
    has_data: bool,
    capture: Vec<u8>,
    capture_limit: usize,
    ended: bool,
}
impl<M> fmt::Debug for ItemStream<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ItemStream")
            .field("framing", &self.framing)
            .field("source", &self.source)
            .field("received_bytes", &self.total)
            .field("closed", &self.ended)
            .finish()
    }
}
impl<M> ItemStream<M> {
    pub(super) fn new<B: ResponseBody + 'static>(
        mut exchange: Exchange<B>,
        decode: fn(JsonValue) -> Result<M, CodecError>,
        framing: Framing,
        source: Source,
        max_item: usize,
    ) -> Self {
        Self {
            body: exchange
                .body
                .take()
                .map(|b| Box::new(b) as Box<dyn ErasedBody>),
            decode,
            framing,
            operation: exchange.op.source,
            source,
            status: exchange.status,
            headers: exchange.headers,
            json: exchange.op.json_limits,
            item_limit: max_item.min(exchange.limits.item),
            chunk_limit: exchange.limits.chunk,
            total_limit: exchange.limits.response,
            total: 0,
            polls: exchange.limits.response.saturating_add(POLL_SLACK),
            empty_chunks: 0,
            chunk: Vec::new(),
            offset: 0,
            line: Vec::new(),
            skip_lf: false,
            first_line: true,
            item_bytes: 0,
            envelope: BTreeMap::new(),
            data: String::new(),
            has_data: false,
            capture: Vec::new(),
            capture_limit: exchange.capture_limit,
            ended: false,
        }
    }
    /// Cancel the transfer without consuming or buffering the remainder.
    pub fn close(&mut self) {
        self.body = None;
        self.chunk = Vec::new();
        self.line = Vec::new();
        self.envelope = BTreeMap::new();
        self.data = String::new();
        self.capture = Vec::new();
        self.headers = Vec::new();
        self.ended = true;
    }
    pub fn is_closed(&self) -> bool {
        self.ended
    }
    pub async fn next(&mut self) -> Option<Result<M, SdkError>> {
        if self.ended {
            return None;
        }
        loop {
            while self.offset < self.chunk.len() {
                let byte = self.chunk[self.offset];
                self.offset += 1;
                if self.skip_lf {
                    self.skip_lf = false;
                    if byte == b'\n' {
                        continue;
                    }
                }
                self.item_bytes = self.item_bytes.saturating_add(1);
                if self.item_bytes > self.item_limit {
                    return Some(Err(self.fail(
                        SdkErrorKind::ResourceLimit,
                        "stream item byte ceiling exceeded",
                        None,
                    )));
                }
                let line_end =
                    byte == b'\n' || self.framing == Framing::ServerSentEvents && byte == b'\r';
                if line_end {
                    self.skip_lf = byte == b'\r';
                    match self.line_item() {
                        Ok(Some(value)) => return Some(self.decode_item(value)),
                        Ok(None) => {}
                        Err(error) => return Some(Err(error)),
                    }
                } else {
                    self.line.push(byte);
                }
            }
            // Do not retain a spent chunk alongside the next one.
            self.chunk = Vec::new();
            self.offset = 0;
            if self.polls == 0 {
                return Some(Err(self.fail(
                    SdkErrorKind::ResourceLimit,
                    "stream polling budget exceeded",
                    None,
                )));
            }
            self.polls -= 1;
            let next = match &mut self.body {
                Some(body) => body.next_chunk().await,
                None => return None,
            };
            match next {
                Ok(None) => {
                    // SSE dispatch requires a blank line. At EOF pending event
                    // data is discarded per the HTML framing algorithm.
                    if self.framing == Framing::JsonLines && !self.line.is_empty() {
                        let value = self.line_item();
                        return match value {
                            Ok(Some(value)) => {
                                let item = self.decode_item(value);
                                self.close();
                                Some(item)
                            }
                            Ok(None) => {
                                self.close();
                                None
                            }
                            Err(error) => Some(Err(error)),
                        };
                    }
                    self.close();
                    return None;
                }
                Ok(Some(chunk)) => {
                    if chunk.is_empty() {
                        self.empty_chunks += 1;
                        if self.empty_chunks > MAX_EMPTY_CHUNKS {
                            return Some(Err(self.fail(
                                SdkErrorKind::ResourceLimit,
                                "too many empty stream chunks",
                                None,
                            )));
                        }
                        continue;
                    }
                    self.empty_chunks = 0;
                    self.capture.extend(
                        chunk
                            .iter()
                            .copied()
                            .take(self.capture_limit.saturating_sub(self.capture.len())),
                    );
                    self.total = self.total.saturating_add(chunk.len());
                    if chunk.len() > self.chunk_limit || self.total > self.total_limit {
                        return Some(Err(self.fail(
                            SdkErrorKind::ResourceLimit,
                            "stream chunk or total byte ceiling exceeded",
                            None,
                        )));
                    }
                    self.chunk = chunk;
                }
                Err(cause) => {
                    return Some(Err(self.fail(
                        SdkErrorKind::Transport,
                        "response item stream failed",
                        Some(cause),
                    )));
                }
            }
        }
    }
    fn decode_item(&mut self, value: JsonValue) -> Result<M, SdkError> {
        (self.decode)(value).map_err(|error| {
            self.fail(
                if codec_resource_limit(&error) {
                    SdkErrorKind::ResourceLimit
                } else {
                    SdkErrorKind::ResponseDecoding
                },
                "stream item failed its source codec",
                Some(Box::new(error)),
            )
        })
    }
    fn line_item(&mut self) -> Result<Option<JsonValue>, SdkError> {
        let mut bytes = std::mem::take(&mut self.line);
        if self.framing == Framing::JsonLines {
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
            self.item_bytes = 0;
            // Blank lines are not JSON values. There is no sentinel or record
            // separator shortcut, and UTF-8 is checked by the exact JSON parser.
            return crate::parse_json_bytes(&bytes, self.json)
                .map(Some)
                .map_err(|e| {
                    self.fail(
                        if e.kind == crate::JsonErrorKind::ResourceLimit {
                            SdkErrorKind::ResourceLimit
                        } else {
                            SdkErrorKind::ResponseDecoding
                        },
                        "JSON-lines record is invalid",
                        Some(Box::new(e)),
                    )
                });
        }
        let text = String::from_utf8_lossy(&bytes);
        let text = if self.first_line {
            self.first_line = false;
            text.strip_prefix('\u{feff}').unwrap_or(&text)
        } else {
            &text
        };
        if text.is_empty() {
            self.item_bytes = 0;
            let mut envelope = std::mem::take(&mut self.envelope);
            if !self.has_data {
                self.data.clear();
                return Ok(None);
            }
            self.has_data = false;
            self.data.pop(); // the final LF appended by a data field
            envelope.insert(
                "data".into(),
                Nullable::Value(Value::String(std::mem::take(&mut self.data))),
            );
            return Ok(Some(Nullable::Value(Value::Object(envelope))));
        }
        if text.starts_with(':') {
            return Ok(None);
        }
        let (field, value) = text.split_once(':').unwrap_or((text, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "data" => {
                if self
                    .data
                    .len()
                    .saturating_add(value.len())
                    .saturating_add(1)
                    > self.item_limit
                {
                    return Err(self.fail(
                        SdkErrorKind::ResourceLimit,
                        "SSE data buffer exceeded the item ceiling",
                        None,
                    ));
                }
                self.data.push_str(value);
                self.data.push('\n');
                self.has_data = true;
            }
            "event" => {
                self.envelope
                    .insert("event".into(), Nullable::Value(Value::String(value.into())));
            }
            "id" if !value.contains('\0') => {
                self.envelope
                    .insert("id".into(), Nullable::Value(Value::String(value.into())));
            }
            "retry" if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) => {
                let token = value.trim_start_matches('0');
                let token = if token.is_empty() { "0" } else { token };
                let number = token.parse().map_err(|e| {
                    self.fail(
                        SdkErrorKind::ResponseDecoding,
                        "invalid SSE retry value",
                        Some(Box::new(e)),
                    )
                })?;
                self.envelope
                    .insert("retry".into(), Nullable::Value(Value::Number(number)));
            }
            _ => {}
        }
        Ok(None)
    }
    fn fail(&mut self, kind: SdkErrorKind, message: &str, cause: Option<BoxError>) -> SdkError {
        let mut error = SdkError::new(kind, self.operation, self.source, message).with_response(
            self.status,
            &self.headers,
            &self.capture,
            self.capture_limit,
            true,
        );
        error.cause = cause;
        self.close();
        error
    }
}

/// Encode finite request-stream items after the generated item codec has
/// validated each item. Responses use the asynchronous pull stream above.
pub fn encode_items<M>(
    op: &Operation,
    source: Source,
    values: &[M],
    codec: fn(&M) -> Result<JsonValue, CodecError>,
    framing: Framing,
    item_limit: usize,
) -> Result<Vec<u8>, SdkError> {
    let mut out = super::parameters::Writer::new(op.source, source, op.limits.request);
    for value in values {
        let value = codec(value).map_err(|e| request_codec_error(op.source, source, e))?;
        let mut item =
            super::parameters::Writer::new(op.source, source, item_limit.min(op.limits.item));
        match framing {
            Framing::JsonLines => {
                item.push(
                    &crate::stringify_json(&value, op.json_limits)
                        .map_err(|e| request_codec_error(op.source, source, e.into()))?,
                )?;
                item.push("\n")?;
            }
            Framing::ServerSentEvents => {
                let Nullable::Value(Value::Object(object)) = &value else {
                    return Err(representation_error(
                        op.source,
                        source,
                        "SSE item must be an event envelope",
                    ));
                };
                if object
                    .keys()
                    .any(|key| !matches!(key.as_str(), "data" | "event" | "id" | "retry"))
                {
                    return Err(representation_error(
                        op.source,
                        source,
                        "SSE envelope contains fields discarded by standard framing",
                    ));
                }
                for name in ["event", "id", "retry"] {
                    if let Some(value) = object.get(name) {
                        let text = super::scalar_text(
                            op.source,
                            source,
                            value,
                            Some(if name == "retry" {
                                super::ScalarType::Integer
                            } else {
                                super::ScalarType::String
                            }),
                        )?;
                        if text.contains(['\r', '\n'])
                            || name == "id" && text.contains('\0')
                            || name == "retry"
                                && (text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()))
                        {
                            return Err(representation_error(
                                op.source,
                                source,
                                "SSE field has no faithful line representation",
                            ));
                        }
                        item.push(name)?;
                        item.push(": ")?;
                        item.push(&text)?;
                        item.push("\n")?;
                    }
                }
                let data = object.get("data").ok_or_else(|| {
                    representation_error(
                        op.source,
                        source,
                        "SSE event requires a data field for dispatch",
                    )
                })?;
                let data =
                    super::scalar_text(op.source, source, data, Some(super::ScalarType::String))?;
                if data.contains('\r') {
                    return Err(representation_error(
                        op.source,
                        source,
                        "SSE data must use LF line breaks",
                    ));
                }
                for line in data.split('\n') {
                    item.push("data: ")?;
                    item.push(line)?;
                    item.push("\n")?;
                }
                item.push("\n")?;
            }
        }
        if item.value.len() > item_limit {
            return Err(resource_error(
                op.source,
                source,
                "stream item exceeds its byte ceiling",
            ));
        }
        out.push(&item.value)?;
    }
    Ok(out.value.into_bytes())
}
