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

#[test]
fn builtin_categories_split_design_from_document() {
    let linter = Linter::spectral_default();
    // The security pack is uniformly design-class: advisory findings about
    // the API's design that a truthful document still triggers.
    for code in [
        "security-server-https-only",
        "security-no-credentials-in-url",
        "security-no-delete-without-id",
        "security-rate-limit-documented",
    ] {
        assert_eq!(
            linter.category_of(code),
            crate::rule::Category::Design,
            "{code} must be design-class"
        );
    }
    // OAS pack rules are document-class: the fix is an edit to the
    // document.
    for code in ["no-$ref-siblings", "path-params", "operation-operationId"] {
        assert_eq!(
            linter.category_of(code),
            crate::rule::Category::Document,
            "{code} must be document-class"
        );
    }
    // Unknown codes (never produced by this linter) default to document.
    assert_eq!(
        linter.category_of("not-a-rule"),
        crate::rule::Category::Document
    );
}

#[test]
fn extends_security_and_override_inherits_category() {
    // The security pack is addressable, so a ruleset can include it and
    // tune individual rules — and a redefinition of a builtin inherits
    // its class rather than resetting it.
    let rs = doc(
        "extends: spectral:security\nrules:\n  security-no-delete-without-id:\n    description: custom\n    given: $.paths.*.delete\n    severity: hint\n    then:\n      function: falsy\n      functionOptions:\n        property: operationId\n",
    );
    let linter = Linter::from_ruleset(&rs).expect("valid ruleset");
    assert_eq!(
        linter.category_of("security-no-delete-without-id"),
        crate::rule::Category::Design,
        "an overridden builtin keeps its category"
    );
    // A doc rule compiled fresh stays document-class.
    let rs2 = doc(
        "rules:\n  custom-check:\n    description: d\n    given: $\n    severity: warn\n    then:\n      function: falsy\n      functionOptions:\n        property: nothing\n",
    );
    let fresh = Linter::from_ruleset(&rs2).expect("valid ruleset");
    assert_eq!(
        fresh.category_of("custom-check"),
        crate::rule::Category::Document
    );
}

#[test]
fn policy_severity_accepts_both_vocabularies() {
    use crate::rule::Severity;
    assert_eq!(Severity::from_policy("error"), Some(Severity::Error));
    assert_eq!(Severity::from_policy("warn"), Some(Severity::Warn));
    assert_eq!(Severity::from_policy("warning"), Some(Severity::Warn));
    assert_eq!(Severity::from_policy("info"), Some(Severity::Info));
    assert_eq!(Severity::from_policy("information"), Some(Severity::Info));
    assert_eq!(Severity::from_policy("hint"), Some(Severity::Hint));
    assert_eq!(Severity::from_policy("off"), Some(Severity::Off));
    assert_eq!(Severity::from_policy("loud"), None);
    // Whitespace and casing are tolerated; the value is committed policy.
    assert_eq!(Severity::from_policy(" Warning "), Some(Severity::Warn));
}

/// The builtin parameter rules match Parameter Objects — path-item,
/// operation and component lists — and never a schema property that
/// happens to be *named* `parameters`.
#[test]
fn parameter_rules_do_not_descend_into_schemas() {
    let linter = Linter::spectral_default();
    let target = "openapi: \"3.1.0\"\n\
                  info:\n  title: t\n  version: \"1\"\n\
                  paths:\n\
                  \x20 /items/{id}:\n\
                  \x20   parameters:\n\
                  \x20     - name: id\n\
                  \x20       in: path\n\
                  \x20       required: true\n\
                  \x20       schema:\n\
                  \x20         type: string\n\
                  \x20   get:\n\
                  \x20     parameters:\n\
                  \x20       - name: filter\n\
                  \x20         in: query\n\
                  \x20         description: filter the items\n\
                  \x20         schema:\n\
                  \x20           type: string\n\
                  \x20     responses:\n\
                  \x20       '200':\n\
                  \x20         description: ok\n\
                  \x20         content:\n\
                  \x20           application/json:\n\
                  \x20             schema:\n\
                  \x20               $ref: '#/components/schemas/Bag'\n\
                  components:\n\
                  \x20 schemas:\n\
                  \x20   Bag:\n\
                  \x20     type: object\n\
                  \x20     properties:\n\
                  \x20       parameters:\n\
                  \x20         description: a property named parameters\n\
                  \x20         type: string\n\
                  \x20 parameters:\n\
                  \x20   X-Token:\n\
                  \x20     name: X-Token\n\
                  \x20     in: header\n\
                  \x20     description: auth token\n\
                  \x20     schema:\n\
                  \x20       type: string\n";
    let hits = run(&linter, target);
    // The schema property named `parameters` is fully described: neither
    // parameter rule may fire on it.
    assert!(
        hits.iter()
            .all(|h| h.path != "/components/schemas/Bag/properties/parameters"),
        "a schema property named `parameters` is not a Parameter Object: {hits:?}"
    );
    // A real operation parameter missing both schema and content is still
    // an error.
    let bad = "openapi: \"3.1.0\"\n\
               info:\n  title: t\n  version: \"1\"\n\
               paths:\n\
               \x20 /a:\n\
               \x20   get:\n\
               \x20     parameters:\n\
               \x20       - name: q\n\
               \x20         in: query\n\
               \x20     responses:\n\
               \x20       '200':\n\
               \x20         description: ok\n";
    let hits = run(&linter, bad);
    assert!(
        hits.iter().any(|h| h.code == "parameter-schema-or-content"),
        "a real schema-less parameter is still an error: {hits:?}"
    );
}

