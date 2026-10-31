//! The rule table: every code the battery emits, with its default
//! severity and the guidance from [`crate::guidance`].

use std::sync::OnceLock;

use suspect_diag::{CheckId, RuleClass, RuleInfo, Severity};

use crate::guidance::{how_to_fix, summary};

/// `(code, check, default severity)` for every emitted code.
#[rustfmt::skip]
const CODES: &[(&str, CheckId, Severity)] = &[
    ("invalid-ref",                          CheckId::Ref,      Severity::Error),
    ("unresolved-ref",                       CheckId::Ref,      Severity::Error),
    ("ref-outside-allowlist",                CheckId::Ref,      Severity::Error),
    ("oas-callback-invalid",                 CheckId::Validate, Severity::Error),
    ("oas-deprecated-operation",             CheckId::Validate, Severity::Info),
    ("oas-discriminator-missing-property",   CheckId::Validate, Severity::Error),
    ("oas-discriminator-unknown-mapping",    CheckId::Validate, Severity::Error),
    ("oas-duplicate-header-param",           CheckId::Validate, Severity::Error),
    ("oas-duplicate-operation-id",           CheckId::Validate, Severity::Error),
    ("oas-empty-path-template",              CheckId::Validate, Severity::Error),
    ("oas-example-type-mismatch",            CheckId::Validate, Severity::Warning),
    ("oas-header-invalid",                   CheckId::Validate, Severity::Error),
    ("oas-http-array-invalid",               CheckId::Validate, Severity::Error),
    ("oas-http-boolean-invalid",             CheckId::Validate, Severity::Error),
    ("oas-http-map-invalid",                 CheckId::Validate, Severity::Error),
    ("oas-license-missing-url",              CheckId::Validate, Severity::Warning),
    ("oas-link-invalid",                     CheckId::Validate, Severity::Error),
    ("oas-media-type-invalid",               CheckId::Validate, Severity::Error),
    ("oas-operation-invalid",                CheckId::Validate, Severity::Error),
    ("oas-operation-missing-operationId",    CheckId::Validate, Severity::Warning),
    ("oas-operation-missing-responses",      CheckId::Validate, Severity::Error),
    ("oas-operation-no-error-response",      CheckId::Validate, Severity::Info),
    ("oas-parameter-invalid",                CheckId::Validate, Severity::Error),
    ("oas-parameter-missing-in",             CheckId::Validate, Severity::Error),
    ("oas-parameter-missing-name",           CheckId::Validate, Severity::Error),
    ("oas-parameter-required-missing",       CheckId::Validate, Severity::Error),
    ("oas-path-item-invalid",                CheckId::Validate, Severity::Error),
    ("oas-path-item-security-invalid",       CheckId::Validate, Severity::Error),
    ("oas-path-param-not-declared",          CheckId::Validate, Severity::Error),
    ("oas-path-trailing-slash",              CheckId::Validate, Severity::Warning),
    ("oas-request-body-content-missing",     CheckId::Validate, Severity::Error),
    ("oas-request-body-invalid",             CheckId::Validate, Severity::Error),
    ("oas-response-entry-invalid",           CheckId::Validate, Severity::Error),
    ("oas-response-invalid",                 CheckId::Validate, Severity::Error),
    ("oas-response-map-invalid",             CheckId::Validate, Severity::Error),
    ("oas-response-missing-description",     CheckId::Validate, Severity::Error),
    ("oas-responses-invalid",                CheckId::Validate, Severity::Error),
    ("oas-schema-instance-invalid",          CheckId::Validate, Severity::Warning),
    ("oas-schema-invalid-keyword",           CheckId::Validate, Severity::Error),
    ("oas-schema-invalid-kind",              CheckId::Validate, Severity::Error),
    ("oas-schema-invalid-type",              CheckId::Validate, Severity::Error),
    ("oas-schema-unknown-type",              CheckId::Validate, Severity::Error),
    ("oas-schema-unsupported-dialect",       CheckId::Validate, Severity::Error),
    ("oas-security-invalid",                 CheckId::Validate, Severity::Error),
    ("oas-security-requirement-invalid",     CheckId::Validate, Severity::Error),
    ("oas-security-scope-invalid",           CheckId::Validate, Severity::Error),
    ("oas-security-scopes-invalid",          CheckId::Validate, Severity::Error),
    ("oas-security-scopes-nonempty",         CheckId::Validate, Severity::Error),
    ("oas-security-unknown-scheme",          CheckId::Validate, Severity::Error),
    ("oas-server-invalid",                   CheckId::Validate, Severity::Error),
    ("oas-server-url-invalid",               CheckId::Validate, Severity::Error),
    ("oas-server-variable-default-invalid",  CheckId::Validate, Severity::Error),
    ("oas-server-variable-enum-invalid",     CheckId::Validate, Severity::Error),
    ("oas-server-variable-invalid",          CheckId::Validate, Severity::Error),
    ("oas-server-variable-unknown",          CheckId::Validate, Severity::Error),
    ("oas-tag-parent-cycle",                 CheckId::Validate, Severity::Error),
    ("oas-tag-parent-unknown",               CheckId::Validate, Severity::Error),
    ("oas-tag-undeclared",                   CheckId::Validate, Severity::Warning),
    ("oas-unused-path-param",                CheckId::Validate, Severity::Warning),
    ("oas-webhook-unsupported-version",      CheckId::Validate, Severity::Error),
    ("swagger-info-required",                CheckId::Swagger,  Severity::Error),
    ("swagger-info-field-required",          CheckId::Swagger,  Severity::Error),
    ("swagger-operation-missing-responses",  CheckId::Swagger,  Severity::Error),
    ("swagger-parameter-missing-name",       CheckId::Swagger,  Severity::Error),
    ("swagger-parameter-missing-in",         CheckId::Swagger,  Severity::Error),
    ("swagger-body-param-schema",            CheckId::Swagger,  Severity::Error),
    ("swagger-parameter-not-in-template",    CheckId::Swagger,  Severity::Error),
    ("swagger-path-param-undeclared",        CheckId::Swagger,  Severity::Error),
    ("swagger-response-missing-description", CheckId::Swagger,  Severity::Error),
    ("swagger-security-undefined",           CheckId::Swagger,  Severity::Error),
    ("swagger-definition-shape",             CheckId::Swagger,  Severity::Error),
    ("sdk-operation-missing-id",             CheckId::Validate, Severity::Info),
    ("sdk-stream-response-untyped",          CheckId::Validate, Severity::Info),
    ("sdk-colon-path-parameters",            CheckId::Validate, Severity::Info),
];

