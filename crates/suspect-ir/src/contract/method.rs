//! HTTP method identity is separate from the platform IR's fixed method enum.

/// A case-sensitive HTTP method token borrowed from an indexed operation.
///
/// Fixed Path Item fields use their uppercase HTTP spelling. OpenAPI 3.2
/// `additionalOperations` keys retain exactly the spelling sent on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HttpMethod<'a>(&'a str);

impl<'a> HttpMethod<'a> {
    /// Accept an HTTP `token` (RFC 9110 §§5.6.2, 9.1), without normalizing case.
    #[must_use]
    pub fn parse(value: &'a str) -> Option<Self> {
        (!value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)))
        .then_some(Self(value))
    }

    /// The exact, case-sensitive method token to send in the request.
    #[must_use]
    pub const fn as_str(self) -> &'a str {
        self.0
    }
}

impl std::fmt::Display for HttpMethod<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

impl AsRef<str> for HttpMethod<'_> {
    fn as_ref(&self) -> &str {
        self.0
    }
}

// Preserve comparisons used by callers of the historical eight-method API.
// There is intentionally no lossy conversion from an arbitrary token to Method.
impl PartialEq<crate::Method> for HttpMethod<'_> {
    fn eq(&self, other: &crate::Method) -> bool {
        self.0 == other.as_str()
    }
}

impl PartialEq<HttpMethod<'_>> for crate::Method {
    fn eq(&self, other: &HttpMethod<'_>) -> bool {
        self.as_str() == other.0
    }
}

pub(super) const FIXED_METHODS: [(&str, &str); 9] = [
    ("get", "GET"),
    ("put", "PUT"),
    ("post", "POST"),
    ("delete", "DELETE"),
    ("options", "OPTIONS"),
    ("head", "HEAD"),
    ("patch", "PATCH"),
    ("trace", "TRACE"),
    ("query", "QUERY"),
];

pub(super) fn additional_method(value: &str) -> Option<HttpMethod<'_>> {
    // HTTP methods are case-sensitive: `get` is not the fixed GET method.
    (!FIXED_METHODS.iter().any(|(_, method)| *method == value))
        .then(|| HttpMethod::parse(value))
        .flatten()
}
