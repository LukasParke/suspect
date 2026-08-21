//! Unit tests for builtin `then` functions. Each function is exercised
//! through a minimal custom ruleset so options parsing and engine wiring are
//! covered together with the function behavior.

use super::{is_2xx_key, is_truthy, template_vars, Casing};
use crate::engine::Linter;
use crate::rule::Severity;
use suspect_low::LowDoc;
use suspect_source::{Source, Uri};

pub(crate) fn doc(yaml: &str) -> LowDoc {
    LowDoc::parse(
        Uri::parse("memory://test.yaml").expect("static test uri"),
        Source::from_vec(yaml.as_bytes().to_vec()),
    )
}

/// Owned projection of a finding so probes can outlive their document.
pub(crate) struct Hit {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub path: String,
}

pub(crate) fn run(linter: &Linter, target: &str) -> Vec<Hit> {
    let target_doc = doc(target);
    linter
        .run(&target_doc)
        .into_iter()
        .map(|f| Hit {
            code: f.code.to_string(),
            severity: f.severity,
            message: f.message,
            path: f.path.to_path(),
        })
        .collect()
}

/// Compiles a single-rule probe ruleset (`rule` is the full rule body —
/// including its `given:` — indented by four spaces under `rules:\n  probe:`)
/// and runs it over `target`.
pub(crate) fn probe(rule_body: &str, target: &str) -> Vec<Hit> {
    let ruleset_yaml = format!("rules:\n  probe:\n{rule_body}");
    let ruleset_doc = doc(&ruleset_yaml);
    let linter = Linter::from_ruleset(&ruleset_doc).expect("valid probe ruleset");
    run(&linter, target)
}

#[test]
fn casing_conventions() {
    assert!(Casing::Camel.matches("camelCase"));
    assert!(Casing::Camel.matches("a1b2"));
    assert!(!Casing::Camel.matches("CamelCase"));
    assert!(!Casing::Camel.matches("camel_case"));
    assert!(Casing::Pascal.matches("PascalCase"));
    assert!(!Casing::Pascal.matches("pascalCase"));
    assert!(Casing::Kebab.matches("kebab-case"));
    assert!(Casing::Kebab.matches("kebab-case-2"));
    assert!(!Casing::Kebab.matches("kebab--case"));
    assert!(!Casing::Kebab.matches("-kebab"));
    assert!(!Casing::Kebab.matches("Kebab-Case"));
    assert!(Casing::Snake.matches("snake_case"));
    assert!(!Casing::Snake.matches("snake__case"));
    assert!(!Casing::Snake.matches("_snake"));
    assert!(Casing::Macro.matches("MACRO_CASE"));
    assert!(Casing::Macro.matches("HTTP_200"));
    assert!(!Casing::Macro.matches("macro_case"));
    assert!(!Casing::Macro.matches("MACRO_"));
    for casing in [Casing::Camel, Casing::Pascal, Casing::Kebab, Casing::Snake, Casing::Macro] {
        assert!(!casing.matches(""), "empty string matches no casing");
    }
}

#[test]
fn template_var_extraction() {
    assert_eq!(template_vars("/pets/{id}/toys/{toyId}"), vec!["id", "toyId"]);
    assert_eq!(template_vars("/pets"), Vec::<String>::new());
    assert_eq!(template_vars("/pets/{}"), Vec::<String>::new());
    assert_eq!(template_vars("/files/{path}"), vec!["path"]);
}

#[test]
fn truthiness() {
    let d = doc("a: null\nb: false\nc: \"\"\nd: 0\ne: []\nf: {}\ng: text\n");
    let root = d.root();
    assert!(!is_truthy(&root.get("a").unwrap()));
    assert!(!is_truthy(&root.get("b").unwrap()));
    assert!(!is_truthy(&root.get("c").unwrap()));
    assert!(is_truthy(&root.get("d").unwrap()));
    assert!(is_truthy(&root.get("e").unwrap()));
    assert!(is_truthy(&root.get("f").unwrap()));
    assert!(is_truthy(&root.get("g").unwrap()));
}

#[test]
fn two_xx_keys() {
    assert!(is_2xx_key("200"));
    assert!(is_2xx_key("204"));
    assert!(is_2xx_key("2XX"));
    assert!(is_2xx_key("2xx"));
    assert!(!is_2xx_key("default"));
    assert!(!is_2xx_key("404"));
    assert!(!is_2xx_key("20"));
    assert!(!is_2xx_key("2000"));
}

