//! Ruleset compilation tests: extends handling, overrides, and error cases.

use crate::engine::Linter;
use crate::functions::tests::{doc, run};

const OAS_DOC: &str = "openapi: \"3.0.0\"\ninfo:\n  title: t\n  version: \"1\"\npaths: {}\n";

#[test]
fn valid_custom_rule_with_pattern() {
    let rs = doc(
        "rules:\n  homepage-format:\n    description: Homepage must be an https URL.\n    given: $.homepage\n    severity: error\n    then:\n      function: pattern\n      functionOptions:\n        match: '^https://'\n",
    );
    let linter = Linter::from_ruleset(&rs).expect("valid ruleset");
    let hits = run(&linter, "homepage: http://insecure\n");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].code, "homepage-format");
    assert_eq!(hits[0].severity, crate::rule::Severity::Error);
    assert_eq!(hits[0].message, "Homepage must be an https URL.");
    let clean = run(&linter, "homepage: https://ok\n");
    assert!(clean.is_empty());
}

#[test]
fn extends_oas_and_override_by_code() {
    let rs = doc(
        "extends: spectral:oas\nrules:\n  operation-operationId:\n    description: custom text\n    given: $.paths.*.get\n    severity: hint\n    then:\n      function: defined\n      functionOptions:\n        property: operationId\n",
    );
    let linter = Linter::from_ruleset(&rs).expect("valid ruleset");
    let target =
        "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /a:\n    get: {}\n";
    let hits = run(&linter, target);
    // Overridden rule: exactly one operation-operationId finding, hint severity.
    let opid: Vec<_> = hits
        .iter()
        .filter(|h| h.code == "operation-operationId")
        .collect();
    assert_eq!(opid.len(), 1);
    assert_eq!(opid[0].severity, crate::rule::Severity::Hint);
    assert_eq!(opid[0].message, "custom text");
    // Other builtin rules still active.
    assert!(hits.iter().any(|h| h.code == "operation-default-response"));
}

#[test]
fn extends_array_composition() {
    let rs = doc("extends:\n  - spectral:oas\n  - spectral:overlay\n");
    let linter = Linter::from_ruleset(&rs).expect("valid ruleset");
    let codes: Vec<&str> = linter.rule_codes().collect();
    assert!(codes.contains(&"operation-operationId"));
    assert!(codes.contains(&"overlay-info-description"));
    assert!(!codes.contains(&"arazzo-step-operation"));
}

#[test]
fn unknown_function_is_bad_rule() {
    let rs = doc("rules:\n  broken:\n    given: $\n    then:\n      function: no-such-function\n");
    let err = Linter::from_ruleset(&rs).expect_err("unknown function must fail");
    match err {
        crate::engine::RulesetError::BadRule { code, message } => {
            assert_eq!(code, "broken");
            assert!(message.contains("unknown function"), "message: {message}");
        }
        other => panic!("expected BadRule, got {other:?}"),
    }
}

#[test]
fn bad_severity_is_bad_rule() {
    for sev in ["fatal", "4", "-1"] {
        let rs = doc(&format!(
            "rules:\n  sev-test:\n    given: $\n    severity: {sev}\n    then:\n      function: truthy\n"
        ));
        let err = Linter::from_ruleset(&rs).expect_err("bad severity must fail");
        assert!(
            matches!(err, crate::engine::RulesetError::BadRule { .. }),
            "severity {sev}: {err:?}"
        );
    }
    // Numeric severities are accepted (0=error .. 3=hint).
    let rs = doc(
        "rules:\n  numeric:\n    given: $.v\n    severity: 2\n    then:\n      function: truthy\n",
    );
    let linter = Linter::from_ruleset(&rs).expect("numeric severity valid");
    let hits = run(&linter, "v: null\n");
    assert_eq!(hits[0].severity, crate::rule::Severity::Info);
}

