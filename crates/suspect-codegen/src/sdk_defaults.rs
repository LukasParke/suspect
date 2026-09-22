//! Versioned SDK behavior defaults: application-selected generation policy that
//! never reads an environment variable, a file, or a remote resource. Like
//! `credential_env`, omitted configuration resolves to documented golden
//! defaults and every accepted value participates in the generation fingerprint.
//!
//! Configuration is normative policy, not an inference result: pagination
//! aliases and per-operation overrides extend or replace built-in detection,
//! `env_prefix` selects the automatic API-key environment convention, and the
//! `oauth` section names the client credentials, supplemental endpoints, and
//! storage/refresh policy for used OAuth 2.0 / OpenID Connect schemes.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Deserializer, Serialize, de};

pub use crate::http_contract::HttpDiagnostic;
use crate::http_protocol::SourceLocation;
use suspect_ir::contract::{Contract, SourceId};

/// Closed version of the SDK defaults policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SdkDefaultsVersion {
    #[serde(rename = "v1")]
    V1,
}

/// Golden SDK defaults. Absent fields mean "use the documented default", and
/// explicit values are never silently combined with different implicit ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SdkDefaults {
    pub version: SdkDefaultsVersion,
    /// Stable service identity for the automatic API-key environment variable
    /// (`<ENV_PREFIX>_API_KEY`). Explicit `credential_env` mappings remain the
    /// precise override and take precedence when both are configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_prefix: Option<String>,
    #[serde(default)]
    pub pagination: PaginationDefaults,
    /// OAuth 2.0 / OpenID Connect lifecycle policy; omitted resolves to the
    /// documented default. Serialization skips the default so existing
    /// fingerprints and captures stay byte-identical.
    #[serde(default, skip_serializing_if = "OAuthDefaults::is_default")]
    pub oauth: OAuthDefaults,
}

impl Default for SdkDefaults {
    fn default() -> Self {
        Self {
            version: SdkDefaultsVersion::V1,
            env_prefix: None,
            pagination: PaginationDefaults::default(),
            oauth: OAuthDefaults::default(),
        }
    }
}

impl SdkDefaults {
    #[must_use]
    pub fn v1() -> Self {
        Self::default()
    }

    /// Typed client-default metadata for compatibility capture. Provenance
    /// stays out of interface equality, mirroring
    /// `CredentialEnvPlan::semantic_descriptor`.
    #[must_use]
    pub fn semantic_descriptor(&self) -> SdkDefaultsDescriptor {
        SdkDefaultsDescriptor {
            version: self.version,
            env_prefix: self.env_prefix.clone(),
            pagination: self.pagination.clone(),
            oauth: self.oauth.clone(),
        }
    }

    fn validate(&self) -> Result<(), String> {
        if let Some(prefix) = &self.env_prefix {
            let bytes = prefix.as_bytes();
            let valid = !bytes.is_empty()
                && bytes.len() <= 64
                && bytes
                    .first()
                    .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
                && bytes
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_');
            if !valid {
                return Err(
                    "sdk_defaults.env_prefix needs a portable 1..=64 byte environment prefix of ASCII letters, digits and underscores starting with a letter or underscore"
                        .into(),
                );
            }
            if prefix.len() + "_API_KEY".len() > 128 {
                return Err(
                    "sdk_defaults.env_prefix must leave room for the composed variable name".into(),
                );
            }
        }
        self.pagination.validate()?;
        self.oauth.validate()
    }
}

impl<'de> Deserialize<'de> for SdkDefaults {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            version: SdkDefaultsVersion,
            #[serde(default)]
            env_prefix: Option<String>,
            #[serde(default)]
            pagination: PaginationDefaults,
            #[serde(default)]
            oauth: OAuthDefaults,
        }
        let raw = Raw::deserialize(deserializer)?;
        let defaults = Self {
            version: raw.version,
            env_prefix: raw.env_prefix,
            pagination: raw.pagination,
            oauth: raw.oauth,
        };
        defaults.validate().map_err(de::Error::custom)?;
        Ok(defaults)
    }
}

