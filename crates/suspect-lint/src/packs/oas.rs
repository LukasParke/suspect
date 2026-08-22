//! The builtin OAS pack: a Spectral-parity subset covering OpenAPI 2.0 and
//! 3.x. Every rule carries its applicable formats; the engine filters by the
//! document's sniffed family.

use crate::functions::Function;
use crate::rule::{FamilySet, Rule, Severity};

const METHOD_PATHS: [&str; 8] = [
    "$.paths.*.get",
    "$.paths.*.put",
    "$.paths.*.post",
    "$.paths.*.delete",
    "$.paths.*.options",
    "$.paths.*.head",
    "$.paths.*.patch",
    "$.paths.*.trace",
];

const SUMMARY_PATHS: [&str; 8] = [
    "$.paths.*.get.summary",
    "$.paths.*.put.summary",
    "$.paths.*.post.summary",
    "$.paths.*.delete.summary",
    "$.paths.*.options.summary",
    "$.paths.*.head.summary",
    "$.paths.*.patch.summary",
    "$.paths.*.trace.summary",
];

const DESCRIPTION_PATHS: [&str; 8] = [
    "$.paths.*.get.description",
    "$.paths.*.put.description",
    "$.paths.*.post.description",
    "$.paths.*.delete.description",
    "$.paths.*.options.description",
    "$.paths.*.head.description",
    "$.paths.*.patch.description",
    "$.paths.*.trace.description",
];

/// The full builtin OAS ruleset at Spectral default severities.
pub(crate) fn rules() -> Vec<Rule> {
    let oas23 = FamilySet::OAS2.union(FamilySet::OAS3);
    let f = FamilySet::OAS3;
    vec![
        Rule::new(
            "oas3-api-servers",
            "OpenAPI 3.x servers must be defined.",
            Severity::Warn,
            f,
            &["$.servers"],
            Function::Truthy,
        ),
        Rule::new(
            "oas3-api-contact",
            "OpenAPI 3.x API description must have a contact object.",
            Severity::Info,
            f,
            &["$.info"],
            property_defined("contact"),
        ),
        Rule::new(
            "info-contact",
            "Info object should contain a contact object.",
            Severity::Warn,
            oas23,
            &["$.info"],
            property_defined("contact"),
        ),
        Rule::new(
            "info-license",
            "Info object should contain a license object.",
            Severity::Warn,
            oas23,
            &["$.info"],
            property_defined("license"),
        ),
        Rule::new(
            "license-url",
            "License object should include `url`.",
            Severity::Warn,
            oas23,
            &["$.info.license"],
            property_defined("url"),
        ),
        Rule::new(
            "openapi-tags",
            "OpenAPI object should have non-empty `tags` array.",
            Severity::Warn,
            oas23,
            &["$.tags"],
            Function::Truthy,
        ),
        Rule::new(
            "operation-tags",
            "Operation should have non-empty `tags` array.",
            Severity::Warn,
            oas23,
            METHOD_PATHS.as_slice(),
            property_defined("tags"),
        ),
        Rule::new(
            "operation-operationId",
            "Operation must have an `operationId`.",
            Severity::Error,
            oas23,
            METHOD_PATHS.as_slice(),
            property_defined("operationId"),
        ),
        Rule::new(
            "operation-summary",
            "Operation should have `summary`.",
            Severity::Warn,
            oas23,
            SUMMARY_PATHS.as_slice(),
            Function::Truthy,
        ),
        Rule::new(
            "operation-description",
            "Operation should have `description`.",
            Severity::Info,
            oas23,
            DESCRIPTION_PATHS.as_slice(),
            Function::Truthy,
        ),
        Rule::new(
            "operation-default-response",
            "Operation must have a `default` or 2XX response.",
            Severity::Error,
            oas23,
            METHOD_PATHS.as_slice(),
            Function::DefaultResponse,
        ),
        Rule::new(
            "operation-success-response",
            "Operation must have at least one 2XX response.",
            Severity::Error,
            oas23,
            METHOD_PATHS.as_slice(),
            Function::SuccessResponse,
        ),
        Rule::new(
            "path-params",
            "Path template variables must be declared as path parameters.",
            Severity::Error,
            oas23,
            &["$.paths.*"],
            Function::PathParams,
        ),
        Rule::new(
            "path-keys-no-trailing-slash",
            "Path keys must not end with a trailing slash.",
            Severity::Warn,
            oas23,
            &["$.paths.*"],
            Function::NoTrailingSlash,
        ),
        Rule::new(
            "no-$ref-siblings",
            "$ref values must not have siblings other than description/summary (3.0).",
            Severity::Error,
            FamilySet::OAS30,
            &["$..['$ref']"],
            Function::RefSiblings,
        ),
        Rule::new(
            "no-$ref-siblings",
            "$ref values must not have siblings other than description/summary (3.1+).",
            Severity::Warn,
            FamilySet::OAS31.union(FamilySet::OAS32),
            &["$..['$ref']"],
            Function::RefSiblings,
        ),
        Rule::new(
            "typed-enum",
            "Enum members must all share a single scalar type.",
            Severity::Error,
            oas23,
            &["$..enum"],
            Function::TypedEnum,
        ),
        Rule::new(
            "no-ambiguous-paths",
            "Path keys must not be ambiguous after normalization (no duplicates).",
            Severity::Error,
            oas23,
            &["$.paths"],
            Function::DuplicateKeys,
        ),
    ]
}

fn property_defined(name: &str) -> Function {
    Function::Defined {
        property: name.into(),
    }
}
