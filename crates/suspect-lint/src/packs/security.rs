//! Security-focused lint rules (OWASP API Top 10 aligned).
//!
//! These are advisory: each flags a pattern that is *permitted by the
//! spec* but correlates with real-world API security incidents.

use crate::functions::Function;
use crate::rule::{FamilySet, Rule, Severity};

/// The security pack, in canonical order.
pub(crate) fn rules() -> Vec<Rule> {
    let oas3 = FamilySet::OAS3;
    let oas23 = FamilySet::OAS2.union(FamilySet::OAS3);
    use super::oas::METHOD_PATHS;
    vec![
        Rule::new(
            "security-server-https-only",
            "Server URLs must use HTTPS — plaintext transport exposes credentials and data.",
            Severity::Error,
            oas3,
            &["$.servers.*.url"],
            Function::HttpsUrl,
        ),
        Rule::new(
            "security-no-basic-auth",
            "HTTP Basic authentication sends credentials on every request without hashing — prefer bearer tokens or OAuth 2.0.",
            Severity::Warn,
            oas23,
            &["$..securitySchemes.*"],
            Function::NoBasicAuth,
        ),
        Rule::new(
            "security-no-apikey-in-query",
            "API keys in query strings leak into server logs, proxy logs, and browser history — use a header or cookie.",
            Severity::Error,
            oas23,
            &["$..securitySchemes.*"],
            Function::NoApiKeyInQuery,
        ),
        Rule::new(
            "security-no-sensitive-params-in-url",
            "Sensitive parameters (token, key, secret, password) must not travel in the URL path or query — URLs leak into logs.",
            Severity::Error,
            oas23,
            &["$..parameters.*"],
            Function::NoSensitiveParams,
        ),
        Rule::new(
            "security-no-credentials-in-url",
            "Path and query parameter names must not look like credentials (token, key, secret, password, credential).",
            Severity::Error,
            oas23,
            &["$.paths.*", "$.paths.*.*.parameters.*"],
            Function::NoSensitiveParams,
        ),
        Rule::new(
            "security-rate-limit-documented",
            "Operations should document rate limiting — a 429 response or rate-limit headers.",
            Severity::Info,
            FamilySet::OAS3,
            METHOD_PATHS.as_slice(),
            Function::RateLimitDocumented,
        ),
        Rule::new(
            "security-jwt-bearer-format",
            "Bearer schemes carrying JWTs should declare `bearerFormat: JWT` so clients parse tokens correctly.",
            Severity::Hint,
            FamilySet::OAS3,
            &["$..securitySchemes.*"],
            Function::JwtBearerFormat,
        ),
        Rule::new(
            "security-oauth-scopes-documented",
            "OAuth 2.0 flows should declare at least one scope — empty scope maps hide the permission model.",
            Severity::Warn,
            FamilySet::OAS3,
            &["$..securitySchemes.*.flows.*"],
            Function::OauthScopesDocumented,
        ),
        Rule::new(
            "security-operation-auth-or-public",
            "Operations without operation-level security inherit the root requirement — mark intentionally public operations with an empty requirement (`security: [{}]`).",
            Severity::Info,
            FamilySet::OAS3,
            METHOD_PATHS.as_slice(),
            Function::Truthy,
        ),
        Rule::new(
            "security-no-delete-without-id",
            "DELETE operations must target a specific resource — collection-wide deletes are destructive and unauthenticated by scope.",
            Severity::Warn,
            oas23,
            &["$.paths.*.delete"],
            Function::DeleteRequiresId,
        ),
    ]
}