/// Automatic detection is the default; `off` disables inference while explicit
/// per-operation configuration remains available through `operations`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaginationMode {
    #[default]
    Auto,
    Off,
}

/// Built-in pagination pattern kinds. `last-item-cursor` is a refinement of
/// `cursor` selected during detection, not a separate configuration value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaginationPattern {
    LimitOffset,
    Cursor,
    PageNumber,
    NextLink,
}

/// Request roles addressable by aliases and per-operation overrides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaginationRequestRole {
    Limit,
    Offset,
    Cursor,
    Page,
}

/// Response roles addressable by per-operation overrides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaginationResponseRole {
    Items,
    Total,
    NextCursor,
    NextOffset,
    HasMore,
    TotalPages,
}

/// The role each alias list extends. Values are normalized request-key synonyms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaginationAliasRole {
    Limit,
    Offset,
    Cursor,
    Page,
}

/// Declared continuation advance policy for a fully manual mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaginationAdvance {
    ItemsReturned,
    NextOffset,
}

/// Declared termination policy for a fully manual mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaginationStop {
    MissingOrNullCursor,
}

/// Per-operation pagination configuration. `false` disables pagination for the
/// operation; the expanded object replaces detection with an exact mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum PaginationOperation {
    Disabled,
    Mapping(PaginationMapping),
}

/// Exact pagination mapping for one operation. Request strings name actual
/// request parameters; response values are JSON Pointers (RFC 6901) relative to
/// the decoded response body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaginationMapping {
    pub pattern: PaginationPattern,
    #[serde(default)]
    pub request: BTreeMap<PaginationRequestRole, String>,
    #[serde(default)]
    pub response: BTreeMap<PaginationResponseRole, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_offset: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advance: Option<PaginationAdvance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<PaginationStop>,
}

impl PaginationMapping {
    /// Roles each pattern requires before a mapping is admitted.
    #[must_use]
    pub fn required_request_roles(pattern: PaginationPattern) -> &'static [PaginationRequestRole] {
        match pattern {
            PaginationPattern::LimitOffset => {
                &[PaginationRequestRole::Limit, PaginationRequestRole::Offset]
            }
            PaginationPattern::Cursor => &[PaginationRequestRole::Cursor],
            PaginationPattern::PageNumber => &[PaginationRequestRole::Page],
            PaginationPattern::NextLink => &[],
        }
    }

    #[must_use]
    pub fn required_response_roles(
        pattern: PaginationPattern,
    ) -> &'static [PaginationResponseRole] {
        match pattern {
            PaginationPattern::LimitOffset | PaginationPattern::PageNumber => {
                &[PaginationResponseRole::Items]
            }
            PaginationPattern::Cursor => &[
                PaginationResponseRole::Items,
                PaginationResponseRole::NextCursor,
            ],
            PaginationPattern::NextLink => &[PaginationResponseRole::Items],
        }
    }
}

impl<'de> Deserialize<'de> for PaginationOperation {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Disabled(bool),
            Mapping(PaginationMapping),
        }
        match Raw::deserialize(deserializer)? {
            Raw::Disabled(false) => Ok(Self::Disabled),
            Raw::Disabled(true) => Err(de::Error::custom(
                "pagination operation `true` is not a mapping; omit the entry or provide an object",
            )),
            Raw::Mapping(mapping) => Ok(Self::Mapping(mapping)),
        }
    }
}

/// Global pagination defaults. Shorthand JSON (`"auto"`, `"off"`) normalizes to
/// the expanded representation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaginationDefaults {
    pub mode: PaginationMode,
    /// Restrict automatic detection to this subset; `None` uses the built-in
    /// default set (limit-offset, cursor, page-number, next-link).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patterns: Option<BTreeSet<PaginationPattern>>,
    /// Extra request-key synonyms per role, extending the built-in vocabulary.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub aliases: BTreeMap<PaginationAliasRole, BTreeSet<String>>,
    /// Exact per-operation mappings or explicit disables, keyed by operation.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub operations: BTreeMap<String, PaginationOperation>,
    /// Proposed SDK fallback page size, subject to source bounds. It never
    /// asserts a server default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_size: Option<u32>,
}

