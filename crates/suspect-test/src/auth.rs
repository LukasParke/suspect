//! Security-scheme credential resolution and injection for plan execution.
//!
//! [`AuthConfig`] maps OpenAPI security scheme names to credential
//! strategies: a static bearer token or API key, or an OAuth 2.0
//! client-credentials flow acquired through the same [`HttpClient`] the
//! runner uses (so canned transports make auth deterministic in tests).
//! Tokens acquired via client credentials are cached until `expires_in`
//! elapses, then re-acquired on the next request.
//!
//! Injection: for each step, the runner collects the scheme names named by
//! the operation's security requirements (alternatives ORed, requirements
//! within one object ANDed) and applies the configured credential for
//! every scheme that has one and is not already satisfied by an explicit
//! step parameter.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::exec::{HttpClient, HttpRequest};
use serde::Deserialize;

/// One configured credential strategy for a security scheme.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Credential {
    /// Injects `Authorization: Bearer {token}` (or a custom header name).
    Bearer {
        /// The token string.
        token: String,
        /// Header override (default `Authorization`).
        #[serde(default)]
        header: Option<String>,
    },
    /// Injects a fixed header (`X-Api-Key` style).
    ApiKey {
        /// Header name.
        name: String,
        /// Header value.
        value: String,
    },
    /// OAuth 2.0 client-credentials acquisition against `tokenUrl`.
    ClientCredentials {
        /// Token endpoint URL.
        token_url: String,
        /// OAuth client id.
        client_id: String,
        /// OAuth client secret.
        client_secret: String,
        /// Optional scope string sent with the grant.
        #[serde(default)]
        scope: Option<String>,
        /// Header override (default `Authorization: Bearer <access_token>`).
        #[serde(default)]
        header: Option<String>,
    },
    /// OAuth 2.0 refresh-token grant against `tokenUrl`: for user-context
    /// tokens acquired out of band (a device-code flow, a manual login).
    ///
    /// The refresh token never appears in a request log: it goes to the
    /// token endpoint over the same transport as the steps, and the
    /// access token it yields is cached exactly like a client-credentials
    /// token. Static placement of the refresh token itself is never
    /// offered on purpose — a refresh token is a long-lived credential,
    /// and treating it as an API key would put it on every request.
    RefreshToken {
        /// Token endpoint URL.
        token_url: String,
        /// OAuth client id.
        client_id: String,
        /// OAuth client secret.
        #[serde(default)]
        client_secret: Option<String>,
        /// The refresh token acquired out of band.
        refresh_token: String,
        /// Optional scope string sent with the grant.
        #[serde(default)]
        scope: Option<String>,
        /// Header override (default `Authorization: Bearer <access_token>`).
        #[serde(default)]
        header: Option<String>,
    },
}

/// Per-scheme credential configuration: `suspect.extensions`-style JSON
/// under the runner's `auth.schemes` section.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AuthConfig {
    /// Credentials by security scheme name.
    #[serde(default)]
    pub schemes: BTreeMap<String, Credential>,
}

/// Interpolates `${VAR}` references in a credentials file's strings.
///
/// The committed credentials file names variables; the environment holds
/// values — so the file can be committed or shared without carrying a
/// single secret. A missing variable is an error that names the variable,
/// never the value that referenced it.
pub fn interpolate_env(value: &serde_json::Value) -> Result<serde_json::Value, String> {
    match value {
        serde_json::Value::String(text) => {
            let mut out = String::with_capacity(text.len());
            let mut rest = text.as_str();
            while let Some(open) = rest.find("${") {
                out.push_str(&rest[..open]);
                let after = &rest[open + 2..];
                let Some(close) = after.find('}') else {
                    return Err(format!("credentials: unterminated ${{ in {text:?}"));
                };
                let name = &after[..close];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        return Err(format!(
                            "credentials: ${{{name}}} is not set in the environment"
                        ));
                    }
                }
                rest = &after[close + 1..];
            }
            out.push_str(rest);
            Ok(serde_json::Value::String(out))
        }
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, value) in map {
                out.insert(key.clone(), interpolate_env(value)?);
            }
            Ok(serde_json::Value::Object(out))
        }
        serde_json::Value::Array(items) => Ok(serde_json::Value::Array(
            items
                .iter()
                .map(interpolate_env)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        other => Ok(other.clone()),
    }
}

/// Runtime auth state: acquired client-credentials tokens keyed by scheme.
#[derive(Default)]
pub struct AuthState {
    tokens: tokio::sync::Mutex<BTreeMap<String, CachedToken>>,
}

struct CachedToken {
    access_token: String,
    expires_at: Option<Instant>,
}

/// One OAuth grant, encoded as a form body. The body is percent-encoded
/// field by field: a secret containing `&`, `=` or `%` is a value, not
/// structure, and concatenating it into a form string would either corrupt
/// the grant or inject fields.
/// The application/x-www-form-urlencoded value set: unreserved marks
/// (`-._~`) stay literal, everything else is percent-encoded. Encoding the
/// unreserved marks too would be spec-legal but reads as garbage in token
/// endpoint logs and breaks naive servers.
const FORM_ENCODE: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

