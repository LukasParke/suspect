//! Generation-time OAuth 2.0 / OpenID Connect lifecycle planning.
//!
//! [`plan`] walks the admitted protocol plan's security requirements and
//! compiles one descriptor per used source scheme. Descriptors carry exactly
//! what the declaration states plus what configuration supplements; endpoints
//! are never invented, and missing or ambiguous configuration only narrows
//! what the plan provides. Runtime helpers gate on what is absent.
//!
//! Lifecycle execution is deliberately out of scope: implicit and password
//! flows are represented (`deprecated_flow`) but never executed by generated
//! code. PKCE authorization-code is the normal interactive default and
//! client-credentials the normal service default; execution support belongs to
//! explicitly selected compatibility helpers.

use std::collections::BTreeMap;

use serde::Serialize;
use suspect_ir::contract::{Contract, SourceId};

use super::model::{
    CredentialHook, CredentialRequirement, OAuthFlow, OAuthFlowKind as DeclaredFlowKind,
    ProtocolPlan,
};
use super::{OperationPlan, Representation, SourceLocation};
pub use crate::http_contract::HttpDiagnostic;
pub use crate::sdk_defaults::{
    OAuthDefaults, OAuthMode, OAuthRefresh, OAuthSchemeConfig, OAuthStorage, SdkDefaults,
};

/// Which kind of source Security Scheme a descriptor compiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OAuthSchemeKind {
    /// `type: oauth2`.
    #[serde(rename = "oauth2")]
    OAuth2,
    /// `type: openIdConnect`.
    #[serde(rename = "open-id-connect")]
    OpenIdConnect,
}

/// Token-endpoint client authentication derived per flow: the standard
/// `client_secret_basic` default for confidential clients with a configured
/// secret variable, and none for public clients (including the fragment-based
/// implicit flow, which has no token endpoint at all).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OAuthClientAuth {
    /// HTTP Basic with the client id and secret (RFC 6749 section 2.3.1).
    ClientSecretBasic,
    /// Public client: only `client_id` is sent.
    None,
}

/// One compiled flow. Endpoint URLs are the declared absolute http(s) URLs;
/// absent declarations stay absent and are never invented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OAuthFlowDescriptor {
    /// Grant this flow performs.
    pub kind: OAuthFlowDescriptorKind,
    /// Declared `authorizationUrl`.
    pub authorization_url: Option<String>,
    /// Declared `tokenUrl`.
    pub token_url: Option<String>,
    /// Declared `refreshUrl`.
    pub refresh_url: Option<String>,
    /// OAS 3.2 device-authorization endpoint.
    pub device_authorization_url: Option<String>,
    /// Declared scope name to description, in sorted order.
    pub scopes: BTreeMap<String, String>,
    /// How the client authenticates to the token endpoint.
    pub client_auth: OAuthClientAuth,
    /// Implicit and password flows are represented for documentation but are
    /// never executed by generated code.
    pub deprecated_flow: bool,
}

/// Declared OAuth grant/flow kinds, independent of the source spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OAuthFlowDescriptorKind {
    /// Implicit grant (deprecated; never executed).
    Implicit,
    /// Resource owner password grant (deprecated; never executed).
    Password,
    /// Client credentials grant.
    ClientCredentials,
    /// Authorization code grant; PKCE is the interactive default.
    AuthorizationCode,
    /// Device authorization grant (RFC 8628, OAS 3.2).
    DeviceAuthorization,
}

/// One used source scheme's compiled OAuth/OIDC lifecycle descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OAuthSchemePlan {
    /// Source Security Scheme name; also the `sdk_defaults.oauth.schemes`
    /// configuration key for supplements.
    pub name: String,
    /// Declared scheme type.
    pub kind: OAuthSchemeKind,
    /// Declared flows in declaration order. Empty for OpenID Connect, whose
    /// flows the discovery document defines at runtime.
    pub flows: Vec<OAuthFlowDescriptor>,
    /// Configured environment variable holding the client id.
    pub client_id_env: Option<String>,
    /// Configured environment variable holding the client secret.
    pub client_secret_env: Option<String>,
    /// Refresh-before-expiry clock skew in seconds; default 30.
    pub refresh_skew_seconds: u32,
    /// Configuration-supplied only; OpenAPI flow objects do not declare one.
    pub revocation_endpoint: Option<String>,
    /// Configuration-supplied only, like `revocation_endpoint`.
    pub introspection_endpoint: Option<String>,
    /// The known discovery/metadata document: the declared `openIdConnectUrl`
    /// for OpenID Connect, otherwise the declared `oauth2MetadataUrl` or the
    /// configured `discovery_url` for OAuth2.
    pub discovery: Option<String>,
    /// Runtime token store.
    pub storage: OAuthStorage,
    /// Runtime refresh policy.
    pub refresh: OAuthRefresh,
}