#[test]
fn truthy_and_falsy_functions() {
    let hits = probe("    given: $.value\n    then:\n      function: truthy\n", "value: null\n");
    assert_eq!(hits.len(), 1, "null must fail truthy");
    let clean = probe("    given: $.value\n    then:\n      function: truthy\n", "value: present\n");
    assert!(clean.is_empty());
    let hits = probe("    given: $.value\n    then:\n      function: falsy\n", "value: present\n");
    assert_eq!(hits.len(), 1, "truthy value must fail falsy");
}

#[test]
fn defined_and_undefined_functions() {
    let missing = probe(
        "    given: $.info\n    then:\n      function: defined\n      functionOptions:\n        property: contact\n",
        "info:\n  title: t\n",
    );
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].path, "/info");
    let present = probe(
        "    given: $.info\n    then:\n      function: defined\n      functionOptions:\n        property: contact\n",
        "info:\n  contact:\n    name: n\n",
    );
    assert!(present.is_empty());
    let hits = probe(
        "    given: $.info\n    then:\n      function: undefined\n      functionOptions:\n        property: deprecated\n",
        "info:\n  deprecated: true\n",
    );
    assert_eq!(hits.len(), 1);
}

#[test]
fn pattern_function() {
    let rule = "    given: $.homepage\n    then:\n      function: pattern\n      functionOptions:\n        match: '^https?://'\n";
    let hits = probe(rule, "homepage: ftp://example.com\n");
    assert_eq!(hits.len(), 1);
    let ok = probe(rule, "homepage: https://example.com\n");
    assert!(ok.is_empty());
    // Non-string scalars are skipped, not reported.
    let numeric = probe(rule, "homepage: 42\n");
    assert!(numeric.is_empty());
}

#[test]
fn casing_function() {
    let hits = probe(
        "    given: $.name\n    then:\n      function: casing\n      functionOptions:\n        casing: camel\n",
        "name: snake_case\n",
    );
    assert_eq!(hits.len(), 1);
    let ok = probe(
        "    given: $.name\n    then:\n      function: casing\n      functionOptions:\n        casing: camel\n",
        "name: camelCase\n",
    );
    assert!(ok.is_empty());
}

#[test]
fn length_function() {
    let too_short = probe(
        "    given: $.word\n    then:\n      function: length\n      functionOptions:\n        length:\n          min: 3\n",
        "word: ab\n",
    );
    assert_eq!(too_short.len(), 1);
    let array_len = probe(
        "    given: $.list\n    then:\n      function: length\n      functionOptions:\n        length:\n          max: 1\n",
        "list:\n  - a\n  - b\n",
    );
    assert_eq!(array_len.len(), 1);
    let ok = probe(
        "    given: $.word\n    then:\n      function: length\n      functionOptions:\n        length:\n          min: 1\n          max: 5\n",
        "word: abcde\n",
    );
    assert!(ok.is_empty());
}

#[test]
fn enumeration_function() {
    let rule =
        "    given: $.color\n    then:\n      function: enumeration\n      functionOptions:\n        values:\n          - red\n          - green\n";
    let hits = probe(rule, "color: blue\n");
    assert_eq!(hits.len(), 1);
    let ok_str = probe(rule, "color: green\n");
    assert!(ok_str.is_empty());
    let numbers =
        "    given: $.code\n    then:\n      function: enumeration\n      functionOptions:\n        values:\n          - 1\n          - 2\n";
    let ok_num = probe(numbers, "code: 2\n");
    assert!(ok_num.is_empty());
    let hit_num = probe(numbers, "code: 3\n");
    assert_eq!(hit_num.len(), 1);
}

#[test]
fn alphabetical_function() {
    let unsorted = probe("    given: $.map\n    then:\n      function: alphabetical\n", "map:\n  zebra: 1\n  apple: 2\n");
    assert_eq!(unsorted.len(), 1);
    assert_eq!(unsorted[0].path, "/map");
    let sorted = probe("    given: $.map\n    then:\n      function: alphabetical\n", "map:\n  apple: 1\n  zebra: 2\n");
    assert!(sorted.is_empty());
}

#[test]
fn xor_function() {
    let xor = "    given: $.obj\n    then:\n      function: xor\n      functionOptions:\n        properties:\n          - a\n          - b\n";
    let both = probe(xor, "obj:\n  a: 1\n  b: 2\n");
    assert_eq!(both.len(), 1);
    let neither = probe(xor, "obj:\n  c: 1\n");
    assert_eq!(neither.len(), 1);
    let exactly_one = probe(xor, "obj:\n  a: 1\n");
    assert!(exactly_one.is_empty());
}