/// Security schemes are referenced by NAME from `security` blocks, never
/// by pointer; the unused-component check honors that.
#[test]
fn unused_security_schemes_are_name_referenced() {
    let linter = Linter::spectral_default();
    let target = "openapi: \"3.0.0\"\n\
                  info:\n  title: t\n  version: \"1\"\n\
                  security:\n\
                  \x20 - token: []\n\
                  paths:\n\
                  \x20 /admin:\n\
                  \x20   get:\n\
                  \x20     security:\n\
                  \x20       - clientIdentifier: []\n\
                  \x20     responses:\n\
                  \x20       '200':\n\
                  \x20         description: ok\n\
                  \x20         content:\n\
                  \x20           application/json:\n\
                  \x20             schema:\n\
                  \x20               $ref: '#/components/schemas/Used'\n\
                  components:\n\
                  \x20 securitySchemes:\n\
                  \x20   token:\n\
                  \x20     type: apiKey\n\
                  \x20     name: X-Api-Token\n\
                  \x20     in: header\n\
                  \x20   clientIdentifier:\n\
                  \x20     type: apiKey\n\
                  \x20     name: X-Client-Identifier\n\
                  \x20     in: header\n\
                  \x20   ghost:\n\
                  \x20     type: apiKey\n\
                  \x20     name: X-Ghost\n\
                  \x20     in: header\n\
                  \x20 schemas:\n\
                  \x20   Used:\n\
                  \x20     type: object\n\
                  \x20   Orphan:\n\
                  \x20     type: object\n";
    let hits = run(&linter, target);
    // Two genuinely unused components: the `ghost` scheme no security
    // block names, and the `Orphan` schema no $ref points at. The
    // name-referenced schemes (`token` at the root, `clientIdentifier` at
    // the operation) and the $ref-referenced `Used` schema stay silent.
    let unused: Vec<_> = hits
        .iter()
        .filter(|h| h.code == "oas3-unused-component")
        .collect();
    assert_eq!(
        unused.len(),
        2,
        "only ghost and Orphan are unused: {hits:?}"
    );
    // (Findings push at the component key node; both survivors are
    // distinguishable by message context in the full battery.)
}

/// A `$ref` to a component parameter declares that parameter exactly as
/// its inline twin would — the check resolves local component pointers.
#[test]
fn path_params_resolve_component_refs() {
    let linter = Linter::spectral_default();
    let target = "openapi: \"3.0.0\"\n\
                  info:\n  title: t\n  version: \"1\"\n\
                  paths:\n\
                  \x20 /{transcodeType}/:/transcode/decision:\n\
                  \x20   get:\n\
                  \x20     parameters:\n\
                  \x20       - $ref: '#/components/parameters/transcodeType'\n\
                  \x20     responses:\n\
                  \x20       '200':\n\
                  \x20         description: ok\n\
                  \x20 /{transcodeType}/:/transcode/raw:\n\
                  \x20   get:\n\
                  \x20     parameters:\n\
                  \x20       - name: token\n\
                  \x20         in: header\n\
                  \x20         schema:\n\
                  \x20           type: string\n\
                  \x20     responses:\n\
                  \x20       '200':\n\
                  \x20         description: ok\n\
                  components:\n\
                  \x20 parameters:\n\
                  \x20   transcodeType:\n\
                  \x20     name: transcodeType\n\
                  \x20     in: path\n\
                  \x20     required: true\n\
                  \x20     schema:\n\
                  \x20       type: string\n";
    let hits = run(&linter, target);
    let findings: Vec<_> = hits.iter().filter(|h| h.code == "path-params").collect();
    // Only the second path — which declares a header instead of the path
    // template variable — is a finding; the $ref on the first satisfies
    // the declaration.
    assert_eq!(findings.len(), 1, "only the undeclared one flags: {hits:?}");
    assert!(
        findings[0].message.contains("raw"),
        "the finding names the undeclared path: {findings:?}"
    );
}

/// The root path `/` is the only legal trailing slash; everything else
/// with a trailing slash is a finding.
#[test]
fn trailing_slash_exempts_only_the_root_path() {
    let linter = Linter::spectral_default();
    let target = "openapi: \"3.0.0\"\n\
                  info:\n  title: t\n  version: \"1\"\n\
                  paths:\n\
                  \x20 /:\n\
                  \x20   get:\n\
                  \x20     responses:\n\
                  \x20       '200':\n\
                  \x20         description: ok\n\
                  \x20 /pets/:\n\
                  \x20   get:\n\
                  \x20     responses:\n\
                  \x20       '200':\n\
                  \x20         description: ok\n";
    let hits = run(&linter, target);
    let slashes: Vec<_> = hits
        .iter()
        .filter(|h| h.code == "path-keys-no-trailing-slash")
        .collect();
    assert_eq!(slashes.len(), 1, "only /pets/ is flagged: {hits:?}");
    assert_eq!(slashes[0].path, "/paths/~1pets~1");
}
