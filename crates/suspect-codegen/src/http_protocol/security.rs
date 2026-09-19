use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use suspect_ir::contract::SourceId;

use super::planner::Planner;
use super::*;

pub(super) fn plan(
    p: &mut Planner<'_>,
    source: Option<&SourceId>,
    operation: &SourceId,
) -> SecurityPlan {
    let Some(source) = source else {
        p.require(Capability::AnonymousSecurity, operation);
        return SecurityPlan::Undeclared {
            source: p.location(operation),
        };
    };
    let Some(raw) = p.contract.source(source).and_then(Value::as_array) else {
        p.error(
            source,
            "http-security-invalid",
            "effective security must be an array of requirement objects",
        );
        return SecurityPlan::NoAuth {
            source: p.location(source),
        };
    };
    if raw.is_empty() {
        p.require(Capability::AnonymousSecurity, source);
        return SecurityPlan::NoAuth {
            source: p.location(source),
        };
    }
    if raw.len() > 1 {
        p.require(Capability::SecurityAlternatives, source);
    }
    let mut alternatives = Vec::new();
    for (index, value) in raw.iter().enumerate() {
        let at = source.child(&index.to_string());
        let Some(map) = value.as_object() else {
            p.error(
                &at,
                "http-security-invalid",
                "security requirement entries must be objects",
            );
            continue;
        };
        if map.is_empty() {
            p.require(Capability::AnonymousSecurity, &at);
        }
        if map.len() > 1 {
            p.require(Capability::ConjunctiveSecurity, &at);
        }
        let mut requirements = Vec::new();
        for (name, values) in map {
            let at = at.child(name);
            let mut names = Vec::new();
            if let Some(values) = values.as_array() {
                for (i, value) in values.iter().enumerate() {
                    let at = at.child(&i.to_string());
                    match value.as_str() {
                        Some(value) => names.push(Located {
                            source: p.location(&at),
                            value: value.to_owned(),
                        }),
                        None => p.error(
                            &at,
                            "http-security-permissions-invalid",
                            "security scopes/roles must be strings",
                        ),
                    }
                }
            } else {
                p.error(
                    &at,
                    "http-security-permissions-invalid",
                    "security scopes/roles must be an array",
                );
            }
            // Match the current Contract's documented, source-document-local
            // implicit connection policy. Never fall back to a similarly named
            // scheme in another document or infer an auth scheme from its name.
            let definition = SourceId::new(source.document().clone(), Default::default())
                .child("components")
                .child("securitySchemes")
                .child(name);
            if p.contract.source(&definition).is_none() {
                p.unsupported(&at, "http-security-scheme-unresolved", "security scheme is not declared in the requirement document; URI-named OAS 3.2 schemes need canonical IR URI-connection indexing");
                continue;
            }
            if let Some(requirement) = scheme(p, &at, name, &definition, names) {
                requirements.push(requirement);
            }
        }
        alternatives.push(SecurityAlternative {
            source: p.location(&at),
            requirements,
        });
    }
    SecurityPlan::Alternatives {
        source: p.location(source),
        alternatives,
    }
}