impl Default for PaginationDefaults {
    fn default() -> Self {
        Self {
            mode: PaginationMode::Auto,
            patterns: None,
            aliases: BTreeMap::new(),
            operations: BTreeMap::new(),
            page_size: None,
        }
    }
}

impl<'de> Deserialize<'de> for PaginationDefaults {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Expanded {
            #[serde(default)]
            mode: PaginationMode,
            #[serde(default)]
            patterns: Option<BTreeSet<PaginationPattern>>,
            #[serde(default)]
            aliases: BTreeMap<PaginationAliasRole, BTreeSet<String>>,
            #[serde(default)]
            operations: BTreeMap<String, PaginationOperation>,
            #[serde(default)]
            page_size: Option<u32>,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Shorthand(PaginationMode),
            Expanded(Expanded),
        }
        Ok(match Raw::deserialize(deserializer)? {
            Raw::Shorthand(mode) => Self {
                mode,
                ..Self::default()
            },
            Raw::Expanded(expanded) => Self {
                mode: expanded.mode,
                patterns: expanded.patterns,
                aliases: expanded.aliases,
                operations: expanded.operations,
                page_size: expanded.page_size,
            },
        })
    }
}

impl PaginationDefaults {
    #[must_use]
    pub fn auto() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn off() -> Self {
        Self {
            mode: PaginationMode::Off,
            ..Self::default()
        }
    }

    fn validate(&self) -> Result<(), String> {
        if let Some(size) = self.page_size
            && (size == 0 || size > 10_000)
        {
            return Err("sdk_defaults.pagination.page_size must be 1..=10_000".into());
        }
        let mut claimed = BTreeMap::<String, PaginationAliasRole>::new();
        for (role, names) in &self.aliases {
            if names.is_empty() {
                return Err(
                    "sdk_defaults.pagination.aliases must not contain empty synonym lists".into(),
                );
            }
            for name in names {
                validate_request_key(name).map_err(|message| {
                    format!("sdk_defaults.pagination.aliases[{role:?}]: {message}")
                })?;
                if let Some(previous) = claimed.insert(name.clone(), *role) {
                    return Err(format!(
                        "sdk_defaults.pagination.aliases: {name:?} is claimed by both {previous:?} and {role:?}"
                    ));
                }
            }
        }
        for (name, operation) in &self.operations {
            validate_operation_name(name)?;
            match operation {
                PaginationOperation::Disabled => {}
                PaginationOperation::Mapping(mapping) => {
                    mapping.validate().map_err(|message| {
                        format!("sdk_defaults.pagination.operations[{name:?}]: {message}")
                    })?;
                }
            }
        }
        Ok(())
    }
}

impl PaginationMapping {
    fn validate(&self) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for (role, name) in &self.request {
            validate_request_key(name).map_err(|message| format!("request.{role:?}: {message}"))?;
            if !seen.insert(name.clone()) {
                return Err(format!("request maps {name:?} to two roles"));
            }
        }
        for role in Self::required_request_roles(self.pattern) {
            if !self.request.contains_key(role) {
                return Err(format!(
                    "pattern {:?} requires request role {role:?}",
                    self.pattern
                ));
            }
        }
        let mut pointers = BTreeSet::new();
        for (role, pointer) in &self.response {
            validate_json_pointer(pointer)
                .map_err(|message| format!("response.{role:?}: {message}"))?;
            if !pointers.insert(pointer.clone()) {
                return Err(format!("response maps {pointer:?} to two roles"));
            }
        }
        for role in Self::required_response_roles(self.pattern) {
            if !self.response.contains_key(role) {
                return Err(format!(
                    "pattern {:?} requires response role {role:?}",
                    self.pattern
                ));
            }
        }
        Ok(())
    }
}