/// Complete generation-time OAuth lifecycle planning for one document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct OAuthPlan {
    /// Configured lifecycle mode; `Off` leaves `schemes` empty.
    pub mode: OAuthMode,
    /// Compiled descriptors for used source schemes, sorted by name.
    pub schemes: Vec<OAuthSchemePlan>,
}

impl OAuthFlowDescriptor {
    /// Whether generated code runs this flow: it is not a deprecated grant
    /// (implicit, password — see ADR 0027) and every endpoint its grant
    /// needs is declared. Every backend uses this one definition, so a flow
    /// missing its token endpoint behaves the same in all twelve SDKs.
    #[must_use]
    pub fn is_executable(&self) -> bool {
        if self.deprecated_flow {
            return false;
        }
        match self.kind {
            OAuthFlowDescriptorKind::ClientCredentials => self.token_url.is_some(),
            OAuthFlowDescriptorKind::AuthorizationCode => {
                self.authorization_url.is_some() && self.token_url.is_some()
            }
            OAuthFlowDescriptorKind::DeviceAuthorization => {
                self.device_authorization_url.is_some() && self.token_url.is_some()
            }
            OAuthFlowDescriptorKind::Implicit | OAuthFlowDescriptorKind::Password => false,
        }
    }
}

impl OAuthSchemePlan {
    /// The flows generated code runs, in declaration order.
    pub fn executable_flows(&self) -> impl Iterator<Item = &OAuthFlowDescriptor> {
        self.flows.iter().filter(|flow| flow.is_executable())
    }

    /// Whether generated code runs a flow of this kind for this scheme.
    #[must_use]
    pub fn executes(&self, kind: OAuthFlowDescriptorKind) -> bool {
        self.executable_flows().any(|flow| flow.kind == kind)
    }

    /// Whether the scheme needs a runtime at all: it has an executable flow
    /// or OpenID Connect discovery (whose flows are known only at runtime).
    #[must_use]
    pub fn is_usable(&self) -> bool {
        self.discovery.is_some() || self.executable_flows().next().is_some()
    }
}

impl OAuthPlan {
    /// The schemes that need a runtime, in plan (name) order.
    pub fn usable_schemes(&self) -> impl Iterator<Item = &OAuthSchemePlan> {
        self.schemes.iter().filter(|scheme| scheme.is_usable())
    }

    /// Whether a backend emits its OAuth lifecycle module.
    #[must_use]
    pub fn emits(&self) -> bool {
        self.mode != OAuthMode::Off && self.usable_schemes().next().is_some()
    }

    /// Whether any scheme executes a flow of this kind.
    #[must_use]
    pub fn executes(&self, kind: OAuthFlowDescriptorKind) -> bool {
        self.schemes.iter().any(|scheme| scheme.executes(kind))
    }

    /// Whether any scheme uses OpenID Connect discovery.
    #[must_use]
    pub fn has_discovery(&self) -> bool {
        self.schemes.iter().any(|scheme| scheme.discovery.is_some())
    }
}

/// One OAuth/OIDC security requirement protecting a streaming operation: the
/// unit every backend's no-replay table is built from.
#[derive(Debug, Clone, Copy)]
pub struct StreamProtectedRequirement<'a, O> {
    /// The requirement's source Security Scheme name.
    pub scheme: &'a str,
    /// The backend's operation (or the bare [`OperationPlan`]) carrying it.
    pub operation: &'a O,
    /// The security requirement itself.
    pub requirement: &'a CredentialRequirement,
}

/// Every OAuth2/OpenID Connect security requirement, naming a scheme in
/// `scheme_names`, on an operation any of whose response media is a
/// [`Representation::Stream`], in operation then alternative then requirement
/// order (duplicates included).
///
/// The 401 refresh-and-replay wrapper never replays these: a stream body
/// cannot be resent once delivered. Backends key their compiled no-replay
/// table by whatever their generated runtime reads back at attach time (the
/// requirement's source pointer, its `document#pointer` address, the
/// operation's pointer or its ID), so this yields the backend's own
/// operation value alongside the requirement; `wire` projects it to the
/// protocol operation.
pub fn stream_protected_requirements<'a, O>(
    operations: impl IntoIterator<Item = &'a O>,
    wire: impl Fn(&'a O) -> &'a OperationPlan,
    scheme_names: &std::collections::BTreeSet<&str>,
) -> impl Iterator<Item = StreamProtectedRequirement<'a, O>>
where
    O: 'a,
{
    operations
        .into_iter()
        .map(move |operation| (operation, wire(operation)))
        .filter(|(_, plan)| {
            plan.responses().iter().any(|response| {
                response
                    .media()
                    .iter()
                    .any(|media| matches!(media.representation(), Representation::Stream { .. }))
            })
        })
        .flat_map(move |(operation, plan)| {
            plan.security()
                .alternatives()
                .iter()
                .flat_map(|alternative| alternative.requirements())
                .filter(move |requirement| {
                    matches!(
                        requirement.credential(),
                        CredentialHook::OAuth2 { .. } | CredentialHook::OpenIdConnect { .. }
                    ) && scheme_names.contains(requirement.name())
                })
                .map(move |requirement| StreamProtectedRequirement {
                    scheme: requirement.name(),
                    operation,
                    requirement,
                })
        })
}