fn scheme(
    p: &mut Planner<'_>,
    use_site: &SourceId,
    name: &str,
    definition: &SourceId,
    names: Vec<Located<String>>,
) -> Option<CredentialRequirement> {
    let origin = p.resolve(definition, false)?;
    let at = &origin.terminal.source;
    let raw = p.object(at, "Security Scheme")?;
    p.known(
        at,
        raw,
        &[
            "type",
            "description",
            "name",
            "in",
            "scheme",
            "bearerFormat",
            "flows",
            "openIdConnectUrl",
            "oauth2MetadataUrl",
            "deprecated",
        ],
    );
    let kind = p.string(at, "type", true)?;
    let description = p.text(&origin, "description", false);
    let deprecated = if raw.contains_key("deprecated") {
        let value = p.boolean(at, "deprecated", false);
        if !p.is_32(at) {
            p.unsupported(
                &at.child("deprecated"),
                "http-version-field",
                "Security Scheme.deprecated requires OAS 3.2",
            );
        }
        Some(Located {
            source: p.location(&at.child("deprecated")),
            value,
        })
    } else {
        None
    };
    // Validate optional declarations even when they are irrelevant to the
    // selected scheme. Wrong types must not vanish through typed views.
    for field in [
        "name",
        "in",
        "scheme",
        "bearerFormat",
        "openIdConnectUrl",
        "oauth2MetadataUrl",
    ] {
        p.string(at, field, false);
    }
    let oauth = matches!(kind.value.as_str(), "oauth2" | "openIdConnect");
    if !oauth && !names.is_empty() {
        if p.is_30(use_site) {
            p.error(
                use_site,
                "http-security-roles-version",
                "OAS 3.0 requires an empty array for non-OAuth security schemes",
            );
        } else {
            p.require(Capability::SecurityRoles, use_site);
        }
    }
    let credential = match kind.value.as_str() {
        "http" => {
            let scheme = p.string(at, "scheme", true)?;
            match scheme.value.to_ascii_lowercase().as_str() {
                "bearer" => CredentialHook::Bearer {
                    bearer_format: p.string(at, "bearerFormat", false),
                },
                "basic" => {
                    p.require(Capability::HttpBasic, at);
                    CredentialHook::Basic
                }
                _ => {
                    p.unsupported(
                        &at.child("scheme"),
                        "http-security-http-scheme",
                        "only declared HTTP bearer and basic credential attachment are planned",
                    );
                    return None;
                }
            }
        }
        "apiKey" => {
            p.require(Capability::ApiKeys, at);
            let name = p.string(at, "name", true)?;
            let location = p.string(at, "in", true)?;
            let location = match location.value.as_str() {
                "header" => ParameterLocation::Header,
                "query" => ParameterLocation::Query,
                "cookie" => ParameterLocation::Cookie,
                _ => {
                    p.error(
                        &location.source.source,
                        "http-security-api-key-location",
                        "API keys require header, query, or cookie location",
                    );
                    return None;
                }
            };
            if name.value.is_empty()
                || matches!(
                    location,
                    ParameterLocation::Header | ParameterLocation::Cookie
                ) && !super::media::token(&name.value)
            {
                p.error(&name.source.source, "http-security-api-key-name", "API key name must be nonempty and a valid header/cookie token in those locations");
            }
            CredentialHook::ApiKey { location, name }
        }
        "oauth2" => {
            p.require(Capability::OAuth2, at);
            let flows = flows(p, &at.child("flows"));
            for scope in &names {
                if !flows
                    .iter()
                    .any(|flow| flow.scopes.contains_key(&scope.value))
                {
                    p.error(
                        &scope.source.source,
                        "http-security-scope-undeclared",
                        "required OAuth scope is absent from every declared flow",
                    );
                }
            }
            let metadata_url = p.string(at, "oauth2MetadataUrl", false);
            if let Some(url) = &metadata_url {
                if !p.is_32(at) {
                    p.unsupported(
                        &url.source.source,
                        "http-version-field",
                        "oauth2MetadataUrl requires OAS 3.2",
                    );
                }
                endpoint(p, url);
            }
            CredentialHook::OAuth2 {
                flows,
                metadata_url,
            }
        }
        "openIdConnect" => {
            p.require(Capability::OpenIdConnect, at);
            let discovery_url = p.string(at, "openIdConnectUrl", true)?;
            endpoint(p, &discovery_url);
            CredentialHook::OpenIdConnect { discovery_url }
        }
        "mutualTLS" => {
            p.unsupported(at, "http-security-mutual-tls", "mutualTLS needs a verified transport certificate capability; it is not an HTTP header credential");
            return None;
        }
        _ => {
            p.error(
                &at.child("type"),
                "http-security-scheme-type",
                "unknown Security Scheme type",
            );
            return None;
        }
    };
    // Inapplicable authentication fields are not a second acquisition protocol.
    for field in [
        "flows",
        "openIdConnectUrl",
        "oauth2MetadataUrl",
        "scheme",
        "bearerFormat",
        "name",
        "in",
    ] {
        let applicable = match field {
            "flows" | "oauth2MetadataUrl" => kind.value == "oauth2",
            "openIdConnectUrl" => kind.value == "openIdConnect",
            "scheme" | "bearerFormat" => kind.value == "http",
            _ => kind.value == "apiKey",
        };
        if raw.contains_key(field) && !applicable {
            p.error(
                &at.child(field),
                "http-security-field-inapplicable",
                format!("{field} does not apply to security type {:?}", kind.value),
            );
        }
    }
    Some(CredentialRequirement {
        source: p.location(use_site),
        name: name.to_owned(),
        scheme: origin,
        description,
        deprecated,
        permissions: if oauth {
            Permissions::Scopes(names)
        } else {
            Permissions::Roles(names)
        },
        credential,
    })
}