/// OAuth lifecycle policy mode. `off` disables OAuth lifecycle planning while
/// the protocol plan keeps describing the declarations themselves.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OAuthMode {
    #[default]
    Auto,
    Off,
}

/// Where compiled OAuth credentials live at runtime. v1 admits only the
/// in-process memory store; the enum is closed so future stores are explicit
/// configuration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OAuthStorage {
    #[default]
    Memory,
}

/// When the generated SDK refreshes OAuth tokens. v1 admits only the on-demand
/// refresh gate; future policies are explicit configuration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OAuthRefresh {
    #[default]
    OnDemand,
}

/// Per-scheme OAuth configuration, keyed by source Security Scheme name.
/// Every field is optional: absent fields narrow what the compiled plan
/// provides, they never invent endpoints.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuthSchemeConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret_env: Option<String>,
    /// Refresh-before-expiry clock skew in seconds (0..=3600, default 30).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_skew_seconds: Option<u32>,
    /// Configuration-supplied only; OpenAPI flow objects do not declare one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revocation_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub introspection_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_url: Option<String>,
}

/// The SDK defaults OAuth lifecycle section. Shorthand JSON (`"auto"`,
/// `"off"`) normalizes to the expanded representation. Absent scheme entries
/// leave declared-scheme-only planning in place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OAuthDefaults {
    pub mode: OAuthMode,
    pub storage: OAuthStorage,
    pub refresh: OAuthRefresh,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub schemes: BTreeMap<String, OAuthSchemeConfig>,
}

impl Default for OAuthDefaults {
    fn default() -> Self {
        Self {
            mode: OAuthMode::Auto,
            storage: OAuthStorage::Memory,
            refresh: OAuthRefresh::OnDemand,
            schemes: BTreeMap::new(),
        }
    }
}

impl OAuthDefaults {
    #[must_use]
    pub fn auto() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn off() -> Self {
        Self {
            mode: OAuthMode::Off,
            ..Self::default()
        }
    }

    /// Whether the section equals the documented default, so serialization and
    /// descriptor captures may omit it without changing any value.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    fn validate(&self) -> Result<(), String> {
        for (name, config) in &self.schemes {
            if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
                return Err(
                    "sdk_defaults.oauth.schemes names must be 1..=256 bytes without control characters"
                        .into(),
                );
            }
            if let Some(variable) = &config.client_id_env {
                crate::credential_env::validate_variable_name(variable).map_err(|message| {
                    format!("sdk_defaults.oauth.schemes[{name:?}].client_id_env {message}")
                })?;
            }
            if let Some(variable) = &config.client_secret_env {
                crate::credential_env::validate_variable_name(variable).map_err(|message| {
                    format!("sdk_defaults.oauth.schemes[{name:?}].client_secret_env {message}")
                })?;
            }
            if let Some(skew) = config.refresh_skew_seconds
                && skew > 3600
            {
                return Err(format!(
                    "sdk_defaults.oauth.schemes[{name:?}].refresh_skew_seconds must be 0..=3600"
                ));
            }
            for (field, endpoint) in [
                ("revocation_endpoint", &config.revocation_endpoint),
                ("introspection_endpoint", &config.introspection_endpoint),
                ("discovery_url", &config.discovery_url),
            ] {
                if let Some(endpoint) = endpoint {
                    validate_oauth_endpoint(endpoint).map_err(|message| {
                        format!("sdk_defaults.oauth.schemes[{name:?}].{field} {message}")
                    })?;
                }
            }
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for OAuthDefaults {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Expanded {
            #[serde(default)]
            mode: OAuthMode,
            #[serde(default)]
            storage: OAuthStorage,
            #[serde(default)]
            refresh: OAuthRefresh,
            #[serde(default)]
            schemes: BTreeMap<String, OAuthSchemeConfig>,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Shorthand(OAuthMode),
            Expanded(Expanded),
        }
        Ok(match Raw::deserialize(deserializer)? {
            Raw::Shorthand(mode) => Self {
                mode,
                ..Self::default()
            },
            Raw::Expanded(expanded) => Self {
                mode: expanded.mode,
                storage: expanded.storage,
                refresh: expanded.refresh,
                schemes: expanded.schemes,
            },
        })
    }
}