enum OAuthGrant<'a> {
    ClientCredentials {
        client_id: &'a str,
        client_secret: &'a str,
        scope: Option<&'a str>,
    },
    RefreshToken {
        client_id: &'a str,
        client_secret: Option<&'a str>,
        refresh_token: &'a str,
        scope: Option<&'a str>,
    },
}

impl OAuthGrant<'_> {
    const fn name(&self) -> &'static str {
        match self {
            Self::ClientCredentials { .. } => "client-credentials",
            Self::RefreshToken { .. } => "refresh-token",
        }
    }

    fn form_body(&self) -> Vec<u8> {
        let mut fields: Vec<(String, String)> = vec![(
            "grant_type".to_owned(),
            match self {
                Self::ClientCredentials { .. } => "client_credentials".to_owned(),
                Self::RefreshToken { .. } => "refresh_token".to_owned(),
            },
        )];
        match self {
            Self::ClientCredentials {
                client_id,
                client_secret,
                scope,
            } => {
                fields.push(("client_id".to_owned(), (*client_id).to_owned()));
                fields.push(("client_secret".to_owned(), (*client_secret).to_owned()));
                if let Some(scope) = scope {
                    fields.push(("scope".to_owned(), (*scope).to_owned()));
                }
            }
            Self::RefreshToken {
                client_id,
                client_secret,
                refresh_token,
                scope,
            } => {
                fields.push(("client_id".to_owned(), (*client_id).to_owned()));
                if let Some(secret) = client_secret {
                    fields.push(("client_secret".to_owned(), (*secret).to_owned()));
                }
                fields.push(("refresh_token".to_owned(), (*refresh_token).to_owned()));
                if let Some(scope) = scope {
                    fields.push(("scope".to_owned(), (*scope).to_owned()));
                }
            }
        }
        let mut body = Vec::new();
        for (key, value) in &fields {
            if !body.is_empty() {
                body.push(b'&');
            }
            body.extend_from_slice(key.as_bytes());
            body.push(b'=');
            let encoded = percent_encoding::utf8_percent_encode(value, FORM_ENCODE).to_string();
            body.extend_from_slice(encoded.as_bytes());
        }
        body
    }
}

/// A credential resolved to concrete wire placement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Injected {
    /// Header name/value pair.
    Header(String, String),
    /// Query parameter name/value pair.
    Query(String, String),
}

impl AuthState {
    /// Resolves the wire placement for `scheme` against `credential`,
    /// acquiring a client-credentials token through `http` when needed
    /// (cached until expiry).
    pub async fn resolve(
        &self,
        http: &dyn HttpClient,
        scheme: &str,
        credential: &Credential,
    ) -> Result<Option<Injected>, String> {
        match credential {
            Credential::Bearer { token, header } => Ok(Some(Injected::Header(
                header.clone().unwrap_or_else(|| "Authorization".to_owned()),
                format!("Bearer {token}"),
            ))),
            Credential::ApiKey { name, value } => {
                Ok(Some(Injected::Header(name.clone(), value.clone())))
            }
            Credential::ClientCredentials {
                token_url,
                client_id,
                client_secret,
                scope,
                header,
            } => {
                let token = self
                    .oauth_token(
                        scheme,
                        http,
                        token_url,
                        &OAuthGrant::ClientCredentials {
                            client_id,
                            client_secret,
                            scope: scope.as_deref(),
                        },
                    )
                    .await?;
                Ok(Some(Injected::Header(
                    header.clone().unwrap_or_else(|| "Authorization".to_owned()),
                    format!("Bearer {token}"),
                )))
            }
            Credential::RefreshToken {
                token_url,
                client_id,
                client_secret,
                refresh_token,
                scope,
                header,
            } => {
                let token = self
                    .oauth_token(
                        scheme,
                        http,
                        token_url,
                        &OAuthGrant::RefreshToken {
                            client_id,
                            client_secret: client_secret.as_deref(),
                            refresh_token,
                            scope: scope.as_deref(),
                        },
                    )
                    .await?;
                Ok(Some(Injected::Header(
                    header.clone().unwrap_or_else(|| "Authorization".to_owned()),
                    format!("Bearer {token}"),
                )))
            }
        }
    }

