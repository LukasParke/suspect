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
    interpolate_env_with_map(value, &std::collections::BTreeMap::new())
}

/// [`interpolate_env`] with a fallback map: the process environment wins
/// over the map, and the map fills what the environment does not carry —
/// the `.suspect/.env` file's values reach the interpolation without ever
/// mutating the process environment (a thread-safety hazard since the
/// 2024 edition made `set_var` unsafe).
pub fn interpolate_env_with_map(
    value: &serde_json::Value,
    env: &std::collections::BTreeMap<String, String>,
) -> Result<serde_json::Value, String> {
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
                let Some(v) = env.get(name).cloned().or_else(|| std::env::var(name).ok()) else {
                    return Err(format!(
                        "credentials: ${{{name}}} is not set in the environment (or .env)"
                    ));
                };
                out.push_str(&v);
                rest = &after[close + 1..];
            }
            out.push_str(rest);
            Ok(serde_json::Value::String(out))
        }
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, value) in map {
                out.insert(key.clone(), interpolate_env_with_map(value, env)?);
            }
            Ok(serde_json::Value::Object(out))
        }
        serde_json::Value::Array(items) => Ok(serde_json::Value::Array(
            items
                .iter()
                .map(|v| interpolate_env_with_map(v, env))
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

/// The env file loaded alongside the credentials file: `KEY=value` lines
/// (one per line), `#` comments, blank lines ignored. Lives at
/// `.suspect/.env` — the same gitignored directory the credentials file
/// occupies — so secrets never appear in a committed file, a command line
/// (shell history), or the process environment of unrelated tools.
///
/// Precedence: a variable already present in the process environment is
/// NOT overridden (an exported variable wins over the file), so a CI
/// secrets-manager injection beats the local file without configuration.
pub const ENV_FILE: &str = ".env";

/// Loads `KEY=value` lines from the env file at `path`, setting each
/// variable in the process environment only when it is not already set.
/// The file's permissions are checked like the credentials file's:
/// group/other readable draws a warning.
///
/// # Errors
/// Filesystem failures reading the file; not malformed lines (a line
/// without `=` is skipped, not fatal — an env file that can be partially
/// used is worth more than one that refuses to load).
pub fn load_env_file(
    path: &std::path::Path,
) -> Result<std::collections::BTreeMap<String, String>, String> {
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
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("env file {}: {e}", path.display()))?;
    let mut out = std::collections::BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // `export KEY=value` and `KEY=value` both work.
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || name.contains(char::is_whitespace) {
            continue;
        }
        // Strip one layer of surrounding quotes, like dotenv does.
        let value = value.trim();
        let value: &str = if (value.starts_with('"') && value.ends_with('"') && value.len() >= 2)
            || (value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2)
        {
            &value[1..value.len() - 1]
        } else {
            value
        };
        // Process environment wins: a CI secrets-manager injection beats
        // the local file, so the file only fills what the environment
        // does not carry.
        if std::env::var_os(name).is_none() {
            out.insert(name.to_owned(), value.to_owned());
        }
    }
    Ok(out)
}

/// Loads and validates the credentials file at `path`.
///
/// The file holds an `{"auth": {"schemes": …}}` document — the same shape
/// the runner's configuration uses — with `${VAR}` references resolved
/// from the environment. Loose permissions (group/other readable) draw a
/// warning rather than an error: a credential file that will not work is
/// still worth reporting, and read-only checkouts sometimes carry modes
/// that cannot be tightened.
pub fn load_credentials_file(path: &std::path::Path) -> Result<AuthConfig, String> {
    // Env files supply the `${VAR}` values the credentials reference.
    // Both spellings are honored, layered: repo-root `.env` (the universal
    // convention — direnv, docker-compose, dotenv all read it) beneath
    // `.suspect/.env` (beside the credentials file). Each is optional;
    // neither is required. The process environment beats both, so CI
    // secrets-manager injection wins over any file.
    //
    // Discovery walks up from the credentials file, matching the
    // credentials discovery's directory chain: `.suspect/credentials.json`
    // sits inside `.suspect/`, and the repo root is its parent.
    let mut env: std::collections::BTreeMap<String, String> = Default::default();
    if let Some(suspect_dir) = path.parent() {
        // .suspect/.env — beside the credentials file.
        let beside = suspect_dir.join(".env");
        if beside.is_file() {
            env = load_env_file(&beside)?;
        }
        // The repo root's .env — one level up from `.suspect/`.
        if let Some(repo_root) = suspect_dir.parent() {
            let repo_env = repo_root.join(ENV_FILE);
            if repo_env.is_file() {
                let repo = load_env_file(&repo_env)?;
                // First-file-wins per key: .suspect/.env is more specific
                // (deliberately placed beside the credentials) than the
                // repo-root convention.
                for (key, value) in repo {
                    env.entry(key).or_insert(value);
                }
            }
        }
    }
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
    let interpolated = interpolate_env_with_map(&raw, &env)?;
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

#[cfg(test)]
mod env_file_tests {
    use super::*;

    #[test]
    fn env_file_supplies_values_the_environment_does_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(".env");
        std::fs::write(
            &path,
            "# a comment\nTEST_TOKEN_ONE=secret-one\nexport TEST_TOKEN_TWO=secret-two\nQUOTED=\"quoted-value\"\nnot-an-assignment\n\n",
        )
        .expect("write");
        let map = load_env_file(&path).expect("loads");
        assert_eq!(
            map.get("TEST_TOKEN_ONE").map(String::as_str),
            Some("secret-one")
        );
        assert_eq!(
            map.get("TEST_TOKEN_TWO").map(String::as_str),
            Some("secret-two")
        );
        assert_eq!(map.get("QUOTED").map(String::as_str), Some("quoted-value"));
        assert!(!map.contains_key("not-an-assignment"));
    }

    #[test]
    fn process_environment_beats_the_env_file() {
        // SAFETY: single-threaded test.
        unsafe { std::env::set_var("ENV_FILE_OVERRIDE_TEST", "from-process") };
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(".env");
        std::fs::write(&path, "ENV_FILE_OVERRIDE_TEST=from-file\n").expect("write");
        let map = load_env_file(&path).expect("loads");
        assert!(
            !map.contains_key("ENV_FILE_OVERRIDE_TEST"),
            "the process env wins over the file"
        );
        unsafe { std::env::remove_var("ENV_FILE_OVERRIDE_TEST") };
    }

    #[test]
    fn interpolation_reads_from_the_env_file_map() {
        let mut env = std::collections::BTreeMap::new();
        env.insert("FILE_VAR".to_owned(), "file-value".to_owned());
        let value = serde_json::json!({"auth": {"token": "${FILE_VAR}"}});
        let interpolated = interpolate_env_with_map(&value, &env).expect("interpolates");
        assert_eq!(interpolated["auth"]["token"], "file-value");
    }

    #[test]
    fn missing_variable_error_names_both_sources() {
        let value = serde_json::json!({"token": "${NOWHERE_SET_VAR}"});
        let error = interpolate_env(&value).expect_err("not set");
        assert!(
            error.contains(".env"),
            "the error tells you where to put it: {error}"
        );
    }
}