fn flows(p: &mut Planner<'_>, source: &SourceId) -> Vec<OAuthFlow> {
    let Some(raw) = p.object(source, "OAuth Flows") else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for (name, _) in raw {
        let at = source.child(name);
        if name.starts_with("x-") {
            p.warn(
                &at,
                "http-extension-uninterpreted",
                "OAuth flow extension has no acquisition semantics",
            );
            continue;
        }
        let kind = match name.as_str() {
            "implicit" => OAuthFlowKind::Implicit,
            "password" => OAuthFlowKind::Password,
            "clientCredentials" => OAuthFlowKind::ClientCredentials,
            "authorizationCode" => OAuthFlowKind::AuthorizationCode,
            "deviceAuthorization" if p.is_32(source) => OAuthFlowKind::DeviceAuthorization,
            _ => {
                p.error(
                    &at,
                    "http-oauth-flow-kind",
                    "unknown OAuth flow for this OpenAPI version",
                );
                continue;
            }
        };
        let Some(raw) = p.object(&at, "OAuth Flow") else {
            continue;
        };
        p.known(
            &at,
            raw,
            &[
                "authorizationUrl",
                "tokenUrl",
                "refreshUrl",
                "deviceAuthorizationUrl",
                "scopes",
            ],
        );
        let authorization_url = p.string(
            &at,
            "authorizationUrl",
            matches!(
                kind,
                OAuthFlowKind::Implicit | OAuthFlowKind::AuthorizationCode
            ),
        );
        let token_url = p.string(&at, "tokenUrl", kind != OAuthFlowKind::Implicit);
        let refresh_url = p.string(&at, "refreshUrl", false);
        let device_authorization_url = p.string(
            &at,
            "deviceAuthorizationUrl",
            kind == OAuthFlowKind::DeviceAuthorization,
        );
        for url in [
            &authorization_url,
            &token_url,
            &refresh_url,
            &device_authorization_url,
        ]
        .into_iter()
        .flatten()
        {
            endpoint(p, url);
        }
        if device_authorization_url.is_some() && kind != OAuthFlowKind::DeviceAuthorization {
            p.error(
                &at.child("deviceAuthorizationUrl"),
                "http-oauth-flow-field",
                "deviceAuthorizationUrl applies only to deviceAuthorization flow",
            );
        }
        let mut scopes = BTreeMap::new();
        let scopes_at = at.child("scopes");
        if let Some(map) = p.contract.source(&scopes_at).and_then(Value::as_object) {
            for key in map.keys() {
                if let Some(description) = p.string(&scopes_at, key, true) {
                    scopes.insert(key.clone(), description);
                }
            }
        } else {
            p.error(
                &scopes_at,
                "http-oauth-scopes-invalid",
                "OAuth flow requires a scope-name to string-description map",
            );
        }
        result.push(OAuthFlow {
            source: p.location(&at),
            kind,
            authorization_url,
            token_url,
            refresh_url,
            device_authorization_url,
            scopes,
        });
    }
    result
}

fn endpoint(p: &mut Planner<'_>, value: &Located<String>) {
    if value.value.is_empty()
        || value
            .value
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
        || !super::servers::percent_triples(&value.value)
    {
        p.error(
            &value.source.source,
            "http-security-url-invalid",
            "credential metadata URL must be a valid URL reference",
        );
        return;
    }
    match url::Url::parse(&value.value) {
        Ok(url) if url.scheme() == "https" && url.has_host() => {}
        Err(url::ParseError::RelativeUrlWithoutBase) if url::Url::parse("https://relative.invalid/").unwrap().join(&value.value).is_ok() => {}
        _ => p.error(&value.source.source, "http-security-url-invalid", "absolute OAuth/OIDC metadata URLs require HTTPS; relative API URL references retain their server base"),
    }
}

pub(super) fn validate_collisions(
    p: &mut Planner<'_>,
    security: &SecurityPlan,
    parameters: &[ParameterPlan],
) {
    for alternative in security.alternatives() {
        let mut targets = BTreeSet::new();
        for requirement in &alternative.requirements {
            let (location, name) = match &requirement.credential {
                CredentialHook::ApiKey { location, name } => (*location, name.value.clone()),
                _ => (ParameterLocation::Header, "authorization".to_owned()),
            };
            let identity = (
                location,
                if location == ParameterLocation::Header {
                    name.to_ascii_lowercase()
                } else {
                    name.clone()
                },
            );
            if location == ParameterLocation::Query
                && parameters
                    .iter()
                    .any(|p| p.location == ParameterLocation::Querystring)
            {
                p.unsupported(&requirement.source.source, "http-querystring-security-conflict", "a query API key would modify the complete querystring after its content codec; credential/content merging requires an explicit supported policy");
            }
            if !targets.insert(identity.clone()) {
                p.unsupported(&requirement.source.source, "http-security-attachment-conflict", "conjunctive credentials target the same wire field; no overwriting or first-scheme selection is inferred");
            }
            if parameters.iter().any(|v| {
                v.location == location
                    && if location == ParameterLocation::Header {
                        v.name.eq_ignore_ascii_case(&name)
                    } else {
                        v.name == name
                    }
            }) {
                p.unsupported(
                    &requirement.source.source,
                    "http-security-parameter-conflict",
                    "a credential and an ordinary parameter target the same wire field",
                );
            }
        }
    }
}