/// Basic absolute-URL admission for configured OAuth endpoints: an http(s)
/// scheme with a non-empty host, and plain HTTP only for loopback targets.
fn validate_oauth_endpoint(endpoint: &str) -> Result<(), String> {
    let url =
        url::Url::parse(endpoint).map_err(|_| "must be an absolute http(s) URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("must be an absolute http(s) URL".into());
    }
    let Some(host) = url.host_str() else {
        return Err("must name a URL host".into());
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let loopback =
        host.eq_ignore_ascii_case("localhost") || host == "::1" || host.starts_with("127.");
    if url.scheme() == "http" && !loopback {
        return Err("plain http is admitted only for loopback endpoints; use https".into());
    }
    Ok(())
}

/// Validate one actual request-key spelling (query parameter or body field).
pub(crate) fn validate_request_key(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        return Err("request keys must be 1..=256 bytes without control characters".into());
    }
    Ok(())
}

fn validate_operation_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        return Err("operation selectors must be 1..=256 bytes without control characters".into());
    }
    Ok(())
}

/// RFC 6901 JSON Pointer: empty selects the whole document; otherwise
/// `/`-prefixed with `~0`/`~1` escapes.
pub(crate) fn validate_json_pointer(pointer: &str) -> Result<(), String> {
    if pointer.is_empty() {
        return Ok(());
    }
    if !pointer.starts_with('/') {
        return Err("response pointers must be empty or start with '/'".into());
    }
    if pointer.len() > 512 {
        return Err("response pointers must be at most 512 bytes".into());
    }
    for part in pointer[1..].split('/') {
        if part.contains('\u{0}') || part.contains('\u{7f}') {
            return Err("response pointer segments must not contain control characters".into());
        }
    }
    Ok(())
}

/// Typed client-default metadata for compatibility capture. Provenance stays
/// out of interface equality, mirroring `CredentialEnvDescriptor`. The OAuth
/// section is omitted from the encoding when it equals the default, so
/// snapshots recorded before it existed still deserialize unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SdkDefaultsDescriptor {
    pub version: SdkDefaultsVersion,
    pub env_prefix: Option<String>,
    pub pagination: PaginationDefaults,
    #[serde(default, skip_serializing_if = "OAuthDefaults::is_default")]
    pub oauth: OAuthDefaults,
}

fn diagnostic(
    contract: &Contract,
    location: Option<&SourceLocation>,
    code: &'static str,
    message: impl Into<String>,
) -> HttpDiagnostic {
    let source = location.map_or_else(
        || SourceId::new(contract.entry().clone(), Default::default()),
        |location| location.source().clone(),
    );
    let at = location.map_or_else(
        || contract.source_span(&source).unwrap_or(0..0),
        SourceLocation::span,
    );
    HttpDiagnostic {
        source,
        at,
        code,
        message: message.into(),
    }
}

/// Re-validate a policy that reached this module through a non-serde path
/// (programmatic construction) before any planner consumes it.
///
/// # Errors
/// Invalid prefixes, page sizes, aliases, per-operation mappings, or OAuth
/// scheme configuration, described with the offending configuration path.
pub fn admit(
    contract: &Contract,
    defaults: &SdkDefaults,
    provenance: Option<&SourceLocation>,
) -> Result<(), HttpDiagnostic> {
    defaults
        .validate()
        .map_err(|message| diagnostic(contract, provenance, "sdk-defaults-config", message))
}