/// Every code the OpenAPI 3.x and Swagger 2.0 batteries emit. `sdk-*`
/// codes are [`RuleClass::Sdk`]; the rest are [`RuleClass::Document`].
#[must_use]
pub fn rules() -> &'static [RuleInfo] {
    static RULES: OnceLock<Vec<RuleInfo>> = OnceLock::new();
    RULES.get_or_init(|| {
        CODES
            .iter()
            .map(|&(code, check, default)| RuleInfo {
                class: if code.starts_with("sdk-") {
                    RuleClass::Sdk
                } else {
                    RuleClass::Document
                },
                how_to_fix: how_to_fix(code),
                ..RuleInfo::new(code, check, default, summary(code))
            })
            .collect()
    })
}

/// Maps a battery severity onto the shared model.
impl From<crate::Severity> for Severity {
    fn from(severity: crate::Severity) -> Self {
        match severity {
            crate::Severity::Error => Self::Error,
            crate::Severity::Warning => Self::Warning,
            crate::Severity::Info => Self::Info,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_has_guidance() {
        for rule in rules() {
            assert!(
                !rule.summary.is_empty(),
                "summary missing for {}",
                rule.code
            );
            assert!(
                !rule.how_to_fix.is_empty(),
                "how-to-fix missing for {}",
                rule.code
            );
        }
    }
}
