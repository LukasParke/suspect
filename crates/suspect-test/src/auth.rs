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
}

/// Per-scheme credential configuration: `suspect.extensions`-style JSON
/// under the runner's `auth.schemes` section.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AuthConfig {
    /// Credentials by security scheme name.
    #[serde(default)]
    pub schemes: BTreeMap<String, Credential>,
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
                    .client_credentials_token(
                        scheme,
                        http,
                        token_url,
                        client_id,
                        client_secret,
                        scope.as_deref(),
                    )
                    .await?;
                Ok(Some(Injected::Header(
                    header.clone().unwrap_or_else(|| "Authorization".to_owned()),
                    format!("Bearer {token}"),
                )))
            }
        }
    }

    /// Acquires (or reuses) a client-credentials access token. The grant
    /// request flows through the same [`HttpClient`] as the steps, so
    /// canned transports cover it deterministically.
    async fn client_credentials_token(
        &self,
        scheme: &str,
        http: &dyn HttpClient,
        token_url: &str,
        client_id: &str,
        client_secret: &str,
        scope: Option<&str>,
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
        let mut form = format!(
            "grant_type=client_credentials&client_id={client_id}&client_secret={client_secret}"
        );
        if let Some(scope) = scope {
            form.push_str("&scope=");
            form.push_str(scope);
        }
        let request = HttpRequest {
            method: "POST".to_owned(),
            url: token_url.to_owned(),
            headers: vec![(
                "content-type".to_owned(),
                "application/x-www-form-urlencoded".to_owned(),
            )],
            body: form.into_bytes().into(),
        };
        let response = http
            .execute(request)
            .await
            .map_err(|e| format!("client-credentials grant for `{scheme}` failed: {e}"))?;
        if response.status != 200 {
            return Err(format!(
                "client-credentials grant for `{scheme}` returned status {}",
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
        if let Some(seconds) = expires_in {
            // Refresh 30s before expiry to avoid clock-skew 401s.
            cache.insert(
                scheme.to_owned(),
                CachedToken {
                    access_token: access_token.clone(),
                    expires_at: Some(
                        Instant::now() + Duration::from_secs(seconds.saturating_sub(30)),
                    ),
                },
            );
        } else {
            cache.insert(
                scheme.to_owned(),
                CachedToken {
                    access_token: access_token.clone(),
                    expires_at: None,
                },
            );
        }
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
                if !request
                    .headers
                    .iter()
                    .any(|(k, _)| k.eq_ignore_ascii_case(name))
                {
                    request.headers.push((name.clone(), value.clone()));
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
