use super::parameters::{PercentEncoding, encode, percent_decode};
use super::{
    BoxError, CredentialKind, CredentialRequirement, Headers, Operation, ParameterLocation,
    SdkError, Security, Source, representation_error, resource_error, validation_error,
};
use std::{borrow::Cow, fmt, sync::Arc};

/// Caller-supplied attachment. OAuth/OIDC use `Authorization` with the complete
/// caller-chosen authorization value, without assuming a bearer token type.
#[derive(Clone)]
pub enum Credential {
    Bearer(String),
    Basic { username: String, password: String },
    ApiKey(String),
    Authorization(String),
}
impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credential(<redacted>)")
    }
}

/// Explicit synchronous caller hook for OAuth/OIDC attachment. Metadata carries
/// source, flow endpoints, scopes/roles and discovery URLs. The SDK performs no
/// discovery, token exchange, refresh or acquisition itself.
pub trait CredentialProvider: Send + Sync {
    fn provide(&self, requirement: &CredentialRequirement) -> Result<Option<Credential>, BoxError>;
}
impl<F> CredentialProvider for F
where
    F: Fn(&CredentialRequirement) -> Result<Option<Credential>, BoxError> + Send + Sync,
{
    fn provide(&self, r: &CredentialRequirement) -> Result<Option<Credential>, BoxError> {
        self(r)
    }
}
#[derive(Clone, PartialEq, Eq)]
enum Key {
    Name(String),
    Source(Source),
}
#[derive(Clone, Default)]
pub struct Credentials {
    entries: Vec<(Key, Credential)>,
    hook: Option<Arc<dyn CredentialProvider>>,
    alternative: Option<usize>,
}
impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("count", &self.entries.len())
            .field("hook", &self.hook.is_some())
            .field("alternative", &self.alternative)
            .finish()
    }
}
impl Credentials {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    fn insert(mut self, key: Key, value: Credential) -> Self {
        if let Some((_, old)) = self.entries.iter_mut().find(|(k, _)| *k == key) {
            *old = value;
        } else {
            self.entries.push((key, value));
        }
        self
    }
    #[must_use]
    pub fn with_bearer(self, scheme: impl Into<String>, token: impl Into<String>) -> Self {
        self.insert(Key::Name(scheme.into()), Credential::Bearer(token.into()))
    }
    #[must_use]
    pub fn with_basic(
        self,
        scheme: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.insert(
            Key::Name(scheme.into()),
            Credential::Basic {
                username: username.into(),
                password: password.into(),
            },
        )
    }
    #[must_use]
    pub fn with_api_key(self, scheme: impl Into<String>, value: impl Into<String>) -> Self {
        self.insert(Key::Name(scheme.into()), Credential::ApiKey(value.into()))
    }
    #[must_use]
    pub fn with_authorization(self, scheme: impl Into<String>, value: impl Into<String>) -> Self {
        self.insert(
            Key::Name(scheme.into()),
            Credential::Authorization(value.into()),
        )
    }
    #[must_use]
    pub fn with_source_bearer(self, source: Source, token: impl Into<String>) -> Self {
        self.insert(Key::Source(source), Credential::Bearer(token.into()))
    }
    #[must_use]
    pub fn with_source_basic(
        self,
        source: Source,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.insert(
            Key::Source(source),
            Credential::Basic {
                username: username.into(),
                password: password.into(),
            },
        )
    }
    #[must_use]
    pub fn with_source_api_key(self, source: Source, value: impl Into<String>) -> Self {
        self.insert(Key::Source(source), Credential::ApiKey(value.into()))
    }
    #[must_use]
    pub fn with_source_authorization(self, source: Source, value: impl Into<String>) -> Self {
        self.insert(Key::Source(source), Credential::Authorization(value.into()))
    }
    #[must_use]
    pub fn with_hook(mut self, hook: impl CredentialProvider + 'static) -> Self {
        self.hook = Some(Arc::new(hook));
        self
    }
    /// Select one exact OR alternative; all its AND members are required.
    #[must_use]
    pub fn select_alternative(mut self, index: usize) -> Self {
        self.alternative = Some(index);
        self
    }
    #[must_use]
    pub fn bearer_token(&self, scheme: &str) -> Option<&str> {
        self.entries.iter().find_map(|(key, v)| match (key, v) {
            (Key::Name(name), Credential::Bearer(token)) if name == scheme => Some(token.as_str()),
            _ => None,
        })
    }
    fn get(&self, r: &CredentialRequirement) -> Option<&Credential> {
        self.entries
            .iter()
            .find(|(key, _)| *key == Key::Source(r.scheme.use_site))
            .or_else(|| {
                self.entries
                    .iter()
                    .find(|(key, _)| matches!(key,Key::Name(name) if name==r.name))
            })
            .map(|(_, value)| value)
    }
    pub(super) fn attach(
        &self,
        op: &Operation,
        query: &mut Vec<String>,
        cookies: &mut Vec<String>,
        headers: &mut Headers,
    ) -> Result<(), SdkError> {
        let Security::Alternatives(alternatives) = op.security else {
            return Ok(());
        };
        let hookable = |r: &CredentialRequirement| {
            matches!(
                r.kind,
                CredentialKind::OAuth2 { .. } | CredentialKind::OpenIdConnect { .. }
            ) && self.hook.is_some()
        };
        let index = if let Some(index) = self.alternative {
            index
        } else {
            alternatives
                .iter()
                .position(|a| {
                    a.requirements
                        .iter()
                        .all(|r| self.get(r).is_some() || hookable(r))
                })
                .ok_or_else(|| {
                    validation_error(
                        op.source,
                        op.source,
                        "credentials do not satisfy any declared security alternative",
                    )
                })?
        };
        let alternative = alternatives.get(index).ok_or_else(|| {
            validation_error(
                op.source,
                op.source,
                "security alternative index is not declared",
            )
        })?;
        for r in alternative.requirements {
            let supplied = if let Some(value) = self.get(r) {
                Some(Cow::Borrowed(value))
            } else if hookable(r) {
                self.hook
                    .as_ref()
                    .expect("explicit hook")
                    .provide(r)
                    .map_err(|cause| {
                        validation_error(op.source, r.source, "caller credential hook failed")
                            .with_cause(cause)
                    })?
                    .map(Cow::Owned)
            } else {
                None
            };
            let value = supplied.ok_or_else(|| {
                validation_error(op.source, r.source, "required credential is absent")
            })?;
            let invalid = || {
                validation_error(
                    op.source,
                    r.scheme.terminal,
                    "credential does not match its declared attachment",
                )
            };
            let (location, name, text): (ParameterLocation, &str, Cow<'_, str>) =
                match (&r.kind, value.as_ref()) {
                    (CredentialKind::Bearer { .. }, Credential::Bearer(token)) => {
                        if token.len() > op.limits.header.saturating_sub(7) {
                            return Err(resource_error(
                                op.source,
                                r.source,
                                "credential exceeds header ceiling",
                            ));
                        }
                        if !bearer(token) {
                            return Err(invalid());
                        }
                        (
                            ParameterLocation::Header,
                            "authorization",
                            Cow::Owned(format!("Bearer {token}")),
                        )
                    }
                    (CredentialKind::Basic, Credential::Basic { username, password }) => {
                        let len = username
                            .len()
                            .saturating_add(password.len())
                            .saturating_add(1);
                        if (len.saturating_add(2) / 3).saturating_mul(4)
                            > op.limits.header.saturating_sub(6)
                        {
                            return Err(resource_error(
                                op.source,
                                r.source,
                                "basic credential exceeds header ceiling",
                            ));
                        }
                        if username.contains(':')
                            || username
                                .chars()
                                .chain(password.chars())
                                .any(char::is_control)
                        {
                            return Err(invalid());
                        }
                        (
                            ParameterLocation::Header,
                            "authorization",
                            Cow::Owned(format!(
                                "Basic {}",
                                base64(format!("{username}:{password}").as_bytes())
                            )),
                        )
                    }
                    (CredentialKind::ApiKey { location, name }, Credential::ApiKey(value)) => {
                        (*location, name.value, Cow::Borrowed(value))
                    }
                    (
                        CredentialKind::OAuth2 { .. } | CredentialKind::OpenIdConnect { .. },
                        Credential::Authorization(value),
                    ) => (
                        ParameterLocation::Header,
                        "authorization",
                        Cow::Borrowed(value),
                    ),
                    _ => return Err(invalid()),
                };
            if text.len() > op.limits.request.min(op.limits.header) {
                return Err(resource_error(
                    op.source,
                    r.source,
                    "credential exceeds its native ceiling",
                ));
            }
            if text.is_empty() || text.chars().any(char::is_control) {
                return Err(invalid());
            }
            match location {
                ParameterLocation::Header => {
                    if headers.iter().any(|(n, _)| n.eq_ignore_ascii_case(name))
                        || name.eq_ignore_ascii_case("accept")
                        || name.eq_ignore_ascii_case("content-type")
                        || name.eq_ignore_ascii_case("cookie")
                    {
                        return Err(representation_error(
                            op.source,
                            r.source,
                            "credential attachment conflicts with another header",
                        ));
                    }
                    headers.push((name.into(), text.into_owned().into_bytes()));
                }
                ParameterLocation::Query | ParameterLocation::Cookie => {
                    let values = if location == ParameterLocation::Query {
                        &mut *query
                    } else {
                        &mut *cookies
                    };
                    for existing in values.iter().flat_map(|v| {
                        v.split(if location == ParameterLocation::Query {
                            "&"
                        } else {
                            "; "
                        })
                    }) {
                        let key = existing.split_once('=').map_or(existing, |(k, _)| k);
                        if key == name
                            || percent_decode(op.source, r.source, key, false)
                                .is_ok_and(|decoded| decoded == name)
                        {
                            return Err(representation_error(
                                op.source,
                                r.source,
                                "credential attachment conflicts with another query or cookie value",
                            ));
                        }
                    }
                    let (name, text) = if location == ParameterLocation::Cookie {
                        if text.bytes().any(|b| {
                            !(b == 0x21
                                || (0x23..=0x2b).contains(&b)
                                || (0x2d..=0x3a).contains(&b)
                                || (0x3c..=0x5b).contains(&b)
                                || (0x5d..=0x7e).contains(&b))
                        }) {
                            return Err(representation_error(
                                op.source,
                                r.source,
                                "API-key cookie values require caller-supplied cookie escaping",
                            ));
                        }
                        (name.to_owned(), text.into_owned())
                    } else {
                        (
                            encode(
                                op.source,
                                r.source,
                                name,
                                PercentEncoding::UriComponent,
                                op.limits.request,
                            )?,
                            encode(
                                op.source,
                                r.source,
                                &text,
                                PercentEncoding::UriComponent,
                                op.limits.request,
                            )?,
                        )
                    };
                    values.push(format!("{name}={text}"));
                }
                ParameterLocation::Path | ParameterLocation::Querystring => return Err(invalid()),
            }
        }
        Ok(())
    }
}
fn bearer(value: &str) -> bool {
    let token = value.trim_end_matches('=');
    !token.is_empty()
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~+/".contains(&b))
}
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for part in bytes.chunks(3) {
        let a = part[0];
        let b = part.get(1).copied().unwrap_or(0);
        let c = part.get(2).copied().unwrap_or(0);
        result.push(char::from(TABLE[usize::from(a >> 2)]));
        result.push(char::from(TABLE[usize::from((a & 3) << 4 | b >> 4)]));
        result.push(if part.len() > 1 {
            char::from(TABLE[usize::from((b & 15) << 2 | c >> 6)])
        } else {
            '='
        });
        result.push(if part.len() > 2 {
            char::from(TABLE[usize::from(c & 63)])
        } else {
            '='
        });
    }
    result
}
