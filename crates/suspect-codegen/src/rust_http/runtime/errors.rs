use super::{BoxError, HeaderSummary, Headers, Source};
use crate::{JsonErrorKind, codecs::CodecError};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdkErrorKind {
    RequestValidation,
    RequestRepresentation,
    Transport,
    ResourceLimit,
    UnexpectedResponse,
    ResponseDecoding,
}

/// Source-linked native failure. Display/debug omit captures and credentials;
/// the underlying cause and bounded capture are available explicitly.
pub struct SdkError {
    pub kind: SdkErrorKind,
    pub operation_source: Source,
    pub source: Source,
    pub status: Option<u16>,
    pub headers: Headers,
    pub raw_capture: Vec<u8>,
    pub truncated: bool,
    pub cause: Option<BoxError>,
    message: String,
}
impl SdkError {
    pub(super) fn new(
        kind: SdkErrorKind,
        operation_source: Source,
        source: Source,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            operation_source,
            source,
            status: None,
            headers: Vec::new(),
            raw_capture: Vec::new(),
            truncated: false,
            cause: None,
            message: message.into(),
        }
    }
    pub(super) fn with_cause(mut self, cause: BoxError) -> Self {
        self.cause = Some(cause);
        self
    }
    pub(super) fn with_response(
        mut self,
        status: u16,
        headers: &Headers,
        bytes: &[u8],
        capture: usize,
        truncated: bool,
    ) -> Self {
        self.status = Some(status);
        self.headers = headers.clone();
        self.raw_capture = bytes[..bytes.len().min(capture)].to_vec();
        self.truncated = truncated || bytes.len() > capture;
        self
    }
}
impl fmt::Debug for SdkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SdkError")
            .field("kind", &self.kind)
            .field("operation_source", &self.operation_source)
            .field("source", &self.source)
            .field("status", &self.status)
            .field("headers", &HeaderSummary(&self.headers))
            .field("raw_capture", &(self.raw_capture.len(), self.truncated))
            .field("has_cause", &self.cause.is_some())
            .finish()
    }
}
impl fmt::Display for SdkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for SdkError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

pub fn request_codec_error(operation: Source, source: Source, error: CodecError) -> SdkError {
    let (kind, message) = match &error {
        CodecError::Invalid(_) => (
            SdkErrorKind::RequestValidation,
            "input violates its source schema",
        ),
        e if codec_resource_limit(e) => (
            SdkErrorKind::ResourceLimit,
            "codec evaluation exceeded its finite budget",
        ),
        _ => (
            SdkErrorKind::RequestRepresentation,
            "input cannot be represented by its source codec",
        ),
    };
    SdkError::new(kind, operation, source, message).with_cause(Box::new(error))
}
pub(super) fn codec_resource_limit(error: &CodecError) -> bool {
    match error {
        CodecError::EvaluationFailure(_) => true,
        CodecError::Json(e) => e.kind == JsonErrorKind::ResourceLimit,
        CodecError::Conversion(f) => matches!(
            f.message.as_str(),
            "native conversion work limit exceeded" | "native conversion depth limit exceeded"
        ),
        CodecError::Invalid(_) => false,
    }
}
pub fn validation_error(operation: Source, source: Source, message: impl Into<String>) -> SdkError {
    SdkError::new(SdkErrorKind::RequestValidation, operation, source, message)
}
pub fn representation_error(
    operation: Source,
    source: Source,
    message: impl Into<String>,
) -> SdkError {
    SdkError::new(
        SdkErrorKind::RequestRepresentation,
        operation,
        source,
        message,
    )
}
pub fn resource_error(operation: Source, source: Source, message: impl Into<String>) -> SdkError {
    SdkError::new(SdkErrorKind::ResourceLimit, operation, source, message)
}