#[test]
fn bad_given_jsonpath_is_jsonpath_error() {
    let rs = doc("rules:\n  badpath:\n    given: $.[\n    then:\n      function: truthy\n");
    let err = Linter::from_ruleset(&rs).expect_err("invalid jsonpath must fail");
    assert!(
        matches!(err, crate::engine::RulesetError::JsonPath(_)),
        "{err:?}"
    );
}

#[test]
fn unknown_extends_target_is_invalid_ruleset() {
    let rs = doc("extends: spectral:nonexistent\n");
    let err = Linter::from_ruleset(&rs).expect_err("unknown extends must fail");
    match err {
        crate::engine::RulesetError::InvalidRuleset { field, .. } => assert_eq!(field, "extends"),
        other => panic!("expected InvalidRuleset, got {other:?}"),
    }
}

#[test]
fn formats_restrict_rule_to_family() {
    let rs = doc(
        "rules:\n  oas3-only:\n    given: $.value\n    formats: [oas3]\n    then:\n      function: truthy\n",
    );
    let linter = Linter::from_ruleset(&rs).expect("valid ruleset");
    let oas3_target = "openapi: \"3.0.0\"\ninfo:\n  title: t\nvalue: null\n";
    assert!(!run(&linter, oas3_target).is_empty(), "fires on OAS3 doc");
    let overlay = doc("overlay: \"1.0.0\"\ninfo: {title: t}\nactions: []\n");
    assert!(
        linter.run(&overlay).is_empty(),
        "must not fire on overlay doc"
    );
}

#[test]
fn severity_off_disables_rule() {
    let rs = doc(
        "extends: spectral:oas\nrules:\n  operation-operationId:\n    severity: off\n    then:\n      function: defined\n      functionOptions:\n        property: operationId\n",
    );
    let linter = Linter::from_ruleset(&rs).expect("valid ruleset");
    let target =
        "openapi: \"3.0.0\"\ninfo: {title: t, version: \"1\"}\npaths:\n  /a:\n    get: {}\n";
    let hits = run(&linter, target);
    assert!(
        !hits.iter().any(|h| h.code == "operation-operationId"),
        "severity off must disable the rule"
    );
}

#[test]
fn json_ruleset_is_supported() {
    let rs = doc(r#"{"rules": {"j-rule": {"given": "$.v", "then": {"function": "truthy"}}}}"#);
    let linter = Linter::from_ruleset(&rs).expect("json ruleset valid");
    let hits = run(&linter, "v: null\n");
    assert_eq!(hits.len(), 1);
}

#[test]
fn default_severity_is_warn() {
    let rs = doc("rules:\n  quiet:\n    given: $.v\n    then:\n      function: truthy\n");
    let linter = Linter::from_ruleset(&rs).expect("valid");
    let hits = run(&linter, "v: null\n");
    assert_eq!(hits[0].severity, crate::rule::Severity::Warn);
}

#[test]
fn spectral_default_compiles_and_targets_oas() {
    let linter = Linter::spectral_default();
    let codes: Vec<&str> = linter.rule_codes().collect();
    for expected in [
        "oas3-api-servers",
        "oas3-api-contact",
        "info-contact",
        "info-license",
        "license-url",
        "openapi-tags",
        "operation-tags",
        "operation-operationId",
        "operation-summary",
        "operation-description",
        "operation-default-response",
        "operation-success-response",
        "path-params",
        "path-keys-no-trailing-slash",
        "no-$ref-siblings",
        "typed-enum",
        "no-ambiguous-paths",
        "overlay-info-description",
        "overlay-action-description",
        "arazzo-workflow-description",
        "arazzo-step-operation",
    ] {
        assert!(codes.contains(&expected), "missing builtin rule {expected}");
    }
    // OAS-only doc: overlay/arazzo rules must not fire.
    assert!(
        run(&linter, OAS_DOC)
            .iter()
            .all(|h| !h.code.starts_with("overlay") && !h.code.starts_with("arazzo"))
    );
}