    /// Acquires (or reuses) an OAuth access token. The grant request
    /// flows through the same [`HttpClient`] as the steps, so canned
    /// transports cover it deterministically, and the token cache is
    /// shared by every grant kind.
    async fn oauth_token(
        &self,
        scheme: &str,
        http: &dyn HttpClient,
        token_url: &str,
        grant: &OAuthGrant<'_>,
    ) -> Result<String, String> {
        {
            let cache = self.tokens.lock().await;
            if let Some(cached) = cache.get(scheme) {
                let expired = cached.expires_at.is_some_and(|at| Instant::now() >= at);
                if !expired {
                    return Ok(cached.access_token.clone());
                }
            }
        }
        let request = HttpRequest {
            method: "POST".to_owned(),
            url: token_url.to_owned(),
            headers: vec![(
                "content-type".to_owned(),
                "application/x-www-form-urlencoded".to_owned(),
            )],
            body: grant.form_body().into(),
        };
        let response = http
            .execute(request)
            .await
            .map_err(|e| format!("{} grant for `{scheme}` failed: {e}", grant.name()))?;
        if response.status != 200 {
            return Err(format!(
                "{} grant for `{scheme}` returned status {}",
                grant.name(),
                response.status
            ));
        }
        let parsed: serde_json::Value = serde_json::from_slice(&response.body)
            .map_err(|e| format!("token response for `{scheme}` is not JSON: {e}"))?;
        let access_token = parsed
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("token response for `{scheme}` carries no access_token"))?
            .to_owned();
        let expires_in = parsed.get("expires_in").and_then(|v| v.as_u64());
        let mut cache = self.tokens.lock().await;
        let expires_at = expires_in.map(|seconds| {
            // Refresh before expiry to avoid clock-skew 401s.
            Instant::now() + Duration::from_secs(seconds.saturating_sub(30).max(1))
        });
        cache.insert(
            scheme.to_owned(),
            CachedToken {
                access_token: access_token.clone(),
                expires_at,
            },
        );
        Ok(access_token)
    }
}

/// Applies resolved credentials to a request: explicit step parameters win
/// (the runner injects only when the request doesn't already carry the
/// target header/query key).
pub fn inject(request: &mut HttpRequest, injected: &[Injected]) {
    for placement in injected {
        match placement {
            Injected::Header(name, value) => {
                match request
                    .headers
                    .iter_mut()
                    .find(|(k, _)| k.eq_ignore_ascii_case(name))
                {
                    // An explicit parameter that resolved to nothing — a
                    // required workflow input that was never supplied —
                    // leaves the request without a credential; injection
                    // fills it. A real value always wins.
                    Some(existing) if existing.1.is_empty() => {
                        existing.1.clone_from(value);
                    }
                    Some(_) => {}
                    None => request.headers.push((name.clone(), value.clone())),
                }
            }
            Injected::Query(name, value) => {
                let sep = if request.url.contains('?') { '&' } else { '?' };
                if !request.url.contains(&format!("{name}=")) {
                    request.url.push(sep);
                    request.url.push_str(name);
                    request.url.push('=');
                    request.url.push_str(value);
                }
            }
        }
    }
}

/// The default credentials file name, relative to the workspace root.
pub const CREDENTIALS_FILE: &str = ".suspect/credentials.json";

/// Loads and validates the credentials file at `path`.
///
/// The file holds an `{"auth": {"schemes": …}}` document — the same shape
/// the runner's configuration uses — with `${VAR}` references resolved
/// from the environment. Loose permissions (group/other readable) draw a
/// warning rather than an error: a credential file that will not work is
/// still worth reporting, and read-only checkouts sometimes carry modes
/// that cannot be tightened.
pub fn load_credentials_file(path: &std::path::Path) -> Result<AuthConfig, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path)
            && meta.permissions().mode() & 0o077 != 0
        {
            eprintln!(
                "suspect: warning: {display} is readable by group/other; tighten it with `chmod 600`",
                display = path.display()
            );
        }
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("credentials file {}: {e}", path.display()))?;
    let raw: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("credentials file {}: invalid JSON: {e}", path.display()))?;
    let interpolated = interpolate_env(&raw)?;
    let root = interpolated
        .get("auth")
        .ok_or_else(|| format!("credentials file {}: no `auth` section", path.display()))?;
    serde_json::from_value::<AuthConfig>(root.clone()).map_err(|e| {
        format!(
            "credentials file {}: invalid auth config: {e}",
            path.display()
        )
    })
}

/// Discovers the credentials file by walking up from `start` to the
/// filesystem root, like `.suspect.yaml` discovery, then falls back to the
/// workspace convention `.suspect/credentials.json`.
///
/// An explicit path (`--credentials`, `SUSPECT_CREDENTIALS`) is used as-is.
/// Discovery returns `None` when no file exists — no credential is a
/// valid state, reported distinctly from a file that fails to parse.
pub fn discover_credentials(start: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut dir = Some(start);
    while let Some(current) = dir {
        let candidate = current.join(CREDENTIALS_FILE);
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = current.parent();
    }
    None
}

/// Parses an [] from the runner-style  JSON document.
/// Parses `auth` configuration from runner settings JSON
/// (`{"auth": {"schemes": {...}}}`).
#[must_use]
pub fn auth_from_config(value: &serde_json::Value) -> AuthConfig {
    value
        .get("auth")
        .and_then(|auth| auth.get("schemes"))
        .and_then(|schemes| serde_json::from_value(schemes.clone()).ok())
        .unwrap_or_default()
}