/// Security-requirement source pointers, per OAuth/OIDC scheme name, that
/// protect a streaming operation (see [`stream_protected_requirements`]).
/// Backends whose runtime reads the bare pointer embed this table; the others
/// build their own key from the same iterator. Only schemes in
/// `scheme_names` are considered.
#[must_use]
pub fn no_replay_requirements<'a>(
    operations: impl IntoIterator<Item = &'a OperationPlan>,
    scheme_names: &std::collections::BTreeSet<&str>,
) -> BTreeMap<String, std::collections::BTreeSet<String>> {
    let mut pointers: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
    for found in stream_protected_requirements(operations, |operation| operation, scheme_names) {
        pointers
            .entry(found.scheme.to_owned())
            .or_default()
            .insert(found.requirement.source().source().pointer().to_owned());
    }
    pointers
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

/// Compile every used OAuth2/openIdConnect source scheme.
///
/// # Errors
/// Configuration that binds no used source scheme or to more than one
/// declaration, declared flow URLs that are not absolute http(s), and a
/// configured client secret with no declared flow that can use one. Missing or
/// ambiguous configuration is never an error: descriptors carry what is known.
pub fn plan(
    contract: &Contract,
    protocol: &ProtocolPlan,
    defaults: Option<&SdkDefaults>,
) -> Result<OAuthPlan, Vec<HttpDiagnostic>> {
    let fallback = OAuthDefaults::default();
    let section = defaults
        .map(|defaults| &defaults.oauth)
        .unwrap_or(&fallback);
    if section.mode == OAuthMode::Off {
        return Ok(OAuthPlan {
            mode: OAuthMode::Off,
            schemes: Vec::new(),
        });
    }
    // One entry per used source scheme name, deduplicated by the terminal
    // declaration every requirement resolves to.
    let mut used: BTreeMap<&str, BTreeMap<&SourceId, &CredentialRequirement>> = BTreeMap::new();
    for requirement in protocol
        .operations()
        .iter()
        .flat_map(|operation| operation.security().alternatives())
        .flat_map(|alternative| alternative.requirements())
    {
        if !matches!(
            requirement.credential(),
            CredentialHook::OAuth2 { .. } | CredentialHook::OpenIdConnect { .. }
        ) {
            continue;
        }
        let terminal = requirement.scheme().terminal().source();
        used.entry(requirement.name())
            .or_default()
            .insert(terminal, requirement);
    }

    let mut schemes = Vec::new();
    let mut errors = Vec::new();
    for (name, declarations) in &used {
        if declarations.len() != 1 {
            errors.push(diagnostic(
                contract,
                None,
                "sdk-oauth-config",
                format!(
                    "source scheme name {name:?} binds {} distinct declarations; OAuth lifecycle configuration keys scheme names, which must stay unambiguous",
                    declarations.len()
                ),
            ));
            continue;
        }
        let requirement = declarations.values().next().expect("one declaration");
        let config = section.schemes.get(*name);
        match compile_scheme(contract, name, requirement, config, section) {
            Ok(scheme) => schemes.push(scheme),
            Err(mut scheme_errors) => errors.append(&mut scheme_errors),
        }
    }
    for name in section.schemes.keys() {
        if !used.contains_key(name.as_str()) {
            errors.push(diagnostic(
                contract,
                None,
                "sdk-oauth-config",
                format!(
                    "sdk_defaults.oauth.schemes[{name:?}] binds no used OAuth2 or openIdConnect security scheme"
                ),
            ));
        }
    }
    if errors.is_empty() {
        Ok(OAuthPlan {
            mode: section.mode,
            schemes,
        })
    } else {
        Err(errors)
    }
}

fn compile_scheme(
    contract: &Contract,
    name: &str,
    requirement: &CredentialRequirement,
    config: Option<&OAuthSchemeConfig>,
    section: &OAuthDefaults,
) -> Result<OAuthSchemePlan, Vec<HttpDiagnostic>> {
    let mut errors = Vec::new();
    let (kind, declared_flows, discovery) = match requirement.credential() {
        CredentialHook::OAuth2 {
            flows,
            metadata_url,
        } => {
            let discovery = metadata_url
                .as_ref()
                .map(|located| located.value().clone())
                .or_else(|| config.and_then(|config| config.discovery_url.clone()));
            (OAuthSchemeKind::OAuth2, flows.as_slice(), discovery)
        }
        CredentialHook::OpenIdConnect { discovery_url } => (
            OAuthSchemeKind::OpenIdConnect,
            &[] as &[OAuthFlow],
            Some(discovery_url.value().clone()),
        ),
        _ => unreachable!("only OAuth2 and openIdConnect requirements are compiled"),
    };
    let flows = compile_flows(contract, declared_flows, config, &mut errors);
    if kind == OAuthSchemeKind::OAuth2
        && !declared_flows
            .iter()
            .any(|flow| flow.kind() != DeclaredFlowKind::Implicit)
        && let Some(secret) = config.and_then(|config| config.client_secret_env.as_ref())
    {
        errors.push(diagnostic(
            contract,
            Some(requirement.scheme().terminal()),
            "sdk-oauth-config",
            format!(
                "sdk_defaults.oauth.schemes[{name:?}].client_secret_env ({secret}) cannot apply: the declaration has no token-endpoint flow, so the client is public"
            ),
        ));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(OAuthSchemePlan {
        name: name.to_owned(),
        kind,
        flows,
        client_id_env: config.and_then(|config| config.client_id_env.clone()),
        client_secret_env: config.and_then(|config| config.client_secret_env.clone()),
        refresh_skew_seconds: config
            .and_then(|config| config.refresh_skew_seconds)
            .unwrap_or(30),
        revocation_endpoint: config.and_then(|config| config.revocation_endpoint.clone()),
        introspection_endpoint: config.and_then(|config| config.introspection_endpoint.clone()),
        discovery,
        storage: section.storage,
        refresh: section.refresh,
    })
}

fn compile_flows(
    contract: &Contract,
    flows: &[OAuthFlow],
    config: Option<&OAuthSchemeConfig>,
    errors: &mut Vec<HttpDiagnostic>,
) -> Vec<OAuthFlowDescriptor> {
    let mut compiled = Vec::new();
    for flow in flows {
        for (field, url) in [
            ("authorizationUrl", flow.authorization_url()),
            ("tokenUrl", flow.token_url()),
            ("refreshUrl", flow.refresh_url()),
            ("deviceAuthorizationUrl", flow.device_authorization_url()),
        ]
        .into_iter()
        .filter_map(|(field, url)| url.map(|url| (field, url)))
        {
            if let Err(message) = ensure_absolute_url(url.value()) {
                errors.push(diagnostic(
                    contract,
                    Some(url.source()),
                    "sdk-oauth-flow",
                    format!("declared flow {field} must be an absolute http(s) URL: {message}"),
                ));
            }
        }
        let client_auth = if flow.kind() == DeclaredFlowKind::Implicit {
            OAuthClientAuth::None
        } else if config
            .and_then(|config| config.client_secret_env.as_ref())
            .is_some()
        {
            OAuthClientAuth::ClientSecretBasic
        } else {
            OAuthClientAuth::None
        };
        compiled.push(OAuthFlowDescriptor {
            kind: match flow.kind() {
                DeclaredFlowKind::Implicit => OAuthFlowDescriptorKind::Implicit,
                DeclaredFlowKind::Password => OAuthFlowDescriptorKind::Password,
                DeclaredFlowKind::ClientCredentials => OAuthFlowDescriptorKind::ClientCredentials,
                DeclaredFlowKind::AuthorizationCode => OAuthFlowDescriptorKind::AuthorizationCode,
                DeclaredFlowKind::DeviceAuthorization => {
                    OAuthFlowDescriptorKind::DeviceAuthorization
                }
            },
            authorization_url: flow.authorization_url().map(|url| url.value().clone()),
            token_url: flow.token_url().map(|url| url.value().clone()),
            refresh_url: flow.refresh_url().map(|url| url.value().clone()),
            device_authorization_url: flow
                .device_authorization_url()
                .map(|url| url.value().clone()),
            scopes: flow
                .scopes()
                .iter()
                .map(|(name, description)| (name.clone(), description.value().clone()))
                .collect(),
            client_auth,
            deprecated_flow: matches!(
                flow.kind(),
                DeclaredFlowKind::Implicit | DeclaredFlowKind::Password
            ),
        });
    }
    compiled
}

/// Compiled descriptors carry executable endpoints; relative references stay a
/// runtime resolution concern of explicitly selected helpers, never a compiled
/// value.
fn ensure_absolute_url(value: &str) -> Result<(), String> {
    match url::Url::parse(value) {
        Ok(url) if matches!(url.scheme(), "http" | "https") && url.has_host() => Ok(()),
        Ok(url) => Err(format!("scheme {:?} is not http(s)", url.scheme())),
        Err(error) => Err(error.to_string()),
    }
}
